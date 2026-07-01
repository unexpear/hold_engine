use std::collections::HashSet;

/// Logical mouse buttons.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}

/// Snapshot of the complete input state for one frame.
/// Updated by the engine before systems run.
///
/// `Clone` lets callers snapshot the live OS input for downstream
/// consumers that want an owned copy (e.g. the determinism replay
/// path in `examples/editor_shell.rs`, which needs to pass either
/// the live input or a synthesised replayed input to gameplay code
/// without disturbing the engine's underlying `&InputState`).
#[derive(Debug, Clone)]
pub struct InputState {
    keys_down: HashSet<u32>,
    keys_just_pressed: HashSet<u32>,
    keys_just_released: HashSet<u32>,

    mouse_pos: (i32, i32),
    mouse_delta: (i32, i32),
    mouse_buttons: HashSet<MouseButton>,
    mouse_just_pressed: HashSet<MouseButton>,
    mouse_just_released: HashSet<MouseButton>,
    mouse_wheel: f32,
}

impl InputState {
    pub fn new() -> Self {
        Self {
            keys_down: HashSet::new(),
            keys_just_pressed: HashSet::new(),
            keys_just_released: HashSet::new(),
            mouse_pos: (0, 0),
            mouse_delta: (0, 0),
            mouse_buttons: HashSet::new(),
            mouse_just_pressed: HashSet::new(),
            mouse_just_released: HashSet::new(),
            mouse_wheel: 0.0,
        }
    }

    // Called by the engine at the start of each frame to clear per-frame state.
    pub(crate) fn begin_frame(&mut self) {
        self.keys_just_pressed.clear();
        self.keys_just_released.clear();
        self.mouse_just_pressed.clear();
        self.mouse_just_released.clear();
        self.mouse_delta = (0, 0);
        self.mouse_wheel = 0.0;
    }

    pub(crate) fn on_key_down(&mut self, vk: u32) {
        if self.keys_down.insert(vk) {
            self.keys_just_pressed.insert(vk);
        }
    }

    pub(crate) fn on_key_up(&mut self, vk: u32) {
        self.keys_down.remove(&vk);
        self.keys_just_released.insert(vk);
    }

    pub(crate) fn on_mouse_move(&mut self, x: i32, y: i32) {
        self.mouse_delta.0 += x - self.mouse_pos.0;
        self.mouse_delta.1 += y - self.mouse_pos.1;
        self.mouse_pos = (x, y);
    }

    pub(crate) fn on_mouse_button(&mut self, btn: MouseButton, pressed: bool) {
        if pressed {
            if self.mouse_buttons.insert(btn) {
                self.mouse_just_pressed.insert(btn);
            }
        } else {
            self.mouse_buttons.remove(&btn);
            self.mouse_just_released.insert(btn);
        }
    }

    pub(crate) fn on_mouse_wheel(&mut self, delta: f32) {
        self.mouse_wheel += delta;
    }

    // ── Public query API ──────────────────────────────────────────────────────

    /// Virtual-key code held this frame.
    pub fn key(&self, vk: u32) -> bool {
        self.keys_down.contains(&vk)
    }
    /// Virtual-key code pressed this frame (leading edge).
    pub fn key_pressed(&self, vk: u32) -> bool {
        self.keys_just_pressed.contains(&vk)
    }
    /// Virtual-key code released this frame (trailing edge).
    pub fn key_released(&self, vk: u32) -> bool {
        self.keys_just_released.contains(&vk)
    }

    pub fn mouse_pos(&self) -> (i32, i32) {
        self.mouse_pos
    }
    pub fn mouse_delta(&self) -> (i32, i32) {
        self.mouse_delta
    }
    pub fn mouse_wheel(&self) -> f32 {
        self.mouse_wheel
    }
    pub fn mouse_button(&self, btn: MouseButton) -> bool {
        self.mouse_buttons.contains(&btn)
    }
    pub fn mouse_pressed(&self, btn: MouseButton) -> bool {
        self.mouse_just_pressed.contains(&btn)
    }
    pub fn mouse_released(&self, btn: MouseButton) -> bool {
        self.mouse_just_released.contains(&btn)
    }

    // ── Iteration accessors used by the determinism recorder ──────────────
    //
    // The HashSet-backed collections are encapsulated; the recorder
    // (`crate::determinism::RecordedInput::capture`) needs to walk them
    // to take a deterministic snapshot. Exposing iterators (rather
    // than the HashSet itself) preserves the invariant that internal
    // mutators stay `pub(crate)` while still letting the recorder
    // sort the contents into a byte-stable vec at the boundary.
    //
    // See `docs/adr/0005-determinism-architecture.md` Q4 for the
    // sorted-vec rationale.

    /// Iterate the virtual-key codes currently held. Order is the
    /// HashSet's hasher-randomised order; the recorder sorts on
    /// capture for byte-stability.
    pub fn keys_down_iter(&self) -> impl Iterator<Item = u32> + '_ {
        self.keys_down.iter().copied()
    }
    /// Iterate virtual-key codes pressed this frame (leading edge).
    pub fn keys_just_pressed_iter(&self) -> impl Iterator<Item = u32> + '_ {
        self.keys_just_pressed.iter().copied()
    }
    /// Iterate virtual-key codes released this frame (trailing edge).
    pub fn keys_just_released_iter(&self) -> impl Iterator<Item = u32> + '_ {
        self.keys_just_released.iter().copied()
    }
    /// Iterate mouse buttons currently held.
    pub fn mouse_buttons_iter(&self) -> impl Iterator<Item = MouseButton> + '_ {
        self.mouse_buttons.iter().copied()
    }
    /// Iterate mouse buttons pressed this frame.
    pub fn mouse_just_pressed_iter(&self) -> impl Iterator<Item = MouseButton> + '_ {
        self.mouse_just_pressed.iter().copied()
    }
    /// Iterate mouse buttons released this frame.
    pub fn mouse_just_released_iter(&self) -> impl Iterator<Item = MouseButton> + '_ {
        self.mouse_just_released.iter().copied()
    }

    /// Overwrite this `InputState` with the contents of a recorded
    /// snapshot. The replayer
    /// (`crate::determinism::InputReplayer::tick_into`) calls this
    /// once per replayed frame to mirror the recording's exact
    /// per-frame state, so the simulation observes the same bytes
    /// it observed during recording.
    ///
    /// Replaces every observable field; nothing carries over from
    /// the previous frame — the recorded snapshot is the truth.
    /// This is the load-bearing property for replay determinism
    /// (ADR 0005 Q1: same recording → same simulation).
    ///
    /// `keys_down` / `keys_just_pressed` / `keys_just_released` vecs
    /// in the snapshot may carry duplicates from a corrupt recording;
    /// `HashSet::insert` is idempotent so duplicates fold into a
    /// single entry. Unknown mouse-button bytes are silently
    /// dropped so a stale recording made against an older format
    /// version still applies partially rather than panicking.
    pub fn apply_recorded(&mut self, input: &crate::determinism::RecordedInput) {
        self.keys_down.clear();
        self.keys_down.extend(input.keys_down.iter().copied());
        self.keys_just_pressed.clear();
        self.keys_just_pressed
            .extend(input.keys_just_pressed.iter().copied());
        self.keys_just_released.clear();
        self.keys_just_released
            .extend(input.keys_just_released.iter().copied());
        self.mouse_pos = input.mouse_pos;
        self.mouse_delta = input.mouse_delta;
        self.mouse_buttons.clear();
        self.mouse_buttons.extend(
            input
                .mouse_buttons
                .iter()
                .filter_map(|b| decode_mouse_byte(*b)),
        );
        self.mouse_just_pressed.clear();
        self.mouse_just_pressed.extend(
            input
                .mouse_just_pressed
                .iter()
                .filter_map(|b| decode_mouse_byte(*b)),
        );
        self.mouse_just_released.clear();
        self.mouse_just_released.extend(
            input
                .mouse_just_released
                .iter()
                .filter_map(|b| decode_mouse_byte(*b)),
        );
        self.mouse_wheel = input.mouse_wheel;
    }
}

/// Decode a recorded mouse-button byte to a [`MouseButton`], mirroring
/// the encoding in [`crate::determinism::encode_mouse_button`].
/// Kept local to input so the `Rust` module tree stays a DAG (input
/// depends on determinism only through `RecordedInput`, not the codec).
fn decode_mouse_byte(byte: u8) -> Option<MouseButton> {
    match byte {
        0 => Some(MouseButton::Left),
        1 => Some(MouseButton::Middle),
        2 => Some(MouseButton::Right),
        _ => None,
    }
}

#[cfg(test)]
impl InputState {
    /// Testing helper: press a virtual-key code (moves it into
    /// both `keys_down` and `keys_just_pressed`). Determinism-harness
    /// fixtures use this to simulate input in
    /// `#[cfg(test)]` code without going through the OS event pump.
    pub fn test_press_key(&mut self, vk: u32) {
        self.on_key_down(vk);
    }
}

impl Default for InputState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::InputState;

    #[test]
    fn mouse_delta_accumulates_multiple_moves_per_frame() {
        let mut input = InputState::new();

        input.on_mouse_move(10, 5);
        input.on_mouse_move(14, 9);

        assert_eq!(input.mouse_delta(), (14, 9));
    }

    #[test]
    fn begin_frame_clears_mouse_delta() {
        let mut input = InputState::new();

        input.on_mouse_move(8, 3);
        input.begin_frame();

        assert_eq!(input.mouse_delta(), (0, 0));
    }
}

// Common Win32 virtual-key constants — re-exported for convenience.
pub mod vk {
    pub const BACKSPACE: u32 = 0x08;
    pub const ESCAPE: u32 = 0x1B;
    pub const SPACE: u32 = 0x20;
    pub const ENTER: u32 = 0x0D;
    pub const LEFT: u32 = 0x25;
    pub const UP: u32 = 0x26;
    pub const RIGHT: u32 = 0x27;
    pub const DOWN: u32 = 0x28;
    pub const SHIFT: u32 = 0x10;
    pub const CTRL: u32 = 0x11;
    pub const ALT: u32 = 0x12;
    pub const TAB: u32 = 0x09;
    pub const F1: u32 = 0x70;
    pub const F2: u32 = 0x71;
    pub const F3: u32 = 0x72;
    pub const F4: u32 = 0x73;
    pub const F5: u32 = 0x74;
    pub const F6: u32 = 0x75;
    pub const F7: u32 = 0x76;
    pub const F8: u32 = 0x77;
    pub const F9: u32 = 0x78;
    pub const F10: u32 = 0x79;
    pub const F11: u32 = 0x7A;
    pub const F12: u32 = 0x7B;
    // Letters: just cast b'A'..=b'Z' as u32 directly (Win32 uses ASCII).
    pub const W: u32 = b'W' as u32;
    pub const A: u32 = b'A' as u32;
    pub const S: u32 = b'S' as u32;
    pub const D: u32 = b'D' as u32;
    pub const Q: u32 = b'Q' as u32;
    pub const E: u32 = b'E' as u32;
    pub const K: u32 = b'K' as u32;
    pub const R: u32 = b'R' as u32;
    pub const P: u32 = b'P' as u32;
    pub const Y: u32 = b'Y' as u32;
    pub const Z: u32 = b'Z' as u32;
    // Digit row (VK_0..VK_9 = ASCII '0'..'9').
    pub const KEY_1: u32 = b'1' as u32;
    pub const KEY_2: u32 = b'2' as u32;
    pub const KEY_3: u32 = b'3' as u32;
    // OEM keys (US layout).
    pub const MINUS: u32 = 0xBD; // VK_OEM_MINUS  (-)
    pub const EQUALS: u32 = 0xBB; // VK_OEM_PLUS   (=)
    pub const LBRACKET: u32 = 0xDB; // VK_OEM_4      ([)
    pub const RBRACKET: u32 = 0xDD; // VK_OEM_6      (])
    pub const SEMICOLON: u32 = 0xBA; // VK_OEM_1      (;)
    pub const APOSTROPHE: u32 = 0xDE; // VK_OEM_7     (')
}
