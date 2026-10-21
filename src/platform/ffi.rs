//! Raw Win32 FFI bindings — no external crates.
#![allow(
    non_snake_case,
    non_camel_case_types,
    dead_code,
    clippy::upper_case_acronyms
)]

use std::ffi::c_void;

// ── Primitive aliases ─────────────────────────────────────────────────────────

pub type BOOL = i32;
pub type BYTE = u8;
pub type WORD = u16;
pub type DWORD = u32;
pub type LONG = i32;
pub type UINT = u32;
pub type ULONG_PTR = usize;
pub type LONG_PTR = isize;

pub type WPARAM = ULONG_PTR;
pub type LPARAM = LONG_PTR;
pub type LRESULT = LONG_PTR;

pub type HANDLE = *mut c_void;
pub type HWND = HANDLE;
pub type HDC = HANDLE;
pub type HINSTANCE = HANDLE;
pub type HMENU = HANDLE;
pub type HBRUSH = HANDLE;
pub type HICON = HANDLE;
pub type HCURSOR = HANDLE;
pub type HBITMAP = HANDLE;
pub type HGDIOBJ = HANDLE;
pub type HGLRC = HANDLE;
pub type HMODULE = HANDLE;
pub type ATOM = WORD;

pub type LPCWSTR = *const u16;
pub type LPWSTR = *mut u16;
pub type LPCSTR = *const i8;
pub type LPVOID = *mut c_void;
pub type WNDPROC = unsafe extern "system" fn(HWND, UINT, WPARAM, LPARAM) -> LRESULT;

// ── Window messages ───────────────────────────────────────────────────────────

pub const WM_DESTROY: UINT = 0x0002;
pub const WM_CLOSE: UINT = 0x0010;
pub const WM_QUIT: UINT = 0x0012;
pub const WM_SIZE: UINT = 0x0005;
pub const WM_PAINT: UINT = 0x000F;
pub const WM_KEYDOWN: UINT = 0x0100;
pub const WM_KEYUP: UINT = 0x0101;
pub const WM_SYSKEYDOWN: UINT = 0x0104;
pub const WM_SYSKEYUP: UINT = 0x0105;
pub const WM_MOUSEMOVE: UINT = 0x0200;
pub const WM_LBUTTONDOWN: UINT = 0x0201;
pub const WM_LBUTTONUP: UINT = 0x0202;
pub const WM_RBUTTONDOWN: UINT = 0x0204;
pub const WM_RBUTTONUP: UINT = 0x0205;
pub const WM_MBUTTONDOWN: UINT = 0x0207;
pub const WM_MBUTTONUP: UINT = 0x0208;
pub const WM_MOUSEWHEEL: UINT = 0x020A;

// ── Window class / style ──────────────────────────────────────────────────────

pub const CS_HREDRAW: UINT = 0x0002;
pub const CS_VREDRAW: UINT = 0x0001;
pub const CS_OWNDC: UINT = 0x0020;
pub const WS_OVERLAPPEDWINDOW: DWORD = 0x00CF0000;
pub const WS_VISIBLE: DWORD = 0x10000000;
pub const SW_SHOW: i32 = 5;
pub const PM_REMOVE: UINT = 0x0001;

// ── GDI ───────────────────────────────────────────────────────────────────────

pub const BI_RGB: DWORD = 0;
pub const DIB_RGB_COLORS: UINT = 0;
pub const SRCCOPY: DWORD = 0x00CC0020;

// ── OpenGL pixel format ───────────────────────────────────────────────────────

pub const PFD_DRAW_TO_WINDOW: DWORD = 0x00000004;
pub const PFD_SUPPORT_OPENGL: DWORD = 0x00000020;
pub const PFD_DOUBLEBUFFER: DWORD = 0x00000001;
pub const PFD_TYPE_RGBA: BYTE = 0;
pub const PFD_MAIN_PLANE: BYTE = 0;

// IDC_ARROW
pub const IDC_ARROW: LPCWSTR = 32512 as LPCWSTR;

// SetWindowLongPtr / GetWindowLongPtr index
pub const GWLP_USERDATA: i32 = -21;

// ── Structs ───────────────────────────────────────────────────────────────────

#[repr(C)]
pub struct WNDCLASSEXW {
    pub cbSize: UINT,
    pub style: UINT,
    pub lpfnWndProc: WNDPROC,
    pub cbClsExtra: i32,
    pub cbWndExtra: i32,
    pub hInstance: HINSTANCE,
    pub hIcon: HICON,
    pub hCursor: HCURSOR,
    pub hbrBackground: HBRUSH,
    pub lpszMenuName: LPCWSTR,
    pub lpszClassName: LPCWSTR,
    pub hIconSm: HICON,
}

#[repr(C)]
pub struct MSG {
    pub hwnd: HWND,
    pub message: UINT,
    pub wParam: WPARAM,
    pub lParam: LPARAM,
    pub time: DWORD,
    pub pt_x: LONG,
    pub pt_y: LONG,
}
impl MSG {
    pub fn zeroed() -> Self {
        unsafe { std::mem::zeroed() }
    }
}

#[repr(C)]
pub struct PAINTSTRUCT {
    pub hdc: HDC,
    pub fErase: BOOL,
    pub rcPaint: RECT,
    pub fRestore: BOOL,
    pub fIncUpdate: BOOL,
    pub rgbReserved: [u8; 32],
}
impl PAINTSTRUCT {
    pub fn zeroed() -> Self {
        unsafe { std::mem::zeroed() }
    }
}

#[repr(C)]
pub struct RECT {
    pub left: LONG,
    pub top: LONG,
    pub right: LONG,
    pub bottom: LONG,
}

#[repr(C)]
pub struct BITMAPINFOHEADER {
    pub biSize: DWORD,
    pub biWidth: LONG,
    pub biHeight: LONG,
    pub biPlanes: WORD,
    pub biBitCount: WORD,
    pub biCompression: DWORD,
    pub biSizeImage: DWORD,
    pub biXPelsPerMeter: LONG,
    pub biYPelsPerMeter: LONG,
    pub biClrUsed: DWORD,
    pub biClrImportant: DWORD,
}

#[repr(C)]
pub struct RGBQUAD {
    pub rgbBlue: BYTE,
    pub rgbGreen: BYTE,
    pub rgbRed: BYTE,
    pub rgbReserved: BYTE,
}

#[repr(C)]
pub struct BITMAPINFO {
    pub bmiHeader: BITMAPINFOHEADER,
    pub bmiColors: [RGBQUAD; 1],
}

#[repr(C)]
pub struct PIXELFORMATDESCRIPTOR {
    pub nSize: WORD,
    pub nVersion: WORD,
    pub dwFlags: DWORD,
    pub iPixelType: BYTE,
    pub cColorBits: BYTE,
    pub cRedBits: BYTE,
    pub cRedShift: BYTE,
    pub cGreenBits: BYTE,
    pub cGreenShift: BYTE,
    pub cBlueBits: BYTE,
    pub cBlueShift: BYTE,
    pub cAlphaBits: BYTE,
    pub cAlphaShift: BYTE,
    pub cAccumBits: BYTE,
    pub cAccumRedBits: BYTE,
    pub cAccumGreenBits: BYTE,
    pub cAccumBlueBits: BYTE,
    pub cAccumAlphaBits: BYTE,
    pub cDepthBits: BYTE,
    pub cStencilBits: BYTE,
    pub cAuxBuffers: BYTE,
    pub iLayerType: BYTE,
    pub bReserved: BYTE,
    pub dwLayerMask: DWORD,
    pub dwVisibleMask: DWORD,
    pub dwDamageMask: DWORD,
}
impl PIXELFORMATDESCRIPTOR {
    pub fn zeroed() -> Self {
        unsafe { std::mem::zeroed() }
    }
}

// ── Linked extern functions ───────────────────────────────────────────────────

#[link(name = "user32")]
extern "system" {
    pub fn RegisterClassExW(lpWndClass: *const WNDCLASSEXW) -> ATOM;
    pub fn UnregisterClassW(lpClassName: LPCWSTR, hInstance: HINSTANCE) -> BOOL;
    pub fn CreateWindowExW(
        dwExStyle: DWORD,
        lpClassName: LPCWSTR,
        lpWindowName: LPCWSTR,
        dwStyle: DWORD,
        x: i32,
        y: i32,
        nWidth: i32,
        nHeight: i32,
        hWndParent: HWND,
        hMenu: HMENU,
        hInstance: HINSTANCE,
        lpParam: LPVOID,
    ) -> HWND;
    pub fn DestroyWindow(hWnd: HWND) -> BOOL;
    pub fn ShowWindow(hWnd: HWND, nCmdShow: i32) -> BOOL;
    pub fn UpdateWindow(hWnd: HWND) -> BOOL;
    pub fn PeekMessageW(
        lpMsg: *mut MSG,
        hWnd: HWND,
        wMsgFilterMin: UINT,
        wMsgFilterMax: UINT,
        wRemoveMsg: UINT,
    ) -> BOOL;
    pub fn TranslateMessage(lpMsg: *const MSG) -> BOOL;
    pub fn DispatchMessageW(lpMsg: *const MSG) -> LRESULT;
    pub fn PostQuitMessage(nExitCode: i32);
    pub fn DefWindowProcW(hWnd: HWND, Msg: UINT, wParam: WPARAM, lParam: LPARAM) -> LRESULT;
    pub fn GetDC(hWnd: HWND) -> HDC;
    pub fn ReleaseDC(hWnd: HWND, hDC: HDC) -> i32;
    pub fn BeginPaint(hWnd: HWND, lpPaint: *mut PAINTSTRUCT) -> HDC;
    pub fn EndPaint(hWnd: HWND, lpPaint: *const PAINTSTRUCT) -> BOOL;
    pub fn InvalidateRect(hWnd: HWND, lpRect: *const RECT, bErase: BOOL) -> BOOL;
    pub fn GetClientRect(hWnd: HWND, lpRect: *mut RECT) -> BOOL;
    pub fn SetWindowTextW(hWnd: HWND, lpString: LPCWSTR) -> BOOL;
    pub fn LoadCursorW(hInstance: HINSTANCE, lpCursorName: LPCWSTR) -> HCURSOR;
    pub fn SetWindowLongPtrW(hWnd: HWND, nIndex: i32, dwNewLong: LONG_PTR) -> LONG_PTR;
    pub fn GetWindowLongPtrW(hWnd: HWND, nIndex: i32) -> LONG_PTR;
    pub fn AdjustWindowRect(lpRect: *mut RECT, dwStyle: DWORD, bMenu: BOOL) -> BOOL;
}

#[link(name = "kernel32")]
extern "system" {
    pub fn GetModuleHandleW(lpModuleName: LPCWSTR) -> HINSTANCE;
    pub fn QueryPerformanceCounter(lpPerformanceCount: *mut i64) -> BOOL;
    pub fn QueryPerformanceFrequency(lpFrequency: *mut i64) -> BOOL;
    pub fn LoadLibraryW(lpLibFileName: LPCWSTR) -> HMODULE;
    pub fn GetProcAddress(hModule: HMODULE, lpProcName: LPCSTR) -> *const c_void;
    pub fn FreeLibrary(hLibModule: HMODULE) -> BOOL;
    pub fn GetLastError() -> DWORD;
}

#[link(name = "gdi32")]
extern "system" {
    pub fn CreateDIBSection(
        hdc: HDC,
        pbmi: *const BITMAPINFO,
        usage: UINT,
        ppvBits: *mut *mut c_void,
        hSection: HANDLE,
        offset: DWORD,
    ) -> HBITMAP;
    pub fn CreateCompatibleDC(hdc: HDC) -> HDC;
    pub fn SelectObject(hdc: HDC, h: HGDIOBJ) -> HGDIOBJ;
    pub fn DeleteObject(ho: HGDIOBJ) -> BOOL;
    pub fn DeleteDC(hdc: HDC) -> BOOL;
    pub fn BitBlt(
        hdc: HDC,
        x: i32,
        y: i32,
        cx: i32,
        cy: i32,
        hdcSrc: HDC,
        x1: i32,
        y1: i32,
        rop: DWORD,
    ) -> BOOL;
    pub fn StretchDIBits(
        hdc: HDC,
        xDest: i32,
        yDest: i32,
        DestWidth: i32,
        DestHeight: i32,
        xSrc: i32,
        ySrc: i32,
        SrcWidth: i32,
        SrcHeight: i32,
        lpBits: *const c_void,
        lpbmi: *const BITMAPINFO,
        iUsage: UINT,
        rop: DWORD,
    ) -> i32;
    pub fn ChoosePixelFormat(hdc: HDC, ppfd: *const PIXELFORMATDESCRIPTOR) -> i32;
    pub fn SetPixelFormat(hdc: HDC, format: i32, ppfd: *const PIXELFORMATDESCRIPTOR) -> BOOL;
    pub fn SwapBuffers(hdc: HDC) -> BOOL;
    pub fn DescribePixelFormat(
        hdc: HDC,
        iPixelFormat: i32,
        nBytes: UINT,
        ppfd: *mut PIXELFORMATDESCRIPTOR,
    ) -> i32;
    pub fn PatBlt(hdc: HDC, x: i32, y: i32, w: i32, h: i32, rop: DWORD) -> BOOL;
}

#[link(name = "opengl32")]
extern "system" {
    pub fn wglCreateContext(hdc: HDC) -> HGLRC;
    pub fn wglMakeCurrent(hdc: HDC, hglrc: HGLRC) -> BOOL;
    pub fn wglDeleteContext(hglrc: HGLRC) -> BOOL;
    pub fn wglGetProcAddress(lpszProc: LPCSTR) -> *const c_void;
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Encode a Rust `&str` as a null-terminated UTF-16 slice.
pub fn wstr(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Encode a Rust `&str` as a null-terminated C string (ASCII only).
pub fn cstr(s: &str) -> Vec<i8> {
    s.bytes()
        .map(|b| b as i8)
        .chain(std::iter::once(0))
        .collect()
}

/// Extract the low word of an LPARAM (e.g. mouse X from WM_MOUSEMOVE).
pub fn loword_lparam(l: LPARAM) -> i32 {
    (l & 0xFFFF) as i16 as i32
}
/// Extract the high word of an LPARAM (e.g. mouse Y from WM_MOUSEMOVE).
pub fn hiword_lparam(l: LPARAM) -> i32 {
    ((l >> 16) & 0xFFFF) as i16 as i32
}
/// Extract the high word of a WPARAM (e.g. wheel delta from WM_MOUSEWHEEL).
pub fn hiword_wparam(w: WPARAM) -> i16 {
    ((w >> 16) & 0xFFFF) as i16
}
