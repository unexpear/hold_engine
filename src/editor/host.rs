//! Headless editor/runtime host with the authoritative command/event surface.
//!
//! `EditorRuntimeHost` owns:
//!  - an [`EditorSceneSession`] (scene document, play-state, IO),
//!  - a live [`World`] (runtime ECS state),
//!  - an [`EditorCamera`] (free-fly editor camera, separate from the scene),
//!  - inspector cursor state (entity / component / field indices + axis),
//!  - persistent dirty tracking (scene modified since last save).
//!
//! The public command surface is [`EditorRuntimeHost::execute`].  All other
//! methods remain available for headless tests and direct API consumers.

use std::path::{Path, PathBuf};

use crate::ecs::components::{ActiveCamera, Camera};
use crate::ecs::{Entity, World};
use crate::input::{vk, InputState, MouseButton};
use crate::math::{Mat4, Vec3};
use crate::runtime_slice::{
    PatrolRouteDef, TerrainPaintLayer, VegetationPaintStroke, VehicleSpawnerDef,
};

use super::{
    command::{EditorAxis, EditorCommand, EditorCommandError, EditorEvent, EditorInspectorCursor},
    protocol::{decode_command, encode_event, EditorProtocolError},
    EditorIoError, EditorPlayState, EditorSceneSession, EditorSessionError, InspectorValue, Project,
    SceneDocument,
};

// ── EditorCamera ──────────────────────────────────────────────────────────────

/// A host-owned free-fly perspective camera for the `Editing` and `Paused`
/// play states.
///
/// Not stored in the [`SceneDocument`] — it is purely a shell-side viewport
/// aid.  During `Playing`, the shell switches to the runtime `ActiveCamera`
/// instead.
#[derive(Debug, Clone)]
pub struct EditorCamera {
    pub position: Vec3,
    /// Horizontal rotation in radians (0 = looking along −Z).
    pub yaw: f32,
    /// Vertical rotation in radians, clamped to ±89°.
    pub pitch: f32,
}

impl EditorCamera {
    pub fn new() -> Self {
        Self {
            position: Vec3::new(0.0, 5.0, 12.0),
            yaw: 0.0,
            pitch: -0.3,
        }
    }

    /// World-space forward direction (unit length).
    pub fn forward(&self) -> Vec3 {
        Vec3::new(
            self.yaw.sin() * self.pitch.cos(),
            self.pitch.sin(),
            -(self.yaw.cos() * self.pitch.cos()),
        )
    }

    /// View matrix (world → camera space).
    pub fn view_matrix(&self) -> Mat4 {
        let fwd = self.forward();
        let right = fwd.cross(Vec3::Y).normalize();
        let up = right.cross(fwd).normalize();
        Mat4::look_at(self.position, self.position + fwd, up)
    }

    /// Combined view-projection matrix for the editor camera.
    pub fn view_proj(&self, fov_deg: f32, aspect: f32, near: f32, far: f32) -> Mat4 {
        let proj = Mat4::perspective_gl(fov_deg.to_radians(), aspect, near, far);
        proj * self.view_matrix()
    }

    /// Update the camera from input.
    ///
    /// Free-fly controls are only active while the **right mouse button** is
    /// held.  When it is released the camera stays still (no accidental drift).
    ///
    /// Key bindings: `WASD` — horizontal strafe/forward, `Q`/`E` — down/up.
    /// `Shift` multiplies speed by 10.
    pub fn update(&mut self, input: &InputState, dt: f32) {
        if !input.mouse_button(MouseButton::Right) {
            return;
        }

        // Mouse look.
        let (dx, dy) = input.mouse_delta();
        self.yaw += dx as f32 * 0.004;
        self.pitch -= dy as f32 * 0.004;
        self.pitch = self.pitch.clamp(-1.55, 1.55); // ±~89°

        // Movement.
        let speed = if input.key(vk::SHIFT) { 40.0 } else { 6.0 };
        let fwd = self.forward();
        let right = fwd.cross(Vec3::Y).normalize();
        let up = Vec3::Y;

        let mut dir = Vec3::ZERO;
        if input.key(vk::W) {
            dir = dir + fwd;
        }
        if input.key(vk::S) {
            dir = dir - fwd;
        }
        if input.key(vk::D) {
            dir = dir + right;
        }
        if input.key(vk::A) {
            dir = dir - right;
        }
        if input.key(vk::E) {
            dir = dir + up;
        }
        if input.key(vk::Q) {
            dir = dir - up;
        }

        if dir.length_sq() > 1e-8 {
            self.position = self.position + dir.normalize() * (speed * dt);
        }
    }
}

impl Default for EditorCamera {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone)]
struct EditorHistorySnapshot {
    scene: SceneDocument,
    cursor: Option<EditorInspectorCursor>,
    persistent_dirty: bool,
}

// ── EditorRuntimeHost ─────────────────────────────────────────────────────────

/// Headless editor/runtime host for the authoritative scene workflow.
///
/// A native UI shell (or test harness) sits on top of this without
/// re-implementing scene loading, inspector mutation, play-mode transitions,
/// or world synchronization.
pub struct EditorRuntimeHost {
    session: EditorSceneSession,
    world: World,

    // ── Inspector cursor ──────────────────────────────────────────────────
    /// Index into `session.scene().entities` of the currently selected entity.
    /// `None` when the scene is empty.
    cursor_entity_idx: Option<usize>,
    /// Index into the selected entity's `components` slice.
    cursor_component_idx: usize,
    /// Index into the selected component's `fields` slice.
    cursor_field_idx: usize,
    /// Active axis for Vec3 field edits.
    cursor_axis: Option<EditorAxis>,

    // ── Dirty tracking ────────────────────────────────────────────────────
    /// `true` when the scene document has unsaved changes (Editing only).
    persistent_dirty: bool,
    /// Monotonic counter bumped on every authored edit. The autosave
    /// scheduler compares it frame to frame to tell "still typing" (restart
    /// the idle debounce) from "paused" (fire the autosave). A bool dirty
    /// flag cannot express that difference — it is already `true` in both
    /// cases. See [`super::AutosaveScheduler`] (M11).
    edit_revision: u64,
    /// Snapshot of `persistent_dirty` taken on `EnterPlayMode`; restored by
    /// `StopPlayMode` so a play cycle does not clear an unsaved modification.
    pre_play_persistent_dirty: bool,
    /// `true` when the live world is stale and requires a rebuild to match the
    /// current scene document.
    world_requires_rebuild: bool,
    /// Authored edit history owned by the authoritative host.
    undo_stack: Vec<EditorHistorySnapshot>,
    /// Redo history paired with [`Self::undo_stack`].
    redo_stack: Vec<EditorHistorySnapshot>,

    // ── Editor camera ─────────────────────────────────────────────────────
    pub editor_camera: EditorCamera,

    // ── Hot-reload status (M7.4.2) ────────────────────────────────────────
    /// Ring buffer of recent hot-reload outcomes the native UI
    /// renders in the Hot-Reload Status pane. Populated by the
    /// game/editor reload pipeline via [`Self::push_hot_reload_event`]
    /// and [`Self::push_hot_reload_failure`]. See ADR 0004 Q9
    /// (M7.4.2) and Q13 for how this surface receives events.
    hot_reload_status: super::hot_reload_status::HotReloadStatus,

    // ── Frame inspector (ADR 0002 Q8 / ADR 0003 Pillar 4 — M9) ───────────
    /// Latest per-pass thumbnail capture from the frame graph, plus
    /// the F10-driven enable flag and a monotonic generation counter.
    /// The native UI reads this to render the Frame Inspector panel;
    /// the editor bridge codec drops the pixels and ships the
    /// metadata as [`super::EditorEvent::FrameCaptured`].
    frame_inspector: super::FrameInspectorState,

    // ── Replay state machine (ADR 0005 — M8.0 live editor wire-up) ───────
    /// Editor-side record/replay finite state machine. The editor
    /// shell drives this via F-key handlers in `App::update`;
    /// per-frame tick methods on [`super::EditorReplayState`]
    /// capture or drive input depending on the active mode.
    /// See `docs/adr/0005-determinism-architecture.md` Q5–Q7.
    replay: super::EditorReplayState,
}

impl EditorRuntimeHost {
    // ── Construction ─────────────────────────────────────────────────────

    pub fn new(session: EditorSceneSession) -> Result<Self, EditorSessionError> {
        let mut host = Self {
            session,
            world: World::new(),
            cursor_entity_idx: None,
            cursor_component_idx: 0,
            cursor_field_idx: 0,
            cursor_axis: None,
            persistent_dirty: false,
            edit_revision: 0,
            pre_play_persistent_dirty: false,
            world_requires_rebuild: false,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            editor_camera: EditorCamera::new(),
            hot_reload_status: super::hot_reload_status::HotReloadStatus::default(),
            frame_inspector: super::FrameInspectorState::new(),
            replay: super::EditorReplayState::new(),
        };
        host.rebuild_world()?;
        // Auto-select the first entity after loading.
        host.init_cursor_to_first();
        Ok(host)
    }

    // ── Hot-reload status (M7.4.2) ──────────────────────────────────────

    /// Push a successful hot-reload event onto the editor's
    /// status ring. Called by the game's per-frame reload code
    /// after each `AssetInvalidated` event. The native UI panel
    /// renders the most recent entries.
    ///
    /// `kind` is the asset-kind label (`"Texture"`,
    /// `"Material"`, `"Shader"`, ...) — matched against the
    /// `AssetKind::as_str()` representation that the editor
    /// protocol uses (ADR 0004 Q13). Stored as a string for
    /// forward-compatibility with future asset kinds the editor
    /// doesn't statically know about.
    pub fn push_hot_reload_event(
        &mut self,
        kind: impl Into<String>,
        handle_index: u32,
        logical_name: impl Into<String>,
    ) {
        self.hot_reload_status
            .push(super::hot_reload_status::HotReloadOutcome::Applied {
                kind: kind.into(),
                handle_index,
                logical_name: logical_name.into(),
            });
    }

    /// Push a hot-reload failure (compile error) onto the
    /// editor's status ring. The previous asset stays live in
    /// the AssetStore; this surface only records the error so
    /// the developer sees it in the panel.
    pub fn push_hot_reload_failure(
        &mut self,
        kind: impl Into<String>,
        logical_name: impl Into<String>,
        error: impl Into<String>,
    ) {
        self.hot_reload_status
            .push(super::hot_reload_status::HotReloadOutcome::Failed {
                kind: kind.into(),
                logical_name: logical_name.into(),
                error: error.into(),
            });
    }

    /// Read-only accessor for the hot-reload status ring. The
    /// native UI's `draw_status` function uses this to populate
    /// the Hot-Reload Status panel.
    pub fn hot_reload_status(&self) -> &super::hot_reload_status::HotReloadStatus {
        &self.hot_reload_status
    }

    /// Mutable accessor — useful for tests and for the editor
    /// command path that clears the status when a new project
    /// opens.
    pub fn hot_reload_status_mut(&mut self) -> &mut super::hot_reload_status::HotReloadStatus {
        &mut self.hot_reload_status
    }

    // ── Frame inspector (M9) ──────────────────────────────────────────────

    /// Read-only accessor for the frame inspector state. Read by the
    /// native UI's `draw_frame_inspector_panel` and by the editor
    /// bridge codec that emits [`super::EditorEvent::FrameCaptured`]
    /// when a capture lands.
    pub fn frame_inspector(&self) -> &super::FrameInspectorState {
        &self.frame_inspector
    }

    /// Mutable accessor for the frame inspector state. The editor
    /// shell reads/writes this every frame: F10 flips
    /// [`super::FrameInspectorState::toggle_enabled`], and when
    /// enabled the shell pushes each capture via
    /// [`super::FrameInspectorState::push_capture`].
    pub fn frame_inspector_mut(&mut self) -> &mut super::FrameInspectorState {
        &mut self.frame_inspector
    }

    // ── Replay state machine (ADR 0005 — M8.0 live editor wire-up) ───────

    /// Read-only accessor for the editor's record/replay finite
    /// state machine. The editor's status panel reads this to
    /// render "REC" / "REPLAY" indicators alongside frame counts;
    /// the protocol codec emits the mode for line-based bridges.
    ///
    /// See `docs/adr/0005-determinism-architecture.md` Q5–Q7.
    pub fn replay(&self) -> &super::EditorReplayState {
        &self.replay
    }

    /// Mutable accessor for the replay state machine. Called by
    /// the editor shell's per-frame App::update to drive the
    /// per-frame `tick_recording` / `tick_replay` methods, plus
    /// state transitions on F-key presses.
    pub fn replay_mut(&mut self) -> &mut super::EditorReplayState {
        &mut self.replay
    }

    /// Drain a [`DispatchSummary`](crate::hot_reload::DispatchSummary)
    /// from the M7.3 reload pipeline into this host: routes
    /// asset reloads to the status panel, runs scene reloads
    /// against the host's live World via the entity-preserving
    /// diff path, and reports per-category counts so callers can
    /// log telemetry or surface aggregate metrics.
    ///
    /// This is the single integration seam between the dispatcher
    /// and the editor's user-visible state. The intended call site
    /// is the engine's reload-dispatch hook
    /// ([`crate::engine::Engine::on_reload_dispatch`]):
    ///
    /// ```ignore
    /// engine.on_reload_dispatch(move || {
    ///     let summary = dispatcher.dispatch(&mut assets);
    ///     host.apply_dispatch_summary(summary);
    /// });
    /// ```
    ///
    /// ## Why this lives on the host (not the dispatcher)
    ///
    /// The dispatcher is asset-pipeline-only by design — it owns
    /// the compile queue receiver and the bindings map, nothing
    /// else. Scene reloads need access to a live `World` and an
    /// `EditorSceneSession`; both live on the host. Putting the
    /// routing here keeps the dispatcher single-responsibility
    /// and avoids forcing a `World` reference into the
    /// hot-reload module.
    ///
    /// ## Failure surfaces
    ///
    /// Compile errors land in the status panel as `Failed` rows.
    /// Scene reloads that fall back to a full rebuild
    /// ([`crate::editor::SceneReloadOutcome::FullRebuild`]) are
    /// recorded as a successful event with a "rebuilt" label —
    /// the rebuild succeeded, just not the diff path. A scene
    /// reload that errors entirely (rare; only on world-rebuild
    /// failure) lands as a `Failed` row with the error message.
    ///
    /// Returns a [`DispatchApplyReport`] with per-category
    /// counts for telemetry.
    ///
    /// See ADR 0004 Q9 (M7.4.2 status panel) + Q14 (scene reload).
    pub fn apply_dispatch_summary(
        &mut self,
        summary: crate::hot_reload::DispatchSummary,
    ) -> DispatchApplyReport {
        let mut report = DispatchApplyReport::default();

        // 1. Asset reloads → status panel as Applied rows. The
        //    handle_index lets the status panel disambiguate
        //    multiple reloads of the same logical name across a
        //    long session.
        for event in &summary.applied {
            self.push_hot_reload_event(
                event.kind.as_str(),
                event.handle_index,
                event.logical_name.clone(),
            );
            report.assets_applied += 1;
        }

        // 2. Scene reloads → drive the entity-preserving diff
        //    against the live World, then route the outcome to
        //    the status panel. Each result moves into
        //    reload_scene_document so we drain by value.
        for scene in summary.scenes_reloaded {
            let logical_name = scene.logical_name.clone();
            match self
                .session
                .reload_scene_document(scene.document, &mut self.world)
            {
                Ok(outcome) => {
                    let label = match &outcome {
                        crate::editor::SceneReloadOutcome::NoChange => "Scene (no-change)",
                        crate::editor::SceneReloadOutcome::Diffed { .. } => "Scene",
                        crate::editor::SceneReloadOutcome::FullRebuild { .. } => "Scene (rebuilt)",
                    };
                    // Scenes have no AssetStore handle; use 0 as
                    // the slot. The logical_name carries the real
                    // identity in the status panel.
                    self.push_hot_reload_event(label, 0, logical_name.clone());
                    match outcome {
                        crate::editor::SceneReloadOutcome::NoChange => {
                            report.scenes_no_change += 1;
                        }
                        crate::editor::SceneReloadOutcome::Diffed {
                            added,
                            removed,
                            patched,
                        } => {
                            report.scenes_diffed += 1;
                            report.scene_entities_added += added;
                            report.scene_entities_removed += removed;
                            report.scene_entities_patched += patched;
                        }
                        crate::editor::SceneReloadOutcome::FullRebuild { .. } => {
                            report.scenes_rebuilt += 1;
                        }
                    }
                }
                Err(err) => {
                    // Reload failed before producing an outcome
                    // (e.g. the rebuild fallback's apply step
                    // returned an error). Record as a Failed row
                    // with the error message; the previous World
                    // state stays as-is on the safety-net path.
                    self.push_hot_reload_failure("Scene", logical_name, err.to_string());
                    report.scenes_failed += 1;
                }
            }
        }

        // 3. Compile errors → status panel as Failed rows. The
        //    previous asset stays live in `AssetStore`; this
        //    surface only records the diagnostic.
        for err in &summary.errors {
            self.push_hot_reload_failure(
                compiled_asset_kind_label(err.kind),
                err.logical_name.clone(),
                err.error.clone(),
            );
            report.errors_recorded += 1;
        }

        // `summary.stale` is telemetry only — no status-panel
        // surface (stale results are a normal consequence of
        // the last-wins discipline; the editor user shouldn't
        // see them as separate events).
        report.stale_dropped = summary.stale.len();

        report
    }

    pub fn from_scene(scene: SceneDocument) -> Result<Self, EditorSessionError> {
        Self::new(EditorSceneSession::new(scene))
    }

    pub fn from_scene_with_project(
        scene: SceneDocument,
        project: Project,
    ) -> Result<Self, EditorSessionError> {
        Self::new(EditorSceneSession::new_with_project(scene, project))
    }

    pub fn open_hsc<P: AsRef<Path>>(path: P) -> Result<Self, EditorSessionError> {
        Self::new(EditorSceneSession::open_hsc(path)?)
    }

    pub fn open_hsc_with_project<P: AsRef<Path>>(
        path: P,
        project: Project,
    ) -> Result<Self, EditorSessionError> {
        Self::new(EditorSceneSession::open_hsc_with_project(path, project)?)
    }

    // ── Accessors ─────────────────────────────────────────────────────────

    pub fn session(&self) -> &EditorSceneSession {
        &self.session
    }

    pub fn project(&self) -> Option<&Project> {
        self.session.project()
    }

    pub fn session_mut(&mut self) -> &mut EditorSceneSession {
        &mut self.session
    }

    pub fn world(&self) -> &World {
        &self.world
    }

    pub fn world_mut(&mut self) -> &mut World {
        &mut self.world
    }

    pub fn play_state(&self) -> EditorPlayState {
        self.session.play_state()
    }

    pub fn selected_entity_id(&self) -> Option<u64> {
        self.session.selected_entity_id()
    }

    /// Whether the scene document has unsaved changes.
    ///
    /// Always `false` during play mode (transient play edits are not
    /// persistent).
    pub fn is_dirty(&self) -> bool {
        self.persistent_dirty && self.play_state() == EditorPlayState::Editing
    }

    /// Record an authored edit: mark the document dirty and bump the revision
    /// the autosave scheduler watches.
    fn mark_edited(&mut self) {
        self.persistent_dirty = true;
        self.edit_revision = self.edit_revision.wrapping_add(1);
    }

    /// Monotonic edit counter. Read by [`super::AutosaveScheduler::tick`] to
    /// restart the idle debounce whenever the document changes.
    pub fn edit_revision(&self) -> u64 {
        self.edit_revision
    }

    /// Whether there is unsaved *authored* work — the raw persistent-dirty
    /// flag, NOT masked by play state (unlike [`Self::is_dirty`]). The autosave
    /// scheduler needs the unmasked value so that an edit made just before
    /// entering play mode is still known to be pending after play ends.
    pub fn has_unsaved_authored_work(&self) -> bool {
        self.persistent_dirty
    }

    // ── Autosave + crash recovery (M11) ───────────────────────────────────

    /// Where this scene's recovery file lives, or `None` for an untitled
    /// scene (no source path ⇒ no stable identity to recover into).
    pub fn recovery_path(&self) -> Option<PathBuf> {
        let source = self.session.source_path()?;
        super::recovery_path_for_scene(source)
    }

    /// Write the current scene document to its recovery file, atomically.
    ///
    /// This is a **safety net, not a save**. Three properties are load-bearing:
    ///
    /// 1. It never writes the author's source file — only the recovery
    ///    sidecar. Overwriting the source with in-progress state would destroy
    ///    the last clean save.
    /// 2. It never clears `persistent_dirty`. The work is still unsaved
    ///    relative to the source; the user must still press Save.
    /// 3. It only runs while `Editing`. During play/replay `session.scene()`
    ///    is the live *mutated* copy, and checkpointing that would capture
    ///    transient simulation state as if it were authored work.
    ///
    /// Uses the full-fidelity `.hscene` format rather than the `.hsc` bridge,
    /// which drops editor-only components — recovery must not lose more than
    /// the crash did.
    ///
    /// Returns `Ok(None)` when there is nothing to do (untitled, or not
    /// Editing).
    pub fn autosave_to_recovery(&self) -> Result<Option<PathBuf>, EditorIoError> {
        if self.play_state() != EditorPlayState::Editing {
            return Ok(None);
        }
        let Some(path) = self.recovery_path() else {
            return Ok(None);
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(EditorIoError::Io)?;
        }
        super::save_scene_doc(&path, self.session.scene())?;
        Ok(Some(path))
    }

    /// Delete the recovery file. Called after an explicit save durably lands:
    /// disk now matches memory, so a leftover sidecar would falsely look like
    /// evidence of a crash on next startup.
    pub fn clear_recovery(&self) {
        if let Some(path) = self.recovery_path() {
            let _ = std::fs::remove_file(path);
        }
    }

    /// A recovery sidecar left over from a previous session that is NEWER than
    /// the on-disk source, if any.
    ///
    /// Its presence means unsaved work never made it into the source file —
    /// the session ended (crashed, or was quit) between an autosave and a
    /// save. An explicit save always deletes the sidecar, so a leftover one is
    /// normally impossible after a clean save.
    ///
    /// **Staleness guard.** `clear_recovery` on save is best-effort (its
    /// error is ignored) and a crash can strike between the source write and
    /// the sidecar delete, so a sidecar OLDER than the source can survive a
    /// save. Recovering it would revert the user to work they already
    /// superseded. We therefore only offer a sidecar strictly newer than the
    /// source; an older-or-equal one represents already-saved work and is not
    /// offered. If there is no source file yet (never saved), any sidecar is
    /// offered.
    ///
    /// Recovery is offered, never applied automatically: silently loading the
    /// sidecar would overwrite whatever file the user deliberately opened.
    pub fn pending_recovery(&self) -> Option<PathBuf> {
        let path = self.recovery_path()?;
        if !path.is_file() {
            return None;
        }
        let sidecar_mtime = std::fs::metadata(&path).and_then(|m| m.modified()).ok()?;
        // Compare against the on-disk source. If the source is unreadable /
        // absent (never saved), the sidecar is the only record of the work —
        // offer it.
        match self
            .session
            .source_path()
            .and_then(|src| std::fs::metadata(src).and_then(|m| m.modified()).ok())
        {
            Some(source_mtime) if sidecar_mtime <= source_mtime => None,
            _ => Some(path),
        }
    }

    fn cmd_recover_autosave(&mut self) -> Result<Vec<EditorEvent>, EditorCommandError> {
        if self.play_state() != EditorPlayState::Editing {
            return Err(EditorCommandError::UnsupportedCommandState {
                command: "RecoverAutosave",
                state: self.play_state(),
            });
        }
        let path = self
            .pending_recovery()
            .ok_or(EditorCommandError::MissingSourcePath)?;

        let recovered = super::load_scene_doc(&path)
            .map_err(|err| EditorCommandError::Session(EditorSessionError::Io(err)))?;

        *self.session.scene_mut() = recovered;
        self.session
            .rebuild_world(&mut self.world)
            .map_err(EditorCommandError::Session)?;
        self.world_requires_rebuild = false;
        // Undo history predates the recovered document; keeping it would let
        // an undo splice the two together into a scene that never existed.
        self.undo_stack.clear();
        self.redo_stack.clear();
        self.init_cursor_to_first();

        // The recovered document differs from the on-disk source, so it is
        // unsaved work. Marking it edited also re-arms the autosave scheduler.
        self.mark_edited();

        Ok(vec![
            EditorEvent::AutosaveRecovered {
                path: path.display().to_string(),
            },
            EditorEvent::WorldRebuilt,
        ])
    }

    fn cmd_discard_autosave(&mut self) -> Result<Vec<EditorEvent>, EditorCommandError> {
        // Idempotent: discarding when there is nothing to discard is a no-op,
        // not an error, so a stale UI prompt cannot fail the command.
        self.clear_recovery();
        Ok(vec![EditorEvent::AutosaveDiscarded])
    }

    /// Whether the live world is stale and needs a rebuild to match the scene.
    pub fn world_requires_rebuild(&self) -> bool {
        self.world_requires_rebuild
    }

    /// Whether an explicit authored undo is currently available.
    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    /// Whether an explicit authored redo is currently available.
    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    /// The current inspector cursor, or `None` if the scene is empty.
    pub fn cursor(&self) -> Option<EditorInspectorCursor> {
        let idx = self.cursor_entity_idx?;
        let scene = self.session.scene();
        let entity = scene.entities.get(idx)?;
        let component = entity.components.get(self.cursor_component_idx);
        let field = component.and_then(|component| component.fields.get(self.cursor_field_idx));
        Some(EditorInspectorCursor {
            entity_id: entity.id,
            component_name: component
                .map(|component| component.type_name.clone())
                .unwrap_or_default(),
            field_name: field.map(|field| field.name.clone()).unwrap_or_default(),
            axis: if field.is_some() {
                self.cursor_axis
            } else {
                None
            },
        })
    }

    // ── Selection (direct API — bypasses execute) ─────────────────────────

    pub fn select_entity(&mut self, entity_id: u64) -> Result<u64, EditorSessionError> {
        self.session.select_entity(entity_id)
    }

    pub fn select_first_with_component(
        &mut self,
        component: &str,
    ) -> Result<u64, EditorSessionError> {
        self.session.select_first_with_component(component)
    }

    pub fn set_selected_value(
        &mut self,
        component: &str,
        field: &str,
        value: InspectorValue,
    ) -> Result<(), EditorSessionError> {
        self.session.set_selected_value(component, field, value)
    }

    pub fn selected_runtime_entity(&self) -> Result<Entity, EditorSessionError> {
        let editor_entity_id = self
            .selected_entity_id()
            .ok_or(EditorSessionError::NoSelection)?;
        self.session.applied_entity(editor_entity_id)
    }

    pub fn applied_entity(&self, editor_entity_id: u64) -> Result<Entity, EditorSessionError> {
        self.session.applied_entity(editor_entity_id)
    }

    // ── Play mode (direct API) ─────────────────────────────────────────────

    pub fn enter_play_mode(&mut self) -> Result<(), EditorSessionError> {
        self.session.enter_play_mode(&mut self.world)?;
        self.world_requires_rebuild = false;
        Ok(())
    }

    pub fn pause_play_mode(&mut self) -> Result<(), EditorSessionError> {
        self.session.pause_play_mode()
    }

    pub fn resume_play_mode(&mut self) -> Result<(), EditorSessionError> {
        self.session.resume_play_mode()
    }

    pub fn stop_play_mode(&mut self) -> Result<(), EditorSessionError> {
        self.session.stop_play_mode(&mut self.world)?;
        self.world_requires_rebuild = false;
        Ok(())
    }

    // ── World sync (direct API) ────────────────────────────────────────────

    pub fn rebuild_world(&mut self) -> Result<&[(u64, Entity)], EditorSessionError> {
        let map = self.session.rebuild_world(&mut self.world)?;
        self.world_requires_rebuild = false;
        Ok(map)
    }

    pub fn patch_world(&mut self) -> Result<(), EditorSessionError> {
        match self.session.patch_world(&mut self.world) {
            Ok(()) => {
                self.world_requires_rebuild = false;
                Ok(())
            }
            Err(EditorSessionError::PatchRequiresRebuild { reason }) => {
                self.world_requires_rebuild = true;
                Err(EditorSessionError::PatchRequiresRebuild { reason })
            }
            Err(err) => Err(err),
        }
    }

    // ── Save (direct API) ─────────────────────────────────────────────────

    pub fn save_hsc<P: AsRef<Path>>(&self, path: P) -> Result<(), EditorSessionError> {
        self.session.save_hsc(path)
    }

    pub fn save_source_hsc(&self) -> Result<(), EditorSessionError> {
        self.session.save_source_hsc()
    }

    // ── Editor camera ──────────────────────────────────────────────────────

    /// Update the editor camera from frame input.
    ///
    /// This is a no-op during `Playing` — the viewport uses the runtime
    /// `ActiveCamera` instead.
    pub fn update_editor_camera(&mut self, input: &InputState, dt: f32) {
        if self.play_state() == EditorPlayState::Playing {
            return;
        }
        self.editor_camera.update(input, dt);
    }

    /// Return the active view-projection matrix for the current frame.
    ///
    /// - `Editing` / `Paused` — uses the host-owned [`EditorCamera`].
    /// - `Playing` — uses the first [`ActiveCamera`] found in the live world.
    ///
    /// Falls back to `Mat4::IDENTITY` if no camera is available.
    pub fn active_view_proj(&self, fov_deg: f32, aspect: f32) -> Mat4 {
        match self.play_state() {
            // Authoring view (editor camera): used while editing, paused,
            // and during deterministic replay — replay drives gameplay
            // state from the recorded stream but the *camera framing* is
            // the editor's job, not the recorded simulation's.
            EditorPlayState::Editing
            | EditorPlayState::Paused
            | EditorPlayState::ReplayingDeterministic => {
                self.editor_camera.view_proj(fov_deg, aspect, 0.1, 1000.0)
            }
            EditorPlayState::Playing => {
                // Find the runtime ActiveCamera and use its view/proj matrices.
                self.world
                    .query::<ActiveCamera>()
                    .filter_map(|(e, _)| self.world.get::<Camera>(e))
                    .next()
                    .map(|cam| cam.proj * cam.view)
                    .unwrap_or(Mat4::IDENTITY)
            }
        }
    }

    // ── Command surface ────────────────────────────────────────────────────

    /// The authoritative shell/bridge entrypoint.
    ///
    /// Processes `command`, updates internal state, applies world mutations,
    /// and returns a list of events describing what changed.  Events are
    /// ordered causally (e.g. [`EditorEvent::FieldEdited`] precedes the patch
    /// result event).
    ///
    /// A return value of `Err` means the command was rejected before any state
    /// changed.  Events on the `Ok` path always reflect real mutations.
    pub fn execute(
        &mut self,
        command: EditorCommand,
    ) -> Result<Vec<EditorEvent>, EditorCommandError> {
        match command {
            EditorCommand::SelectNextEntity => self.cmd_cycle_entity(1),
            EditorCommand::SelectPrevEntity => self.cmd_cycle_entity(-1),
            EditorCommand::SelectEntity { entity_id } => self.cmd_select_entity(entity_id),
            EditorCommand::SelectNextComponent => self.cmd_cycle_component(1),
            EditorCommand::SelectPrevComponent => self.cmd_cycle_component(-1),
            EditorCommand::SelectComponent {
                entity_id,
                component_name,
            } => self.cmd_select_component(entity_id, &component_name),
            EditorCommand::SelectNextField => self.cmd_cycle_field(1),
            EditorCommand::SelectPrevField => self.cmd_cycle_field(-1),
            EditorCommand::SelectField {
                entity_id,
                component_name,
                field_name,
            } => self.cmd_select_field(entity_id, &component_name, &field_name),
            EditorCommand::SetAxis(axis) => self.cmd_set_axis(axis),
            EditorCommand::ApplyDelta(delta) => self.cmd_apply_delta(delta),
            EditorCommand::EnterPlayMode => self.cmd_enter_play_mode(),
            EditorCommand::PausePlayMode => self.cmd_pause_play_mode(),
            EditorCommand::ResumePlayMode => self.cmd_resume_play_mode(),
            EditorCommand::StopPlayMode => self.cmd_stop_play_mode(),
            EditorCommand::Save => self.cmd_save(),
            EditorCommand::Undo => self.cmd_undo(),
            EditorCommand::Redo => self.cmd_redo(),
            EditorCommand::ReloadSource => self.cmd_reload_source(),
            EditorCommand::ForcePatch => self.cmd_force_patch(),
            EditorCommand::ForceRebuild => self.cmd_force_rebuild(),
            EditorCommand::RecoverAutosave => self.cmd_recover_autosave(),
            EditorCommand::DiscardAutosave => self.cmd_discard_autosave(),
            EditorCommand::SetTerrainSettings {
                size_meters,
                chunk_size_meters,
                height_scale_meters,
            } => self.cmd_set_terrain_settings(size_meters, chunk_size_meters, height_scale_meters),
            EditorCommand::UpsertTerrainPaintLayer {
                name,
                material,
                tiling,
                weightmap_channel,
            } => self.cmd_upsert_terrain_paint_layer(name, material, tiling, weightmap_channel),
            EditorCommand::RemoveTerrainPaintLayer { name } => {
                self.cmd_remove_terrain_paint_layer(name)
            }
            EditorCommand::PaintVegetation {
                prototype,
                center,
                radius,
                density,
            } => self.cmd_paint_vegetation(prototype, center, radius, density),
            EditorCommand::EraseVegetation {
                prototype,
                center,
                radius,
            } => self.cmd_erase_vegetation(prototype, center, radius),
            EditorCommand::UpsertPatrolRoute {
                name,
                points,
                looped,
            } => self.cmd_upsert_patrol_route(name, points, looped),
            EditorCommand::RemovePatrolRoute { name } => self.cmd_remove_patrol_route(name),
            EditorCommand::UpsertVehicleSpawner {
                name,
                mesh,
                material,
                position,
                facing_yaw_deg,
            } => self.cmd_upsert_vehicle_spawner(name, mesh, material, position, facing_yaw_deg),
            EditorCommand::RemoveVehicleSpawner { name } => self.cmd_remove_vehicle_spawner(name),
        }
    }

    /// Decode a bridge-protocol command message, execute it, and return the
    /// resulting events encoded back into bridge-protocol messages.
    pub fn execute_protocol_message(
        &mut self,
        message: &str,
    ) -> Result<Vec<String>, EditorProtocolError> {
        let command = decode_command(message)?;
        let events = self
            .execute(command)
            .map_err(EditorProtocolError::Command)?;
        Ok(events.iter().map(encode_event).collect())
    }

    // ── Private: cursor helpers ────────────────────────────────────────────

    /// Set cursor to the first entity / component / field in the scene.
    fn init_cursor_to_first(&mut self) {
        let count = self.session.scene().entities.len();
        if count == 0 {
            self.cursor_entity_idx = None;
            return;
        }
        self.apply_cursor_selection(0, 0, 0);
    }

    /// Apply a selection (entity index already validated) and sync session.
    fn apply_cursor_entity(&mut self, idx: usize) {
        self.apply_cursor_selection(idx, 0, 0);
    }

    fn apply_cursor_selection(
        &mut self,
        entity_idx: usize,
        component_idx: usize,
        field_idx: usize,
    ) {
        let entity = &self.session.scene().entities[entity_idx];
        let component_count = entity.components.len();
        let next_component_idx = if component_count == 0 {
            0
        } else {
            component_idx.min(component_count - 1)
        };
        let next_field_idx = entity
            .components
            .get(next_component_idx)
            .map(|component| {
                if component.fields.is_empty() {
                    0
                } else {
                    field_idx.min(component.fields.len() - 1)
                }
            })
            .unwrap_or(0);
        let id = entity.id;

        self.cursor_entity_idx = Some(entity_idx);
        self.cursor_component_idx = next_component_idx;
        self.cursor_field_idx = next_field_idx;
        self.cursor_axis = None;
        let _ = self.session.select_entity(id);
    }

    fn find_entity_index(&self, entity_id: u64) -> Option<usize> {
        self.session
            .scene()
            .entities
            .iter()
            .position(|entity| entity.id == entity_id)
    }

    fn find_component_index(&self, entity_idx: usize, component_name: &str) -> Option<usize> {
        self.session
            .scene()
            .entities
            .get(entity_idx)?
            .components
            .iter()
            .position(|component| component.type_name == component_name)
    }

    fn find_field_index(
        &self,
        entity_idx: usize,
        component_idx: usize,
        field_name: &str,
    ) -> Option<usize> {
        self.session
            .scene()
            .entities
            .get(entity_idx)?
            .components
            .get(component_idx)?
            .fields
            .iter()
            .position(|field| field.name == field_name)
    }

    /// Current field value, resolved from cursor indices.
    fn current_field_value(&self) -> Option<&InspectorValue> {
        let idx = self.cursor_entity_idx?;
        let scene = self.session.scene();
        let entity = scene.entities.get(idx)?;
        let comp = entity.components.get(self.cursor_component_idx)?;
        comp.fields.get(self.cursor_field_idx).map(|f| &f.value)
    }

    /// Current (component_name, field_name) pair.
    fn current_component_field_names(&self) -> Option<(String, String)> {
        let idx = self.cursor_entity_idx?;
        let scene = self.session.scene();
        let entity = scene.entities.get(idx)?;
        let comp = entity.components.get(self.cursor_component_idx)?;
        let field = comp.fields.get(self.cursor_field_idx)?;
        Some((comp.type_name.clone(), field.name.clone()))
    }

    /// Restore the cursor to `(entity_id, component_name, field_name)` after a
    /// rebuild, clamping each level to 0 when the target is not found.
    fn restore_cursor(
        &mut self,
        target_entity_id: Option<u64>,
        target_component: &str,
        target_field: &str,
        target_axis: Option<EditorAxis>,
    ) {
        let entities = &self.session.scene().entities;
        if entities.is_empty() {
            self.cursor_entity_idx = None;
            return;
        }

        // Find entity index by id; clamp to 0 if not found.
        let entity_idx = target_entity_id
            .and_then(|id| entities.iter().position(|e| e.id == id))
            .unwrap_or(0);

        // Find component by name; clamp to 0 if not found.
        let entity = &entities[entity_idx];
        let comp_idx = entity
            .components
            .iter()
            .position(|c| c.type_name == target_component)
            .unwrap_or(0);

        // Find field by name; clamp to 0 if not found.
        let field_idx = entity
            .components
            .get(comp_idx)
            .and_then(|c| c.fields.iter().position(|f| f.name == target_field))
            .unwrap_or(0);
        self.apply_cursor_selection(entity_idx, comp_idx, field_idx);
        if matches!(self.current_field_value(), Some(InspectorValue::Vec3(_))) {
            self.cursor_axis = target_axis;
        }
    }

    fn restore_cursor_snapshot(&mut self, cursor: Option<&EditorInspectorCursor>) {
        if let Some(cursor) = cursor {
            self.restore_cursor(
                Some(cursor.entity_id),
                &cursor.component_name,
                &cursor.field_name,
                cursor.axis,
            );
        } else {
            self.init_cursor_to_first();
        }
    }

    fn capture_history_snapshot(&self) -> EditorHistorySnapshot {
        EditorHistorySnapshot {
            scene: self.session.scene().clone(),
            cursor: self.cursor(),
            persistent_dirty: self.persistent_dirty,
        }
    }

    fn push_undo_snapshot(&mut self, snapshot: EditorHistorySnapshot) {
        self.undo_stack.push(snapshot);
        self.redo_stack.clear();
    }

    fn restore_history_snapshot(
        &mut self,
        snapshot: EditorHistorySnapshot,
        history_event: EditorEvent,
    ) -> Result<Vec<EditorEvent>, EditorCommandError> {
        let EditorHistorySnapshot {
            scene,
            cursor,
            persistent_dirty,
        } = snapshot;
        *self.session.scene_mut() = scene;
        self.session
            .rebuild_world(&mut self.world)
            .map_err(EditorCommandError::Session)?;
        self.persistent_dirty = persistent_dirty;
        self.pre_play_persistent_dirty = persistent_dirty;
        // The document content changed, so any autosave sidecar is now stale.
        // Bump the revision so the scheduler re-checkpoints the new state; and
        // if the undo/redo landed on a clean state, delete the sidecar so a
        // later startup does not offer recovery of work the user just undid.
        self.edit_revision = self.edit_revision.wrapping_add(1);
        if !persistent_dirty {
            self.clear_recovery();
        }
        self.world_requires_rebuild = false;
        self.restore_cursor_snapshot(cursor.as_ref());

        let mut events = vec![history_event, EditorEvent::WorldRebuilt];
        if let Some(cursor) = self.cursor() {
            events.push(EditorEvent::SelectionChanged(cursor));
        }
        Ok(events)
    }

    /// Auto-attempt `patch_world`; translate the result into events.
    fn auto_patch_events(&mut self) -> Result<Vec<EditorEvent>, EditorCommandError> {
        match self.session.patch_world(&mut self.world) {
            Ok(()) => {
                self.world_requires_rebuild = false;
                Ok(vec![EditorEvent::PatchApplied])
            }
            Err(EditorSessionError::PatchRequiresRebuild { reason }) => {
                self.world_requires_rebuild = true;
                Ok(vec![EditorEvent::PatchRequiresRebuild { reason }])
            }
            Err(err) => Err(EditorCommandError::Session(err)),
        }
    }

    fn begin_runtime_slice_edit(
        &mut self,
        command: &'static str,
    ) -> Result<Option<EditorHistorySnapshot>, EditorCommandError> {
        let play_state = self.play_state();
        if play_state == EditorPlayState::Playing {
            return Err(EditorCommandError::UnsupportedCommandState {
                command,
                state: EditorPlayState::Playing,
            });
        }
        if play_state == EditorPlayState::Editing {
            Ok(Some(self.capture_history_snapshot()))
        } else {
            Ok(None)
        }
    }

    fn finish_runtime_slice_edit(
        &mut self,
        history_snapshot: Option<EditorHistorySnapshot>,
        path: impl Into<String>,
    ) -> Result<Vec<EditorEvent>, EditorCommandError> {
        if let Some(snapshot) = history_snapshot {
            self.push_undo_snapshot(snapshot);
            self.mark_edited();
        }
        let mut events = vec![EditorEvent::RuntimeSliceUpdated { path: path.into() }];
        events.extend(self.auto_patch_events()?);
        Ok(events)
    }

    fn validate_runtime_slice_name(
        &self,
        kind: &'static str,
        name: &str,
    ) -> Result<String, EditorCommandError> {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return Err(EditorCommandError::InvalidRuntimeSliceEdit {
                reason: format!("runtime-slice {kind} name cannot be empty"),
            });
        }
        Ok(trimmed.to_string())
    }

    // ── Private: command implementations ──────────────────────────────────

    fn cmd_cycle_entity(&mut self, dir: i32) -> Result<Vec<EditorEvent>, EditorCommandError> {
        let count = self.session.scene().entities.len();
        if count == 0 {
            return Ok(vec![]);
        }

        let current = self.cursor_entity_idx.unwrap_or(0);
        let next = ((current as i32 + dir).rem_euclid(count as i32)) as usize;
        self.apply_cursor_entity(next);

        Ok(vec![EditorEvent::SelectionChanged(self.cursor().unwrap())])
    }

    fn cmd_select_entity(
        &mut self,
        entity_id: u64,
    ) -> Result<Vec<EditorEvent>, EditorCommandError> {
        self.session
            .entity(entity_id)
            .map_err(EditorCommandError::Session)?;
        let entity_idx = self
            .find_entity_index(entity_id)
            .ok_or(EditorCommandError::NoSelection)?;
        self.apply_cursor_selection(entity_idx, 0, 0);
        Ok(vec![EditorEvent::SelectionChanged(self.cursor().unwrap())])
    }

    fn cmd_cycle_component(&mut self, dir: i32) -> Result<Vec<EditorEvent>, EditorCommandError> {
        let idx = self
            .cursor_entity_idx
            .ok_or(EditorCommandError::NoSelection)?;
        let entity = &self.session.scene().entities[idx];
        let count = entity.components.len();
        if count == 0 {
            return Ok(vec![]);
        }

        let next = ((self.cursor_component_idx as i32 + dir).rem_euclid(count as i32)) as usize;
        self.cursor_component_idx = next;
        self.cursor_field_idx = 0;
        self.cursor_axis = None;

        Ok(vec![EditorEvent::SelectionChanged(self.cursor().unwrap())])
    }

    fn cmd_select_component(
        &mut self,
        entity_id: u64,
        component_name: &str,
    ) -> Result<Vec<EditorEvent>, EditorCommandError> {
        self.session
            .entity(entity_id)
            .map_err(EditorCommandError::Session)?;
        let entity_idx = self
            .find_entity_index(entity_id)
            .ok_or(EditorCommandError::NoSelection)?;
        let component_idx = self
            .find_component_index(entity_idx, component_name)
            .ok_or(EditorCommandError::NoField)?;
        self.apply_cursor_selection(entity_idx, component_idx, 0);
        Ok(vec![EditorEvent::SelectionChanged(self.cursor().unwrap())])
    }

    fn cmd_cycle_field(&mut self, dir: i32) -> Result<Vec<EditorEvent>, EditorCommandError> {
        let idx = self
            .cursor_entity_idx
            .ok_or(EditorCommandError::NoSelection)?;
        let entity = &self.session.scene().entities[idx];
        let comp = entity
            .components
            .get(self.cursor_component_idx)
            .ok_or(EditorCommandError::NoField)?;
        let count = comp.fields.len();
        if count == 0 {
            return Ok(vec![]);
        }

        let next = ((self.cursor_field_idx as i32 + dir).rem_euclid(count as i32)) as usize;
        self.cursor_field_idx = next;
        self.cursor_axis = None;

        Ok(vec![EditorEvent::SelectionChanged(self.cursor().unwrap())])
    }

    fn cmd_select_field(
        &mut self,
        entity_id: u64,
        component_name: &str,
        field_name: &str,
    ) -> Result<Vec<EditorEvent>, EditorCommandError> {
        self.session
            .entity(entity_id)
            .map_err(EditorCommandError::Session)?;
        let entity_idx = self
            .find_entity_index(entity_id)
            .ok_or(EditorCommandError::NoSelection)?;
        let component_idx = self
            .find_component_index(entity_idx, component_name)
            .ok_or(EditorCommandError::NoField)?;
        let field_idx = self
            .find_field_index(entity_idx, component_idx, field_name)
            .ok_or(EditorCommandError::NoField)?;
        self.apply_cursor_selection(entity_idx, component_idx, field_idx);
        Ok(vec![EditorEvent::SelectionChanged(self.cursor().unwrap())])
    }

    fn cmd_set_axis(&mut self, axis: EditorAxis) -> Result<Vec<EditorEvent>, EditorCommandError> {
        // Only meaningful for Vec3 fields; silently accept for others (no error).
        self.cursor_axis = Some(axis);
        if let Some(cursor) = self.cursor() {
            Ok(vec![EditorEvent::SelectionChanged(cursor)])
        } else {
            Ok(vec![])
        }
    }

    fn cmd_apply_delta(&mut self, delta: f32) -> Result<Vec<EditorEvent>, EditorCommandError> {
        let play_state = self.play_state();

        // Playing blocks field edits.
        if play_state == EditorPlayState::Playing {
            return Err(EditorCommandError::UnsupportedCommandState {
                command: "ApplyDelta",
                state: EditorPlayState::Playing,
            });
        }

        // Resolve current field.
        let cursor = self.cursor().ok_or(EditorCommandError::NoSelection)?;
        let current_value = self
            .current_field_value()
            .ok_or(EditorCommandError::NoField)?
            .clone();

        let (comp_name, field_name) = self
            .current_component_field_names()
            .ok_or(EditorCommandError::NoField)?;

        let new_value = match &current_value {
            InspectorValue::Number(n) => InspectorValue::Number(n + delta),
            // An integer field nudges by the rounded delta and stays an
            // integer. `f32 -> i64` saturates and `saturating_add` guards
            // overflow, so a nudge can never produce a fractional or wrapped
            // value. `round()` is half-away-from-zero, so |delta| < 0.5 is a
            // no-op and |delta| in [0.5, 1.0) still moves by one unit; the
            // stepper buttons only ever send +/-1.0 (native_ui.rs), so in
            // practice a nudge always moves the field by exactly one.
            InspectorValue::Integer(n) => {
                InspectorValue::Integer(n.saturating_add(delta.round() as i64))
            }
            InspectorValue::Vec3(v) => {
                let axis = self.cursor_axis.unwrap_or(EditorAxis::X);
                let mut arr = *v;
                match axis {
                    EditorAxis::X => arr[0] += delta,
                    EditorAxis::Y => arr[1] += delta,
                    EditorAxis::Z => arr[2] += delta,
                }
                InspectorValue::Vec3(arr)
            }
            InspectorValue::Text(_) => {
                return Err(EditorCommandError::UnsupportedFieldKind {
                    field: field_name.clone(),
                })
            }
            InspectorValue::Bool(_) => {
                return Err(EditorCommandError::UnsupportedFieldKind {
                    field: field_name.clone(),
                })
            }
        };

        // Mutate scene document.
        let history_snapshot = if play_state == EditorPlayState::Editing {
            Some(self.capture_history_snapshot())
        } else {
            None
        };

        self.session
            .set_selected_value(&comp_name, &field_name, new_value.clone())
            .map_err(EditorCommandError::Session)?;

        // Persistent dirty — only in Editing; Paused edits are transient PIE overrides.
        if let Some(snapshot) = history_snapshot {
            self.push_undo_snapshot(snapshot);
            self.mark_edited();
        }

        let field_event = EditorEvent::FieldEdited {
            cursor: EditorInspectorCursor {
                axis: self.cursor_axis,
                ..cursor
            },
            new_value,
        };

        let mut events = vec![field_event];
        events.extend(self.auto_patch_events()?);
        Ok(events)
    }

    fn cmd_set_terrain_settings(
        &mut self,
        size_meters: Vec3,
        chunk_size_meters: u32,
        height_scale_meters: f32,
    ) -> Result<Vec<EditorEvent>, EditorCommandError> {
        let history = self.begin_runtime_slice_edit("SetTerrainSettings")?;
        self.session.scene_mut().set_terrain_settings(
            size_meters,
            chunk_size_meters,
            height_scale_meters,
        );
        self.finish_runtime_slice_edit(history, "terrain")
    }

    fn cmd_upsert_terrain_paint_layer(
        &mut self,
        name: String,
        material: Option<String>,
        tiling: f32,
        weightmap_channel: u8,
    ) -> Result<Vec<EditorEvent>, EditorCommandError> {
        let history = self.begin_runtime_slice_edit("UpsertTerrainPaintLayer")?;
        let name = self.validate_runtime_slice_name("terrain paint layer", &name)?;
        self.session
            .scene_mut()
            .upsert_terrain_paint_layer(TerrainPaintLayer {
                name: name.clone(),
                material,
                tiling,
                weightmap_channel,
            });
        self.finish_runtime_slice_edit(history, format!("terrain.paint_layers.{name}"))
    }

    fn cmd_remove_terrain_paint_layer(
        &mut self,
        name: String,
    ) -> Result<Vec<EditorEvent>, EditorCommandError> {
        let history = self.begin_runtime_slice_edit("RemoveTerrainPaintLayer")?;
        let name = self.validate_runtime_slice_name("terrain paint layer", &name)?;
        if !self.session.scene_mut().remove_terrain_paint_layer(&name) {
            return Err(EditorCommandError::MissingRuntimeSliceItem {
                kind: "terrain paint layer",
                name,
            });
        }
        self.finish_runtime_slice_edit(history, "terrain.paint_layers")
    }

    fn cmd_paint_vegetation(
        &mut self,
        prototype: String,
        center: Vec3,
        radius: f32,
        density: f32,
    ) -> Result<Vec<EditorEvent>, EditorCommandError> {
        let history = self.begin_runtime_slice_edit("PaintVegetation")?;
        let prototype = self.validate_runtime_slice_name("vegetation prototype", &prototype)?;
        self.session
            .scene_mut()
            .paint_vegetation(VegetationPaintStroke {
                prototype: prototype.clone(),
                center,
                radius,
                density,
            });
        self.finish_runtime_slice_edit(history, format!("vegetation_paints.{prototype}"))
    }

    fn cmd_erase_vegetation(
        &mut self,
        prototype: Option<String>,
        center: Vec3,
        radius: f32,
    ) -> Result<Vec<EditorEvent>, EditorCommandError> {
        let history = self.begin_runtime_slice_edit("EraseVegetation")?;
        let normalized_prototype = match prototype {
            Some(name) => Some(self.validate_runtime_slice_name("vegetation prototype", &name)?),
            None => None,
        };
        self.session
            .scene_mut()
            .erase_vegetation(normalized_prototype.as_deref(), center, radius);
        self.finish_runtime_slice_edit(history, "vegetation_paints")
    }

    fn cmd_upsert_patrol_route(
        &mut self,
        name: String,
        points: Vec<Vec3>,
        looped: bool,
    ) -> Result<Vec<EditorEvent>, EditorCommandError> {
        let history = self.begin_runtime_slice_edit("UpsertPatrolRoute")?;
        let name = self.validate_runtime_slice_name("patrol route", &name)?;
        self.session
            .scene_mut()
            .upsert_patrol_route(PatrolRouteDef {
                name: name.clone(),
                points,
                looped,
            });
        self.finish_runtime_slice_edit(history, format!("patrol_routes.{name}"))
    }

    fn cmd_remove_patrol_route(
        &mut self,
        name: String,
    ) -> Result<Vec<EditorEvent>, EditorCommandError> {
        let history = self.begin_runtime_slice_edit("RemovePatrolRoute")?;
        let name = self.validate_runtime_slice_name("patrol route", &name)?;
        if !self.session.scene_mut().remove_patrol_route(&name) {
            return Err(EditorCommandError::MissingRuntimeSliceItem {
                kind: "patrol route",
                name,
            });
        }
        self.finish_runtime_slice_edit(history, "patrol_routes")
    }

    fn cmd_upsert_vehicle_spawner(
        &mut self,
        name: String,
        mesh: Option<String>,
        material: Option<String>,
        position: Vec3,
        facing_yaw_deg: f32,
    ) -> Result<Vec<EditorEvent>, EditorCommandError> {
        let history = self.begin_runtime_slice_edit("UpsertVehicleSpawner")?;
        let name = self.validate_runtime_slice_name("vehicle spawner", &name)?;
        self.session
            .scene_mut()
            .upsert_vehicle_spawner(VehicleSpawnerDef {
                name: name.clone(),
                mesh,
                material,
                position,
                facing_yaw_deg,
            });
        self.finish_runtime_slice_edit(history, format!("vehicle_spawners.{name}"))
    }

    fn cmd_remove_vehicle_spawner(
        &mut self,
        name: String,
    ) -> Result<Vec<EditorEvent>, EditorCommandError> {
        let history = self.begin_runtime_slice_edit("RemoveVehicleSpawner")?;
        let name = self.validate_runtime_slice_name("vehicle spawner", &name)?;
        if !self.session.scene_mut().remove_vehicle_spawner(&name) {
            return Err(EditorCommandError::MissingRuntimeSliceItem {
                kind: "vehicle spawner",
                name,
            });
        }
        self.finish_runtime_slice_edit(history, "vehicle_spawners")
    }

    fn cmd_enter_play_mode(&mut self) -> Result<Vec<EditorEvent>, EditorCommandError> {
        if self.play_state() != EditorPlayState::Editing {
            return Err(EditorCommandError::UnsupportedCommandState {
                command: "EnterPlayMode",
                state: self.play_state(),
            });
        }
        self.pre_play_persistent_dirty = self.persistent_dirty;
        self.session
            .enter_play_mode(&mut self.world)
            .map_err(EditorCommandError::Session)?;
        self.world_requires_rebuild = false;
        Ok(vec![EditorEvent::PlayModeChanged(EditorPlayState::Playing)])
    }

    fn cmd_pause_play_mode(&mut self) -> Result<Vec<EditorEvent>, EditorCommandError> {
        if self.play_state() != EditorPlayState::Playing {
            return Err(EditorCommandError::UnsupportedCommandState {
                command: "PausePlayMode",
                state: self.play_state(),
            });
        }
        self.session
            .pause_play_mode()
            .map_err(EditorCommandError::Session)?;
        Ok(vec![EditorEvent::PlayModeChanged(EditorPlayState::Paused)])
    }

    fn cmd_resume_play_mode(&mut self) -> Result<Vec<EditorEvent>, EditorCommandError> {
        if self.play_state() != EditorPlayState::Paused {
            return Err(EditorCommandError::UnsupportedCommandState {
                command: "ResumePlayMode",
                state: self.play_state(),
            });
        }
        self.session
            .resume_play_mode()
            .map_err(EditorCommandError::Session)?;
        Ok(vec![EditorEvent::PlayModeChanged(EditorPlayState::Playing)])
    }

    fn cmd_stop_play_mode(&mut self) -> Result<Vec<EditorEvent>, EditorCommandError> {
        let state = self.play_state();
        if state == EditorPlayState::Editing {
            return Err(EditorCommandError::UnsupportedCommandState {
                command: "StopPlayMode",
                state,
            });
        }

        // Preserve cursor identity for restoration.
        let (saved_entity_id, saved_comp, saved_field, saved_axis) = self
            .cursor()
            .map(|c| (Some(c.entity_id), c.component_name, c.field_name, c.axis))
            .unwrap_or((None, String::new(), String::new(), None));

        self.session
            .stop_play_mode(&mut self.world)
            .map_err(EditorCommandError::Session)?;

        // Restore pre-play dirty flag; discard transient play edits.
        self.persistent_dirty = self.pre_play_persistent_dirty;
        self.world_requires_rebuild = false;

        // Restore cursor (clamped if the entity/component/field no longer exists).
        self.restore_cursor(saved_entity_id, &saved_comp, &saved_field, saved_axis);

        Ok(vec![
            EditorEvent::PlayModeChanged(EditorPlayState::Editing),
            EditorEvent::WorldRebuilt,
        ])
    }

    fn cmd_save(&mut self) -> Result<Vec<EditorEvent>, EditorCommandError> {
        if self.play_state() != EditorPlayState::Editing {
            return Err(EditorCommandError::UnsupportedCommandState {
                command: "Save",
                state: self.play_state(),
            });
        }
        if self.session.source_path().is_none() {
            return Err(EditorCommandError::MissingSourcePath);
        }
        self.session
            .save_source_hsc()
            .map_err(EditorCommandError::Session)?;
        // Only past this point has the write durably landed (atomic_write
        // returned Ok). A failed save leaves persistent_dirty set AND the
        // recovery file in place, so the unsaved work survives either way.
        self.persistent_dirty = false;
        self.clear_recovery();
        Ok(vec![EditorEvent::Saved])
    }

    fn cmd_undo(&mut self) -> Result<Vec<EditorEvent>, EditorCommandError> {
        if self.play_state() != EditorPlayState::Editing {
            return Err(EditorCommandError::UnsupportedCommandState {
                command: "Undo",
                state: self.play_state(),
            });
        }

        let snapshot = self
            .undo_stack
            .pop()
            .ok_or(EditorCommandError::HistoryEmpty { command: "Undo" })?;
        self.redo_stack.push(self.capture_history_snapshot());
        self.restore_history_snapshot(snapshot, EditorEvent::UndoApplied)
    }

    fn cmd_redo(&mut self) -> Result<Vec<EditorEvent>, EditorCommandError> {
        if self.play_state() != EditorPlayState::Editing {
            return Err(EditorCommandError::UnsupportedCommandState {
                command: "Redo",
                state: self.play_state(),
            });
        }

        let snapshot = self
            .redo_stack
            .pop()
            .ok_or(EditorCommandError::HistoryEmpty { command: "Redo" })?;
        self.undo_stack.push(self.capture_history_snapshot());
        self.restore_history_snapshot(snapshot, EditorEvent::RedoApplied)
    }

    fn cmd_reload_source(&mut self) -> Result<Vec<EditorEvent>, EditorCommandError> {
        if self.play_state() != EditorPlayState::Editing {
            return Err(EditorCommandError::UnsupportedCommandState {
                command: "ReloadSource",
                state: self.play_state(),
            });
        }
        let source_path = self
            .session
            .source_path()
            .ok_or(EditorCommandError::MissingSourcePath)?
            .to_path_buf();
        let project = self.session.project().cloned();

        let (saved_entity_id, saved_comp, saved_field, saved_axis) = self
            .cursor()
            .map(|c| (Some(c.entity_id), c.component_name, c.field_name, c.axis))
            .unwrap_or((None, String::new(), String::new(), None));

        self.session = match project {
            Some(project) => EditorSceneSession::open_hsc_with_project(&source_path, project),
            None => EditorSceneSession::open_hsc(&source_path),
        }
        .map_err(EditorCommandError::Session)?;
        self.session
            .rebuild_world(&mut self.world)
            .map_err(EditorCommandError::Session)?;
        self.persistent_dirty = false;
        self.pre_play_persistent_dirty = false;
        self.world_requires_rebuild = false;
        self.undo_stack.clear();
        self.redo_stack.clear();

        self.restore_cursor(saved_entity_id, &saved_comp, &saved_field, saved_axis);
        Ok(vec![EditorEvent::Reloaded, EditorEvent::WorldRebuilt])
    }

    fn cmd_force_patch(&mut self) -> Result<Vec<EditorEvent>, EditorCommandError> {
        self.auto_patch_events()
    }

    fn cmd_force_rebuild(&mut self) -> Result<Vec<EditorEvent>, EditorCommandError> {
        // Preserve cursor for restoration.
        let (saved_entity_id, saved_comp, saved_field, saved_axis) = self
            .cursor()
            .map(|c| (Some(c.entity_id), c.component_name, c.field_name, c.axis))
            .unwrap_or((None, String::new(), String::new(), None));

        self.session
            .rebuild_world(&mut self.world)
            .map_err(EditorCommandError::Session)?;
        self.world_requires_rebuild = false;

        self.restore_cursor(saved_entity_id, &saved_comp, &saved_field, saved_axis);

        Ok(vec![EditorEvent::WorldRebuilt])
    }
}

/// Per-category counts produced by
/// [`EditorRuntimeHost::apply_dispatch_summary`]. Useful for
/// telemetry, post-frame logging, and tests that need to assert
/// what shape the dispatcher delivered without re-walking the
/// summary themselves.
///
/// Every field is monotonically incremented across a single
/// `apply_dispatch_summary` call — i.e. counts a single dispatch's
/// outcomes, not a session-wide aggregate. Callers that want
/// session totals fold these into their own counters.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DispatchApplyReport {
    /// Number of asset reloads (Texture / NormalMap / Mesh /
    /// Material / Shader) that landed in `AssetStore`.
    pub assets_applied: usize,
    /// Number of scenes where the new authored content equaled
    /// the previous snapshot — the diff path's zero-cost branch.
    pub scenes_no_change: usize,
    /// Number of scenes that took the diff path (entity set
    /// stable, components patched in place).
    pub scenes_diffed: usize,
    /// Number of scenes that fell back to a full rebuild (the
    /// safety-net path; runtime-only state is lost).
    pub scenes_rebuilt: usize,
    /// Number of scenes whose reload errored entirely. Counts
    /// failures from the rebuild fallback's apply step; ordinary
    /// diff failures fall back to rebuild rather than error out.
    pub scenes_failed: usize,
    /// Aggregate of authored entities added across all `Diffed`
    /// scene reloads in this dispatch.
    pub scene_entities_added: usize,
    /// Aggregate of authored entities removed across all
    /// `Diffed` scene reloads.
    pub scene_entities_removed: usize,
    /// Aggregate of authored entities patched in place across
    /// all `Diffed` scene reloads.
    pub scene_entities_patched: usize,
    /// Number of compile-error rows pushed onto the status panel.
    pub errors_recorded: usize,
    /// Stale results filtered out by the last-wins discipline
    /// (telemetry only — these don't reach the status panel).
    pub stale_dropped: usize,
}

impl DispatchApplyReport {
    /// Whether this dispatch produced any user-visible activity
    /// at all. Convenient for skipping per-frame logging when
    /// nothing happened.
    pub fn has_activity(&self) -> bool {
        self.assets_applied > 0
            || self.scenes_no_change > 0
            || self.scenes_diffed > 0
            || self.scenes_rebuilt > 0
            || self.scenes_failed > 0
            || self.errors_recorded > 0
    }

    /// Total scene reloads observed (any outcome). Sum of the
    /// four scene_* outcome counters.
    pub fn total_scenes(&self) -> usize {
        self.scenes_no_change + self.scenes_diffed + self.scenes_rebuilt + self.scenes_failed
    }
}

/// Map a [`crate::assets::CompiledAssetKind`] to a stable string
/// label for the editor's status panel. Mirrors
/// [`crate::assets::AssetKind::as_str`] so labels stay consistent
/// between the two enums (they describe overlapping but distinct
/// concepts: compile-time vs. runtime asset kinds).
fn compiled_asset_kind_label(kind: crate::assets::CompiledAssetKind) -> &'static str {
    match kind {
        crate::assets::CompiledAssetKind::Mesh => "Mesh",
        crate::assets::CompiledAssetKind::Material => "Material",
        crate::assets::CompiledAssetKind::Texture => "Texture",
        crate::assets::CompiledAssetKind::TerrainLayerSet => "TerrainLayerSet",
        crate::assets::CompiledAssetKind::VegetationPrototype => "VegetationPrototype",
        crate::assets::CompiledAssetKind::Skeleton => "Skeleton",
        crate::assets::CompiledAssetKind::AnimationClip => "AnimationClip",
        crate::assets::CompiledAssetKind::AudioClip => "AudioClip",
        crate::assets::CompiledAssetKind::Shader => "Shader",
        crate::assets::CompiledAssetKind::Scene => "Scene",
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ecs::components::ParticleEmitter;
    use crate::{ActiveCamera, Camera, Transform, Vec3};
    use std::time::{SystemTime, UNIX_EPOCH};

    // ── Helpers ───────────────────────────────────────────────────────────

    fn nanos() -> u128 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    }

    fn tmp_dir(suffix: &str) -> String {
        format!(
            "{}/hold_host_m2_{}_{}",
            std::env::temp_dir().display(),
            suffix,
            nanos()
        )
    }

    fn write_two_entity_scene(suffix: &str) -> String {
        let root = tmp_dir(suffix);
        std::fs::create_dir_all(&root).unwrap();
        let path = format!("{root}/test.hsc");
        std::fs::write(
            &path,
            r#"
scene "TestScene"

entity "camera" {
  transform { pos 0 2 10 rot 0 0 0 1 scale 1 1 1 }
  camera { fov 60 near 0.1 far 1000 }
  active_camera
}

entity "player" {
  transform { pos 0 1 0 rot 0 0 0 1 scale 1 1 1 }
  stats { health 100 mana 25 attack 8 defense 4 speed 3 }
}
"#,
        )
        .unwrap();
        path
    }

    fn write_single_entity_scene(suffix: &str) -> String {
        let root = tmp_dir(suffix);
        std::fs::create_dir_all(&root).unwrap();
        let path = format!("{root}/test.hsc");
        std::fs::write(
            &path,
            r#"
scene "Single"
entity "cam" {
  transform { pos 0 5 0 rot 0 0 0 1 scale 1 1 1 }
  camera { fov 75 near 0.1 far 500 }
  active_camera
}
"#,
        )
        .unwrap();
        path
    }

    /// A scene whose particle emitter carries a `seed` past f32's 2^24
    /// exact-integer ceiling — a value a `Number`-backed field could not even
    /// represent, so the integer nudge path is exercised on a real hazard.
    fn write_particle_scene(suffix: &str) -> String {
        let root = tmp_dir(suffix);
        std::fs::create_dir_all(&root).unwrap();
        let path = format!("{root}/test.hsc");
        std::fs::write(
            &path,
            r#"
scene "Fx"
entity "sparks" {
  transform { pos 0 0 0 rot 0 0 0 1 scale 1 1 1 }
  particle_emitter { max_particles 20000001 seed 3000000003 }
}
"#,
        )
        .unwrap();
        path
    }

    // ── Open / basic ──────────────────────────────────────────────────────

    #[test]
    fn host_opens_and_populates_world() {
        let path = write_two_entity_scene("open");
        let host = EditorRuntimeHost::open_hsc(&path).unwrap();
        assert_eq!(host.session().scene().entities.len(), 2);
        assert_eq!(host.session().last_world_map().len(), 2);
    }

    // ── Selection cycling ─────────────────────────────────────────────────

    #[test]
    fn cycle_entity_wraps_around() {
        let path = write_two_entity_scene("cycle");
        let mut host = EditorRuntimeHost::open_hsc(&path).unwrap();

        // Starts at entity 0.
        assert_eq!(host.cursor_entity_idx, Some(0));

        // Forward: 0 → 1.
        host.execute(EditorCommand::SelectNextEntity).unwrap();
        assert_eq!(host.cursor_entity_idx, Some(1));

        // Forward: 1 → 0 (wraps).
        host.execute(EditorCommand::SelectNextEntity).unwrap();
        assert_eq!(host.cursor_entity_idx, Some(0));

        // Backward: 0 → 1 (wraps).
        host.execute(EditorCommand::SelectPrevEntity).unwrap();
        assert_eq!(host.cursor_entity_idx, Some(1));
    }

    #[test]
    fn cycle_entity_emits_selection_changed_event() {
        let path = write_two_entity_scene("ev");
        let mut host = EditorRuntimeHost::open_hsc(&path).unwrap();

        let events = host.execute(EditorCommand::SelectNextEntity).unwrap();
        assert!(
            events
                .iter()
                .any(|e| matches!(e, EditorEvent::SelectionChanged(_))),
            "SelectNextEntity must emit SelectionChanged"
        );
    }

    #[test]
    fn cycle_component_resets_field_index() {
        let path = write_two_entity_scene("comp");
        let mut host = EditorRuntimeHost::open_hsc(&path).unwrap();

        // Manually advance the field.
        host.cursor_field_idx = 2;
        host.execute(EditorCommand::SelectNextComponent).unwrap();
        assert_eq!(
            host.cursor_field_idx, 0,
            "component change must reset field to 0"
        );
    }

    #[test]
    fn select_component_command_supports_fieldless_components() {
        let path = write_two_entity_scene("fieldless_component");
        let mut host = EditorRuntimeHost::open_hsc(&path).unwrap();
        let camera_id = host
            .session()
            .scene()
            .entities
            .iter()
            .find(|entity| entity.name == "camera")
            .unwrap()
            .id;

        let events = host
            .execute(EditorCommand::SelectComponent {
                entity_id: camera_id,
                component_name: "ActiveCamera".into(),
            })
            .unwrap();

        assert!(events
            .iter()
            .any(|event| matches!(event, EditorEvent::SelectionChanged(_))));
        let cursor = host.cursor().unwrap();
        assert_eq!(cursor.entity_id, camera_id);
        assert_eq!(cursor.component_name, "ActiveCamera");
        assert!(cursor.field_name.is_empty());
    }

    #[test]
    fn select_field_command_targets_named_field() {
        let path = write_two_entity_scene("select_field");
        let mut host = EditorRuntimeHost::open_hsc(&path).unwrap();
        let player_id = host
            .session()
            .scene()
            .entities
            .iter()
            .find(|entity| entity.name == "player")
            .unwrap()
            .id;

        host.execute(EditorCommand::SelectField {
            entity_id: player_id,
            component_name: "Stats".into(),
            field_name: "health".into(),
        })
        .unwrap();

        let cursor = host.cursor().unwrap();
        assert_eq!(cursor.entity_id, player_id);
        assert_eq!(cursor.component_name, "Stats");
        assert_eq!(cursor.field_name, "health");
    }

    // ── Scalar edits ──────────────────────────────────────────────────────

    #[test]
    fn apply_delta_to_number_field_patches_world() {
        let path = write_two_entity_scene("delta");
        let mut host = EditorRuntimeHost::open_hsc(&path).unwrap();

        // Navigate to camera entity (idx 0), Camera component, fov_deg field.
        // The cursor starts at entity 0, component 0 (Transform), field 0 (position).
        // We need the Camera component and its fov_deg field.
        // Find camera entity.
        let cam_id = host
            .session()
            .scene()
            .entities
            .iter()
            .find(|e| e.name == "camera")
            .unwrap()
            .id;
        host.select_entity(cam_id).unwrap();
        let cam_idx = host
            .session()
            .scene()
            .entities
            .iter()
            .position(|e| e.id == cam_id)
            .unwrap();
        host.cursor_entity_idx = Some(cam_idx);

        // Find Camera component.
        let cam_comp_idx = host.session().scene().entities[cam_idx]
            .components
            .iter()
            .position(|c| c.type_name == "Camera")
            .unwrap();
        host.cursor_component_idx = cam_comp_idx;

        // Find fov_deg field.
        let fov_idx = host.session().scene().entities[cam_idx].components[cam_comp_idx]
            .fields
            .iter()
            .position(|f| f.name == "fov_deg")
            .unwrap();
        host.cursor_field_idx = fov_idx;

        let events = host.execute(EditorCommand::ApplyDelta(5.0)).unwrap();

        // FieldEdited must be present.
        assert!(
            events
                .iter()
                .any(|e| matches!(e, EditorEvent::FieldEdited { .. })),
            "ApplyDelta must emit FieldEdited"
        );

        // Runtime Camera fov must have changed.
        let runtime_cam = host.applied_entity(cam_id).unwrap();
        let camera = host.world().get::<Camera>(runtime_cam).unwrap();
        assert!(
            (camera.fov.to_degrees() - 65.0).abs() < 0.01,
            "fov_deg should be 60+5=65, got {}",
            camera.fov.to_degrees()
        );
    }

    /// The integer nudge path, end to end. A `seed` field is nudged and the
    /// value is checked in BOTH the editor document AND the live runtime world
    /// (`ParticleEmitter.seed`), because they travel different conversions: the
    /// initial open runs `apply_scene_document_to_world`, while each edit
    /// re-patches through `patch_particle_emitter_component`. Both must keep the
    /// seed exact — an f32 hop on either would corrupt it (Bug B, ADR 0005).
    ///
    /// The seed is a genuine f32 hazard (it, +1, and +2 all collapse to the
    /// same f32), so a `Number`-backed field could not distinguish these
    /// values at all; every assertion below is meaningful only because the
    /// field is Integer. The deltas exercise `round()` semantics: 1.0 moves by
    /// one, 0.1 is a no-op (rounds to 0), 0.6 moves by one (rounds away from
    /// zero — distinguishing `round()` from truncation).
    #[test]
    fn apply_delta_nudges_integer_seed_by_whole_units() {
        // Precondition: seed and its nudged neighbours are all f32 hazards.
        for v in [3_000_000_003u32, 3_000_000_004, 3_000_000_005] {
            assert_eq!(
                (v as f32) as u32,
                3_000_000_000,
                "seed neighbours must be f32-indistinguishable or this test proves nothing"
            );
        }

        let path = write_particle_scene("delta_seed");
        let mut host = EditorRuntimeHost::open_hsc(&path).unwrap();

        let ent_id = host
            .session()
            .scene()
            .entities
            .iter()
            .find(|e| e.name == "sparks")
            .unwrap()
            .id;
        host.select_entity(ent_id).unwrap();
        let ent_idx = host
            .session()
            .scene()
            .entities
            .iter()
            .position(|e| e.id == ent_id)
            .unwrap();
        host.cursor_entity_idx = Some(ent_idx);
        let comp_idx = host.session().scene().entities[ent_idx]
            .components
            .iter()
            .position(|c| c.type_name == "ParticleEmitter")
            .unwrap();
        host.cursor_component_idx = comp_idx;
        let seed_idx = host.session().scene().entities[ent_idx].components[comp_idx]
            .fields
            .iter()
            .position(|f| f.name == "seed")
            .unwrap();
        host.cursor_field_idx = seed_idx;

        let runtime = host.applied_entity(ent_id).unwrap();
        let world_seed = |host: &EditorRuntimeHost| {
            host.world().get::<ParticleEmitter>(runtime).unwrap().seed
        };
        let doc_seed = |host: &EditorRuntimeHost| {
            host.session()
                .scene()
                .get_value(ent_id, "ParticleEmitter", "seed")
                .cloned()
        };
        // max_particles (20_000_001, also an f32 hazard: odd value past 2^24)
        // rides the same two conversions; it must stay exact across a re-patch.
        let world_max = |host: &EditorRuntimeHost| {
            host.world()
                .get::<ParticleEmitter>(runtime)
                .unwrap()
                .max_particles
        };
        assert_eq!((20_000_001u32 as f32) as u32 % 2, 0, "max is an f32 hazard");

        // The authored seed parsed exactly (Integer, not a truncated f32), and
        // the initial world build (apply path) carried it through exactly.
        assert_eq!(doc_seed(&host), Some(InspectorValue::Integer(3_000_000_003)));
        assert_eq!(world_seed(&host), 3_000_000_003, "apply path must be exact");
        assert_eq!(world_max(&host), 20_000_001, "apply path max must be exact");

        // A whole-unit nudge moves the integer by exactly 1 — in the document
        // AND in the live world (patch path must be exact). The re-patch also
        // re-reads max_particles, which must survive unchanged and exact.
        host.execute(EditorCommand::ApplyDelta(1.0)).unwrap();
        assert_eq!(doc_seed(&host), Some(InspectorValue::Integer(3_000_000_004)));
        assert_eq!(world_seed(&host), 3_000_000_004, "patch path must be exact");
        assert_eq!(world_max(&host), 20_000_001, "patch path max must be exact");

        // A sub-half fine nudge rounds to zero — a no-op on a discrete field.
        host.execute(EditorCommand::ApplyDelta(0.1)).unwrap();
        assert_eq!(doc_seed(&host), Some(InspectorValue::Integer(3_000_000_004)));

        // A delta in [0.5, 1.0) rounds AWAY from zero to one — this pins
        // `round()`, since truncation would make it a no-op instead.
        host.execute(EditorCommand::ApplyDelta(0.6)).unwrap();
        assert_eq!(doc_seed(&host), Some(InspectorValue::Integer(3_000_000_005)));
        assert_eq!(world_seed(&host), 3_000_000_005, "patch path must be exact");
    }

    // ── Vec3 axis edits ───────────────────────────────────────────────────

    #[test]
    fn apply_delta_to_vec3_field_uses_active_axis() {
        let path = write_single_entity_scene("vec3");
        let mut host = EditorRuntimeHost::open_hsc(&path).unwrap();

        // Cursor starts at entity 0 (cam), component 0 (Transform), field 0 (position).
        host.cursor_axis = Some(EditorAxis::Y);

        let events = host.execute(EditorCommand::ApplyDelta(3.0)).unwrap();
        assert!(events
            .iter()
            .any(|e| matches!(e, EditorEvent::FieldEdited { .. })));

        // Scene doc position.y should be 5 + 3 = 8.
        let cam_id = host.session().scene().entities[0].id;
        let pos = host
            .session()
            .scene()
            .get_value(cam_id, "Transform", "position")
            .unwrap();
        if let InspectorValue::Vec3(arr) = pos {
            assert!(
                (arr[1] - 8.0).abs() < 0.01,
                "Y should be 8.0, got {}",
                arr[1]
            );
        } else {
            panic!("expected Vec3");
        }
    }

    // ── Text field rejection ──────────────────────────────────────────────

    // (Stats and Camera don't have Text fields in the current scene format,
    //  but we can synthesise an InspectorValue::Text to confirm the error path.)

    // ── Play / pause / resume / stop ─────────────────────────────────────

    #[test]
    fn apply_delta_rejects_text_field_using_field_name() {
        let path = write_single_entity_scene("text_field");
        let mut host = EditorRuntimeHost::open_hsc(&path).unwrap();

        host.session.scene_mut().entities[0].components[0]
            .fields
            .push(crate::editor::InspectorField {
                name: "label".into(),
                value: InspectorValue::Text("camera-root".into()),
            });
        host.cursor_entity_idx = Some(0);
        host.cursor_component_idx = 0;
        host.cursor_field_idx = host.session.scene().entities[0].components[0].fields.len() - 1;

        let err = host.execute(EditorCommand::ApplyDelta(1.0)).unwrap_err();
        match err {
            EditorCommandError::UnsupportedFieldKind { field } => {
                assert_eq!(field, "label");
            }
            other => panic!("expected UnsupportedFieldKind, got {other:?}"),
        }
    }

    #[test]
    fn play_pause_resume_stop_full_cycle() {
        let path = write_two_entity_scene("play");
        let mut host = EditorRuntimeHost::open_hsc(&path).unwrap();

        // Enter play.
        let evs = host.execute(EditorCommand::EnterPlayMode).unwrap();
        assert!(evs
            .iter()
            .any(|e| matches!(e, EditorEvent::PlayModeChanged(EditorPlayState::Playing))));
        assert_eq!(host.play_state(), EditorPlayState::Playing);

        // Pause.
        let evs = host.execute(EditorCommand::PausePlayMode).unwrap();
        assert!(evs
            .iter()
            .any(|e| matches!(e, EditorEvent::PlayModeChanged(EditorPlayState::Paused))));
        assert_eq!(host.play_state(), EditorPlayState::Paused);

        // Resume.
        let evs = host.execute(EditorCommand::ResumePlayMode).unwrap();
        assert!(evs
            .iter()
            .any(|e| matches!(e, EditorEvent::PlayModeChanged(EditorPlayState::Playing))));
        assert_eq!(host.play_state(), EditorPlayState::Playing);

        // Stop.
        let evs = host.execute(EditorCommand::StopPlayMode).unwrap();
        assert!(evs
            .iter()
            .any(|e| matches!(e, EditorEvent::PlayModeChanged(EditorPlayState::Editing))));
        assert_eq!(host.play_state(), EditorPlayState::Editing);
    }

    #[test]
    fn paused_transient_edits_are_discarded_on_stop() {
        let path = write_single_entity_scene("paused");
        let mut host = EditorRuntimeHost::open_hsc(&path).unwrap();

        let cam_id = host.session().scene().entities[0].id;
        let orig_fov = {
            if let Some(InspectorValue::Number(n)) = host
                .session()
                .scene()
                .get_value(cam_id, "Camera", "fov_deg")
            {
                *n
            } else {
                75.0
            }
        };

        host.execute(EditorCommand::EnterPlayMode).unwrap();
        host.execute(EditorCommand::PausePlayMode).unwrap();

        // Apply a transient delta to fov.
        // Navigate to Camera.fov_deg.
        let cam_idx = 0usize;
        host.cursor_entity_idx = Some(cam_idx);
        let cam_comp_idx = host.session().scene().entities[cam_idx]
            .components
            .iter()
            .position(|c| c.type_name == "Camera")
            .unwrap();
        host.cursor_component_idx = cam_comp_idx;
        let fov_idx = host.session().scene().entities[cam_idx].components[cam_comp_idx]
            .fields
            .iter()
            .position(|f| f.name == "fov_deg")
            .unwrap();
        host.cursor_field_idx = fov_idx;
        host.execute(EditorCommand::ApplyDelta(20.0)).unwrap();

        // Stop — authored snapshot must be restored.
        host.execute(EditorCommand::StopPlayMode).unwrap();

        let restored_fov = host
            .session()
            .scene()
            .get_value(cam_id, "Camera", "fov_deg")
            .cloned();
        assert_eq!(
            restored_fov,
            Some(InspectorValue::Number(orig_fov)),
            "fov_deg must revert to {} after stop",
            orig_fov
        );
    }

    #[test]
    fn persistent_dirty_survives_play_cycle() {
        let path = write_single_entity_scene("dirty");
        let mut host = EditorRuntimeHost::open_hsc(&path).unwrap();

        // Dirty the scene in Editing.
        host.cursor_entity_idx = Some(0);
        host.cursor_component_idx = 0; // Transform
        host.cursor_field_idx = 0; // position
        host.cursor_axis = Some(EditorAxis::X);
        host.execute(EditorCommand::ApplyDelta(1.0)).unwrap();
        assert!(host.is_dirty(), "should be dirty after edit");

        host.execute(EditorCommand::EnterPlayMode).unwrap();
        assert!(!host.is_dirty(), "not dirty during play (playing state)");

        host.execute(EditorCommand::StopPlayMode).unwrap();
        assert!(host.is_dirty(), "dirty flag must survive a play cycle");
    }

    #[test]
    fn runtime_slice_authoring_commands_mutate_document_and_undo() {
        let path = write_single_entity_scene("runtime_slice_authoring");
        let mut host = EditorRuntimeHost::open_hsc(&path).unwrap();

        let terrain_events = host
            .execute(EditorCommand::SetTerrainSettings {
                size_meters: Vec3::new(2048.0, 384.0, 2048.0),
                chunk_size_meters: 128,
                height_scale_meters: 96.0,
            })
            .unwrap();
        assert!(terrain_events.iter().any(|event| matches!(
            event,
            EditorEvent::RuntimeSliceUpdated { path } if path == "terrain"
        )));
        assert!(host.is_dirty());
        assert!(host.can_undo());
        assert_eq!(
            host.session().scene().runtime_slice.terrain.size_meters,
            Vec3::new(2048.0, 384.0, 2048.0)
        );

        let route_events = host
            .execute(EditorCommand::UpsertPatrolRoute {
                name: "beach_patrol".into(),
                points: vec![Vec3::new(4.0, 2.0, 8.0), Vec3::new(8.0, 2.0, 16.0)],
                looped: true,
            })
            .unwrap();
        assert!(route_events.iter().any(|event| matches!(
            event,
            EditorEvent::RuntimeSliceUpdated { path } if path == "patrol_routes.beach_patrol"
        )));
        assert_eq!(host.session().scene().runtime_slice.patrol_routes.len(), 1);

        let undo_events = host.execute(EditorCommand::Undo).unwrap();
        assert!(undo_events
            .iter()
            .any(|event| matches!(event, EditorEvent::UndoApplied)));
        assert!(host
            .session()
            .scene()
            .runtime_slice
            .patrol_routes
            .is_empty());
        assert_eq!(
            host.session().scene().runtime_slice.terrain.size_meters,
            Vec3::new(2048.0, 384.0, 2048.0)
        );

        host.execute(EditorCommand::Undo).unwrap();
        assert_eq!(
            host.session().scene().runtime_slice.terrain.size_meters,
            Vec3::new(1024.0, 256.0, 1024.0)
        );
    }

    #[test]
    fn runtime_slice_remove_unknown_returns_typed_error() {
        let path = write_single_entity_scene("runtime_slice_remove_unknown");
        let mut host = EditorRuntimeHost::open_hsc(&path).unwrap();

        let err = host
            .execute(EditorCommand::RemoveVehicleSpawner {
                name: "missing_jeep".into(),
            })
            .unwrap_err();
        match err {
            EditorCommandError::MissingRuntimeSliceItem { kind, name } => {
                assert_eq!(kind, "vehicle spawner");
                assert_eq!(name, "missing_jeep");
            }
            other => panic!("expected MissingRuntimeSliceItem, got {other:?}"),
        }
    }

    #[test]
    fn runtime_slice_authoring_rejected_during_play() {
        let path = write_single_entity_scene("runtime_slice_play_reject");
        let mut host = EditorRuntimeHost::open_hsc(&path).unwrap();

        host.execute(EditorCommand::EnterPlayMode).unwrap();
        let err = host
            .execute(EditorCommand::SetTerrainSettings {
                size_meters: Vec3::new(512.0, 128.0, 512.0),
                chunk_size_meters: 32,
                height_scale_meters: 32.0,
            })
            .unwrap_err();
        assert!(matches!(
            err,
            EditorCommandError::UnsupportedCommandState {
                command: "SetTerrainSettings",
                state: EditorPlayState::Playing
            }
        ));
    }

    #[test]
    fn undo_redo_restore_authored_scene_and_world() {
        let path = write_single_entity_scene("undo_redo");
        let mut host = EditorRuntimeHost::open_hsc(&path).unwrap();
        let cam_id = host.session().scene().entities[0].id;

        host.cursor_entity_idx = Some(0);
        host.cursor_component_idx = 0;
        host.cursor_field_idx = 0;
        host.cursor_axis = Some(EditorAxis::Y);

        host.execute(EditorCommand::ApplyDelta(2.0)).unwrap();
        assert!(
            host.can_undo(),
            "an authored edit should create undo history"
        );
        assert!(
            !host.can_redo(),
            "redo should stay empty until an undo happens"
        );
        assert!(host.is_dirty(), "authored edit should dirty the scene");
        assert_eq!(
            host.session()
                .scene()
                .get_value(cam_id, "Transform", "position"),
            Some(&InspectorValue::Vec3([0.0, 7.0, 0.0]))
        );

        let undo_events = host.execute(EditorCommand::Undo).unwrap();
        assert!(undo_events
            .iter()
            .any(|event| matches!(event, EditorEvent::UndoApplied)));
        assert!(undo_events
            .iter()
            .any(|event| matches!(event, EditorEvent::WorldRebuilt)));
        assert_eq!(
            host.session()
                .scene()
                .get_value(cam_id, "Transform", "position"),
            Some(&InspectorValue::Vec3([0.0, 5.0, 0.0]))
        );
        let runtime_cam = host.applied_entity(cam_id).unwrap();
        let transform = host.world().get::<Transform>(runtime_cam).unwrap();
        assert_eq!(transform.position, Vec3::new(0.0, 5.0, 0.0));
        assert_eq!(host.cursor().unwrap().axis, Some(EditorAxis::Y));
        assert!(
            !host.is_dirty(),
            "undo should restore the pre-edit dirty flag"
        );
        assert!(
            !host.can_undo(),
            "single edit should consume the only undo entry"
        );
        assert!(host.can_redo(), "redo should become available after undo");

        let redo_events = host.execute(EditorCommand::Redo).unwrap();
        assert!(redo_events
            .iter()
            .any(|event| matches!(event, EditorEvent::RedoApplied)));
        assert!(redo_events
            .iter()
            .any(|event| matches!(event, EditorEvent::WorldRebuilt)));
        assert_eq!(
            host.session()
                .scene()
                .get_value(cam_id, "Transform", "position"),
            Some(&InspectorValue::Vec3([0.0, 7.0, 0.0]))
        );
        let runtime_cam = host.applied_entity(cam_id).unwrap();
        let transform = host.world().get::<Transform>(runtime_cam).unwrap();
        assert_eq!(transform.position, Vec3::new(0.0, 7.0, 0.0));
        assert_eq!(host.cursor().unwrap().axis, Some(EditorAxis::Y));
        assert!(host.is_dirty(), "redo should restore the edited dirty flag");
        assert!(host.can_undo(), "undo should be available again after redo");
        assert!(
            !host.can_redo(),
            "redo stack should be exhausted after redo"
        );
    }

    #[test]
    fn undo_without_history_returns_typed_error() {
        let path = write_single_entity_scene("undo_empty");
        let mut host = EditorRuntimeHost::open_hsc(&path).unwrap();

        let err = host.execute(EditorCommand::Undo).unwrap_err();
        match err {
            EditorCommandError::HistoryEmpty { command } => assert_eq!(command, "Undo"),
            other => panic!("expected HistoryEmpty for Undo, got {other:?}"),
        }
    }

    // ── Patch / rebuild ───────────────────────────────────────────────────

    #[test]
    fn force_patch_emits_patch_applied() {
        let path = write_single_entity_scene("patch");
        let mut host = EditorRuntimeHost::open_hsc(&path).unwrap();

        let evs = host.execute(EditorCommand::ForcePatch).unwrap();
        assert!(
            evs.iter().any(|e| matches!(e, EditorEvent::PatchApplied)),
            "ForcePatch on a synced world must emit PatchApplied"
        );
    }

    #[test]
    fn force_rebuild_emits_world_rebuilt() {
        let path = write_two_entity_scene("rebuild");
        let mut host = EditorRuntimeHost::open_hsc(&path).unwrap();

        let evs = host.execute(EditorCommand::ForceRebuild).unwrap();
        assert!(
            evs.iter().any(|e| matches!(e, EditorEvent::WorldRebuilt)),
            "ForceRebuild must emit WorldRebuilt"
        );
    }

    // ── Selection preservation across rebuild ─────────────────────────────

    #[test]
    fn cursor_restored_by_name_after_rebuild() {
        let path = write_two_entity_scene("restore");
        let mut host = EditorRuntimeHost::open_hsc(&path).unwrap();

        // Navigate to player entity, Stats component.
        let player_idx = host
            .session()
            .scene()
            .entities
            .iter()
            .position(|e| e.name == "player")
            .unwrap();
        host.cursor_entity_idx = Some(player_idx);
        let stats_comp_idx = host.session().scene().entities[player_idx]
            .components
            .iter()
            .position(|c| c.type_name == "Stats")
            .unwrap();
        host.cursor_component_idx = stats_comp_idx;

        let player_id = host.session().scene().entities[player_idx].id;
        host.select_entity(player_id).unwrap();

        // Record component/field names.
        let comp_name = host.session().scene().entities[player_idx].components[stats_comp_idx]
            .type_name
            .clone();
        let field_name = host.session().scene().entities[player_idx].components[stats_comp_idx]
            .fields[0]
            .name
            .clone();

        host.execute(EditorCommand::ForceRebuild).unwrap();

        let cursor = host.cursor().unwrap();
        assert_eq!(cursor.entity_id, player_id);
        assert_eq!(cursor.component_name, comp_name);
        assert_eq!(cursor.field_name, field_name);
    }

    // ── UnsupportedCommandState errors ────────────────────────────────────

    #[test]
    fn save_rejected_during_play() {
        let path = write_single_entity_scene("save_play");
        let mut host = EditorRuntimeHost::open_hsc(&path).unwrap();

        host.execute(EditorCommand::EnterPlayMode).unwrap();
        let err = host.execute(EditorCommand::Save).unwrap_err();
        assert!(
            matches!(err, EditorCommandError::UnsupportedCommandState { .. }),
            "Save must be rejected in Playing state"
        );
    }

    #[test]
    fn apply_delta_rejected_during_play() {
        let path = write_single_entity_scene("delta_play");
        let mut host = EditorRuntimeHost::open_hsc(&path).unwrap();

        host.execute(EditorCommand::EnterPlayMode).unwrap();
        let err = host.execute(EditorCommand::ApplyDelta(1.0)).unwrap_err();
        assert!(
            matches!(err, EditorCommandError::UnsupportedCommandState { .. }),
            "ApplyDelta must be rejected in Playing state"
        );
    }

    // ── Existing host tests (smoke) ────────────────────────────────────────

    #[test]
    fn reload_source_restores_scene_and_clears_dirty() {
        let path = write_single_entity_scene("reload");
        let mut host = EditorRuntimeHost::open_hsc(&path).unwrap();
        let cursor = host.cursor().unwrap();

        host.execute(EditorCommand::ApplyDelta(2.0)).unwrap();
        assert!(host.is_dirty());

        let events = host.execute(EditorCommand::ReloadSource).unwrap();
        assert!(events
            .iter()
            .any(|event| matches!(event, EditorEvent::Reloaded)));
        assert!(events
            .iter()
            .any(|event| matches!(event, EditorEvent::WorldRebuilt)));
        assert!(!host.is_dirty());
        assert!(!host.world_requires_rebuild());
        assert_eq!(
            host.session()
                .scene()
                .get_value(cursor.entity_id, "Transform", "position"),
            Some(&InspectorValue::Vec3([0.0, 5.0, 0.0]))
        );
    }

    #[test]
    fn reload_source_preserves_project_context() {
        let path = write_single_entity_scene("reload_project");
        let project = Project::new("host_project", tmp_dir("reload_project_root"));
        let mut host = EditorRuntimeHost::open_hsc_with_project(&path, project.clone()).unwrap();

        host.execute(EditorCommand::ApplyDelta(2.0)).unwrap();
        let events = host.execute(EditorCommand::ReloadSource).unwrap();

        assert!(events
            .iter()
            .any(|event| matches!(event, EditorEvent::Reloaded)));
        assert_eq!(host.project(), Some(&project));
        assert_eq!(host.session().source_path(), Some(Path::new(&path)));
        assert!(!host.is_dirty());
    }

    #[test]
    fn stale_force_patch_marks_world_as_rebuild_required() {
        let path = write_single_entity_scene("stale_patch");
        let mut host = EditorRuntimeHost::open_hsc(&path).unwrap();
        let runtime_entity = host.selected_runtime_entity().unwrap();

        host.world_mut().despawn(runtime_entity);
        let events = host.execute(EditorCommand::ForcePatch).unwrap();

        assert!(events
            .iter()
            .any(|event| matches!(event, EditorEvent::PatchRequiresRebuild { .. })));
        assert!(host.world_requires_rebuild());
    }

    #[test]
    fn protocol_message_executes_and_roundtrips_to_encoded_events() {
        let path = write_two_entity_scene("protocol");
        let mut host = EditorRuntimeHost::open_hsc(&path).unwrap();

        let encoded_events = host
            .execute_protocol_message("cmd|SelectNextEntity")
            .unwrap();
        assert_eq!(encoded_events.len(), 1);

        let decoded_event = crate::editor::protocol::decode_event(&encoded_events[0]).unwrap();
        match decoded_event {
            EditorEvent::SelectionChanged(cursor) => {
                assert_eq!(cursor.entity_id, host.cursor().unwrap().entity_id);
            }
            other => panic!("expected SelectionChanged, got {other:?}"),
        }
    }

    #[test]
    fn host_opens_scene_and_rebuilds_runtime_world() {
        let path = write_two_entity_scene("host_open");
        let mut host = EditorRuntimeHost::open_hsc(&path).unwrap();
        let camera_id = host.select_first_with_component("Camera").unwrap();
        let runtime_camera = host.applied_entity(camera_id).unwrap();

        assert!(host.world().has_component::<Transform>(runtime_camera));
        assert!(host.world().has_component::<Camera>(runtime_camera));
        assert!(host.world().has_component::<ActiveCamera>(runtime_camera));
    }

    #[test]
    fn host_mutates_and_patches_live_world() {
        let path = write_two_entity_scene("host_patch");
        let mut host = EditorRuntimeHost::open_hsc(&path).unwrap();
        let camera_id = host.select_first_with_component("Camera").unwrap();
        let runtime_camera = host.applied_entity(camera_id).unwrap();

        host.set_selected_value("Camera", "fov_deg", InspectorValue::Number(75.0))
            .unwrap();
        host.set_selected_value(
            "Transform",
            "position",
            InspectorValue::Vec3([3.0, 5.0, 9.0]),
        )
        .unwrap();
        host.patch_world().unwrap();

        assert_eq!(host.applied_entity(camera_id).unwrap(), runtime_camera);
        let transform = host.world().get::<Transform>(runtime_camera).unwrap();
        assert_eq!(transform.position, Vec3::new(3.0, 5.0, 9.0));
        let camera = host.world().get::<Camera>(runtime_camera).unwrap();
        assert!((camera.fov.to_degrees() - 75.0).abs() < 0.001);
    }

    #[test]
    fn host_play_pause_stop_restores_authored_scene() {
        let path = write_two_entity_scene("host_play");
        let mut host = EditorRuntimeHost::open_hsc(&path).unwrap();
        let camera_id = host.select_first_with_component("Camera").unwrap();

        host.enter_play_mode().unwrap();
        assert_eq!(host.play_state(), EditorPlayState::Playing);
        host.set_selected_value("Camera", "fov_deg", InspectorValue::Number(95.0))
            .unwrap();
        host.pause_play_mode().unwrap();
        assert_eq!(host.play_state(), EditorPlayState::Paused);

        host.stop_play_mode().unwrap();
        assert_eq!(host.play_state(), EditorPlayState::Editing);
        assert_eq!(
            host.session()
                .scene()
                .get_value(camera_id, "Camera", "fov_deg"),
            Some(&InspectorValue::Number(60.0))
        );
    }

    // ── apply_dispatch_summary tests (M7.4.2 wire-up) ──────────────────────

    /// Empty summary is a no-op: nothing pushed to the status
    /// panel, report is all zeros, `has_activity` returns false.
    /// Pillar 2 (iteration speed): per-frame dispatch with no
    /// pending reloads must pay zero cost on the editor side.
    #[test]
    fn apply_dispatch_summary_empty_is_no_op() {
        let path = write_two_entity_scene("apply_empty");
        let mut host = EditorRuntimeHost::open_hsc(&path).unwrap();
        let entries_before = host.hot_reload_status().len();

        let report = host.apply_dispatch_summary(crate::hot_reload::DispatchSummary::default());

        assert_eq!(host.hot_reload_status().len(), entries_before);
        assert_eq!(report, DispatchApplyReport::default());
        assert!(!report.has_activity());
    }

    /// Asset reloads (Texture, Mesh, etc.) route to the status
    /// panel as Applied rows. The kind label uses
    /// `AssetKind::as_str()` so the panel matches the editor
    /// protocol's wire-format labels (ADR 0004 Q13).
    #[test]
    fn apply_dispatch_summary_routes_asset_reloads_to_status_panel() {
        let path = write_two_entity_scene("apply_assets");
        let mut host = EditorRuntimeHost::open_hsc(&path).unwrap();
        let entries_before = host.hot_reload_status().len();

        let mut summary = crate::hot_reload::DispatchSummary::default();
        summary.applied.push(crate::assets::AssetInvalidated {
            kind: crate::assets::AssetKind::Texture,
            handle_index: 7,
            logical_name: "textures/crate".to_string(),
        });
        summary.applied.push(crate::assets::AssetInvalidated {
            kind: crate::assets::AssetKind::Shader,
            handle_index: 2,
            logical_name: "shaders/lit".to_string(),
        });

        let report = host.apply_dispatch_summary(summary);

        assert_eq!(report.assets_applied, 2);
        assert!(report.has_activity());
        assert_eq!(host.hot_reload_status().len(), entries_before + 2);
    }

    /// Compile errors surface as Failed rows. The previous asset
    /// stays live (the dispatcher already preserved it); this
    /// surface only records the diagnostic for the panel.
    #[test]
    fn apply_dispatch_summary_routes_compile_errors_as_failed_rows() {
        let path = write_two_entity_scene("apply_errors");
        let mut host = EditorRuntimeHost::open_hsc(&path).unwrap();
        assert!(host.hot_reload_status().last_failure().is_none());

        let mut summary = crate::hot_reload::DispatchSummary::default();
        summary.errors.push(crate::hot_reload::CompileErr {
            logical_name: "shaders/broken".to_string(),
            kind: crate::assets::CompiledAssetKind::Shader,
            error: "shader source missing fragment section".to_string(),
            generation: crate::hot_reload::Generation(3),
        });

        let report = host.apply_dispatch_summary(summary);

        assert_eq!(report.errors_recorded, 1);
        assert_eq!(report.assets_applied, 0);
        // The status panel surfaced the failure: the most recent
        // failure entry carries the logical name + error message
        // of the failed reload.
        let last = host
            .hot_reload_status()
            .last_failure()
            .expect("failure recorded");
        assert_eq!(last.outcome.logical_name(), "shaders/broken");
        assert_eq!(last.outcome.kind(), "Shader");
        assert!(last.outcome.is_failure());
    }

    /// Scene reloads route to `EditorSceneSession::reload_scene_document`
    /// against the host's owned World. The diff path runs;
    /// runtime entity identity is preserved across the reload.
    /// The status panel records a single "Scene" event.
    #[test]
    fn apply_dispatch_summary_drives_scene_diff_against_host_world() {
        let path = write_two_entity_scene("apply_scene_diff");
        let mut host = EditorRuntimeHost::open_hsc(&path).unwrap();
        let camera_id = host.select_first_with_component("Camera").unwrap();
        let runtime_camera_before = host.applied_entity(camera_id).unwrap();
        let entries_before = host.hot_reload_status().len();

        // Build a new SceneDocument with a tweaked camera fov.
        // Apply via the dispatcher-summary path; the host should
        // run reload_scene_document and update the live World.
        let mut new_doc = host.session().scene().clone();
        new_doc.set_value(
            camera_id,
            "Camera",
            "fov_deg",
            InspectorValue::Number(85.0),
        );

        let mut summary = crate::hot_reload::DispatchSummary::default();
        summary
            .scenes_reloaded
            .push(crate::hot_reload::SceneReloadResult {
                logical_name: "levels/test".to_string(),
                source_path: std::path::PathBuf::from(&path),
                document: new_doc,
            });

        let report = host.apply_dispatch_summary(summary);

        assert_eq!(report.scenes_diffed, 1);
        assert_eq!(report.scenes_no_change, 0);
        assert_eq!(report.scenes_rebuilt, 0);
        assert_eq!(report.scenes_failed, 0);
        // `patched` counts retained-and-re-patched entities; the
        // two-entity test scene has no adds/removes so both
        // entities count as patched.
        assert!(report.scene_entities_patched >= 1);
        assert_eq!(report.scene_entities_added, 0);
        assert_eq!(report.scene_entities_removed, 0);

        // Runtime Entity identity preserved across the reload.
        let runtime_camera_after = host.applied_entity(camera_id).unwrap();
        assert_eq!(runtime_camera_before, runtime_camera_after);

        // The new fov landed in the live World.
        let camera = host.world().get::<Camera>(runtime_camera_after).unwrap();
        assert!((camera.fov.to_degrees() - 85.0).abs() < 0.001);

        // Status panel got exactly one new entry.
        assert_eq!(host.hot_reload_status().len(), entries_before + 1);
    }

    /// Scene reload where the new document equals the previous
    /// authored snapshot reports `scenes_no_change` (the
    /// zero-cost fast path) and still emits a status-panel
    /// event so developers see "save fired but content was
    /// already current."
    #[test]
    fn apply_dispatch_summary_records_scene_no_change_outcome() {
        let path = write_two_entity_scene("apply_scene_no_change");
        let mut host = EditorRuntimeHost::open_hsc(&path).unwrap();
        let entries_before = host.hot_reload_status().len();

        let unchanged_doc = host.session().scene().clone();
        let mut summary = crate::hot_reload::DispatchSummary::default();
        summary
            .scenes_reloaded
            .push(crate::hot_reload::SceneReloadResult {
                logical_name: "levels/test".to_string(),
                source_path: std::path::PathBuf::from(&path),
                document: unchanged_doc,
            });

        let report = host.apply_dispatch_summary(summary);

        assert_eq!(report.scenes_no_change, 1);
        assert_eq!(report.scenes_diffed, 0);
        assert_eq!(report.total_scenes(), 1);
        // Status panel records the no-change with the
        // disambiguating label.
        assert_eq!(host.hot_reload_status().len(), entries_before + 1);
    }

    /// Mixed summary: assets + scenes + errors + stale results
    /// in the same dispatch all route to their respective
    /// destinations and the report aggregates counts correctly.
    /// This is the realistic shape of a frame-boundary dispatch
    /// after a developer's burst-save sweep.
    #[test]
    fn apply_dispatch_summary_aggregates_mixed_outcomes() {
        let path = write_two_entity_scene("apply_mixed");
        let mut host = EditorRuntimeHost::open_hsc(&path).unwrap();

        let mut summary = crate::hot_reload::DispatchSummary::default();
        summary.applied.push(crate::assets::AssetInvalidated {
            kind: crate::assets::AssetKind::Mesh,
            handle_index: 1,
            logical_name: "meshes/crate".to_string(),
        });
        summary.errors.push(crate::hot_reload::CompileErr {
            logical_name: "materials/grass".to_string(),
            kind: crate::assets::CompiledAssetKind::Material,
            error: "missing albedo dependency".to_string(),
            generation: crate::hot_reload::Generation(1),
        });
        summary
            .stale
            .push(crate::hot_reload::DroppedResult {
                logical_name: "textures/dirt".to_string(),
                generation: crate::hot_reload::Generation(2),
                kind: crate::assets::CompiledAssetKind::Texture,
            });

        let report = host.apply_dispatch_summary(summary);

        assert_eq!(report.assets_applied, 1);
        assert_eq!(report.errors_recorded, 1);
        assert_eq!(report.stale_dropped, 1);
        assert_eq!(report.scenes_diffed, 0);
        assert!(report.has_activity());
    }

    /// `DispatchApplyReport::has_activity` returns false only
    /// when every counter is zero; any non-zero category flips
    /// it to true. Pin the contract because per-frame logging
    /// often gates on this predicate to avoid log spam on
    /// idle frames.
    #[test]
    fn dispatch_apply_report_has_activity_predicate_is_correct() {
        let mut report = DispatchApplyReport::default();
        assert!(!report.has_activity());
        report.assets_applied = 1;
        assert!(report.has_activity());

        let mut report = DispatchApplyReport::default();
        report.scenes_diffed = 1;
        assert!(report.has_activity());

        let mut report = DispatchApplyReport::default();
        // stale_dropped is telemetry only — does NOT count as
        // user-visible activity. The panel never surfaces stale
        // results.
        report.stale_dropped = 5;
        assert!(!report.has_activity());
    }

    // ── M11: autosave contract ───────────────────────────────────────────
    //
    // Autosave is a safety net, not a save. Three properties are load-bearing
    // and each has a test: it never writes the source, it never clears the
    // dirty flag, and it never runs outside Editing.

    /// Make an authored edit so the host is dirty and the revision advances.
    fn dirty_the_host(host: &mut EditorRuntimeHost) {
        let before = host.edit_revision();
        host.execute(EditorCommand::SetTerrainSettings {
            size_meters: Vec3::new(2048.0, 384.0, 2048.0),
            chunk_size_meters: 128,
            height_scale_meters: 96.0,
        })
        .unwrap();
        assert!(
            host.is_dirty(),
            "test setup: host should be dirty after an edit"
        );
        assert_ne!(
            host.edit_revision(),
            before,
            "an authored edit must bump edit_revision"
        );
    }

    /// Block until a file written *after* this call is guaranteed a strictly
    /// greater mtime than `reference`'s.
    ///
    /// The recovery freshness check ([`EditorRuntimeHost::pending_recovery`])
    /// compares source and sidecar mtimes with `<=`. Filesystem timestamp
    /// granularity is coarse on some Windows configurations, so two writes a
    /// few hundred microseconds apart can share an mtime — making a
    /// genuinely-newer sidecar look stale and flaking any test that writes the
    /// source and sidecar in quick succession (worse under parallel I/O load).
    /// Rather than sleep a guessed fixed amount, poll a probe file against the
    /// *actual* filesystem clock (which can be coarser than `SystemTime::now`)
    /// until it reports a strictly greater mtime; time only moves forward, so
    /// any later write is then also strictly newer than `reference`.
    fn wait_for_fresh_mtime_after(reference: impl AsRef<Path>) {
        let reference = reference.as_ref();
        let ref_mtime = std::fs::metadata(reference)
            .and_then(|m| m.modified())
            .expect("reference file must exist and expose an mtime");
        let probe = {
            let mut name = reference
                .file_name()
                .expect("reference path must have a file name")
                .to_os_string();
            name.push(".freshprobe");
            reference.with_file_name(name)
        };
        // ~5s hang-guard only; crossing one timestamp tick takes at most a few
        // ms, so the loop returns almost immediately in practice.
        for _ in 0..5000 {
            std::fs::write(&probe, b"probe").unwrap();
            let probe_mtime = std::fs::metadata(&probe).and_then(|m| m.modified()).ok();
            let _ = std::fs::remove_file(&probe);
            if probe_mtime.is_some_and(|t| t > ref_mtime) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }

    /// Autosave writes the recovery sidecar and leaves the author's source
    /// file byte-for-byte untouched. Overwriting the source with in-progress
    /// state would destroy the user's last clean save.
    #[test]
    fn autosave_writes_recovery_sidecar_and_never_touches_the_source() {
        let path = write_single_entity_scene("autosave_sidecar");
        let source_before = std::fs::read(&path).unwrap();
        let mut host = EditorRuntimeHost::open_hsc(&path).unwrap();
        dirty_the_host(&mut host);

        let written = host.autosave_to_recovery().unwrap().expect("should autosave");
        assert!(written.exists(), "recovery file must exist");
        assert_eq!(
            std::fs::read(&path).unwrap(),
            source_before,
            "autosave must NOT modify the source scene"
        );
        // The recovery file is the full-fidelity .hscene document.
        let text = std::fs::read_to_string(&written).unwrap();
        assert!(text.starts_with(super::super::HSCENE_DOC_HEADER_V2));
    }

    /// Autosave must not clear the dirty flag: the work is still unsaved
    /// relative to the source, and the user must still press Save.
    #[test]
    fn autosave_does_not_clear_the_dirty_flag() {
        let path = write_single_entity_scene("autosave_dirty");
        let mut host = EditorRuntimeHost::open_hsc(&path).unwrap();
        dirty_the_host(&mut host);

        host.autosave_to_recovery().unwrap();
        assert!(
            host.is_dirty(),
            "autosave is a safety net, not a save — dirty must persist"
        );
    }

    /// During play/replay `session.scene()` is the live mutated copy.
    /// Checkpointing it would capture transient simulation state as authored
    /// work, so autosave is a no-op outside Editing.
    #[test]
    fn autosave_is_a_noop_during_play_mode() {
        let path = write_single_entity_scene("autosave_play");
        let mut host = EditorRuntimeHost::open_hsc(&path).unwrap();
        dirty_the_host(&mut host);
        // Save first so entering play is clean, then re-dirty is impossible in
        // play; we just assert the guard rejects the write.
        host.execute(EditorCommand::EnterPlayMode).unwrap();
        assert_eq!(host.play_state(), EditorPlayState::Playing);

        assert!(
            host.autosave_to_recovery().unwrap().is_none(),
            "autosave must not run during play mode"
        );
    }

    /// An untitled scene has no stable identity to recover into.
    #[test]
    fn autosave_skips_untitled_scene() {
        let scene = SceneDocument {
            name: "Untitled".into(),
            runtime_slice: Default::default(),
            entities: Vec::new(),
        };
        let host = EditorRuntimeHost::new(EditorSceneSession::new(scene)).unwrap();
        assert!(host.recovery_path().is_none());
        assert!(host.autosave_to_recovery().unwrap().is_none());
    }

    /// An explicit save deletes the recovery file: disk now matches memory, so
    /// a leftover sidecar would falsely look like crash evidence on startup.
    #[test]
    fn explicit_save_clears_the_recovery_file() {
        let path = write_single_entity_scene("autosave_cleared");
        let mut host = EditorRuntimeHost::open_hsc(&path).unwrap();
        dirty_the_host(&mut host);

        let recovery = host.autosave_to_recovery().unwrap().unwrap();
        assert!(recovery.exists());

        host.execute(EditorCommand::Save).unwrap();
        assert!(!host.is_dirty());
        assert!(
            !recovery.exists(),
            "a successful save must clear the recovery sidecar"
        );
    }

    // ── M11: crash recovery ──────────────────────────────────────────────

    /// The end-to-end promise: edit, autosave, "crash" (drop the host without
    /// saving), reopen — the unsaved work is detected and recoverable, and the
    /// recovered scene is still marked unsaved.
    #[test]
    fn unsaved_work_survives_a_crash_and_is_recoverable() {
        let path = write_single_entity_scene("recover_roundtrip");
        // Guarantee the sidecar lands in a strictly later mtime tick than the
        // source, so the freshness check offers it (see wait_for_fresh_mtime_after).
        wait_for_fresh_mtime_after(&path);

        // Session 1: edit, autosave, then "crash" (never save).
        let recovered_terrain = {
            let mut host = EditorRuntimeHost::open_hsc(&path).unwrap();
            dirty_the_host(&mut host);
            host.autosave_to_recovery().unwrap().unwrap();
            host.session().scene().runtime_slice.clone()
            // host dropped without Save — exactly a crash from disk's view.
        };

        // Session 2: reopen. The source is untouched, so the freshly loaded
        // scene lacks the edit — but the sidecar is found.
        let mut host = EditorRuntimeHost::open_hsc(&path).unwrap();
        assert!(!host.is_dirty(), "a freshly opened scene is clean");
        assert_ne!(
            host.session().scene().runtime_slice,
            recovered_terrain,
            "the source file must NOT contain the unsaved edit"
        );
        let pending = host.pending_recovery().expect("crash evidence must be found");
        assert!(pending.is_file());

        // Recover: the unsaved edit comes back, and is still unsaved.
        host.execute(EditorCommand::RecoverAutosave).unwrap();
        assert_eq!(
            host.session().scene().runtime_slice,
            recovered_terrain,
            "recovery must restore the unsaved edit"
        );
        assert!(
            host.is_dirty(),
            "recovered work is still unsaved relative to the source"
        );
    }

    /// Recovery is never automatic: merely opening a scene with a pending
    /// sidecar must leave the on-disk document loaded, untouched.
    #[test]
    fn recovery_is_offered_not_applied_automatically() {
        let path = write_single_entity_scene("recover_prompted");
        wait_for_fresh_mtime_after(&path);
        {
            let mut host = EditorRuntimeHost::open_hsc(&path).unwrap();
            dirty_the_host(&mut host);
            host.autosave_to_recovery().unwrap().unwrap();
        }

        let host = EditorRuntimeHost::open_hsc(&path).unwrap();
        assert!(host.pending_recovery().is_some(), "sidecar present");
        assert!(
            !host.is_dirty(),
            "opening must not silently install the recovery document"
        );
    }

    /// Declining recovery deletes the sidecar and keeps the on-disk scene.
    #[test]
    fn discarding_recovery_deletes_the_sidecar_and_keeps_the_source() {
        let path = write_single_entity_scene("recover_discard");
        let source_before = std::fs::read(&path).unwrap();
        wait_for_fresh_mtime_after(&path);
        {
            let mut host = EditorRuntimeHost::open_hsc(&path).unwrap();
            dirty_the_host(&mut host);
            host.autosave_to_recovery().unwrap().unwrap();
        }

        let mut host = EditorRuntimeHost::open_hsc(&path).unwrap();
        assert!(host.pending_recovery().is_some());
        host.execute(EditorCommand::DiscardAutosave).unwrap();

        assert!(host.pending_recovery().is_none(), "sidecar must be gone");
        assert_eq!(
            std::fs::read(&path).unwrap(),
            source_before,
            "discarding must not touch the source scene"
        );
    }

    /// Discarding when there is nothing to discard is a no-op, not an error —
    /// a stale UI prompt must not be able to fail the command.
    #[test]
    fn discarding_without_a_sidecar_is_idempotent() {
        let path = write_single_entity_scene("recover_idempotent");
        let mut host = EditorRuntimeHost::open_hsc(&path).unwrap();
        assert!(host.pending_recovery().is_none());
        host.execute(EditorCommand::DiscardAutosave).unwrap();
        host.execute(EditorCommand::DiscardAutosave).unwrap();
    }

    /// Staleness guard (review): a recovery sidecar OLDER than the source (it
    /// survived a save because clear_recovery is best-effort, or a crash struck
    /// between the source write and the delete) must NOT be offered — recovering
    /// it would revert the user to already-saved work.
    #[test]
    fn stale_sidecar_older_than_source_is_not_offered() {
        let path = write_single_entity_scene("recover_stale");
        // The sidecar must be strictly newer than the source to be offered.
        wait_for_fresh_mtime_after(&path);
        let mut host = EditorRuntimeHost::open_hsc(&path).unwrap();
        dirty_the_host(&mut host);
        let sidecar = host.autosave_to_recovery().unwrap().unwrap();
        assert!(host.pending_recovery().is_some(), "fresh sidecar is offered");

        // Make the source strictly newer than the sidecar (simulating a save
        // that landed after the autosave but whose clear_recovery was lost).
        wait_for_fresh_mtime_after(&sidecar);
        std::fs::write(&path, std::fs::read(&path).unwrap()).unwrap();

        assert!(
            host.pending_recovery().is_none(),
            "a sidecar older than the source must not be offered"
        );
        // Clean it up.
        let _ = std::fs::remove_file(sidecar);
    }

    /// RecoverAutosave against a corrupt/truncated sidecar (exactly what a
    /// crash mid-autosave could leave, though atomic_write makes it unlikely)
    /// surfaces a typed error and leaves the loaded scene untouched.
    #[test]
    fn recover_from_corrupt_sidecar_surfaces_typed_error_and_preserves_state() {
        let path = write_single_entity_scene("recover_corrupt");
        let mut host = EditorRuntimeHost::open_hsc(&path).unwrap();
        let recovery = host.recovery_path().unwrap();
        std::fs::create_dir_all(recovery.parent().unwrap()).unwrap();
        // Write the sidecar strictly after the source so the staleness guard
        // offers it. (The write below sets the sidecar mtime; the ordering must
        // be established BEFORE it, not after.)
        wait_for_fresh_mtime_after(&path);
        // A V2 .hscene header with a half-written entity and NO terminator —
        // the strict parser must reject it (M11.4).
        std::fs::write(
            &recovery,
            format!("{}\nname|X\nentity|1|a\n", super::super::HSCENE_DOC_HEADER_V2),
        )
        .unwrap();
        assert!(host.pending_recovery().is_some());

        let entities_before = host.session().scene().entities.len();
        let result = host.execute(EditorCommand::RecoverAutosave);
        assert!(result.is_err(), "corrupt sidecar recovery must error");
        assert_eq!(
            host.session().scene().entities.len(),
            entities_before,
            "a failed recovery must not partially mutate the scene"
        );
        let _ = std::fs::remove_file(recovery);
    }

    /// A clean save leaves no sidecar, so reopening does not falsely claim a
    /// crash. Guards against the recovery prompt crying wolf after every save.
    #[test]
    fn clean_save_leaves_no_crash_evidence() {
        let path = write_single_entity_scene("recover_no_false_positive");
        {
            let mut host = EditorRuntimeHost::open_hsc(&path).unwrap();
            dirty_the_host(&mut host);
            host.autosave_to_recovery().unwrap().unwrap();
            host.execute(EditorCommand::Save).unwrap();
        }
        let host = EditorRuntimeHost::open_hsc(&path).unwrap();
        assert!(
            host.pending_recovery().is_none(),
            "a clean save must leave no crash evidence"
        );
    }
}
