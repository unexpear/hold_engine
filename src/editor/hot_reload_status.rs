//! Editor-side ring buffer of hot-reload events for the
//! "Hot-Reload Status" panel.
//!
//! Pillar 2 (iteration speed) is invisible to anyone not reading
//! stdout until the editor surfaces it. This module owns the
//! data structure that backs the panel: a bounded ring buffer of
//! recent reload outcomes plus running counters that the UI
//! renders in a corner of the editor viewport.
//!
//! See ADR 0004 Q9 (M7.4.2) and Q13 (the editor protocol codec
//! that delivers these events from remote clients).
//!
//! ## Why a ring buffer
//!
//! Hot-reload is a per-save event, but a busy editing session
//! produces dozens of saves per minute. The panel shows the last
//! few outcomes — older ones scroll off. Bounding the buffer
//! keeps the editor's memory footprint flat regardless of session
//! length and keeps the panel render cost constant.
//!
//! Default capacity is 32 entries (covers ~5 minutes of
//! moderately active editing); callers can construct with a
//! different capacity for tests or extreme sessions.

use std::time::{SystemTime, UNIX_EPOCH};

/// Default capacity of the editor's hot-reload status ring
/// buffer. Tuned for "show the last ~32 events" without
/// unbounded memory growth across long editing sessions.
pub const DEFAULT_HOT_RELOAD_CAPACITY: usize = 32;

/// Outcome of a single hot-reload attempt as the editor
/// observes it. The success path (`Applied`) carries the same
/// fields that flow through `EditorEvent::AssetInvalidated` and
/// the failure path (`Failed`) mirrors `EditorEvent::CompileFailed`,
/// so the editor's protocol layer feeds events into this buffer
/// with no translation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HotReloadOutcome {
    Applied {
        /// Asset kind label (`"Texture"`, `"NormalMap"`,
        /// `"Mesh"`, `"Material"`, `"Shader"`, ...). Stored as
        /// a string rather than an enum so the editor can render
        /// future kinds it doesn't statically know about — see
        /// ADR 0004 Q13's forward-compatibility rationale.
        kind: String,
        handle_index: u32,
        logical_name: String,
    },
    Failed {
        kind: String,
        logical_name: String,
        error: String,
    },
}

impl HotReloadOutcome {
    pub fn kind(&self) -> &str {
        match self {
            Self::Applied { kind, .. } | Self::Failed { kind, .. } => kind,
        }
    }

    pub fn logical_name(&self) -> &str {
        match self {
            Self::Applied { logical_name, .. } | Self::Failed { logical_name, .. } => {
                logical_name
            }
        }
    }

    pub fn is_failure(&self) -> bool {
        matches!(self, Self::Failed { .. })
    }
}

/// One entry in the ring buffer: a hot-reload outcome plus the
/// wall-clock instant the editor observed it.
///
/// `observed_ms_since_epoch` is a Unix milliseconds timestamp,
/// not a `std::time::Instant`, because the editor's protocol
/// layer carries timestamps over IPC and `Instant` is opaque.
/// The trade-off: comparisons across different machines aren't
/// meaningful unless their clocks agree, but the panel only
/// renders relative-to-now so this is fine. Pillar 4
/// (debuggability) is satisfied: the timestamp shows up in the
/// panel and the developer can see "5s ago" / "30s ago" when
/// triaging which save broke the build.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HotReloadEntry {
    pub outcome: HotReloadOutcome,
    pub observed_ms_since_epoch: u64,
}

/// Editor-side store of recent hot-reload events. The native UI
/// reads this each frame; the protocol layer pushes new events
/// in as they arrive.
///
/// Two access patterns it has to satisfy cheaply:
/// 1. Render: iterate most-recent-first, render up to N rows.
/// 2. Query: how many failures in this session? what was the
///    last error? — the panel summary line shows both.
#[derive(Debug, Clone)]
pub struct HotReloadStatus {
    /// Ring storage. `entries[head]` is the oldest, the
    /// most-recent-first iterator walks backward modulo
    /// capacity.
    entries: Vec<HotReloadEntry>,
    head: usize,
    /// Whether the ring has wrapped at least once. Distinguishes
    /// "buffer not full yet" (entries[..len] are real, rest is
    /// uninitialized) from "buffer full, head points at oldest"
    /// (every slot is real).
    wrapped: bool,
    capacity: usize,

    // Running counters that survive ring eviction.
    total_applied: u64,
    total_failed: u64,
    last_failure_index: Option<usize>,
}

impl Default for HotReloadStatus {
    fn default() -> Self {
        Self::with_capacity(DEFAULT_HOT_RELOAD_CAPACITY)
    }
}

impl HotReloadStatus {
    /// Build a status buffer with explicit capacity. Capacity of
    /// 0 collapses to 1 (a degenerate single-slot buffer)
    /// because most consumers assume `most_recent()` returns
    /// `Some` after at least one push; a true zero-capacity
    /// buffer would silently swallow every event.
    pub fn with_capacity(capacity: usize) -> Self {
        let cap = capacity.max(1);
        Self {
            entries: Vec::with_capacity(cap),
            head: 0,
            wrapped: false,
            capacity: cap,
            total_applied: 0,
            total_failed: 0,
            last_failure_index: None,
        }
    }

    /// Push a new outcome, capturing the current wall-clock as
    /// the observation timestamp. Convenience wrapper over
    /// [`Self::push_with_time`] for production callers.
    pub fn push(&mut self, outcome: HotReloadOutcome) {
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            .min(u64::MAX as u128) as u64;
        self.push_with_time(outcome, now_ms);
    }

    /// Push with an explicit timestamp. Tests use this so
    /// behaviour is deterministic; production calls
    /// [`Self::push`].
    pub fn push_with_time(&mut self, outcome: HotReloadOutcome, ms_since_epoch: u64) {
        let is_failure = outcome.is_failure();
        let entry = HotReloadEntry {
            outcome,
            observed_ms_since_epoch: ms_since_epoch,
        };

        if self.entries.len() < self.capacity {
            // Buffer not yet full — append.
            let idx = self.entries.len();
            self.entries.push(entry);
            if is_failure {
                self.total_failed = self.total_failed.saturating_add(1);
                self.last_failure_index = Some(idx);
            } else {
                self.total_applied = self.total_applied.saturating_add(1);
            }
        } else {
            // Buffer full — overwrite the oldest slot.
            let evict_idx = self.head;
            self.entries[evict_idx] = entry;
            self.head = (self.head + 1) % self.capacity;
            self.wrapped = true;
            if is_failure {
                self.total_failed = self.total_failed.saturating_add(1);
                self.last_failure_index = Some(evict_idx);
            } else {
                self.total_applied = self.total_applied.saturating_add(1);
                // The previous last_failure_index might point at
                // the slot we just evicted. Re-scan to find the
                // most recent surviving failure.
                if self.last_failure_index == Some(evict_idx) {
                    self.last_failure_index = self.scan_for_last_failure();
                }
            }
        }
    }

    /// Number of entries currently in the buffer. Equals
    /// `capacity` once the ring has wrapped.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Total successful reloads observed since this status
    /// buffer was constructed. Survives ring eviction — the
    /// panel summary line shows "N applied" across the whole
    /// session, not just the visible window.
    pub fn total_applied(&self) -> u64 {
        self.total_applied
    }

    /// Total failed reloads observed. Same survives-eviction
    /// semantics.
    pub fn total_failed(&self) -> u64 {
        self.total_failed
    }

    /// Most recent entry, regardless of outcome. `None` when
    /// the buffer is empty.
    pub fn most_recent(&self) -> Option<&HotReloadEntry> {
        if self.entries.is_empty() {
            return None;
        }
        if !self.wrapped {
            return self.entries.last();
        }
        // Wrapped: most recent sits just before head (wrapping).
        let idx = (self.head + self.capacity - 1) % self.capacity;
        self.entries.get(idx)
    }

    /// Most recent failure currently in the ring. `None` if the
    /// buffer never saw a failure or if every failure has been
    /// evicted.
    pub fn last_failure(&self) -> Option<&HotReloadEntry> {
        self.last_failure_index.and_then(|i| self.entries.get(i))
    }

    /// Iterate entries most-recent-first. Used by the panel
    /// renderer to draw the top N rows.
    pub fn iter_recent(&self) -> impl Iterator<Item = &HotReloadEntry> + '_ {
        let len = self.entries.len();
        let cap = self.capacity;
        let head = self.head;
        let wrapped = self.wrapped;
        (0..len).map(move |i| {
            // Walk backward from head-1 (most recent) to
            // head (oldest) modulo capacity.
            let logical_position = if wrapped {
                (head + cap - 1 - i) % cap
            } else {
                len - 1 - i
            };
            &self.entries[logical_position]
        })
    }

    /// Drop every entry and reset counters. Used when the
    /// editor closes a project.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.head = 0;
        self.wrapped = false;
        self.total_applied = 0;
        self.total_failed = 0;
        self.last_failure_index = None;
    }

    fn scan_for_last_failure(&self) -> Option<usize> {
        let mut best: Option<(usize, u64)> = None;
        for (i, entry) in self.entries.iter().enumerate() {
            if entry.outcome.is_failure() {
                match best {
                    None => best = Some((i, entry.observed_ms_since_epoch)),
                    Some((_, ts)) if entry.observed_ms_since_epoch > ts => {
                        best = Some((i, entry.observed_ms_since_epoch))
                    }
                    _ => {}
                }
            }
        }
        best.map(|(i, _)| i)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn applied(name: &str, idx: u32) -> HotReloadOutcome {
        HotReloadOutcome::Applied {
            kind: "Texture".into(),
            handle_index: idx,
            logical_name: name.into(),
        }
    }

    fn failed(name: &str, error: &str) -> HotReloadOutcome {
        HotReloadOutcome::Failed {
            kind: "Texture".into(),
            logical_name: name.into(),
            error: error.into(),
        }
    }

    /// A fresh buffer reports empty + no counts. Baseline.
    #[test]
    fn fresh_buffer_is_empty() {
        let s = HotReloadStatus::default();
        assert!(s.is_empty());
        assert_eq!(s.total_applied(), 0);
        assert_eq!(s.total_failed(), 0);
        assert!(s.most_recent().is_none());
        assert!(s.last_failure().is_none());
    }

    /// Push one applied event: counters tick, most_recent
    /// returns it, last_failure stays None.
    #[test]
    fn push_applied_updates_counter_and_most_recent() {
        let mut s = HotReloadStatus::with_capacity(8);
        s.push_with_time(applied("textures/foo", 1), 100);
        assert_eq!(s.len(), 1);
        assert_eq!(s.total_applied(), 1);
        assert_eq!(s.total_failed(), 0);
        let recent = s.most_recent().unwrap();
        assert_eq!(recent.outcome.logical_name(), "textures/foo");
        assert_eq!(recent.observed_ms_since_epoch, 100);
        assert!(s.last_failure().is_none());
    }

    /// Push one failure: failure counter ticks, last_failure
    /// returns it.
    #[test]
    fn push_failed_updates_failure_state() {
        let mut s = HotReloadStatus::with_capacity(8);
        s.push_with_time(failed("textures/bad", "syntax error"), 200);
        assert_eq!(s.total_failed(), 1);
        let last = s.last_failure().unwrap();
        match &last.outcome {
            HotReloadOutcome::Failed { error, .. } => assert_eq!(error, "syntax error"),
            _ => panic!("expected Failed"),
        }
    }

    /// Ring eviction: pushing past capacity overwrites the
    /// oldest entry. Counters keep accumulating across
    /// eviction (total_applied is session-total, not
    /// visible-only).
    #[test]
    fn ring_evicts_oldest_when_capacity_exceeded() {
        let mut s = HotReloadStatus::with_capacity(3);
        s.push_with_time(applied("a", 1), 10);
        s.push_with_time(applied("b", 2), 20);
        s.push_with_time(applied("c", 3), 30);
        s.push_with_time(applied("d", 4), 40);

        assert_eq!(s.len(), 3); // capped
        assert_eq!(s.total_applied(), 4); // counter survives
        let names: Vec<&str> = s
            .iter_recent()
            .map(|e| e.outcome.logical_name())
            .collect();
        // Most recent first; oldest ("a") evicted.
        assert_eq!(names, vec!["d", "c", "b"]);
    }

    /// Eviction of the only failure clears `last_failure`.
    /// Pillar 4 (debuggability): the panel's "last error" line
    /// must reflect what's *currently* in the ring, not stale
    /// pre-eviction state.
    #[test]
    fn last_failure_clears_when_evicted() {
        let mut s = HotReloadStatus::with_capacity(2);
        s.push_with_time(failed("a", "oops"), 10);
        assert!(s.last_failure().is_some());
        // Push two successes — failure gets evicted.
        s.push_with_time(applied("b", 1), 20);
        s.push_with_time(applied("c", 2), 30);
        assert!(
            s.last_failure().is_none(),
            "evicted failure should drop out of last_failure"
        );
        assert_eq!(s.total_failed(), 1, "counter still records it");
    }

    /// When multiple failures live in the ring and the oldest
    /// is evicted, `last_failure` falls back to the
    /// most-recent surviving failure.
    #[test]
    fn last_failure_falls_back_to_surviving_failure() {
        let mut s = HotReloadStatus::with_capacity(3);
        s.push_with_time(failed("oldest_fail", "e1"), 10);
        s.push_with_time(applied("filler", 1), 20);
        s.push_with_time(failed("newer_fail", "e2"), 30);
        // Buffer now: [oldest_fail, filler, newer_fail].
        // Push two more to evict oldest_fail.
        s.push_with_time(applied("d", 2), 40); // evicts oldest_fail
        let last = s.last_failure().expect("newer_fail should remain");
        assert_eq!(last.outcome.logical_name(), "newer_fail");
    }

    /// Iteration walks most-recent-first whether or not the
    /// ring has wrapped. The renderer relies on this to draw
    /// rows top-to-bottom in chronological reverse order.
    #[test]
    fn iter_recent_is_most_recent_first_in_both_states() {
        // Pre-wrap.
        let mut s = HotReloadStatus::with_capacity(5);
        for i in 0..3 {
            s.push_with_time(applied(&format!("a{}", i), i), (i as u64) * 10);
        }
        let names: Vec<String> = s
            .iter_recent()
            .map(|e| e.outcome.logical_name().to_string())
            .collect();
        assert_eq!(names, vec!["a2", "a1", "a0"]);

        // Post-wrap: push enough to wrap.
        for i in 3..7 {
            s.push_with_time(applied(&format!("a{}", i), i), (i as u64) * 10);
        }
        let names: Vec<String> = s
            .iter_recent()
            .map(|e| e.outcome.logical_name().to_string())
            .collect();
        // Capacity 5, last 5 pushes were a2..a6, so most recent
        // first: a6, a5, a4, a3, a2.
        assert_eq!(names, vec!["a6", "a5", "a4", "a3", "a2"]);
    }

    /// Capacity of 0 collapses to 1. Without this, callers
    /// constructing with a config-derived value of 0 would
    /// silently lose every event — exactly the kind of failure
    /// mode the principle says to engineer against.
    #[test]
    fn zero_capacity_collapses_to_one() {
        let mut s = HotReloadStatus::with_capacity(0);
        assert_eq!(s.capacity(), 1);
        s.push_with_time(applied("first", 1), 10);
        s.push_with_time(applied("second", 2), 20);
        let recent = s.most_recent().unwrap();
        assert_eq!(recent.outcome.logical_name(), "second");
    }

    /// `clear` resets every state field. Used when the editor
    /// closes a project; the new project shouldn't see ghost
    /// entries from the old one.
    #[test]
    fn clear_resets_all_state() {
        let mut s = HotReloadStatus::with_capacity(4);
        s.push_with_time(applied("a", 1), 10);
        s.push_with_time(failed("b", "oops"), 20);
        s.clear();
        assert!(s.is_empty());
        assert_eq!(s.total_applied(), 0);
        assert_eq!(s.total_failed(), 0);
        assert!(s.most_recent().is_none());
        assert!(s.last_failure().is_none());
    }
}
