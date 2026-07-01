//! Per-frame input recorder.
//!
//! See `docs/adr/0005-determinism-architecture.md` Q5 for the
//! "tick once per frame, before schedule.run" rationale and Q10
//! for the per-frame budget (< 10μs on i3-class CPU at typical
//! input density).

use crate::input::InputState;

use super::recording::{InputRecording, RecordedFrame, RecordedInput};

/// Captures the engine's per-frame `(InputState, dt)` pair into a
/// growing [`InputRecording`]. Single-responsibility: snapshot +
/// append, nothing else. The recorder doesn't read the simulation,
/// doesn't poll the OS, doesn't allocate beyond the per-frame
/// `Vec::push`.
///
/// Threading: not `Sync`. The recorder lives on the same thread as
/// the engine's run loop because `InputState` is consumed there.
/// Sending recordings between threads is a separate concern (the
/// codec produces `Vec<u8>` which is `Send`).
///
/// Drop the recorder to keep the recording; call
/// [`Self::into_recording`] to move it out by value (the recorder
/// is dead after that).
pub struct InputRecorder {
    recording: InputRecording,
}

impl InputRecorder {
    /// Construct a fresh recorder for a simulation seeded with
    /// `initial_seed`. Replay reads this seed from the recording
    /// header and seeds the simulation's RNG before tick 0 — see
    /// ADR 0005 Q4.
    pub fn new(initial_seed: u64) -> Self {
        Self {
            recording: InputRecording::new(initial_seed),
        }
    }

    /// Number of frames captured so far.
    pub fn len(&self) -> usize {
        self.recording.len()
    }

    /// Whether no frames have been captured yet.
    pub fn is_empty(&self) -> bool {
        self.recording.is_empty()
    }

    /// Append a frame to the recording. Call once per engine tick,
    /// before the schedule runs, so the captured snapshot reflects
    /// the inputs the simulation was about to consume.
    ///
    /// Per-frame cost: one `RecordedInput::capture` (sorts a few
    /// small vecs — ~50 entries at typical density) plus a
    /// `Vec::push`. Single-digit microseconds on i3-class hardware
    /// per ADR 0005 Q10.
    pub fn tick(&mut self, input: &InputState, dt: f32) {
        self.recording.frames.push(RecordedFrame {
            dt,
            input: RecordedInput::capture(input),
        });
    }

    /// Borrow the recording-in-progress. Useful for the editor
    /// shell's "show me how many frames I've recorded" indicator.
    pub fn recording(&self) -> &InputRecording {
        &self.recording
    }

    /// Consume the recorder and return the captured recording. The
    /// recorder is dead after this call; new recordings need a
    /// fresh `InputRecorder::new`.
    pub fn into_recording(self) -> InputRecording {
        self.recording
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh recorder has zero frames and the supplied seed.
    /// Replay seeding depends on the seed surviving construction;
    /// pin it.
    #[test]
    fn fresh_recorder_has_no_frames_and_carries_seed() {
        let recorder = InputRecorder::new(0xDEAD_BEEF);
        assert!(recorder.is_empty());
        assert_eq!(recorder.len(), 0);
        assert_eq!(recorder.recording().initial_seed, 0xDEAD_BEEF);
    }

    /// Each `tick` call appends exactly one frame. The recorded
    /// `dt` and input snapshot match what was passed in. Pillar 4
    /// (debuggability): no silent drops, no batching, one frame in
    /// = one frame out.
    #[test]
    fn tick_appends_exactly_one_frame_with_supplied_dt() {
        let mut recorder = InputRecorder::new(0);
        let mut state = InputState::new();
        state.test_press_key(42);
        recorder.tick(&state, 0.016);
        recorder.tick(&state, 0.017);
        recorder.tick(&state, 0.015);
        assert_eq!(recorder.len(), 3);
        assert_eq!(recorder.recording().frames[0].dt, 0.016);
        assert_eq!(recorder.recording().frames[1].dt, 0.017);
        assert_eq!(recorder.recording().frames[2].dt, 0.015);
        // Each captured frame carries the keys_down state at
        // tick time.
        for frame in &recorder.recording().frames {
            assert_eq!(frame.input.keys_down, vec![42]);
        }
    }

    /// `into_recording` moves the recording out by value. The same
    /// number of frames are present; the data isn't lost.
    #[test]
    fn into_recording_moves_captured_data_out_by_value() {
        let mut recorder = InputRecorder::new(7);
        let state = InputState::new();
        recorder.tick(&state, 0.01);
        recorder.tick(&state, 0.01);
        let recording = recorder.into_recording();
        assert_eq!(recording.len(), 2);
        assert_eq!(recording.initial_seed, 7);
    }

    /// Recorder doesn't observe state mutations after the tick —
    /// the snapshot is owned, not borrowed. Mutating the source
    /// `InputState` after `tick` doesn't change captured frames.
    /// This is the "owned snapshot" property from ADR 0005 Q4.
    #[test]
    fn captured_frames_are_owned_snapshots_not_borrows() {
        let mut recorder = InputRecorder::new(0);
        let mut state = InputState::new();
        state.test_press_key(1);
        recorder.tick(&state, 0.01);
        // Mutate after capture; the captured frame must not change.
        state.test_press_key(2);
        assert_eq!(recorder.recording().frames[0].input.keys_down, vec![1]);
    }
}
