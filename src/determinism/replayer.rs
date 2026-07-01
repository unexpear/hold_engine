//! Input replayer.
//!
//! Walks an [`InputRecording`] frame-by-frame, mutating an
//! [`InputState`] in place so the simulation observes the exact
//! same per-frame inputs the recorder captured. The engine's
//! normal run loop sees the replayed `InputState` as if it had
//! come from the OS.
//!
//! See `docs/adr/0005-determinism-architecture.md` Q6 for the
//! design and Q1 for the same-binary same-input contract.

use crate::input::InputState;

use super::recording::{InputRecording, RecordedFrame};

/// Drives an `InputState` from a captured [`InputRecording`].
///
/// Lifetime: `'rec` — the replayer borrows the recording rather
/// than owning it so a single recording can be replayed multiple
/// times (the harness in
/// [`super::harness::record_and_replay_frame_digests`] does
/// exactly that — runs the same recording twice with fresh state
/// to confirm digests match).
///
/// Replay finishes when the recording is exhausted; no looping
/// in v1 (ADR 0005 Q6 — replay length equals recording length,
/// exact). Callers detect end via [`Self::is_finished`] or by
/// observing `None` from [`Self::next_frame`].
pub struct InputReplayer<'rec> {
    recording: &'rec InputRecording,
    cursor: usize,
}

impl<'rec> InputReplayer<'rec> {
    /// Construct a replayer borrowing the supplied recording.
    /// Cursor starts at frame 0.
    pub fn new(recording: &'rec InputRecording) -> Self {
        Self {
            recording,
            cursor: 0,
        }
    }

    /// Recording the replayer is reading from. Useful when the
    /// caller needs the seed (`recording().initial_seed`) to
    /// re-seed the simulation's RNG before tick 0.
    pub fn recording(&self) -> &'rec InputRecording {
        self.recording
    }

    /// Frame index the next [`Self::next_frame`] call would read.
    /// Equals `recording().len()` when replay is finished.
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// Whether all frames have been replayed. After
    /// [`Self::is_finished`] returns true, [`Self::next_frame`]
    /// returns `None` indefinitely; callers should stop driving
    /// the engine through this replayer.
    pub fn is_finished(&self) -> bool {
        self.cursor >= self.recording.frames.len()
    }

    /// Advance to the next recorded frame. Returns the frame data
    /// (`dt` + input snapshot) so the caller can both apply the
    /// input and feed `dt` into the engine's tick. Returns `None`
    /// when the recording is exhausted.
    ///
    /// This method advances the cursor on every call — calling
    /// twice consumes two frames. To peek without advancing, use
    /// [`Self::peek_frame`].
    pub fn next_frame(&mut self) -> Option<&'rec RecordedFrame> {
        let frame = self.recording.frames.get(self.cursor)?;
        self.cursor += 1;
        Some(frame)
    }

    /// Read the next frame without advancing the cursor. Returns
    /// the same data as [`Self::next_frame`] would, but a
    /// subsequent call to either method still sees the same
    /// frame.
    pub fn peek_frame(&self) -> Option<&'rec RecordedFrame> {
        self.recording.frames.get(self.cursor)
    }

    /// Reset the cursor to frame 0. The replayer is ready to
    /// replay from the beginning. Used by the harness to drive
    /// two passes of the same recording (ADR 0005 Q9: harness
    /// runs replay twice and compares digests).
    pub fn rewind(&mut self) {
        self.cursor = 0;
    }

    /// Advance one frame and apply its input snapshot to `state`,
    /// returning the recorded `dt` so the caller can pass it to
    /// the schedule. Returns `None` when the recording is
    /// exhausted.
    ///
    /// The most common replay-loop shape:
    /// ```ignore
    /// while let Some(dt) = replayer.tick_into(&mut input) {
    ///     schedule.run(&mut world, dt);
    /// }
    /// ```
    pub fn tick_into(&mut self, state: &mut InputState) -> Option<f32> {
        let frame = self.next_frame()?;
        state.apply_recorded(&frame.input);
        Some(frame.dt)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::determinism::{InputRecorder, InputRecording, RecordedFrame, RecordedInput};

    fn frame(dt: f32, key: u32) -> RecordedFrame {
        RecordedFrame {
            dt,
            input: RecordedInput {
                keys_down: vec![key],
                keys_just_pressed: vec![key],
                keys_just_released: vec![],
                mouse_pos: (0, 0),
                mouse_delta: (0, 0),
                mouse_buttons: vec![],
                mouse_just_pressed: vec![],
                mouse_just_released: vec![],
                mouse_wheel: 0.0,
            },
        }
    }

    /// Empty recording is finished from the start; `next_frame`
    /// returns None on first call.
    #[test]
    fn empty_recording_is_finished_immediately() {
        let recording = InputRecording::new(0);
        let mut replayer = InputReplayer::new(&recording);
        assert!(replayer.is_finished());
        assert!(replayer.next_frame().is_none());
        assert_eq!(replayer.cursor(), 0);
    }

    /// `next_frame` walks the recording in order and `is_finished`
    /// flips at the boundary. Cursor matches the position
    /// reported externally.
    #[test]
    fn next_frame_walks_recording_and_finishes_correctly() {
        let recording = InputRecording {
            format_version: super::super::recording::INPUT_RECORDING_FORMAT_VERSION,
            initial_seed: 0,
            frames: vec![frame(0.016, 1), frame(0.017, 2), frame(0.018, 3)],
        };
        let mut replayer = InputReplayer::new(&recording);

        assert_eq!(replayer.next_frame().unwrap().dt, 0.016);
        assert_eq!(replayer.cursor(), 1);
        assert_eq!(replayer.next_frame().unwrap().dt, 0.017);
        assert_eq!(replayer.next_frame().unwrap().dt, 0.018);
        assert!(replayer.is_finished());
        assert!(replayer.next_frame().is_none());
        // Past the end, the replayer keeps returning None — no
        // panic, no out-of-bounds read.
        assert!(replayer.next_frame().is_none());
    }

    /// `peek_frame` doesn't advance the cursor. Two consecutive
    /// peeks see the same frame; a subsequent `next_frame` then
    /// observes that frame and advances.
    #[test]
    fn peek_frame_does_not_advance_cursor() {
        let recording = InputRecording {
            format_version: super::super::recording::INPUT_RECORDING_FORMAT_VERSION,
            initial_seed: 0,
            frames: vec![frame(0.016, 7)],
        };
        let mut replayer = InputReplayer::new(&recording);
        let peek_a = replayer.peek_frame().unwrap().dt;
        let peek_b = replayer.peek_frame().unwrap().dt;
        assert_eq!(peek_a, peek_b);
        assert_eq!(replayer.cursor(), 0);
        let observed = replayer.next_frame().unwrap().dt;
        assert_eq!(observed, peek_a);
        assert_eq!(replayer.cursor(), 1);
    }

    /// `rewind` resets to frame 0 so the same recording replays
    /// twice. The harness's load-bearing pattern: replay twice,
    /// confirm digests match.
    #[test]
    fn rewind_replays_recording_from_the_beginning() {
        let recording = InputRecording {
            format_version: super::super::recording::INPUT_RECORDING_FORMAT_VERSION,
            initial_seed: 0,
            frames: vec![frame(0.016, 1), frame(0.017, 2)],
        };
        let mut replayer = InputReplayer::new(&recording);
        replayer.next_frame();
        replayer.next_frame();
        assert!(replayer.is_finished());
        replayer.rewind();
        assert!(!replayer.is_finished());
        assert_eq!(replayer.cursor(), 0);
        assert_eq!(replayer.next_frame().unwrap().input.keys_down, vec![1]);
    }

    /// `tick_into` mutates a target `InputState` and returns the
    /// recorded `dt`. Round-trip test: record three frames with
    /// distinct keys, replay them, observe the same key sequence
    /// in the replayed state. This is the load-bearing property
    /// the determinism harness depends on.
    #[test]
    fn tick_into_round_trips_recorder_to_replayer_state() {
        let mut recorder = InputRecorder::new(42);
        let mut source = InputState::new();

        source.test_press_key(11);
        recorder.tick(&source, 0.016);
        source = InputState::new();
        source.test_press_key(22);
        recorder.tick(&source, 0.017);
        source = InputState::new();
        source.test_press_key(33);
        recorder.tick(&source, 0.018);

        let recording = recorder.into_recording();
        let mut replayer = InputReplayer::new(&recording);
        let mut target = InputState::new();

        let dt0 = replayer.tick_into(&mut target).unwrap();
        assert_eq!(dt0, 0.016);
        assert!(target.key(11));
        let dt1 = replayer.tick_into(&mut target).unwrap();
        assert_eq!(dt1, 0.017);
        assert!(target.key(22));
        // Frame 2 should clear the previously-held key 22 because
        // each replay frame is the truth — apply_recorded
        // overwrites all observable fields (ADR 0005 Q1).
        assert!(!target.key(11));
        let dt2 = replayer.tick_into(&mut target).unwrap();
        assert_eq!(dt2, 0.018);
        assert!(target.key(33));
        assert!(!target.key(22));

        // Recording exhausted.
        assert!(replayer.tick_into(&mut target).is_none());
    }

    /// The replayer borrows the recording for `'rec`, so the same
    /// recording can be replayed via two replayers (or one
    /// rewound replayer) without cloning. This is the property
    /// the harness uses for "replay twice with fresh state."
    #[test]
    fn recording_is_borrowed_so_two_replayers_share_it() {
        let recording = InputRecording {
            format_version: super::super::recording::INPUT_RECORDING_FORMAT_VERSION,
            initial_seed: 0,
            frames: vec![frame(0.016, 5)],
        };
        let mut replayer_a = InputReplayer::new(&recording);
        let mut replayer_b = InputReplayer::new(&recording);
        assert_eq!(replayer_a.next_frame().unwrap().dt, 0.016);
        assert_eq!(replayer_b.next_frame().unwrap().dt, 0.016);
    }
}
