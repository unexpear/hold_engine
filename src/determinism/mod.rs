//! Deterministic record-replay and frame-digest infrastructure.
//!
//! Pillar 4 of ADR 0003 names "Determinism and debuggability" as a
//! load-bearing differentiation pillar:
//!
//! > Given the same seed and the same input stream, the simulation
//! > produces identical state bit-for-bit across runs. This unlocks
//! > reliable replays, networking, reproducible bug reports, and
//! > deterministic CI tests.
//!
//! This module ships the four real components that make that
//! promise enforceable, in-house with zero external crates:
//!
//! - [`InputRecording`] — versioned, byte-stable on-disk format
//!   for a captured input stream + initial seed.
//! - [`InputRecorder`] — observes the engine's per-frame
//!   `(InputState, dt)` pair and appends to an `InputRecording`.
//! - [`InputReplayer`] — drives an [`InputState`] from a
//!   recording so the simulation sees the same frames it saw
//!   the first time.
//! - [`DeterministicHasher`] + [`frame_digest`] — a fixed-seed
//!   FNV-1a-64 hasher and the frame-digest helpers the harness
//!   uses to compare simulation states across replays.
//!
//! The [`harness`] submodule's
//! [`record_and_replay_frame_digests`](harness::record_and_replay_frame_digests)
//! is the load-bearing CI test: it runs a real schedule against
//! a real `World`, captures frame digests across two replays of
//! the same recording, and asserts equality.
//!
//! See `docs/adr/0005-determinism-architecture.md` for the
//! contract and the hardware-portability bounds (the recorder
//! runs in single-digit microseconds per frame on i3-class CPUs,
//! ~80 bytes per frame, no SIMD/AVX, no parallelism).

pub mod digest;
pub mod harness;
pub mod hasher;
pub mod recorder;
pub mod recording;
pub mod replayer;

pub use digest::{frame_digest_from_components, FrameDigest};
pub use harness::record_and_replay_frame_digests;
pub use hasher::DeterministicHasher;
pub use recorder::InputRecorder;
pub use recording::{
    decode_input_recording, encode_input_recording, InputRecording, InputRecordingError,
    RecordedFrame, RecordedInput, INPUT_RECORDING_FORMAT_VERSION, INPUT_RECORDING_MAGIC,
};
pub use replayer::InputReplayer;
