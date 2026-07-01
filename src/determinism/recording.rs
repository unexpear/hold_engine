//! Input recording data types and on-disk codec.
//!
//! See `docs/adr/0005-determinism-architecture.md` Q4 for the layout
//! decision and Q10 for the hardware-portability bounds (~80 bytes
//! per frame at typical input density).

use crate::input::{InputState, MouseButton};

/// 8-byte magic header identifying a hold-engine input recording.
/// Matches the shape of `COMPILED_ASSET_MAGIC` from the asset
/// pipeline so the codecs read alike.
pub const INPUT_RECORDING_MAGIC: [u8; 8] = *b"HOLDREC1";

/// On-disk format version for the recording layout. A breaking
/// change to [`RecordedFrame`] (e.g. gamepad axes added in M-future)
/// bumps this; the decoder rejects mismatched versions rather than
/// silently misinterpreting the bytes.
pub const INPUT_RECORDING_FORMAT_VERSION: u32 = 1;

/// A captured stream of inputs. Self-contained: replaying it
/// requires no other artifact except the engine binary that
/// recorded it.
///
/// Only [`PartialEq`], not [`Eq`], because [`RecordedFrame`]
/// carries `f32` values (`dt`, `mouse_wheel`). NaN-aware equality
/// is the wrong shape for a recording — comparing two recordings
/// for equality means "do these produce the same simulation," and
/// that's a partial equivalence on `f32` (NaN ≠ NaN).
#[derive(Debug, Clone, PartialEq)]
pub struct InputRecording {
    /// Format version. Decoders reject recordings whose version
    /// doesn't match [`INPUT_RECORDING_FORMAT_VERSION`].
    pub format_version: u32,
    /// RNG seed in effect at frame 0. Replay must seed the
    /// simulation's RNG with this value before tick 0 runs;
    /// otherwise stochastic systems diverge from the recording.
    pub initial_seed: u64,
    /// Per-frame inputs. `frames[i]` is what the engine fed to
    /// frame `i` of the original run. Length == replay length.
    pub frames: Vec<RecordedFrame>,
}

impl InputRecording {
    /// Construct a fresh recording with the supplied seed and no
    /// frames. The recorder appends frames via
    /// [`crate::determinism::InputRecorder::tick`].
    pub fn new(initial_seed: u64) -> Self {
        Self {
            format_version: INPUT_RECORDING_FORMAT_VERSION,
            initial_seed,
            frames: Vec::new(),
        }
    }

    /// Number of frames in this recording.
    pub fn len(&self) -> usize {
        self.frames.len()
    }

    /// Whether the recording is empty (no frames captured yet).
    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }
}

/// One captured frame: the `dt` the engine fed to the schedule
/// plus the input state the gameplay code saw.
#[derive(Debug, Clone, PartialEq)]
pub struct RecordedFrame {
    /// Wall-clock dt the engine passed to `schedule.run`. Replay
    /// feeds this back so float integration stays bit-identical
    /// (ADR 0005 Q7 — replay uses recorded dt; fixed timestep is
    /// a future additive concern).
    pub dt: f32,
    /// Input snapshot the simulation observed at frame start.
    pub input: RecordedInput,
}

/// Owned, deterministic snapshot of a single frame's
/// [`InputState`]. The HashSet-backed input collections are
/// captured as sorted `Vec`s so two recordings of the same engine
/// state encode byte-identically.
///
/// See ADR 0005 Q4 for the rationale (byte-stability + on-disk
/// stable layout); the reconstructed `InputState` rebuilds the
/// HashSets internally from these sorted vecs.
#[derive(Debug, Clone, PartialEq)]
pub struct RecordedInput {
    /// Virtual-key codes held this frame. Sorted ascending.
    pub keys_down: Vec<u32>,
    /// Virtual-key codes pressed this frame (leading edge).
    /// Sorted ascending.
    pub keys_just_pressed: Vec<u32>,
    /// Virtual-key codes released this frame (trailing edge).
    /// Sorted ascending.
    pub keys_just_released: Vec<u32>,
    pub mouse_pos: (i32, i32),
    pub mouse_delta: (i32, i32),
    /// Mouse buttons held. Sorted ascending by encoded byte.
    pub mouse_buttons: Vec<u8>,
    pub mouse_just_pressed: Vec<u8>,
    pub mouse_just_released: Vec<u8>,
    pub mouse_wheel: f32,
}

impl RecordedInput {
    /// Capture a deterministic snapshot from the live `InputState`.
    /// HashSet contents are sorted on capture so the resulting
    /// vecs are byte-stable across runs (HashSets iterate in
    /// hasher-randomised order; sorting at the recording boundary
    /// is the gate that turns randomised iteration into
    /// reproducible bytes — ADR 0005 Q2).
    pub fn capture(state: &InputState) -> Self {
        let mut keys_down: Vec<u32> = state.keys_down_iter().collect();
        keys_down.sort_unstable();
        let mut keys_just_pressed: Vec<u32> = state.keys_just_pressed_iter().collect();
        keys_just_pressed.sort_unstable();
        let mut keys_just_released: Vec<u32> = state.keys_just_released_iter().collect();
        keys_just_released.sort_unstable();
        let mut mouse_buttons: Vec<u8> = state
            .mouse_buttons_iter()
            .map(encode_mouse_button)
            .collect();
        mouse_buttons.sort_unstable();
        let mut mouse_just_pressed: Vec<u8> = state
            .mouse_just_pressed_iter()
            .map(encode_mouse_button)
            .collect();
        mouse_just_pressed.sort_unstable();
        let mut mouse_just_released: Vec<u8> = state
            .mouse_just_released_iter()
            .map(encode_mouse_button)
            .collect();
        mouse_just_released.sort_unstable();
        Self {
            keys_down,
            keys_just_pressed,
            keys_just_released,
            mouse_pos: state.mouse_pos(),
            mouse_delta: state.mouse_delta(),
            mouse_buttons,
            mouse_just_pressed,
            mouse_just_released,
            mouse_wheel: state.mouse_wheel(),
        }
    }
}

/// Errors surfaced by the [`InputRecording`] codec. Each variant
/// names a specific corruption mode the decoder caught at the
/// boundary; no string-typed errors per ADR 0003 Pillar 4.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputRecordingError {
    /// First 8 bytes weren't [`INPUT_RECORDING_MAGIC`]. The bytes
    /// aren't a hold-engine input recording at all.
    InvalidMagic,
    /// Magic matched but the format version is one this binary
    /// doesn't recognise. Old recordings refuse to decode rather
    /// than silently misinterpret a layout change.
    UnsupportedVersion(u32),
    /// Stream ended before the decoder finished reading the
    /// declared payload. Truncated file or layout mismatch.
    UnexpectedEof,
    /// A length prefix would overflow `usize` on this platform.
    /// Effectively impossible for valid recordings; surfaced for
    /// completeness and to make malicious-input rejection
    /// explicit rather than implicit.
    LengthOverflow,
    /// A mouse-button byte fell outside the encoded range
    /// (0..=2). Indicates corruption or a future format that
    /// added buttons; either way, refuse rather than guess.
    InvalidMouseButton(u8),
}

impl std::fmt::Display for InputRecordingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidMagic => f.write_str("input recording: invalid magic header"),
            Self::UnsupportedVersion(v) => {
                write!(f, "input recording: unsupported format version {v}")
            }
            Self::UnexpectedEof => f.write_str("input recording: stream ended mid-frame"),
            Self::LengthOverflow => f.write_str("input recording: length prefix overflowed usize"),
            Self::InvalidMouseButton(b) => {
                write!(f, "input recording: invalid mouse-button encoding {b}")
            }
        }
    }
}

impl std::error::Error for InputRecordingError {}

// ── Mouse button encoding ─────────────────────────────────────────────────

/// Encode a [`MouseButton`] as a single byte. Stable across format
/// versions for backward compatibility; future buttons get fresh
/// integer codes rather than re-numbering.
pub(crate) fn encode_mouse_button(button: MouseButton) -> u8 {
    match button {
        MouseButton::Left => 0,
        MouseButton::Middle => 1,
        MouseButton::Right => 2,
    }
}

/// Decode a single byte back to a [`MouseButton`]. Returns
/// [`InputRecordingError::InvalidMouseButton`] for any value the
/// current format doesn't recognise.
pub(crate) fn decode_mouse_button(byte: u8) -> Result<MouseButton, InputRecordingError> {
    match byte {
        0 => Ok(MouseButton::Left),
        1 => Ok(MouseButton::Middle),
        2 => Ok(MouseButton::Right),
        other => Err(InputRecordingError::InvalidMouseButton(other)),
    }
}

// ── Codec ─────────────────────────────────────────────────────────────────

/// Encode an [`InputRecording`] into a self-contained byte buffer.
/// The output is byte-stable: the same recording produces the same
/// bytes across runs, across processes, across machines (within
/// the v1 same-binary same-architecture scope from ADR 0005 Q1).
///
/// Layout:
/// ```text
/// magic        : [u8; 8] == INPUT_RECORDING_MAGIC
/// version      : u32 LE
/// initial_seed : u64 LE
/// frame_count  : u32 LE
/// frames       : repeated RecordedFrame
///
/// RecordedFrame:
///   dt                       : f32 LE (raw bits)
///   keys_down                : len(u32 LE) + repeated u32 LE (sorted)
///   keys_just_pressed        : same shape
///   keys_just_released       : same shape
///   mouse_pos                : (i32 LE, i32 LE)
///   mouse_delta              : (i32 LE, i32 LE)
///   mouse_buttons            : len(u32 LE) + repeated u8 (sorted)
///   mouse_just_pressed       : same shape
///   mouse_just_released      : same shape
///   mouse_wheel              : f32 LE (raw bits)
/// ```
pub fn encode_input_recording(recording: &InputRecording) -> Vec<u8> {
    // Pre-compute capacity so encode does at most one allocation
    // (Pillar 5: footprint discipline; ADR 0005 Q10 holds the
    // recorder to ~80 bytes per frame at typical density —
    // pre-sizing keeps that bound honest).
    let mut frame_bytes = 0usize;
    for frame in &recording.frames {
        frame_bytes += 4 // dt
            + 4 + frame.input.keys_down.len() * 4
            + 4 + frame.input.keys_just_pressed.len() * 4
            + 4 + frame.input.keys_just_released.len() * 4
            + 8 // mouse_pos
            + 8 // mouse_delta
            + 4 + frame.input.mouse_buttons.len()
            + 4 + frame.input.mouse_just_pressed.len()
            + 4 + frame.input.mouse_just_released.len()
            + 4; // mouse_wheel
    }
    let header_bytes = 8 + 4 + 8 + 4;
    let mut out = Vec::with_capacity(header_bytes + frame_bytes);

    out.extend_from_slice(&INPUT_RECORDING_MAGIC);
    out.extend_from_slice(&recording.format_version.to_le_bytes());
    out.extend_from_slice(&recording.initial_seed.to_le_bytes());
    out.extend_from_slice(&(recording.frames.len() as u32).to_le_bytes());

    for frame in &recording.frames {
        out.extend_from_slice(&frame.dt.to_le_bytes());
        write_u32_vec(&mut out, &frame.input.keys_down);
        write_u32_vec(&mut out, &frame.input.keys_just_pressed);
        write_u32_vec(&mut out, &frame.input.keys_just_released);
        out.extend_from_slice(&frame.input.mouse_pos.0.to_le_bytes());
        out.extend_from_slice(&frame.input.mouse_pos.1.to_le_bytes());
        out.extend_from_slice(&frame.input.mouse_delta.0.to_le_bytes());
        out.extend_from_slice(&frame.input.mouse_delta.1.to_le_bytes());
        write_u8_vec(&mut out, &frame.input.mouse_buttons);
        write_u8_vec(&mut out, &frame.input.mouse_just_pressed);
        write_u8_vec(&mut out, &frame.input.mouse_just_released);
        out.extend_from_slice(&frame.input.mouse_wheel.to_le_bytes());
    }

    out
}

/// Decode a byte buffer back into an [`InputRecording`]. Validates
/// the magic header, format version, and frame structure; any
/// inconsistency surfaces as a typed [`InputRecordingError`]
/// rather than a panic.
pub fn decode_input_recording(bytes: &[u8]) -> Result<InputRecording, InputRecordingError> {
    let mut cursor = Cursor::new(bytes);
    if cursor.read_array::<8>()? != INPUT_RECORDING_MAGIC {
        return Err(InputRecordingError::InvalidMagic);
    }
    let version = cursor.read_u32()?;
    if version != INPUT_RECORDING_FORMAT_VERSION {
        return Err(InputRecordingError::UnsupportedVersion(version));
    }
    let initial_seed = cursor.read_u64()?;
    let frame_count = cursor.read_u32()? as usize;
    let mut frames = Vec::with_capacity(frame_count);
    for _ in 0..frame_count {
        let dt = cursor.read_f32()?;
        let keys_down = cursor.read_u32_vec()?;
        let keys_just_pressed = cursor.read_u32_vec()?;
        let keys_just_released = cursor.read_u32_vec()?;
        let mouse_pos = (cursor.read_i32()?, cursor.read_i32()?);
        let mouse_delta = (cursor.read_i32()?, cursor.read_i32()?);
        let mouse_buttons = cursor.read_u8_vec()?;
        let mouse_just_pressed = cursor.read_u8_vec()?;
        let mouse_just_released = cursor.read_u8_vec()?;
        let mouse_wheel = cursor.read_f32()?;
        // Validate every mouse-button byte at decode time so the
        // replayer never has to recheck. ADR 0003 Pillar 4: failures
        // surface at the boundary, not deep inside the simulation.
        for &b in &mouse_buttons {
            decode_mouse_button(b)?;
        }
        for &b in &mouse_just_pressed {
            decode_mouse_button(b)?;
        }
        for &b in &mouse_just_released {
            decode_mouse_button(b)?;
        }
        frames.push(RecordedFrame {
            dt,
            input: RecordedInput {
                keys_down,
                keys_just_pressed,
                keys_just_released,
                mouse_pos,
                mouse_delta,
                mouse_buttons,
                mouse_just_pressed,
                mouse_just_released,
                mouse_wheel,
            },
        });
    }
    Ok(InputRecording {
        format_version: version,
        initial_seed,
        frames,
    })
}

// ── Codec helpers ─────────────────────────────────────────────────────────

fn write_u32_vec(out: &mut Vec<u8>, v: &[u32]) {
    out.extend_from_slice(&(v.len() as u32).to_le_bytes());
    for x in v {
        out.extend_from_slice(&x.to_le_bytes());
    }
}

fn write_u8_vec(out: &mut Vec<u8>, v: &[u8]) {
    out.extend_from_slice(&(v.len() as u32).to_le_bytes());
    out.extend_from_slice(v);
}

struct Cursor<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    fn read_array<const N: usize>(&mut self) -> Result<[u8; N], InputRecordingError> {
        let end = self
            .pos
            .checked_add(N)
            .ok_or(InputRecordingError::LengthOverflow)?;
        if end > self.bytes.len() {
            return Err(InputRecordingError::UnexpectedEof);
        }
        let mut out = [0u8; N];
        out.copy_from_slice(&self.bytes[self.pos..end]);
        self.pos = end;
        Ok(out)
    }

    fn read_u32(&mut self) -> Result<u32, InputRecordingError> {
        Ok(u32::from_le_bytes(self.read_array::<4>()?))
    }

    fn read_u64(&mut self) -> Result<u64, InputRecordingError> {
        Ok(u64::from_le_bytes(self.read_array::<8>()?))
    }

    fn read_i32(&mut self) -> Result<i32, InputRecordingError> {
        Ok(i32::from_le_bytes(self.read_array::<4>()?))
    }

    fn read_f32(&mut self) -> Result<f32, InputRecordingError> {
        Ok(f32::from_le_bytes(self.read_array::<4>()?))
    }

    fn read_u32_vec(&mut self) -> Result<Vec<u32>, InputRecordingError> {
        let len = self.read_u32()? as usize;
        let mut out = Vec::with_capacity(len);
        for _ in 0..len {
            out.push(self.read_u32()?);
        }
        Ok(out)
    }

    fn read_u8_vec(&mut self) -> Result<Vec<u8>, InputRecordingError> {
        let len = self.read_u32()? as usize;
        let end = self
            .pos
            .checked_add(len)
            .ok_or(InputRecordingError::LengthOverflow)?;
        if end > self.bytes.len() {
            return Err(InputRecordingError::UnexpectedEof);
        }
        let out = self.bytes[self.pos..end].to_vec();
        self.pos = end;
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_frame(dt: f32) -> RecordedFrame {
        RecordedFrame {
            dt,
            input: RecordedInput {
                keys_down: vec![10, 20, 30],
                keys_just_pressed: vec![20],
                keys_just_released: vec![],
                mouse_pos: (100, 200),
                mouse_delta: (5, -3),
                mouse_buttons: vec![0],
                mouse_just_pressed: vec![],
                mouse_just_released: vec![],
                mouse_wheel: 1.5,
            },
        }
    }

    /// Round-trip: encode a recording, decode the bytes, get the
    /// original back exactly. Pillar 4 (debuggability): the codec
    /// must be lossless or replay diverges from record.
    #[test]
    fn input_recording_roundtrip_preserves_every_field() {
        let recording = InputRecording {
            format_version: INPUT_RECORDING_FORMAT_VERSION,
            initial_seed: 0xDEAD_BEEF_CAFE_BABE,
            frames: vec![sample_frame(0.016), sample_frame(0.017), sample_frame(0.015)],
        };
        let bytes = encode_input_recording(&recording);
        let decoded = decode_input_recording(&bytes).expect("decode");
        assert_eq!(decoded, recording);
    }

    /// Empty recording (zero frames) round-trips. Edge case: the
    /// frame-count prefix is zero and no frame data follows.
    #[test]
    fn input_recording_roundtrip_handles_zero_frames() {
        let recording = InputRecording::new(42);
        let bytes = encode_input_recording(&recording);
        assert_eq!(bytes.len(), 8 + 4 + 8 + 4); // header only
        let decoded = decode_input_recording(&bytes).expect("decode");
        assert_eq!(decoded, recording);
    }

    /// Decoder rejects bytes that don't start with the magic
    /// header. Tests pin the promise that random bytes can't be
    /// silently misinterpreted as a recording.
    #[test]
    fn decode_rejects_invalid_magic() {
        let err = decode_input_recording(b"not-a-real-recording-here").unwrap_err();
        assert_eq!(err, InputRecordingError::InvalidMagic);
    }

    /// Decoder rejects format versions it doesn't recognise. Old
    /// recordings refuse to decode rather than guess at a layout
    /// change. ADR 0005 Q4: the version is the breaking-change
    /// gate.
    #[test]
    fn decode_rejects_unsupported_version() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&INPUT_RECORDING_MAGIC);
        bytes.extend_from_slice(&999u32.to_le_bytes()); // bogus version
        bytes.extend_from_slice(&0u64.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        let err = decode_input_recording(&bytes).unwrap_err();
        assert_eq!(err, InputRecordingError::UnsupportedVersion(999));
    }

    /// Truncated recording surfaces as `UnexpectedEof`, not panic
    /// or silent partial decode. Pillar 4 (typed errors): every
    /// failure mode has a name.
    #[test]
    fn decode_rejects_truncated_recording() {
        let recording = InputRecording {
            format_version: INPUT_RECORDING_FORMAT_VERSION,
            initial_seed: 1,
            frames: vec![sample_frame(0.016)],
        };
        let mut bytes = encode_input_recording(&recording);
        bytes.pop(); // chop the last byte
        let err = decode_input_recording(&bytes).unwrap_err();
        assert_eq!(err, InputRecordingError::UnexpectedEof);
    }

    /// Decoder rejects mouse-button bytes outside the encoded
    /// range. Hand-craft a corrupt recording with byte 99 in
    /// mouse_buttons; the decoder catches it.
    #[test]
    fn decode_rejects_invalid_mouse_button_encoding() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&INPUT_RECORDING_MAGIC);
        bytes.extend_from_slice(&INPUT_RECORDING_FORMAT_VERSION.to_le_bytes());
        bytes.extend_from_slice(&0u64.to_le_bytes()); // seed
        bytes.extend_from_slice(&1u32.to_le_bytes()); // 1 frame
        // Frame body
        bytes.extend_from_slice(&0.016f32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes()); // keys_down: 0
        bytes.extend_from_slice(&0u32.to_le_bytes()); // keys_just_pressed: 0
        bytes.extend_from_slice(&0u32.to_le_bytes()); // keys_just_released: 0
        bytes.extend_from_slice(&0i32.to_le_bytes()); // mouse_pos.x
        bytes.extend_from_slice(&0i32.to_le_bytes()); // mouse_pos.y
        bytes.extend_from_slice(&0i32.to_le_bytes()); // mouse_delta.x
        bytes.extend_from_slice(&0i32.to_le_bytes()); // mouse_delta.y
        bytes.extend_from_slice(&1u32.to_le_bytes()); // mouse_buttons len = 1
        bytes.push(99); // bogus button code
        bytes.extend_from_slice(&0u32.to_le_bytes()); // mouse_just_pressed: 0
        bytes.extend_from_slice(&0u32.to_le_bytes()); // mouse_just_released: 0
        bytes.extend_from_slice(&0.0f32.to_le_bytes()); // mouse_wheel

        let err = decode_input_recording(&bytes).unwrap_err();
        assert_eq!(err, InputRecordingError::InvalidMouseButton(99));
    }

    /// Encoding twice produces byte-identical output — the
    /// load-bearing property for cross-run reproducibility (ADR
    /// 0005 Q4: byte-stable on-disk layout).
    #[test]
    fn encode_is_byte_deterministic_across_calls() {
        let recording = InputRecording {
            format_version: INPUT_RECORDING_FORMAT_VERSION,
            initial_seed: 0xCAFE,
            frames: vec![sample_frame(0.016), sample_frame(0.017)],
        };
        let bytes_a = encode_input_recording(&recording);
        let bytes_b = encode_input_recording(&recording);
        assert_eq!(bytes_a, bytes_b);
    }

    /// `RecordedInput::capture` produces byte-stable output even
    /// when the underlying HashSet was populated in different
    /// orders. Constructs two `InputState`s that *would* iterate
    /// in different hasher-randomised orders but should encode
    /// identically. (We can't control HashMap insertion order
    /// from a unit test directly, but we can insert the same
    /// keys via different paths and check the captured snapshot
    /// matches.)
    #[test]
    fn recorded_input_capture_is_sorted_for_byte_stability() {
        let mut state = InputState::new();
        // Press in arbitrary order; sorted vec should land in
        // ascending order regardless of insertion sequence.
        state.on_key_down(50);
        state.on_key_down(10);
        state.on_key_down(30);
        let captured = RecordedInput::capture(&state);
        assert_eq!(captured.keys_down, vec![10, 30, 50]);
        assert_eq!(captured.keys_just_pressed, vec![10, 30, 50]);
    }
}
