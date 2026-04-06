#[cfg(target_os = "windows")]
pub mod ffi;

#[cfg(target_os = "windows")]
pub mod window;

#[cfg(target_os = "windows")]
pub use window::Window;

#[cfg(not(target_os = "windows"))]
pub mod window {
    use std::ffi::c_void;

    use crate::input::InputState;

    /// Minimal non-Windows window shim so the crate compiles cross-platform.
    /// Runtime windowing is only implemented on Windows in this codebase.
    pub struct Window {
        pub hwnd: *mut c_void,
        width: u32,
        height: u32,
    }

    impl Window {
        pub fn new(_title: &str, width: u32, height: u32) -> Self {
            eprintln!("[platform] Window is only implemented on Windows; using non-Windows shim");
            Self { hwnd: std::ptr::null_mut(), width, height }
        }

        pub fn show(&self) {}
        pub fn set_title(&self, _title: &str) {}
        pub fn width(&self) -> u32 { self.width }
        pub fn height(&self) -> u32 { self.height }

        /// Non-Windows shim: returns `false` immediately so `Engine::run`
        /// exits without attempting unsupported Win32 message pumping.
        pub fn poll_events(&mut self, input: &mut InputState) -> bool {
            input.begin_frame();
            false
        }
    }
}

#[cfg(not(target_os = "windows"))]
pub use window::Window;
