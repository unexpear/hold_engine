//! OpenGL renderer backend.
//!
//! Sets up a WGL context on the window's DC, then loads OpenGL function
//! pointers at runtime — no `gl` crate needed.
//!
//! Rendering primitives will be added here (VAO/VBO/shaders) as the engine grows.

use std::ffi::c_void;
use std::ptr::null_mut;
use crate::platform::ffi::*;
use super::{Color, Renderer};

// ── OpenGL types ──────────────────────────────────────────────────────────────

type GLuint    = u32;
type GLint     = i32;
type GLfloat   = f32;
type GLenum    = u32;
type GLbitfield= u32;
type GLboolean = u8;
type GLsizei   = i32;
type GLchar    = i8;

const GL_COLOR_BUFFER_BIT: GLbitfield = 0x00004000;
const GL_DEPTH_BUFFER_BIT: GLbitfield = 0x00000100;
const GL_DEPTH_TEST:       GLenum     = 0x0B71;
const GL_TRUE:             GLboolean  = 1;
const GL_FALSE:            GLboolean  = 0;

// ── Function pointer types ────────────────────────────────────────────────────

type GlClearColor = unsafe extern "system" fn(GLfloat, GLfloat, GLfloat, GLfloat);
type GlClear      = unsafe extern "system" fn(GLbitfield);
type GlViewport   = unsafe extern "system" fn(GLint, GLint, GLsizei, GLsizei);
type GlEnable     = unsafe extern "system" fn(GLenum);
type GlDisable    = unsafe extern "system" fn(GLenum);

// ── GL function loader ────────────────────────────────────────────────────────

/// Load a core (opengl32.dll) or extension (wglGetProcAddress) function.
///
/// Core functions (like glClear) are in opengl32.dll;
/// modern functions (like glCreateShader) come from wglGetProcAddress.
unsafe fn load_gl_fn(name: &str) -> *const c_void {
    let cname = cstr(name);
    // Try wglGetProcAddress first (extensions / modern GL).
    let ptr = wglGetProcAddress(cname.as_ptr());
    if !ptr.is_null() && ptr as usize > 3 {
        return ptr;
    }
    // Fall back to opengl32.dll for legacy core functions.
    let module_name = wstr("opengl32.dll");
    let module = LoadLibraryW(module_name.as_ptr());
    if module.is_null() { return null_mut(); }
    GetProcAddress(module, cname.as_ptr())
}

macro_rules! gl_fn {
    ($ctx:expr, $name:ident, $ty:ty) => {
        unsafe {
            let ptr = load_gl_fn(stringify!($name));
            if ptr.is_null() {
                eprintln!("[GL] Failed to load {}", stringify!($name));
                None
            } else {
                Some(std::mem::transmute::<*const c_void, $ty>(ptr))
            }
        }
    };
}

// ── Loaded GL functions ───────────────────────────────────────────────────────

struct GlFns {
    clear_color: GlClearColor,
    clear:       GlClear,
    viewport:    GlViewport,
    enable:      GlEnable,
}

impl GlFns {
    unsafe fn load() -> Self {
        let clear_color = std::mem::transmute::<*const c_void, GlClearColor>(
            load_gl_fn("glClearColor")
        );
        let clear = std::mem::transmute::<*const c_void, GlClear>(
            load_gl_fn("glClear")
        );
        let viewport = std::mem::transmute::<*const c_void, GlViewport>(
            load_gl_fn("glViewport")
        );
        let enable = std::mem::transmute::<*const c_void, GlEnable>(
            load_gl_fn("glEnable")
        );
        Self { clear_color, clear, viewport, enable }
    }
}

// ── OpenGLRenderer ────────────────────────────────────────────────────────────

pub struct OpenGLRenderer {
    hwnd:   HWND,
    hdc:    HDC,
    hglrc:  HGLRC,
    width:  u32,
    height: u32,
    gl:     GlFns,
}

impl OpenGLRenderer {
    /// Create a WGL context on `hwnd`. Panics if the driver doesn't support GL.
    pub fn new(hwnd: HWND, width: u32, height: u32) -> Self {
        unsafe {
            let hdc = GetDC(hwnd);
            assert!(!hdc.is_null(), "GetDC failed");

            // Choose a pixel format that supports OpenGL + double buffering.
            let pfd = PIXELFORMATDESCRIPTOR {
                nSize:       std::mem::size_of::<PIXELFORMATDESCRIPTOR>() as WORD,
                nVersion:    1,
                dwFlags:     PFD_DRAW_TO_WINDOW | PFD_SUPPORT_OPENGL | PFD_DOUBLEBUFFER,
                iPixelType:  PFD_TYPE_RGBA,
                cColorBits:  32,
                cDepthBits:  24,
                cStencilBits:8,
                iLayerType:  PFD_MAIN_PLANE,
                ..PIXELFORMATDESCRIPTOR::zeroed()
            };

            let fmt = ChoosePixelFormat(hdc, &pfd);
            assert!(fmt != 0, "ChoosePixelFormat failed");
            let ok = SetPixelFormat(hdc, fmt, &pfd);
            assert!(ok != 0, "SetPixelFormat failed");

            let hglrc = wglCreateContext(hdc);
            assert!(!hglrc.is_null(), "wglCreateContext failed");
            let ok = wglMakeCurrent(hdc, hglrc);
            assert!(ok != 0, "wglMakeCurrent failed");

            let gl = GlFns::load();

            // Enable depth test by default.
            (gl.enable)(GL_DEPTH_TEST);

            Self { hwnd, hdc, hglrc, width, height, gl }
        }
    }

    /// Returns the WGL rendering context handle (for extension setup).
    pub fn hglrc(&self) -> HGLRC { self.hglrc }
    /// Returns the device context handle.
    pub fn hdc(&self) -> HDC { self.hdc }
}

impl Renderer for OpenGLRenderer {
    fn width(&self)  -> u32 { self.width }
    fn height(&self) -> u32 { self.height }

    fn resize(&mut self, width: u32, height: u32) {
        self.width  = width;
        self.height = height;
        unsafe { (self.gl.viewport)(0, 0, width as GLsizei, height as GLsizei); }
    }

    fn begin_frame(&mut self) {}

    fn clear(&mut self, color: Color) {
        unsafe {
            (self.gl.clear_color)(color.r, color.g, color.b, color.a);
            (self.gl.clear)(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);
        }
    }

    fn end_frame(&mut self) {
        unsafe { SwapBuffers(self.hdc); }
    }
}

impl Drop for OpenGLRenderer {
    fn drop(&mut self) {
        unsafe {
            wglMakeCurrent(null_mut(), null_mut());
            wglDeleteContext(self.hglrc);
            ReleaseDC(self.hwnd, self.hdc);
        }
    }
}
