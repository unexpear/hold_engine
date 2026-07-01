//! Editor-side state for the M9 frame inspector.
//!
//! Holds the most recent per-pass thumbnail capture produced by
//! [`crate::renderer::FrameGraph::execute_with_capture`], plus the
//! enabled/disabled toggle and a monotonic generation counter used by
//! the protocol event to coalesce stale bridge updates.
//!
//! Design decisions (ADR 0002 Q8 + ADR 0003 Pillar 4):
//!
//! - **Always-latest, no history.** Every capture overwrites the
//!   previous one. Capture goes through `glReadPixels`, which stalls
//!   the GPU pipeline; keeping a history would multiply the cost with
//!   no editor-visible benefit. If the user wants to compare two
//!   frames, they toggle F10 off, run their action, toggle F10 back on.
//! - **Generation counter monotonic across enable/disable cycles.** The
//!   bridge codec (and future coalescing on the browser side) treats
//!   this as the "cache key" for the current capture. Never resets.
//! - **Pixel data owned inside the state.** The renderer surrenders the
//!   `Vec<PassThumbnail>` at capture time; the state moves it in and
//!   later hands out an immutable slice. No further copies until the
//!   editor's rendering path decides to upload as a texture (future
//!   work, tracked with the M9 textured-quad follow-up).
//!
//! See:
//! - `docs/adr/0002-material-pipeline.md` Q8 — frame graph as a typed
//!   node graph with per-pass thumbnails as a first-class inspector
//!   surface.
//! - `docs/adr/0003-differentiation-and-competitive-posture.md` Pillar 4
//!   — determinism and debuggability.

use crate::renderer::PassThumbnail;

/// Editor state for the M9 frame inspector. Lives on
/// [`super::EditorRuntimeHost`]; drivers (the editor shell) push
/// captures in via [`Self::push_capture`], and the native UI (or
/// protocol codec) reads them out via [`Self::latest`] and
/// [`Self::generation`].
#[derive(Debug, Clone, Default)]
pub struct FrameInspectorState {
    enabled: bool,
    latest: Option<Vec<PassThumbnail>>,
    generation: u64,
}

impl FrameInspectorState {
    /// Fresh state: disabled, no capture yet, generation 0.
    ///
    /// Kept alongside `Default` because "disabled + empty" is the
    /// meaningful initial state; a `new()` constructor makes it clear
    /// at call sites that no I/O runs during construction.
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether the shell should call
    /// [`crate::renderer::FrameGraph::execute_with_capture`] this
    /// frame (`true`) or the cheaper
    /// [`crate::renderer::FrameGraph::execute`] path (`false`).
    ///
    /// Read by the editor shell every frame; setting is the F10 toggle.
    /// When flipped `false`, [`Self::latest`] is *not* cleared — the
    /// previous capture stays visible so the user can inspect it after
    /// disabling capture. Toggling back on overwrites the first frame.
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Set the enabled flag. Does not touch the latest capture or the
    /// generation counter.
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// Toggle the enabled flag, returning the new state. Convenience
    /// for the shell's F10 handler.
    pub fn toggle_enabled(&mut self) -> bool {
        self.enabled = !self.enabled;
        self.enabled
    }

    /// Replace the latest capture with a fresh one. Bumps the
    /// generation counter and returns the new generation number so
    /// callers can plumb it into a protocol event without a second
    /// borrow.
    ///
    /// An empty vec is a legitimate capture (the frame graph had no
    /// passes); the panel will render an empty strip rather than the
    /// "waiting for capture" placeholder.
    pub fn push_capture(&mut self, thumbnails: Vec<PassThumbnail>) -> u64 {
        self.generation = self.generation.saturating_add(1);
        self.latest = Some(thumbnails);
        self.generation
    }

    /// Discard the latest capture without changing the enabled flag or
    /// the generation counter. Used when scene state changes in a way
    /// that would make the last capture confusing (world rebuild,
    /// scene reload).
    pub fn clear_latest(&mut self) {
        self.latest = None;
    }

    /// The most recent capture, if any. Slice borrows are tied to the
    /// state's lifetime.
    pub fn latest(&self) -> Option<&[PassThumbnail]> {
        self.latest.as_deref()
    }

    /// Monotonic capture counter, incremented every time
    /// [`Self::push_capture`] runs. Read by the protocol codec so
    /// bridge clients can coalesce stale updates.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Number of passes in the latest capture, or zero if no capture
    /// has landed yet. Convenience for the panel's header text.
    pub fn pass_count(&self) -> usize {
        self.latest.as_ref().map(|v| v.len()).unwrap_or(0)
    }
}

/// Compact per-pass metadata carried in the [`super::EditorEvent::FrameCaptured`]
/// protocol event. Mirrors [`PassThumbnail`] minus the pixel data —
/// bridge protocols are line-based and cannot carry
/// hundreds-of-kilobytes of RGBA per capture. Bridge clients that want
/// pixels fetch them through a future dedicated channel (parallel to
/// the browser backend's `/api/project-file`, tracked with the M9
/// textured-quad follow-up).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameInspectorPass {
    /// Pass index within the compiled frame graph's execution order.
    /// Stable across frames while the graph topology is unchanged.
    pub index: u32,
    pub name: String,
    pub width: u32,
    pub height: u32,
}

impl FrameInspectorPass {
    /// Extract the protocol-safe metadata subset from a
    /// [`PassThumbnail`], dropping the pixel data.
    pub fn from_thumbnail(t: &PassThumbnail) -> Self {
        Self {
            index: t.pass_index as u32,
            name: t.pass_name.clone(),
            width: t.width,
            height: t.height,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn thumb(index: usize, name: &str, w: u32, h: u32) -> PassThumbnail {
        PassThumbnail {
            pass_index: index,
            pass_name: name.into(),
            width: w,
            height: h,
            data: vec![0u8; (w * h * 4) as usize],
        }
    }

    /// A newly constructed state has no capture, is disabled, and its
    /// generation counter is zero — the load-bearing "quiet default"
    /// property so the editor shell doesn't accidentally render
    /// inspector chrome before the user opts in.
    #[test]
    fn fresh_state_is_quiet_and_disabled() {
        let state = FrameInspectorState::new();
        assert!(!state.is_enabled());
        assert_eq!(state.latest(), None);
        assert_eq!(state.generation(), 0);
        assert_eq!(state.pass_count(), 0);
    }

    /// Pushing a capture must bump the generation counter and expose
    /// the passes exactly once. The counter is the "cache key" for
    /// bridge clients — if it doesn't move, coalescing breaks.
    #[test]
    fn push_capture_bumps_generation_and_replaces_latest() {
        let mut state = FrameInspectorState::new();
        let gen1 = state.push_capture(vec![thumb(0, "Clear", 128, 72), thumb(1, "Scene", 128, 72)]);
        assert_eq!(gen1, 1);
        assert_eq!(state.generation(), 1);
        assert_eq!(state.pass_count(), 2);
        assert_eq!(state.latest().unwrap()[1].pass_name, "Scene");

        // Second capture overwrites, not appends.
        let gen2 = state.push_capture(vec![thumb(0, "Only", 128, 72)]);
        assert_eq!(gen2, 2);
        assert_eq!(state.pass_count(), 1);
        assert_eq!(state.latest().unwrap()[0].pass_name, "Only");
    }

    /// Toggle flips the enabled flag and returns the new state. The
    /// shell's F10 handler reads the return value directly for its
    /// action-log message.
    #[test]
    fn toggle_enabled_flips_and_reports_new_state() {
        let mut state = FrameInspectorState::new();
        assert!(state.toggle_enabled());
        assert!(state.is_enabled());
        assert!(!state.toggle_enabled());
        assert!(!state.is_enabled());
    }

    /// Disabling capture must leave the latest capture intact so the
    /// user can inspect the last frame after turning inspector off.
    /// Only [`clear_latest`] should discard it explicitly.
    #[test]
    fn disable_preserves_latest_capture() {
        let mut state = FrameInspectorState::new();
        state.set_enabled(true);
        let _ = state.push_capture(vec![thumb(0, "Clear", 128, 72)]);
        state.set_enabled(false);
        assert!(state.latest().is_some());
        state.clear_latest();
        assert!(state.latest().is_none());
        // clear_latest must NOT touch generation — it's a display
        // reset, not a new capture event.
        assert_eq!(state.generation(), 1);
    }

    /// Extracting protocol metadata drops the pixel data but preserves
    /// index/name/dims. The pixel drop is the reason the protocol event
    /// exists — line-based bridges can't carry RGBA blobs.
    #[test]
    fn frame_inspector_pass_from_thumbnail_drops_pixels_keeps_metadata() {
        let t = thumb(3, "DebugLines", 128, 72);
        let p = FrameInspectorPass::from_thumbnail(&t);
        assert_eq!(p.index, 3);
        assert_eq!(p.name, "DebugLines");
        assert_eq!(p.width, 128);
        assert_eq!(p.height, 72);
    }
}
