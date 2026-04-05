//! OpenGL renderer backend.
//!
//! Sets up a WGL context on the window's DC, then loads OpenGL function
//! pointers at runtime — no `gl` crate needed.
//!
//! Supports uploading meshes (VAO/VBO/EBO), compiling GLSL shaders,
//! uploading RGBA8 textures, and issuing indexed draw calls.

use std::ffi::c_void;
use std::ptr::null_mut;
use crate::assets::mesh::Vertex;
use crate::assets::types::GpuMesh;
use crate::assets::mesh::MeshData;
use crate::platform::ffi::*;
use crate::math::{Mat4, Vec3};
use super::{Color, Renderer};

// ── OpenGL primitive types ────────────────────────────────────────────────────

type GLuint    = u32;
type GLint     = i32;
type GLfloat   = f32;
type GLenum    = u32;
type GLbitfield= u32;
type GLboolean = u8;
type GLsizei   = i32;
type GLsizeiptr= isize;
type GLchar    = i8;

// ── Core clear/enable constants ───────────────────────────────────────────────

const GL_COLOR_BUFFER_BIT: GLbitfield = 0x0000_4000;
const GL_DEPTH_BUFFER_BIT: GLbitfield = 0x0000_0100;
const GL_DEPTH_TEST:       GLenum     = 0x0B71;
#[allow(dead_code)]
const GL_TRUE:             GLboolean  = 1;
#[allow(dead_code)]
const GL_FALSE:            GLboolean  = 0;

// ── Buffer / draw constants ───────────────────────────────────────────────────

const GL_ARRAY_BUFFER:         GLenum = 0x8892;
const GL_ELEMENT_ARRAY_BUFFER: GLenum = 0x8893;
const GL_STATIC_DRAW:          GLenum = 0x88B4;
#[allow(dead_code)]
const GL_DYNAMIC_DRAW:         GLenum = 0x88B8;

const GL_TRIANGLES:            GLenum = 0x0004;
#[allow(dead_code)]
const GL_UNSIGNED_SHORT:       GLenum = 0x1403;
#[allow(dead_code)]
const GL_UNSIGNED_INT:         GLenum = 0x1405;
const GL_FLOAT:                GLenum = 0x1406;

// ── Shader constants ──────────────────────────────────────────────────────────

const GL_VERTEX_SHADER:        GLenum = 0x8B31;
const GL_FRAGMENT_SHADER:      GLenum = 0x8B30;
const GL_COMPILE_STATUS:       GLenum = 0x8B81;
const GL_LINK_STATUS:          GLenum = 0x8B82;
const GL_INFO_LOG_LENGTH:      GLenum = 0x8B84;

// ── Texture constants ─────────────────────────────────────────────────────────

const GL_TEXTURE_2D:           GLenum = 0x0DE1;
#[allow(dead_code)]
const GL_TEXTURE0:             GLenum = 0x84C0;
const GL_RGBA:                 GLenum = 0x1908;
#[allow(dead_code)]
const GL_RGB:                  GLenum = 0x1907;
const GL_UNSIGNED_BYTE:        GLenum = 0x1401;
const GL_TEXTURE_MIN_FILTER:   GLenum = 0x2801;
const GL_TEXTURE_MAG_FILTER:   GLenum = 0x2800;
const GL_LINEAR:               GLint  = 0x2601;
const GL_LINEAR_MIPMAP_LINEAR: GLint  = 0x2703;
const GL_TEXTURE_WRAP_S:       GLenum = 0x2802;
const GL_TEXTURE_WRAP_T:       GLenum = 0x2803;
const GL_REPEAT:               GLint  = 0x2901;
#[allow(dead_code)]
const GL_CLAMP_TO_EDGE:        GLint  = 0x812F;

// ── Depth / blend constants ───────────────────────────────────────────────────

#[allow(dead_code)]
const GL_LEQUAL:               GLenum = 0x0203;


// ── Function pointer types ────────────────────────────────────────────────────

// Core clear / state
type GlClearColor = unsafe extern "system" fn(GLfloat, GLfloat, GLfloat, GLfloat);
type GlClear      = unsafe extern "system" fn(GLbitfield);
type GlViewport   = unsafe extern "system" fn(GLint, GLint, GLsizei, GLsizei);
type GlEnable     = unsafe extern "system" fn(GLenum);
#[allow(dead_code)]
type GlDisable    = unsafe extern "system" fn(GLenum);

// VAO
type GlGenVertexArrays      = unsafe extern "system" fn(GLsizei, *mut GLuint);
type GlBindVertexArray      = unsafe extern "system" fn(GLuint);
type GlDeleteVertexArrays   = unsafe extern "system" fn(GLsizei, *const GLuint);

// Buffer objects
type GlGenBuffers            = unsafe extern "system" fn(GLsizei, *mut GLuint);
type GlBindBuffer            = unsafe extern "system" fn(GLenum, GLuint);
type GlBufferData            = unsafe extern "system" fn(GLenum, GLsizeiptr, *const c_void, GLenum);
type GlDeleteBuffers         = unsafe extern "system" fn(GLsizei, *const GLuint);
type GlVertexAttribPointer   = unsafe extern "system" fn(GLuint, GLint, GLenum, GLboolean, GLsizei, *const c_void);
type GlEnableVertexAttribArray = unsafe extern "system" fn(GLuint);

// Shaders
type GlCreateShader     = unsafe extern "system" fn(GLenum) -> GLuint;
type GlShaderSource     = unsafe extern "system" fn(GLuint, GLsizei, *const *const GLchar, *const GLint);
type GlCompileShader    = unsafe extern "system" fn(GLuint);
type GlGetShaderiv      = unsafe extern "system" fn(GLuint, GLenum, *mut GLint);
type GlGetShaderInfoLog = unsafe extern "system" fn(GLuint, GLsizei, *mut GLsizei, *mut GLchar);
type GlDeleteShader     = unsafe extern "system" fn(GLuint);
type GlCreateProgram    = unsafe extern "system" fn() -> GLuint;
type GlAttachShader     = unsafe extern "system" fn(GLuint, GLuint);
type GlLinkProgram      = unsafe extern "system" fn(GLuint);
type GlGetProgramiv     = unsafe extern "system" fn(GLuint, GLenum, *mut GLint);
type GlGetProgramInfoLog= unsafe extern "system" fn(GLuint, GLsizei, *mut GLsizei, *mut GLchar);
type GlUseProgram       = unsafe extern "system" fn(GLuint);
type GlDeleteProgram    = unsafe extern "system" fn(GLuint);

// Draw
type GlDrawElements = unsafe extern "system" fn(GLenum, GLsizei, GLenum, *const c_void);
#[allow(dead_code)]
type GlDrawArrays   = unsafe extern "system" fn(GLenum, GLint, GLsizei);

// Uniforms
type GlGetUniformLocation = unsafe extern "system" fn(GLuint, *const GLchar) -> GLint;
type GlUniformMatrix4fv   = unsafe extern "system" fn(GLint, GLsizei, GLboolean, *const GLfloat);
#[allow(dead_code)]
type GlUniform1i          = unsafe extern "system" fn(GLint, GLint);
type GlUniform3fv         = unsafe extern "system" fn(GLint, GLsizei, *const GLfloat);
type GlUniform4fv         = unsafe extern "system" fn(GLint, GLsizei, *const GLfloat);

// Textures
type GlGenTextures      = unsafe extern "system" fn(GLsizei, *mut GLuint);
type GlDeleteTextures   = unsafe extern "system" fn(GLsizei, *const GLuint);
type GlBindTexture      = unsafe extern "system" fn(GLenum, GLuint);
type GlTexImage2D       = unsafe extern "system" fn(GLenum, GLint, GLint, GLsizei, GLsizei, GLint, GLenum, GLenum, *const c_void);
type GlTexParameteri    = unsafe extern "system" fn(GLenum, GLenum, GLint);
type GlActiveTexture    = unsafe extern "system" fn(GLenum);
type GlGenerateMipmap   = unsafe extern "system" fn(GLenum);
type GlPixelStorei      = unsafe extern "system" fn(GLenum, GLint);

// State
#[allow(dead_code)]
type GlDepthFunc        = unsafe extern "system" fn(GLenum);
#[allow(dead_code)]
type GlBlendFunc        = unsafe extern "system" fn(GLenum, GLenum);
#[allow(dead_code)]
type GlBlendEquation    = unsafe extern "system" fn(GLenum);

// ── GL function loader ────────────────────────────────────────────────────────

/// Load a core (opengl32.dll) or extension (wglGetProcAddress) function.
///
/// Core functions (like glClear) are in opengl32.dll;
/// modern functions (like glCreateShader) come from wglGetProcAddress.
/// The `opengl32` module handle must have been loaded once at startup and
/// passed here — it is NOT freed by this function.
unsafe fn load_gl_fn(name: &str, opengl32: HMODULE) -> *const c_void {
    let cname = cstr(name);
    // Try wglGetProcAddress first (extensions / modern GL).
    let ptr = wglGetProcAddress(cname.as_ptr());
    if !ptr.is_null() && ptr as usize > 3 {
        return ptr;
    }
    // Fall back to opengl32.dll for legacy core functions.
    if opengl32.is_null() { return null_mut(); }
    GetProcAddress(opengl32, cname.as_ptr())
}

macro_rules! gl_fn {
    ($ctx:expr, $name:ident, $ty:ty) => {
        unsafe {
            let ptr = load_gl_fn(stringify!($name), $ctx);
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
    // Core clear / state
    clear_color: GlClearColor,
    clear:       GlClear,
    viewport:    GlViewport,
    enable:      GlEnable,

    // VAO
    gen_vertex_arrays:    GlGenVertexArrays,
    bind_vertex_array:    GlBindVertexArray,
    delete_vertex_arrays: GlDeleteVertexArrays,

    // Buffer objects
    gen_buffers:                GlGenBuffers,
    bind_buffer:                GlBindBuffer,
    buffer_data:                GlBufferData,
    delete_buffers:             GlDeleteBuffers,
    vertex_attrib_pointer:      GlVertexAttribPointer,
    enable_vertex_attrib_array: GlEnableVertexAttribArray,

    // Shaders
    create_shader:       GlCreateShader,
    shader_source:       GlShaderSource,
    compile_shader:      GlCompileShader,
    get_shaderiv:        GlGetShaderiv,
    get_shader_info_log: GlGetShaderInfoLog,
    delete_shader:       GlDeleteShader,
    create_program:      GlCreateProgram,
    attach_shader:       GlAttachShader,
    link_program:        GlLinkProgram,
    get_programiv:       GlGetProgramiv,
    get_program_info_log:GlGetProgramInfoLog,
    use_program:         GlUseProgram,
    delete_program:      GlDeleteProgram,

    // Draw
    draw_elements: GlDrawElements,

    // Uniforms
    get_uniform_location: GlGetUniformLocation,
    uniform_matrix4fv:    GlUniformMatrix4fv,
    uniform3fv:           GlUniform3fv,
    uniform4fv:           GlUniform4fv,

    // Textures
    gen_textures:    GlGenTextures,
    delete_textures: GlDeleteTextures,
    bind_texture:    GlBindTexture,
    tex_image2d:     GlTexImage2D,
    tex_parameteri:  GlTexParameteri,
    active_texture:  GlActiveTexture,
    generate_mipmap: GlGenerateMipmap,
    pixel_storei:    GlPixelStorei,
}

impl GlFns {
    unsafe fn load(opengl32: HMODULE) -> Self {
        // ── Core — always present ──────────────────────────────────────────────
        let clear_color = std::mem::transmute::<*const c_void, GlClearColor>(
            load_gl_fn("glClearColor", opengl32)
        );
        let clear = std::mem::transmute::<*const c_void, GlClear>(
            load_gl_fn("glClear", opengl32)
        );
        let viewport = std::mem::transmute::<*const c_void, GlViewport>(
            load_gl_fn("glViewport", opengl32)
        );
        let enable = std::mem::transmute::<*const c_void, GlEnable>(
            load_gl_fn("glEnable", opengl32)
        );

        // ── Modern GL — unwrap: a missing entry means the driver is broken ─────
        let gen_vertex_arrays    = gl_fn!(opengl32, glGenVertexArrays,    GlGenVertexArrays   ).unwrap();
        let bind_vertex_array    = gl_fn!(opengl32, glBindVertexArray,    GlBindVertexArray   ).unwrap();
        let delete_vertex_arrays = gl_fn!(opengl32, glDeleteVertexArrays, GlDeleteVertexArrays).unwrap();

        let gen_buffers                = gl_fn!(opengl32, glGenBuffers,               GlGenBuffers              ).unwrap();
        let bind_buffer                = gl_fn!(opengl32, glBindBuffer,               GlBindBuffer              ).unwrap();
        let buffer_data                = gl_fn!(opengl32, glBufferData,               GlBufferData              ).unwrap();
        let delete_buffers             = gl_fn!(opengl32, glDeleteBuffers,            GlDeleteBuffers           ).unwrap();
        let vertex_attrib_pointer      = gl_fn!(opengl32, glVertexAttribPointer,      GlVertexAttribPointer     ).unwrap();
        let enable_vertex_attrib_array = gl_fn!(opengl32, glEnableVertexAttribArray,  GlEnableVertexAttribArray ).unwrap();

        let create_shader        = gl_fn!(opengl32, glCreateShader,       GlCreateShader      ).unwrap();
        let shader_source        = gl_fn!(opengl32, glShaderSource,       GlShaderSource      ).unwrap();
        let compile_shader       = gl_fn!(opengl32, glCompileShader,      GlCompileShader     ).unwrap();
        let get_shaderiv         = gl_fn!(opengl32, glGetShaderiv,        GlGetShaderiv       ).unwrap();
        let get_shader_info_log  = gl_fn!(opengl32, glGetShaderInfoLog,   GlGetShaderInfoLog  ).unwrap();
        let delete_shader        = gl_fn!(opengl32, glDeleteShader,       GlDeleteShader      ).unwrap();
        let create_program       = gl_fn!(opengl32, glCreateProgram,      GlCreateProgram     ).unwrap();
        let attach_shader        = gl_fn!(opengl32, glAttachShader,       GlAttachShader      ).unwrap();
        let link_program         = gl_fn!(opengl32, glLinkProgram,        GlLinkProgram       ).unwrap();
        let get_programiv        = gl_fn!(opengl32, glGetProgramiv,       GlGetProgramiv      ).unwrap();
        let get_program_info_log = gl_fn!(opengl32, glGetProgramInfoLog,  GlGetProgramInfoLog ).unwrap();
        let use_program          = gl_fn!(opengl32, glUseProgram,         GlUseProgram        ).unwrap();
        let delete_program       = gl_fn!(opengl32, glDeleteProgram,      GlDeleteProgram     ).unwrap();

        let draw_elements = gl_fn!(opengl32, glDrawElements, GlDrawElements).unwrap();

        let get_uniform_location = gl_fn!(opengl32, glGetUniformLocation, GlGetUniformLocation).unwrap();
        let uniform_matrix4fv    = gl_fn!(opengl32, glUniformMatrix4fv,   GlUniformMatrix4fv  ).unwrap();
        let uniform3fv           = gl_fn!(opengl32, glUniform3fv,         GlUniform3fv        ).unwrap();
        let uniform4fv           = gl_fn!(opengl32, glUniform4fv,         GlUniform4fv        ).unwrap();

        // Texture functions are GL 1.1 core — fall back to opengl32.dll automatically.
        let gen_textures    = gl_fn!(opengl32, glGenTextures,    GlGenTextures   ).unwrap();
        let delete_textures = gl_fn!(opengl32, glDeleteTextures, GlDeleteTextures).unwrap();
        let bind_texture    = gl_fn!(opengl32, glBindTexture,    GlBindTexture   ).unwrap();
        let tex_image2d     = gl_fn!(opengl32, glTexImage2D,     GlTexImage2D    ).unwrap();
        let tex_parameteri  = gl_fn!(opengl32, glTexParameteri,  GlTexParameteri ).unwrap();
        let active_texture  = gl_fn!(opengl32, glActiveTexture,  GlActiveTexture ).unwrap();
        let generate_mipmap = gl_fn!(opengl32, glGenerateMipmap, GlGenerateMipmap).unwrap();
        let pixel_storei    = gl_fn!(opengl32, glPixelStorei,    GlPixelStorei   ).unwrap();

        Self {
            clear_color, clear, viewport, enable,
            gen_vertex_arrays, bind_vertex_array, delete_vertex_arrays,
            gen_buffers, bind_buffer, buffer_data, delete_buffers,
            vertex_attrib_pointer, enable_vertex_attrib_array,
            create_shader, shader_source, compile_shader,
            get_shaderiv, get_shader_info_log, delete_shader,
            create_program, attach_shader, link_program,
            get_programiv, get_program_info_log, use_program, delete_program,
            draw_elements,
            get_uniform_location, uniform_matrix4fv, uniform3fv, uniform4fv,
            gen_textures, delete_textures, bind_texture, tex_image2d, tex_parameteri,
            active_texture, generate_mipmap, pixel_storei,
        }
    }
}

// ── OpenGLRenderer ────────────────────────────────────────────────────────────

pub struct OpenGLRenderer {
    hwnd:     HWND,
    hdc:      HDC,
    hglrc:    HGLRC,
    opengl32: HMODULE,
    width:    u32,
    height:   u32,
    gl:       GlFns,
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

            // Load opengl32.dll once and keep the handle alive for the lifetime
            // of this renderer (used as fallback in load_gl_fn).
            let lib_name = wstr("opengl32.dll");
            let opengl32 = LoadLibraryW(lib_name.as_ptr());
            // opengl32.dll should always be available on Windows — warn if not.
            if opengl32.is_null() {
                eprintln!("[GL] Warning: LoadLibraryW(\"opengl32.dll\") failed");
            }

            let gl = GlFns::load(opengl32);

            // Enable depth test by default.
            (gl.enable)(GL_DEPTH_TEST);

            Self { hwnd, hdc, hglrc, opengl32, width, height, gl }
        }
    }

    /// Returns the WGL rendering context handle (for extension setup).
    pub fn hglrc(&self) -> HGLRC { self.hglrc }
    /// Returns the device context handle.
    pub fn hdc(&self) -> HDC { self.hdc }

    // ── Geometry upload ───────────────────────────────────────────────────────

    /// Upload geometry to the GPU.  Returns `(vao, vbo, ebo)`.
    ///
    /// The caller is responsible for storing these IDs (e.g. in a [`GpuMesh`])
    /// and passing them to [`Self::draw_vao`] and [`Self::delete_vao`].
    pub fn upload_mesh_raw(&self, vertices: &[Vertex], indices: &[u32]) -> (u32, u32, u32) {
        unsafe {
            let gl = &self.gl;

            let mut vao: GLuint = 0;
            (gl.gen_vertex_arrays)(1, &mut vao);
            (gl.bind_vertex_array)(vao);

            let mut bufs: [GLuint; 2] = [0; 2];
            (gl.gen_buffers)(2, bufs.as_mut_ptr());
            let vbo = bufs[0];
            let ebo = bufs[1];

            // Upload vertex data.
            (gl.bind_buffer)(GL_ARRAY_BUFFER, vbo);
            (gl.buffer_data)(
                GL_ARRAY_BUFFER,
                (vertices.len() * std::mem::size_of::<Vertex>()) as GLsizeiptr,
                vertices.as_ptr() as *const c_void,
                GL_STATIC_DRAW,
            );

            // Upload index data.
            (gl.bind_buffer)(GL_ELEMENT_ARRAY_BUFFER, ebo);
            (gl.buffer_data)(
                GL_ELEMENT_ARRAY_BUFFER,
                (indices.len() * std::mem::size_of::<u32>()) as GLsizeiptr,
                indices.as_ptr() as *const c_void,
                GL_STATIC_DRAW,
            );

            let stride = std::mem::size_of::<Vertex>() as GLsizei; // 32 bytes

            // location 0 — position (vec3, byte offset 0)
            (gl.vertex_attrib_pointer)(
                0, 3, GL_FLOAT, GL_FALSE, stride,
                0 as *const c_void,
            );
            (gl.enable_vertex_attrib_array)(0);

            // location 1 — normal (vec3, byte offset 12)
            (gl.vertex_attrib_pointer)(
                1, 3, GL_FLOAT, GL_FALSE, stride,
                12 as *const c_void,
            );
            (gl.enable_vertex_attrib_array)(1);

            // location 2 — uv (vec2, byte offset 24)
            (gl.vertex_attrib_pointer)(
                2, 2, GL_FLOAT, GL_FALSE, stride,
                24 as *const c_void,
            );
            (gl.enable_vertex_attrib_array)(2);

            // Unbind VAO so subsequent state changes don't corrupt it.
            (gl.bind_vertex_array)(0);

            (vao, vbo, ebo)
        }
    }

    // ── Shader compilation ────────────────────────────────────────────────────

    /// Compile and link a GLSL program.  Returns the GL program ID.
    ///
    /// Panics with the driver's info log on compile or link error so problems
    /// surface immediately during development.
    pub fn compile_shader_program(&self, vert_src: &str, frag_src: &str) -> u32 {
        unsafe {
            let gl = &self.gl;

            let vert = self.compile_stage(GL_VERTEX_SHADER,   vert_src);
            let frag = self.compile_stage(GL_FRAGMENT_SHADER, frag_src);

            let program = (gl.create_program)();
            (gl.attach_shader)(program, vert);
            (gl.attach_shader)(program, frag);
            (gl.link_program)(program);

            // Check link status.
            let mut status: GLint = 0;
            (gl.get_programiv)(program, GL_LINK_STATUS, &mut status);
            if status == 0 {
                let mut log_len: GLint = 0;
                (gl.get_programiv)(program, GL_INFO_LOG_LENGTH, &mut log_len);
                let mut log = vec![0u8; log_len as usize];
                (gl.get_program_info_log)(
                    program, log_len, null_mut(),
                    log.as_mut_ptr() as *mut GLchar,
                );
                let msg = String::from_utf8_lossy(&log);
                panic!("[GL] Program link error:\n{}", msg);
            }

            // Shaders are linked; individual objects are no longer needed.
            (gl.delete_shader)(vert);
            (gl.delete_shader)(frag);

            program
        }
    }

    /// Compile a single shader stage.  Panics on error.
    unsafe fn compile_stage(&self, kind: GLenum, src: &str) -> GLuint {
        let gl = &self.gl;
        let shader = (gl.create_shader)(kind);

        // glShaderSource expects an array of C string pointers + lengths.
        let ptr    = src.as_ptr() as *const GLchar;
        let length = src.len() as GLint;
        (gl.shader_source)(shader, 1, &ptr, &length);
        (gl.compile_shader)(shader);

        let mut status: GLint = 0;
        (gl.get_shaderiv)(shader, GL_COMPILE_STATUS, &mut status);
        if status == 0 {
            let mut log_len: GLint = 0;
            (gl.get_shaderiv)(shader, GL_INFO_LOG_LENGTH, &mut log_len);
            let mut log = vec![0u8; log_len as usize];
            (gl.get_shader_info_log)(
                shader, log_len, null_mut(),
                log.as_mut_ptr() as *mut GLchar,
            );
            let stage = if kind == GL_VERTEX_SHADER { "vertex" } else { "fragment" };
            let msg   = String::from_utf8_lossy(&log);
            panic!("[GL] {} shader compile error:\n{}", stage, msg);
        }

        shader
    }

    // ── Texture upload ────────────────────────────────────────────────────────

    /// Upload RGBA8 pixel data to a new 2-D GL texture.  Returns the GL texture ID.
    ///
    /// Mip-maps are generated automatically; linear filtering is set on all levels.
    pub fn upload_texture_rgba8(&self, width: u32, height: u32, data: &[u8]) -> u32 {
        debug_assert_eq!(
            data.len(),
            (width * height * 4) as usize,
            "upload_texture_rgba8: data length mismatch"
        );
        unsafe {
            let gl = &self.gl;

            let mut tex: GLuint = 0;
            (gl.gen_textures)(1, &mut tex);
            (gl.bind_texture)(GL_TEXTURE_2D, tex);

            // Ensure tightly packed rows.
            (gl.pixel_storei)(0x0CF5 /* GL_UNPACK_ALIGNMENT */, 1);

            (gl.tex_image2d)(
                GL_TEXTURE_2D,
                0,                      // mip level
                GL_RGBA as GLint,       // internal format
                width  as GLsizei,
                height as GLsizei,
                0,                      // border (must be 0)
                GL_RGBA,                // source format
                GL_UNSIGNED_BYTE,
                data.as_ptr() as *const c_void,
            );

            (gl.generate_mipmap)(GL_TEXTURE_2D);

            (gl.tex_parameteri)(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR_MIPMAP_LINEAR);
            (gl.tex_parameteri)(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_LINEAR);
            (gl.tex_parameteri)(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S,     GL_REPEAT);
            (gl.tex_parameteri)(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T,     GL_REPEAT);

            (gl.bind_texture)(GL_TEXTURE_2D, 0);

            tex
        }
    }

    // ── Draw call ─────────────────────────────────────────────────────────────

    /// Draw a VAO with a compiled shader program.
    ///
    /// # Parameters
    /// - `vao`         — vertex array object ID (from [`Self::upload_mesh_raw`])
    /// - `index_count` — number of indices to draw
    /// - `program`     — GL shader program ID (from [`Self::compile_shader_program`])
    /// - `mvp`         — combined model-view-projection matrix
    /// - `model`       — model matrix (used for world-space normal transform)
    /// - `color`       — RGBA base colour (maps to `u_color`)
    /// - `light_dir`   — world-space light direction (maps to `u_light_dir`; ignored by flat shader)
    pub fn draw_vao(
        &self,
        vao:         u32,
        index_count: u32,
        program:     u32,
        mvp:         &Mat4,
        model:       &Mat4,
        color:       Color,
        light_dir:   Vec3,
    ) {
        unsafe {
            let gl = &self.gl;

            (gl.use_program)(program);

            // Helper: resolve a uniform location from a null-terminated byte string.
            // Returns -1 if the uniform is not active (e.g. flat shader ignores u_model).
            macro_rules! loc {
                ($name:expr) => {{
                    let cname: &[u8] = $name;
                    (gl.get_uniform_location)(program, cname.as_ptr() as *const GLchar)
                }};
            }

            // u_mvp
            let loc_mvp = loc!(b"u_mvp\0");
            if loc_mvp >= 0 {
                // Mat4 cols is [[f32;4];4] — that's 16 contiguous f32s in column-major order,
                // which is exactly what glUniformMatrix4fv expects.
                (gl.uniform_matrix4fv)(
                    loc_mvp, 1, GL_FALSE,
                    mvp.cols.as_ptr() as *const GLfloat,
                );
            }

            // u_model
            let loc_model = loc!(b"u_model\0");
            if loc_model >= 0 {
                (gl.uniform_matrix4fv)(
                    loc_model, 1, GL_FALSE,
                    model.cols.as_ptr() as *const GLfloat,
                );
            }

            // u_color
            let loc_color = loc!(b"u_color\0");
            if loc_color >= 0 {
                let rgba = [color.r, color.g, color.b, color.a];
                (gl.uniform4fv)(loc_color, 1, rgba.as_ptr());
            }

            // u_light_dir
            let loc_light = loc!(b"u_light_dir\0");
            if loc_light >= 0 {
                let dir = [light_dir.x, light_dir.y, light_dir.z];
                (gl.uniform3fv)(loc_light, 1, dir.as_ptr());
            }

            (gl.bind_vertex_array)(vao);
            (gl.draw_elements)(
                GL_TRIANGLES,
                index_count as GLsizei,
                GL_UNSIGNED_INT,
                null_mut(),
            );
            (gl.bind_vertex_array)(0);
        }
    }

    // ── Cleanup helpers ───────────────────────────────────────────────────────

    /// Delete the GPU buffers and VAO for a mesh previously created by
    /// [`Self::upload_mesh_raw`].
    pub fn delete_vao(&self, vao: u32, vbo: u32, ebo: u32) {
        unsafe {
            let gl = &self.gl;
            (gl.delete_vertex_arrays)(1, &vao);
            let bufs = [vbo, ebo];
            (gl.delete_buffers)(2, bufs.as_ptr());
        }
    }

    /// Delete a compiled shader program.
    pub fn delete_program(&self, program: u32) {
        unsafe { (self.gl.delete_program)(program); }
    }

    /// Delete a texture previously created by [`Self::upload_texture_rgba8`].
    pub fn delete_texture(&self, tex_id: u32) {
        unsafe { (self.gl.delete_textures)(1, &tex_id); }
    }

    // ---- MeshData convenience methods ------------------------------------

    /// Upload a MeshData (from OBJ or procedural) and return a drawable GpuMesh.
    pub fn upload_mesh(&self, mesh: &MeshData) -> GpuMesh {
        let (vao, vbo, ebo) = self.upload_mesh_raw(&mesh.vertices, &mesh.indices);
        GpuMesh {
            vao,
            vbo,
            ebo,
            index_count: mesh.indices.len() as u32,
        }
    }

    /// Draw a GpuMesh with a compiled shader program.
    pub fn draw_mesh(
        &self,
        mesh:      &GpuMesh,
        program:   u32,
        mvp:       &Mat4,
        model:     &Mat4,
        color:     Color,
        light_dir: Vec3,
    ) {
        self.draw_vao(mesh.vao, mesh.index_count, program, mvp, model, color, light_dir);
    }

    /// Free all GPU resources owned by a GpuMesh.
    pub fn free_mesh(&self, mesh: GpuMesh) {
        self.delete_vao(mesh.vao, mesh.vbo, mesh.ebo);
    }
}

// ── Renderer trait impl ───────────────────────────────────────────────────────

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

// ── Drop ─────────────────────────────────────────────────────────────────────

impl Drop for OpenGLRenderer {
    fn drop(&mut self) {
        unsafe {
            wglMakeCurrent(null_mut(), null_mut());
            wglDeleteContext(self.hglrc);
            ReleaseDC(self.hwnd, self.hdc);
            if !self.opengl32.is_null() {
                FreeLibrary(self.opengl32);
            }
        }
    }
}
