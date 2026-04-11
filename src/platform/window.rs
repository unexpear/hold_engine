use super::ffi::*;
use crate::input::{InputState, MouseButton};
use std::cell::RefCell;
use std::ptr::null_mut;

// ── Event queue (thread-local, single window assumed) ─────────────────────────

#[derive(Debug)]
pub enum Event {
    KeyDown(u32),
    KeyUp(u32),
    MouseMove { x: i32, y: i32 },
    MouseButton { button: MouseButton, pressed: bool },
    MouseWheel(f32),
    Resize { width: u32, height: u32 },
    CloseRequested,
}

thread_local! {
    static PENDING: RefCell<Vec<Event>> = RefCell::new(Vec::new());
    static CLOSE_REQUESTED: RefCell<bool> = RefCell::new(false);
}

fn push_event(e: Event) {
    PENDING.with(|q| q.borrow_mut().push(e));
}

pub(crate) fn drain_events() -> Vec<Event> {
    PENDING.with(|q| q.borrow_mut().drain(..).collect())
}

pub(crate) fn was_close_requested() -> bool {
    CLOSE_REQUESTED.with(|b| *b.borrow())
}

// ── Window procedure ──────────────────────────────────────────────────────────

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: UINT,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_CLOSE => {
            CLOSE_REQUESTED.with(|b| *b.borrow_mut() = true);
            push_event(Event::CloseRequested);
            0
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            0
        }
        WM_SIZE => {
            let w = (lparam & 0xFFFF) as u32;
            let h = ((lparam >> 16) & 0xFFFF) as u32;
            push_event(Event::Resize {
                width: w,
                height: h,
            });
            0
        }
        WM_KEYDOWN | WM_SYSKEYDOWN => {
            push_event(Event::KeyDown(wparam as u32));
            0
        }
        WM_KEYUP | WM_SYSKEYUP => {
            push_event(Event::KeyUp(wparam as u32));
            0
        }
        WM_MOUSEMOVE => {
            push_event(Event::MouseMove {
                x: loword_lparam(lparam),
                y: hiword_lparam(lparam),
            });
            0
        }
        WM_LBUTTONDOWN => {
            push_event(Event::MouseButton {
                button: MouseButton::Left,
                pressed: true,
            });
            0
        }
        WM_LBUTTONUP => {
            push_event(Event::MouseButton {
                button: MouseButton::Left,
                pressed: false,
            });
            0
        }
        WM_RBUTTONDOWN => {
            push_event(Event::MouseButton {
                button: MouseButton::Right,
                pressed: true,
            });
            0
        }
        WM_RBUTTONUP => {
            push_event(Event::MouseButton {
                button: MouseButton::Right,
                pressed: false,
            });
            0
        }
        WM_MBUTTONDOWN => {
            push_event(Event::MouseButton {
                button: MouseButton::Middle,
                pressed: true,
            });
            0
        }
        WM_MBUTTONUP => {
            push_event(Event::MouseButton {
                button: MouseButton::Middle,
                pressed: false,
            });
            0
        }
        WM_MOUSEWHEEL => {
            let delta = hiword_wparam(wparam) as f32 / 120.0;
            push_event(Event::MouseWheel(delta));
            0
        }
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::zeroed();
            BeginPaint(hwnd, &mut ps);
            EndPaint(hwnd, &ps);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

// ── Window ────────────────────────────────────────────────────────────────────

pub struct Window {
    pub hwnd: HWND,
    pub hinstance: HINSTANCE,
    pub hdc: HDC,
    width: u32,
    height: u32,
    class_name: Vec<u16>,
}

impl Window {
    pub fn new(title: &str, width: u32, height: u32) -> Self {
        unsafe {
            let hinstance = GetModuleHandleW(null_mut());
            let class_name = wstr("ForgeEngineWindow");

            let wc = WNDCLASSEXW {
                cbSize: std::mem::size_of::<WNDCLASSEXW>() as UINT,
                style: CS_HREDRAW | CS_VREDRAW | CS_OWNDC,
                lpfnWndProc: wnd_proc,
                cbClsExtra: 0,
                cbWndExtra: 0,
                hInstance: hinstance,
                hIcon: null_mut(),
                hCursor: LoadCursorW(null_mut(), IDC_ARROW),
                hbrBackground: null_mut(),
                lpszMenuName: null_mut(),
                lpszClassName: class_name.as_ptr(),
                hIconSm: null_mut(),
            };
            RegisterClassExW(&wc);

            // Adjust window size so the *client* area is exactly width×height.
            let mut rect = RECT {
                left: 0,
                top: 0,
                right: width as LONG,
                bottom: height as LONG,
            };
            AdjustWindowRect(&mut rect, WS_OVERLAPPEDWINDOW, 0);
            let win_w = rect.right - rect.left;
            let win_h = rect.bottom - rect.top;

            let title_w = wstr(title);
            let hwnd = CreateWindowExW(
                0,
                class_name.as_ptr(),
                title_w.as_ptr(),
                WS_OVERLAPPEDWINDOW,
                // CW_USEDEFAULT
                0x8000_0000u32 as i32,
                0x8000_0000u32 as i32,
                win_w,
                win_h,
                null_mut(),
                null_mut(),
                hinstance,
                null_mut(),
            );
            assert!(!hwnd.is_null(), "CreateWindowExW failed");

            let hdc = GetDC(hwnd);
            assert!(!hdc.is_null(), "GetDC failed");

            Window {
                hwnd,
                hinstance,
                hdc,
                width,
                height,
                class_name,
            }
        }
    }

    pub fn show(&self) {
        unsafe {
            ShowWindow(self.hwnd, SW_SHOW);
            UpdateWindow(self.hwnd);
        }
    }

    pub fn set_title(&self, title: &str) {
        unsafe {
            SetWindowTextW(self.hwnd, wstr(title).as_ptr());
        }
    }

    pub fn width(&self) -> u32 {
        self.width
    }
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Poll OS messages. Returns `false` when the window should close.
    pub fn poll_events(&mut self, input: &mut InputState) -> bool {
        input.begin_frame();

        unsafe {
            let mut msg = MSG::zeroed();
            while PeekMessageW(&mut msg, null_mut(), 0, 0, PM_REMOVE) != 0 {
                if msg.message == WM_QUIT {
                    return false;
                }
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }

        if was_close_requested() {
            return false;
        }

        for event in drain_events() {
            match event {
                Event::KeyDown(vk) => input.on_key_down(vk),
                Event::KeyUp(vk) => input.on_key_up(vk),
                Event::MouseMove { x, y } => input.on_mouse_move(x, y),
                Event::MouseButton { button, pressed } => input.on_mouse_button(button, pressed),
                Event::MouseWheel(d) => input.on_mouse_wheel(d),
                Event::Resize { width, height } => {
                    self.width = width;
                    self.height = height;
                }
                Event::CloseRequested => return false,
            }
        }
        true
    }
}

impl Drop for Window {
    fn drop(&mut self) {
        unsafe {
            ReleaseDC(self.hwnd, self.hdc);
            DestroyWindow(self.hwnd);
            UnregisterClassW(self.class_name.as_ptr(), self.hinstance);
        }
    }
}
