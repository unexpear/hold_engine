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
        self.mouse_delta = (x - self.mouse_pos.0, y - self.mouse_pos.1);
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
    pub fn key(&self, vk: u32) -> bool { self.keys_down.contains(&vk) }
    /// Virtual-key code pressed this frame (leading edge).
    pub fn key_pressed(&self, vk: u32) -> bool { self.keys_just_pressed.contains(&vk) }
    /// Virtual-key code released this frame (trailing edge).
    pub fn key_released(&self, vk: u32) -> bool { self.keys_just_released.contains(&vk) }

    pub fn mouse_pos(&self) -> (i32, i32) { self.mouse_pos }
    pub fn mouse_delta(&self) -> (i32, i32) { self.mouse_delta }
    pub fn mouse_wheel(&self) -> f32 { self.mouse_wheel }
    pub fn mouse_button(&self, btn: MouseButton) -> bool { self.mouse_buttons.contains(&btn) }
    pub fn mouse_pressed(&self, btn: MouseButton) -> bool { self.mouse_just_pressed.contains(&btn) }
    pub fn mouse_released(&self, btn: MouseButton) -> bool { self.mouse_just_released.contains(&btn) }
}

impl Default for InputState {
    fn default() -> Self { Self::new() }
}

// Common Win32 virtual-key constants — re-exported for convenience.
pub mod vk {
    pub const ESCAPE: u32 = 0x1B;
    pub const SPACE: u32  = 0x20;
    pub const ENTER: u32  = 0x0D;
    pub const LEFT: u32   = 0x25;
    pub const UP: u32     = 0x26;
    pub const RIGHT: u32  = 0x27;
    pub const DOWN: u32   = 0x28;
    pub const SHIFT: u32  = 0x10;
    pub const CTRL: u32   = 0x11;
    pub const ALT: u32    = 0x12;
    pub const TAB: u32    = 0x09;
    pub const F1: u32     = 0x70;
    pub const F2: u32     = 0x71;
    pub const F3: u32     = 0x72;
    pub const F4: u32     = 0x73;
    pub const F5: u32     = 0x74;
    pub const F11: u32    = 0x7A;
    pub const F12: u32    = 0x7B;
    // Letters: just cast b'A'..=b'Z' as u32 directly (Win32 uses ASCII).
    pub const W: u32 = b'W' as u32;
    pub const A: u32 = b'A' as u32;
    pub const S: u32 = b'S' as u32;
    pub const D: u32 = b'D' as u32;
    pub const Q: u32 = b'Q' as u32;
    pub const E: u32 = b'E' as u32;
}
