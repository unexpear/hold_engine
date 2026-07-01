//! Editor-side record/replay state machine.
//!
//! Closes the live-editor loop on the determinism work shipped in
//! `src/determinism/`: the editor can record a play session and
//! replay it deterministically without restarting. Press one key
//! to start recording, play for a while, press it again to stop;
//! press another key to replay the captured stream.
//!
//! ## Architecture
//!
//! [`EditorReplayState`] is a finite state machine with three
//! modes:
//!
//! - [`ReplayMode::Normal`] — no recording, no replay. The
//!   simulation runs against live OS input.
//! - [`ReplayMode::Recording`] — every frame, the live OS input
//!   plus its `dt` is appended to an [`InputRecorder`]. The
//!   simulation still runs against live input; recording is a
//!   passive observer.
//! - [`ReplayMode::Replaying { cursor }`] — the simulation runs
//!   against input synthesized from the most recently completed
//!   recording. When the cursor reaches the end of the
//!   recording, the state machine returns to `Normal`
//!   automatically.
//!
//! ## Why not put this on `EditorRuntimeHost` directly?
//!
//! Single-responsibility: the host already owns the scene
//! session, the live `World`, the inspector cursor, the editor
//! camera, the undo stack, and the hot-reload status panel.
//! Adding the replay state machine inline would push the host
//! past 20 fields. Keeping the FSM in its own module — owned by
//! the host as a single field — preserves the host's role as
//! integration coordinator without bloating it.
//!
//! ## Editor / simulation input split (v1 trade-off)
//!
//! The replay path is currently driven from inside the editor's
//! `App::update`, which sees `&InputState` after the engine's
//! schedule has already run. Editor demos like
//! `examples/editor_shell.rs` work fine because their schedule
//! is empty — all gameplay logic lives in `App::update` and
//! consumes the replayed input from there. A production game
//! whose schedule contains real gameplay systems would need an
//! engine-level pre-schedule hook to overwrite input *before*
//! `schedule.run` consumes it. That hook is tracked as a v2
//! follow-up; for v1 the App::update path is sufficient and
//! exercises the determinism contract end-to-end.
//!
//! See `docs/adr/0005-determinism-architecture.md` Q5–Q7 for the
//! recording/replay design and Q1 for the same-binary same-input
//! contract this state machine enforces in the live editor.

use crate::determinism::{InputRecorder, InputRecording};
use crate::input::InputState;

/// Three observable states the editor's record/replay machine
/// can be in. Transitions only happen via the explicit methods
/// on [`EditorReplayState`] — no implicit state changes inside
/// `tick_recording` / `tick_replay` except the cursor advance
/// and the natural Replay → Normal transition when the recording
/// runs out.
///
/// `Replaying` carries its cursor by value so the FSM is
/// self-contained — no separate cursor field on the parent
/// state struct, no risk of the cursor getting out of sync with
/// the mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplayMode {
    /// No recording, no replay. Simulation runs against live
    /// OS input.
    Normal,
    /// Live OS input is captured into the active
    /// [`InputRecorder`] each frame; simulation also runs
    /// against live input.
    Recording,
    /// Simulation runs against input synthesised from the most
    /// recently completed recording. `cursor` is the index of
    /// the *next* frame to apply.
    Replaying { cursor: usize },
}

impl ReplayMode {
    /// Whether the simulation should be locked against
    /// hot-reload during this mode. Mirrors the
    /// `EditorPlayState::locks_reloads` predicate but operates
    /// on the replay state rather than the play state — they're
    /// independent concerns even though both happen to lock
    /// reloads when active.
    pub fn locks_reloads(self) -> bool {
        matches!(self, Self::Replaying { .. })
    }
}

/// Outcome of a single frame's replay tick. The editor's
/// `App::update` uses this to decide whether to keep driving
/// the replay or transition back to normal mode (and surface a
/// "replay complete" message).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ReplayTickOutcome {
    /// Not currently replaying; tick was a no-op.
    NotReplaying,
    /// A recorded frame was applied. The `dt` the engine should
    /// use this frame is in `dt`; the cursor was advanced one
    /// position.
    Replayed { dt: f32 },
    /// The recording was just exhausted on this tick. The state
    /// machine has already transitioned back to `Normal`. The
    /// editor surfaces a "replay finished" log entry on this.
    Finished,
}

/// Editor-side record/replay finite state machine. Owned by
/// [`super::EditorRuntimeHost`] as a single field; mutated only
/// through the explicit transition methods + per-frame ticks.
///
/// Only the most recently completed recording is retained
/// (`last_recording`). Starting a new recording discards the
/// previous one — a deliberate choice for v1 simplicity. A future
/// "named recordings" surface (save/load `.holdrec` files via the
/// codec from `src/determinism/recording.rs`) can layer on top
/// without changing this FSM.
pub struct EditorReplayState {
    mode: ReplayMode,
    /// Active recorder, only present when `mode == Recording`.
    /// Moved out of the option on `stop_recording` and converted
    /// into an `InputRecording` stored in `last_recording`.
    recorder: Option<InputRecorder>,
    /// The most recently completed recording. Populated by
    /// [`Self::stop_recording`]; consumed by [`Self::start_replay`]
    /// (which keeps the recording around for re-replay — replay
    /// borrows `last_recording`, doesn't move it).
    last_recording: Option<InputRecording>,
}

impl EditorReplayState {
    /// Construct a fresh state machine in `Normal` mode with no
    /// recording loaded.
    pub fn new() -> Self {
        Self {
            mode: ReplayMode::Normal,
            recorder: None,
            last_recording: None,
        }
    }

    /// Current observable mode. The editor's status panel reads
    /// this to render "REC" / "REPLAY" / blank.
    pub fn mode(&self) -> ReplayMode {
        self.mode
    }

    /// Convenience predicate.
    pub fn is_normal(&self) -> bool {
        matches!(self.mode, ReplayMode::Normal)
    }
    /// Convenience predicate.
    pub fn is_recording(&self) -> bool {
        matches!(self.mode, ReplayMode::Recording)
    }
    /// Convenience predicate.
    pub fn is_replaying(&self) -> bool {
        matches!(self.mode, ReplayMode::Replaying { .. })
    }

    /// Number of frames captured in the active recording so far,
    /// or in the last completed recording if not currently
    /// recording. `None` if no recording has ever started.
    pub fn frame_count(&self) -> Option<usize> {
        if let Some(rec) = self.recorder.as_ref() {
            Some(rec.len())
        } else {
            self.last_recording.as_ref().map(|r| r.len())
        }
    }

    /// Cursor position during replay. `None` outside `Replaying`
    /// mode. Useful for the editor's "replay 142 / 600" indicator.
    pub fn replay_cursor(&self) -> Option<usize> {
        if let ReplayMode::Replaying { cursor } = self.mode {
            Some(cursor)
        } else {
            None
        }
    }

    /// Borrow the most recently completed recording. Useful for
    /// the editor's "save .holdrec" path or for tests that want
    /// to inspect what was captured.
    pub fn last_recording(&self) -> Option<&InputRecording> {
        self.last_recording.as_ref()
    }

    /// Begin a fresh recording. If currently recording, the
    /// in-flight recording is discarded (start fresh); if
    /// currently replaying, replay is cancelled first. The
    /// previously completed `last_recording`, if any, is also
    /// discarded — only one recording exists at a time in v1.
    ///
    /// `seed` is the RNG seed the recorder embeds in the
    /// recording header. Replay re-applies this seed to the
    /// simulation's RNG before tick 0 (ADR 0005 Q4).
    pub fn start_recording(&mut self, seed: u64) {
        // Cancel whatever was happening. start_recording is the
        // user's "fresh state, please" signal.
        self.mode = ReplayMode::Recording;
        self.recorder = Some(InputRecorder::new(seed));
        self.last_recording = None;
    }

    /// Stop recording and finalise the captured input stream.
    /// The completed recording becomes available via
    /// [`Self::last_recording`] and can be replayed with
    /// [`Self::start_replay`].
    ///
    /// No-op if not currently recording. Returns the number of
    /// frames captured (0 if there was no recorder), useful for
    /// the editor's "captured N frames" log entry.
    pub fn stop_recording(&mut self) -> usize {
        if !self.is_recording() {
            return 0;
        }
        self.mode = ReplayMode::Normal;
        let recording = self
            .recorder
            .take()
            .map(|r| r.into_recording())
            .unwrap_or_else(|| InputRecording::new(0));
        let len = recording.len();
        self.last_recording = Some(recording);
        len
    }

    /// Begin replaying the most recently completed recording.
    /// Returns `true` if replay started; `false` if no recording
    /// is loaded (the editor's UI surfaces a "no recording
    /// available — press F11 first" message in that case).
    ///
    /// If currently recording, recording is stopped first (and
    /// the just-completed recording becomes the replay target).
    /// Cancels any in-flight replay, restarting from frame 0.
    pub fn start_replay(&mut self) -> bool {
        if self.is_recording() {
            self.stop_recording();
        }
        if self.last_recording.is_some() {
            self.mode = ReplayMode::Replaying { cursor: 0 };
            true
        } else {
            false
        }
    }

    /// Stop an active replay. The state machine returns to
    /// `Normal`. No-op if not currently replaying.
    pub fn stop_replay(&mut self) {
        if self.is_replaying() {
            self.mode = ReplayMode::Normal;
        }
    }

    /// Per-frame recording tick: append the live input + dt to
    /// the active recorder if `mode == Recording`. No-op in any
    /// other mode. Call once per frame from `App::update` before
    /// running gameplay logic.
    pub fn tick_recording(&mut self, input: &InputState, dt: f32) {
        if matches!(self.mode, ReplayMode::Recording) {
            if let Some(recorder) = self.recorder.as_mut() {
                recorder.tick(input, dt);
            }
        }
    }

    /// Per-frame replay tick: if replaying, advance the cursor
    /// and overwrite `target` with the recorded frame's input
    /// snapshot. Returns a [`ReplayTickOutcome`] describing what
    /// happened — the editor surfaces the `Finished` variant as
    /// a log entry.
    ///
    /// When the recording is exhausted, automatically transitions
    /// the state machine back to `Normal`. The editor then sees
    /// `is_replaying() == false` on the next tick and clears its
    /// "REPLAY" indicator.
    pub fn tick_replay(&mut self, target: &mut InputState) -> ReplayTickOutcome {
        let cursor = match self.mode {
            ReplayMode::Replaying { cursor } => cursor,
            _ => return ReplayTickOutcome::NotReplaying,
        };
        let Some(recording) = self.last_recording.as_ref() else {
            // Defensive: we shouldn't be in Replaying mode without
            // a recording. Clean up and report finished so the
            // editor surfaces the unexpected state rather than
            // silently sticking in Replaying.
            self.mode = ReplayMode::Normal;
            return ReplayTickOutcome::Finished;
        };
        match recording.frames.get(cursor) {
            Some(frame) => {
                target.apply_recorded(&frame.input);
                let dt = frame.dt;
                self.mode = ReplayMode::Replaying {
                    cursor: cursor + 1,
                };
                ReplayTickOutcome::Replayed { dt }
            }
            None => {
                // Recording exhausted; auto-transition to Normal
                // so the editor's next-frame check shows "not
                // replaying" without the user pressing anything.
                self.mode = ReplayMode::Normal;
                ReplayTickOutcome::Finished
            }
        }
    }
}

impl Default for EditorReplayState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fresh state is Normal, with no recording loaded and zero
    /// frame count. Pin the construction defaults — a future
    /// regression that initialised in Recording mode would
    /// silently start capturing on every editor launch.
    #[test]
    fn fresh_state_is_normal_with_no_recording() {
        let s = EditorReplayState::new();
        assert!(s.is_normal());
        assert!(!s.is_recording());
        assert!(!s.is_replaying());
        assert_eq!(s.mode(), ReplayMode::Normal);
        assert_eq!(s.frame_count(), None);
        assert_eq!(s.replay_cursor(), None);
        assert!(s.last_recording().is_none());
    }

    /// `start_recording` transitions to Recording, creates a
    /// recorder with the supplied seed, and discards any prior
    /// last_recording (the user wanted fresh state).
    #[test]
    fn start_recording_transitions_and_seeds_recorder() {
        let mut s = EditorReplayState::new();
        // Seed a prior recording to confirm it gets cleared.
        s.start_recording(99);
        s.tick_recording(&InputState::new(), 0.016);
        s.stop_recording();
        assert!(s.last_recording().is_some());

        // start_recording with a new seed should clear last_recording.
        s.start_recording(42);
        assert!(s.is_recording());
        assert_eq!(s.frame_count(), Some(0));
        assert!(s.last_recording().is_none());
    }

    /// `tick_recording` appends frames while Recording and is a
    /// no-op in any other mode. Pillar 4 (debuggability): tests
    /// pin "no implicit captures outside Recording mode."
    #[test]
    fn tick_recording_appends_only_in_recording_mode() {
        let mut s = EditorReplayState::new();
        s.tick_recording(&InputState::new(), 0.016); // Normal: no-op
        assert_eq!(s.frame_count(), None);

        s.start_recording(1);
        s.tick_recording(&InputState::new(), 0.016);
        s.tick_recording(&InputState::new(), 0.017);
        assert_eq!(s.frame_count(), Some(2));

        s.stop_recording();
        // Now Normal again. tick_recording must not append.
        s.tick_recording(&InputState::new(), 0.018);
        assert_eq!(s.frame_count(), Some(2));
    }

    /// `stop_recording` returns the captured frame count and
    /// makes the recording available to `last_recording`. This
    /// is the load-bearing handoff between record and replay.
    #[test]
    fn stop_recording_finalises_and_exposes_recording() {
        let mut s = EditorReplayState::new();
        s.start_recording(7);
        s.tick_recording(&InputState::new(), 0.016);
        s.tick_recording(&InputState::new(), 0.017);
        s.tick_recording(&InputState::new(), 0.018);

        let n = s.stop_recording();
        assert_eq!(n, 3);
        assert!(s.is_normal());
        let rec = s.last_recording().expect("recording present");
        assert_eq!(rec.len(), 3);
        assert_eq!(rec.initial_seed, 7);
    }

    /// `start_replay` returns false when no recording is loaded
    /// and true when one is. Tests pin the handshake the editor's
    /// UI uses to decide whether to surface "no recording — press
    /// F11 first" vs actually entering replay.
    #[test]
    fn start_replay_handshake_signals_recording_availability() {
        let mut s = EditorReplayState::new();
        // No recording yet.
        assert!(!s.start_replay());
        assert!(s.is_normal());

        // Capture something.
        s.start_recording(1);
        s.tick_recording(&InputState::new(), 0.016);
        s.stop_recording();

        // Now replay starts.
        assert!(s.start_replay());
        assert!(s.is_replaying());
        assert_eq!(s.replay_cursor(), Some(0));
    }

    /// `start_replay` while recording stops the recording first
    /// and then starts replay against the just-captured stream.
    /// This is the "press replay mid-record" UX path.
    #[test]
    fn start_replay_while_recording_finalises_and_replays() {
        let mut s = EditorReplayState::new();
        s.start_recording(3);
        s.tick_recording(&InputState::new(), 0.016);
        s.tick_recording(&InputState::new(), 0.017);

        let started = s.start_replay();
        assert!(started);
        assert!(s.is_replaying());
        // The just-captured 2-frame recording is the replay
        // target. The cursor starts at 0.
        let rec = s.last_recording().expect("recording present");
        assert_eq!(rec.len(), 2);
        assert_eq!(s.replay_cursor(), Some(0));
    }

    /// `tick_replay` advances the cursor frame-by-frame and
    /// returns the recorded `dt` plus an `Replayed` outcome.
    /// On exhaustion, transitions to Normal automatically and
    /// returns `Finished`.
    #[test]
    fn tick_replay_advances_then_finishes_automatically() {
        let mut s = EditorReplayState::new();
        s.start_recording(0);
        s.tick_recording(&InputState::new(), 0.016);
        s.tick_recording(&InputState::new(), 0.017);
        s.stop_recording();
        assert!(s.start_replay());

        let mut buf = InputState::new();

        match s.tick_replay(&mut buf) {
            ReplayTickOutcome::Replayed { dt } => assert_eq!(dt, 0.016),
            other => panic!("expected Replayed, got {:?}", other),
        }
        assert_eq!(s.replay_cursor(), Some(1));

        match s.tick_replay(&mut buf) {
            ReplayTickOutcome::Replayed { dt } => assert_eq!(dt, 0.017),
            other => panic!("expected Replayed, got {:?}", other),
        }
        assert_eq!(s.replay_cursor(), Some(2));

        // Exhausted: returns Finished and transitions to Normal.
        match s.tick_replay(&mut buf) {
            ReplayTickOutcome::Finished => {}
            other => panic!("expected Finished, got {:?}", other),
        }
        assert!(s.is_normal());

        // Subsequent ticks are NotReplaying — no auto-restart.
        match s.tick_replay(&mut buf) {
            ReplayTickOutcome::NotReplaying => {}
            other => panic!("expected NotReplaying after finish, got {:?}", other),
        }
    }

    /// `stop_replay` mid-stream cancels the replay and returns
    /// to Normal without exhausting the recording. The recording
    /// stays available for re-replay (the editor's "scrub back"
    /// future feature would build on this).
    #[test]
    fn stop_replay_cancels_mid_stream_and_preserves_recording() {
        let mut s = EditorReplayState::new();
        s.start_recording(0);
        s.tick_recording(&InputState::new(), 0.016);
        s.tick_recording(&InputState::new(), 0.017);
        s.tick_recording(&InputState::new(), 0.018);
        s.stop_recording();
        s.start_replay();

        let mut buf = InputState::new();
        s.tick_replay(&mut buf);
        s.stop_replay();
        assert!(s.is_normal());

        // Recording still available; replay can restart.
        let rec = s.last_recording().expect("recording preserved");
        assert_eq!(rec.len(), 3);
        assert!(s.start_replay());
        assert_eq!(s.replay_cursor(), Some(0)); // restart from 0
    }

    /// `tick_replay` actually mutates the target InputState to
    /// match the recorded frame. Pillar 1 (correctness): the
    /// load-bearing property the determinism contract depends
    /// on — if this didn't actually overwrite input, the replay
    /// would silently feed live OS input to the simulation.
    #[test]
    fn tick_replay_overwrites_target_input_state() {
        let mut s = EditorReplayState::new();
        s.start_recording(0);
        // Record one frame with key 99 pressed.
        let mut source = InputState::new();
        source.test_press_key(99);
        s.tick_recording(&source, 0.016);
        s.stop_recording();
        s.start_replay();

        // Drain into a target with completely different state.
        let mut target = InputState::new();
        target.test_press_key(7);
        target.test_press_key(13);
        assert!(target.key(7));

        match s.tick_replay(&mut target) {
            ReplayTickOutcome::Replayed { .. } => {}
            other => panic!("expected Replayed, got {:?}", other),
        }
        // Recorded key landed; pre-existing keys cleared (since
        // apply_recorded overwrites the whole HashSet).
        assert!(target.key(99));
        assert!(!target.key(7));
        assert!(!target.key(13));
    }

    /// `ReplayMode::locks_reloads` returns true only for
    /// `Replaying`. The editor uses this to decide whether the
    /// hot-reload dispatcher should buffer reloads until replay
    /// ends — same shape as `EditorPlayState::locks_reloads` but
    /// scoped to the replay state machine.
    #[test]
    fn replay_mode_locks_reloads_only_during_replaying() {
        assert!(!ReplayMode::Normal.locks_reloads());
        assert!(!ReplayMode::Recording.locks_reloads());
        assert!(ReplayMode::Replaying { cursor: 0 }.locks_reloads());
        assert!(ReplayMode::Replaying { cursor: 999 }.locks_reloads());
    }

    /// `frame_count` reports during recording (live count from
    /// the recorder) and after recording (count of last
    /// completed recording). A regression that cleared the count
    /// on stop_recording would silently lie to the editor's UI.
    #[test]
    fn frame_count_reports_during_and_after_recording() {
        let mut s = EditorReplayState::new();
        assert_eq!(s.frame_count(), None);

        s.start_recording(0);
        s.tick_recording(&InputState::new(), 0.016);
        s.tick_recording(&InputState::new(), 0.017);
        assert_eq!(s.frame_count(), Some(2));

        s.stop_recording();
        assert_eq!(s.frame_count(), Some(2));
    }
}
