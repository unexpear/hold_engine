//! State machine that collapses a burst of raw file-system events
//! into a single settled [`FileChange`] per path.
//!
//! Why coalescing matters: editors and DCC tools save by writing a
//! temp file then renaming, which from the watcher's perspective looks
//! like Add+Modify+Remove on the temp + Rename → final-name. Without
//! coalescing the compile queue runs three or four redundant compiles
//! per save. With coalescing, one compile per save.
//!
//! Design discipline: the coalescer is a pure state machine — no
//! threads, no syscalls, no `Instant::now()` calls. The caller owns
//! time and feeds it in via [`Coalescer::record`] and
//! [`Coalescer::drain_expired`]. That makes the coalescer fully
//! testable in isolation and gives the watcher thread one place to
//! read the clock per loop iteration.
//!
//! See ADR 0004 Q4.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use super::event::{FileChange, FileChangeKind};

/// Raw event flavours the coalescer understands. The watcher thread
/// translates Win32 `FILE_ACTION_*` codes into these before feeding
/// them in.
///
/// Renames are split into the watcher's two raw events
/// (`RenamedOldName` + `RenamedNewName`) and the coalescer pairs them
/// into a single `Renamed` output if it sees both within the
/// coalescing window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RawFileEvent {
    Created(PathBuf),
    Modified(PathBuf),
    Removed(PathBuf),
    RenamedOldName(PathBuf),
    RenamedNewName(PathBuf),
}

impl RawFileEvent {
    pub fn path(&self) -> &PathBuf {
        match self {
            Self::Created(p)
            | Self::Modified(p)
            | Self::Removed(p)
            | Self::RenamedOldName(p)
            | Self::RenamedNewName(p) => p,
        }
    }
}

/// Pending state per path inside the coalescer's window.
#[derive(Debug)]
struct PendingEntry {
    settled_kind: SettledKind,
    last_seen: Instant,
}

/// What state has the path settled into so far this window? This is
/// the running answer to "if the window expired right now, what
/// `FileChange` would we emit?"
#[derive(Debug, Clone, PartialEq, Eq)]
enum SettledKind {
    Created,
    Modified,
    Removed,
    /// Half a rename — we saw the old-name event but not the
    /// new-name event yet. If the window expires here, we treat
    /// this as a `Removed` (the old path no longer exists from
    /// downstream's perspective).
    PendingRenameOldName,
    /// Half a rename — we saw the new-name event but not the
    /// old-name event yet. Treat as `Created` if the window expires.
    PendingRenameNewName,
    /// Full rename pair: settled into the new path with the old
    /// path captured for downstream identity preservation.
    Renamed { old_path: PathBuf },
}

/// Coalesces raw file-system events into settled [`FileChange`]s
/// after a configurable quiet window per path.
pub struct Coalescer {
    pending: HashMap<PathBuf, PendingEntry>,
    /// Half-rename pairing buffer. When a `RenamedOldName` arrives
    /// we record its path here keyed by … well, there's no obvious
    /// key from the event alone, since the OS reports the old name
    /// and the new name in two consecutive events. We use a
    /// "last-old-name" slot (Win32 guarantees the new-name event
    /// follows the old-name event immediately within the same
    /// `ReadDirectoryChangesW` notification batch).
    last_old_rename_path: Option<(PathBuf, Instant)>,
    window: Duration,
}

impl Coalescer {
    /// Construct a coalescer with the given quiet window. The
    /// engine default is 50ms (ADR 0004 Q4) — the window is named
    /// rather than hard-coded so M7.1's calibration capture can
    /// adjust it without touching this module.
    pub fn new(window: Duration) -> Self {
        Self {
            pending: HashMap::new(),
            last_old_rename_path: None,
            window,
        }
    }

    /// Number of paths currently in the coalescing window.
    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }

    /// Window duration this coalescer was constructed with.
    pub fn window(&self) -> Duration {
        self.window
    }

    /// Record a raw file-system event and update the per-path
    /// pending state. Pure function — no syscalls, no clock reads.
    pub fn record(&mut self, raw: RawFileEvent, now: Instant) {
        match raw {
            RawFileEvent::Created(path) => {
                self.update_path(path, SettledKind::Created, now);
            }
            RawFileEvent::Modified(path) => {
                self.update_modified(path, now);
            }
            RawFileEvent::Removed(path) => {
                self.update_removed(path, now);
            }
            RawFileEvent::RenamedOldName(path) => {
                self.last_old_rename_path = Some((path.clone(), now));
                self.update_path(path, SettledKind::PendingRenameOldName, now);
            }
            RawFileEvent::RenamedNewName(new_path) => {
                if let Some((old_path, old_seen)) = self.last_old_rename_path.take() {
                    // Pair with the most recent old-name event if it
                    // fell within the window; otherwise treat as a
                    // standalone Created event.
                    if now.saturating_duration_since(old_seen) <= self.window {
                        // Drop the half-rename old-path entry and
                        // install the paired Renamed under new_path.
                        self.pending.remove(&old_path);
                        self.pending.insert(
                            new_path,
                            PendingEntry {
                                settled_kind: SettledKind::Renamed { old_path },
                                last_seen: now,
                            },
                        );
                        return;
                    }
                }
                self.update_path(new_path, SettledKind::PendingRenameNewName, now);
            }
        }
    }

    fn update_modified(&mut self, path: PathBuf, now: Instant) {
        // Modified preserves Created/Renamed status if already set.
        if let Some(entry) = self.pending.get_mut(&path) {
            entry.last_seen = now;
            entry.settled_kind = match &entry.settled_kind {
                SettledKind::Created => SettledKind::Created,
                SettledKind::Renamed { old_path } => SettledKind::Renamed {
                    old_path: old_path.clone(),
                },
                SettledKind::Removed => SettledKind::Modified, // resurrected
                _ => SettledKind::Modified,
            };
            return;
        }
        self.pending.insert(
            path,
            PendingEntry {
                settled_kind: SettledKind::Modified,
                last_seen: now,
            },
        );
    }

    fn update_removed(&mut self, path: PathBuf, now: Instant) {
        if let Some(entry) = self.pending.get_mut(&path) {
            // Created followed by Removed in the same window:
            // file appeared and disappeared. Suppress entirely by
            // removing the pending entry; the watcher's downstream
            // never sees the spurious churn.
            if matches!(entry.settled_kind, SettledKind::Created) {
                self.pending.remove(&path);
                return;
            }
            entry.last_seen = now;
            entry.settled_kind = SettledKind::Removed;
            return;
        }
        self.pending.insert(
            path,
            PendingEntry {
                settled_kind: SettledKind::Removed,
                last_seen: now,
            },
        );
    }

    fn update_path(&mut self, path: PathBuf, kind: SettledKind, now: Instant) {
        self.pending.insert(
            path,
            PendingEntry {
                settled_kind: kind,
                last_seen: now,
            },
        );
    }

    /// Drain entries whose quiet window has expired (i.e. no event
    /// has arrived for that path within `self.window` of `now`) and
    /// emit them as settled [`FileChange`]s.
    ///
    /// The watcher thread calls this on a timeout-based cadence so
    /// a save burst that ends doesn't sit forever in the pending
    /// map.
    pub fn drain_expired(&mut self, now: Instant) -> Vec<FileChange> {
        let window = self.window;
        let mut out = Vec::new();
        let mut to_remove = Vec::new();
        for (path, entry) in &self.pending {
            if now.saturating_duration_since(entry.last_seen) >= window {
                if let Some(change) = settled_to_file_change(path, &entry.settled_kind) {
                    out.push(change);
                }
                to_remove.push(path.clone());
            }
        }
        for path in to_remove {
            self.pending.remove(&path);
        }
        // Also expire the half-rename pairing buffer if it's stale.
        if let Some((_, old_seen)) = &self.last_old_rename_path {
            if now.saturating_duration_since(*old_seen) >= window {
                self.last_old_rename_path = None;
            }
        }
        out
    }

    /// Drain *all* pending entries regardless of window state.
    /// Called on shutdown or when the watcher needs to flush
    /// definitively (e.g. a request from the editor to "apply
    /// any pending reloads now").
    pub fn drain_all(&mut self) -> Vec<FileChange> {
        let mut out = Vec::new();
        for (path, entry) in self.pending.drain() {
            if let Some(change) = settled_to_file_change(&path, &entry.settled_kind) {
                out.push(change);
            }
        }
        self.last_old_rename_path = None;
        out
    }
}

fn settled_to_file_change(path: &std::path::Path, kind: &SettledKind) -> Option<FileChange> {
    match kind {
        SettledKind::Created => Some(FileChange::created(path.to_path_buf())),
        SettledKind::Modified => Some(FileChange::modified(path.to_path_buf())),
        SettledKind::Removed => Some(FileChange::removed(path.to_path_buf())),
        SettledKind::PendingRenameOldName => {
            // Old-name event without a paired new-name within the
            // window. The file went away from this path's
            // perspective, so emit Removed.
            Some(FileChange::removed(path.to_path_buf()))
        }
        SettledKind::PendingRenameNewName => {
            // New-name event without a paired old-name. From this
            // path's perspective the file appeared.
            Some(FileChange::created(path.to_path_buf()))
        }
        SettledKind::Renamed { old_path } => Some(FileChange::new(
            path.to_path_buf(),
            FileChangeKind::Renamed {
                old_path: old_path.clone(),
            },
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t0() -> Instant {
        Instant::now()
    }

    fn p(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    /// Single Created event followed by quiet ≥ window emits exactly
    /// one Created. Baseline happy path.
    #[test]
    fn single_created_emits_after_window() {
        let mut c = Coalescer::new(Duration::from_millis(50));
        let t = t0();
        c.record(RawFileEvent::Created(p("foo.png")), t);
        // Same-time drain: window not yet expired → empty.
        let out0 = c.drain_expired(t);
        assert!(out0.is_empty());
        // Past the window → emits.
        let out1 = c.drain_expired(t + Duration::from_millis(50));
        assert_eq!(out1.len(), 1);
        assert_eq!(out1[0], FileChange::created(p("foo.png")));
    }

    /// Modified+Modified bursts collapse to one Modified. The classic
    /// "every keystroke triggers a reload" anti-pattern.
    #[test]
    fn modified_burst_collapses_to_one_event() {
        let mut c = Coalescer::new(Duration::from_millis(50));
        let t = t0();
        c.record(RawFileEvent::Modified(p("foo.png")), t);
        c.record(
            RawFileEvent::Modified(p("foo.png")),
            t + Duration::from_millis(10),
        );
        c.record(
            RawFileEvent::Modified(p("foo.png")),
            t + Duration::from_millis(20),
        );
        // Drain at t + 80ms: last event was at +20, window is 50ms,
        // so quiet duration is 60ms >= 50ms → emits.
        let out = c.drain_expired(t + Duration::from_millis(80));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0], FileChange::modified(p("foo.png")));
    }

    /// Created followed by Modified before the window expires emits
    /// Created (the file appeared and was further changed before
    /// settling).
    #[test]
    fn created_then_modified_emits_created() {
        let mut c = Coalescer::new(Duration::from_millis(50));
        let t = t0();
        c.record(RawFileEvent::Created(p("foo.png")), t);
        c.record(
            RawFileEvent::Modified(p("foo.png")),
            t + Duration::from_millis(10),
        );
        let out = c.drain_expired(t + Duration::from_millis(80));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0], FileChange::created(p("foo.png")));
    }

    /// Created followed by Removed within the window suppresses both
    /// — the file appeared and disappeared, no downstream observer
    /// should see the noise.
    #[test]
    fn created_then_removed_in_window_suppresses_both() {
        let mut c = Coalescer::new(Duration::from_millis(50));
        let t = t0();
        c.record(RawFileEvent::Created(p("foo.png")), t);
        c.record(
            RawFileEvent::Removed(p("foo.png")),
            t + Duration::from_millis(10),
        );
        let out = c.drain_expired(t + Duration::from_millis(80));
        assert!(out.is_empty(), "expected suppression, got {:?}", out);
    }

    /// Modified followed by Removed emits Removed — the file's gone
    /// and we don't bother surfacing the intermediate edits.
    #[test]
    fn modified_then_removed_emits_removed() {
        let mut c = Coalescer::new(Duration::from_millis(50));
        let t = t0();
        c.record(RawFileEvent::Modified(p("foo.png")), t);
        c.record(
            RawFileEvent::Removed(p("foo.png")),
            t + Duration::from_millis(10),
        );
        let out = c.drain_expired(t + Duration::from_millis(80));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0], FileChange::removed(p("foo.png")));
    }

    /// Removed followed by Modified means the path was deleted and
    /// then recreated and re-edited — emits Modified (from
    /// downstream's perspective, the asset is back and needs
    /// reload).
    #[test]
    fn removed_then_modified_emits_modified() {
        let mut c = Coalescer::new(Duration::from_millis(50));
        let t = t0();
        c.record(RawFileEvent::Removed(p("foo.png")), t);
        c.record(
            RawFileEvent::Modified(p("foo.png")),
            t + Duration::from_millis(10),
        );
        let out = c.drain_expired(t + Duration::from_millis(80));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0], FileChange::modified(p("foo.png")));
    }

    /// Two paths each with their own burst stay independent. The
    /// per-path window doesn't collide.
    #[test]
    fn independent_paths_emit_independently() {
        let mut c = Coalescer::new(Duration::from_millis(50));
        let t = t0();
        c.record(RawFileEvent::Modified(p("foo.png")), t);
        c.record(
            RawFileEvent::Modified(p("bar.obj")),
            t + Duration::from_millis(5),
        );
        c.record(
            RawFileEvent::Modified(p("foo.png")),
            t + Duration::from_millis(15),
        );
        let out = c.drain_expired(t + Duration::from_millis(80));
        assert_eq!(out.len(), 2);
        let paths: Vec<_> = out.iter().map(|c| c.path.clone()).collect();
        assert!(paths.contains(&p("foo.png")));
        assert!(paths.contains(&p("bar.obj")));
    }

    /// RenamedOldName + RenamedNewName in the same window pair into
    /// one Renamed event. Captures the canonical save-by-rename
    /// pattern.
    #[test]
    fn rename_pair_emits_renamed() {
        let mut c = Coalescer::new(Duration::from_millis(50));
        let t = t0();
        c.record(RawFileEvent::RenamedOldName(p("foo.png.tmp")), t);
        c.record(
            RawFileEvent::RenamedNewName(p("foo.png")),
            t + Duration::from_millis(5),
        );
        let out = c.drain_expired(t + Duration::from_millis(80));
        assert_eq!(out.len(), 1);
        assert_eq!(
            out[0],
            FileChange::renamed(p("foo.png.tmp"), p("foo.png"))
        );
    }

    /// A half-rename (just RenamedOldName, no pair within window)
    /// surfaces as Removed. The OS may have failed to deliver the
    /// new-name notification, or the rename never completed —
    /// either way the file's gone from the old path.
    #[test]
    fn half_rename_old_name_falls_through_as_removed() {
        let mut c = Coalescer::new(Duration::from_millis(50));
        let t = t0();
        c.record(RawFileEvent::RenamedOldName(p("foo.png")), t);
        let out = c.drain_expired(t + Duration::from_millis(80));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0], FileChange::removed(p("foo.png")));
    }

    /// `drain_all` flushes everything regardless of window state.
    /// Used on shutdown so pending events aren't lost.
    #[test]
    fn drain_all_flushes_unsettled_state() {
        let mut c = Coalescer::new(Duration::from_millis(500));
        let t = t0();
        c.record(RawFileEvent::Modified(p("a.png")), t);
        c.record(
            RawFileEvent::Created(p("b.png")),
            t + Duration::from_millis(10),
        );
        // drain_expired at t + 50 (well inside the 500ms window) is
        // empty.
        let out = c.drain_expired(t + Duration::from_millis(50));
        assert!(out.is_empty());
        // drain_all flushes everything.
        let out = c.drain_all();
        assert_eq!(out.len(), 2);
        assert_eq!(c.pending_count(), 0);
    }

    /// Multiple rapid events on the same path keep extending the
    /// window — coalescing waits for genuine quiet, not absolute
    /// time. Without this, a continuous save (autosave every 10ms)
    /// would never emit.
    #[test]
    fn rapid_events_extend_the_window() {
        let mut c = Coalescer::new(Duration::from_millis(50));
        let t = t0();
        for offset_ms in 0..200u64 {
            // 200 events spaced 10ms apart, total span 2 seconds.
            c.record(
                RawFileEvent::Modified(p("foo.png")),
                t + Duration::from_millis(offset_ms * 10),
            );
        }
        // Drain at the time of the last event: still active, window
        // not expired.
        let out = c.drain_expired(t + Duration::from_millis(199 * 10));
        assert!(
            out.is_empty(),
            "expected no emission while events are still arriving"
        );
        // After 50ms of quiet, emits exactly one.
        let out = c.drain_expired(t + Duration::from_millis(199 * 10 + 80));
        assert_eq!(out.len(), 1);
    }
}
