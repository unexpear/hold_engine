//! OpenGL renderer backend.
//!
//! Sets up a WGL context on the window's DC, then loads OpenGL function
//! pointers at runtime — no `gl` crate needed.
//!
//! Supports uploading meshes (VAO/VBO/EBO), compiling GLSL shaders,
//! uploading RGBA8 textures, and issuing indexed draw calls.

use super::{
    opengl_shaders, Color, DebugLine, DrawItem, LightItem, ParticleBatch, PassThumbnail,
    RenderFrame, RenderView, Renderer, ScreenQuad, TerrainBatch, TerrainPatchItem, VegetationBatch,
    WaterBatch,
};
use crate::assets::mesh::{MeshData, Vertex};
use crate::assets::types::{
    ColorSpace, GpuMesh, GpuTexture, Material, MeshHandle, NormalMap, Shader, ShaderHandle,
    Texture, TextureFormat, TextureHandle,
};
use crate::assets::AssetStore;
use crate::math::{Mat4, Vec3};
use crate::platform::ffi::*;
use crate::terrain::Heightmap;
use std::collections::HashMap;
use std::ffi::c_void;
use std::ptr::null_mut;

// ── OpenGL primitive types ────────────────────────────────────────────────────

type GLuint = u32;
type GLint = i32;
type GLfloat = f32;
type GLenum = u32;
type GLbitfield = u32;
type GLboolean = u8;
type GLsizei = i32;
type GLsizeiptr = isize;
type GLchar = i8;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct TerrainPatchMeshKey {
    origin_x_bits: u32,
    origin_z_bits: u32,
    size_bits: u32,
    height_scale_bits: u32,
    lod_level: u8,
}

// ── Core clear/enable constants ───────────────────────────────────────────────

const GL_COLOR_BUFFER_BIT: GLbitfield = 0x0000_4000;
const GL_DEPTH_BUFFER_BIT: GLbitfield = 0x0000_0100;
const GL_DEPTH_TEST: GLenum = 0x0B71;
#[allow(dead_code)]
const GL_TRUE: GLboolean = 1;
#[allow(dead_code)]
const GL_FALSE: GLboolean = 0;

// ── Buffer / draw constants ───────────────────────────────────────────────────

const GL_ARRAY_BUFFER: GLenum = 0x8892;
const GL_ELEMENT_ARRAY_BUFFER: GLenum = 0x8893;
const GL_STATIC_DRAW: GLenum = 0x88B4;
#[allow(dead_code)]
const GL_DYNAMIC_DRAW: GLenum = 0x88B8;

const GL_LINES: GLenum = 0x0001;
const GL_TRIANGLES: GLenum = 0x0004;
#[allow(dead_code)]
const GL_UNSIGNED_SHORT: GLenum = 0x1403;
#[allow(dead_code)]
const GL_UNSIGNED_INT: GLenum = 0x1405;
const GL_FLOAT: GLenum = 0x1406;

// ── Shader constants ──────────────────────────────────────────────────────────

const GL_VERTEX_SHADER: GLenum = 0x8B31;
const GL_FRAGMENT_SHADER: GLenum = 0x8B30;
const GL_COMPILE_STATUS: GLenum = 0x8B81;
const GL_LINK_STATUS: GLenum = 0x8B82;
const GL_INFO_LOG_LENGTH: GLenum = 0x8B84;

// ── Texture constants ─────────────────────────────────────────────────────────

const GL_TEXTURE_2D: GLenum = 0x0DE1;
#[allow(dead_code)]
const GL_TEXTURE0: GLenum = 0x84C0;
/// Second texture unit. Reserved for the normal-map sampler in shaders that
/// use one (see `TEXTURED_NORMAL_FRAG`). The OpenGL convention is
/// `GL_TEXTUREn = GL_TEXTURE0 + n`.
const GL_TEXTURE1: GLenum = 0x84C1;
const GL_RGBA: GLenum = 0x1908;
#[allow(dead_code)]
const GL_RGB: GLenum = 0x1907;
/// Internal format that performs the sRGB→linear decode on the GPU at
/// sample time. Used by the OpenGL upload path when a [`Texture`]'s
/// `color_space` is [`ColorSpace::Srgb`]; values fed into the fragment
/// shader by `texture()` calls are guaranteed linear without per-shader
/// gamma fix-ups. See ADR 0002 Q3 and `OpenGLRenderer::upload_texture`.
const GL_SRGB8_ALPHA8: GLint = 0x8C43;
const GL_UNSIGNED_BYTE: GLenum = 0x1401;
const GL_TEXTURE_MIN_FILTER: GLenum = 0x2801;
const GL_TEXTURE_MAG_FILTER: GLenum = 0x2800;
const GL_LINEAR: GLint = 0x2601;
const GL_LINEAR_MIPMAP_LINEAR: GLint = 0x2703;
const GL_TEXTURE_WRAP_S: GLenum = 0x2802;
const GL_TEXTURE_WRAP_T: GLenum = 0x2803;
const GL_REPEAT: GLint = 0x2901;
#[allow(dead_code)]
const GL_CLAMP_TO_EDGE: GLint = 0x812F;

// ── Depth / blend constants ───────────────────────────────────────────────────

#[allow(dead_code)]
const GL_LEQUAL: GLenum = 0x0203;
const GL_BLEND: GLenum = 0x0BE2;
const GL_SRC_ALPHA: GLenum = 0x0302;
const GL_ONE_MINUS_SRC_ALPHA: GLenum = 0x0303;

/// Framebuffer sRGB encoding. When enabled (via `glEnable`), the GPU
/// converts every linear shader output to its sRGB-encoded byte
/// representation before storing it in the colour buffer. Combined with
/// sRGB-decoded input textures (see `GL_SRGB8_ALPHA8`), this gives a
/// fully linear shading pipeline with display-correct output. See ADR
/// 0002 Q3.
const GL_FRAMEBUFFER_SRGB: GLenum = 0x8DB9;

// ── Function pointer types ────────────────────────────────────────────────────

// Core clear / state
type GlClearColor = unsafe extern "system" fn(GLfloat, GLfloat, GLfloat, GLfloat);
type GlClear = unsafe extern "system" fn(GLbitfield);
type GlViewport = unsafe extern "system" fn(GLint, GLint, GLsizei, GLsizei);
type GlEnable = unsafe extern "system" fn(GLenum);
#[allow(dead_code)]
type GlDisable = unsafe extern "system" fn(GLenum);

// VAO
type GlGenVertexArrays = unsafe extern "system" fn(GLsizei, *mut GLuint);
type GlBindVertexArray = unsafe extern "system" fn(GLuint);
type GlDeleteVertexArrays = unsafe extern "system" fn(GLsizei, *const GLuint);

// Buffer objects
type GlGenBuffers = unsafe extern "system" fn(GLsizei, *mut GLuint);
type GlBindBuffer = unsafe extern "system" fn(GLenum, GLuint);
type GlBufferData = unsafe extern "system" fn(GLenum, GLsizeiptr, *const c_void, GLenum);
type GlDeleteBuffers = unsafe extern "system" fn(GLsizei, *const GLuint);
type GlVertexAttribPointer =
    unsafe extern "system" fn(GLuint, GLint, GLenum, GLboolean, GLsizei, *const c_void);
type GlEnableVertexAttribArray = unsafe extern "system" fn(GLuint);

// Shaders
type GlCreateShader = unsafe extern "system" fn(GLenum) -> GLuint;
type GlShaderSource =
    unsafe extern "system" fn(GLuint, GLsizei, *const *const GLchar, *const GLint);
type GlCompileShader = unsafe extern "system" fn(GLuint);
type GlGetShaderiv = unsafe extern "system" fn(GLuint, GLenum, *mut GLint);
type GlGetShaderInfoLog = unsafe extern "system" fn(GLuint, GLsizei, *mut GLsizei, *mut GLchar);
type GlDeleteShader = unsafe extern "system" fn(GLuint);
type GlCreateProgram = unsafe extern "system" fn() -> GLuint;
type GlAttachShader = unsafe extern "system" fn(GLuint, GLuint);
type GlLinkProgram = unsafe extern "system" fn(GLuint);
type GlGetProgramiv = unsafe extern "system" fn(GLuint, GLenum, *mut GLint);
type GlGetProgramInfoLog = unsafe extern "system" fn(GLuint, GLsizei, *mut GLsizei, *mut GLchar);
type GlUseProgram = unsafe extern "system" fn(GLuint);
type GlDeleteProgram = unsafe extern "system" fn(GLuint);

// Draw
type GlDrawElements = unsafe extern "system" fn(GLenum, GLsizei, GLenum, *const c_void);
type GlDrawArrays = unsafe extern "system" fn(GLenum, GLint, GLsizei);

// Uniforms
type GlGetUniformLocation = unsafe extern "system" fn(GLuint, *const GLchar) -> GLint;
type GlUniformMatrix4fv = unsafe extern "system" fn(GLint, GLsizei, GLboolean, *const GLfloat);
#[allow(dead_code)]
type GlUniform1i = unsafe extern "system" fn(GLint, GLint);
type GlUniform3fv = unsafe extern "system" fn(GLint, GLsizei, *const GLfloat);
type GlUniform4fv = unsafe extern "system" fn(GLint, GLsizei, *const GLfloat);

// Textures
type GlGenTextures = unsafe extern "system" fn(GLsizei, *mut GLuint);
type GlDeleteTextures = unsafe extern "system" fn(GLsizei, *const GLuint);
type GlBindTexture = unsafe extern "system" fn(GLenum, GLuint);
type GlTexImage2D = unsafe extern "system" fn(
    GLenum,
    GLint,
    GLint,
    GLsizei,
    GLsizei,
    GLint,
    GLenum,
    GLenum,
    *const c_void,
);
type GlTexParameteri = unsafe extern "system" fn(GLenum, GLenum, GLint);
type GlActiveTexture = unsafe extern "system" fn(GLenum);
type GlGenerateMipmap = unsafe extern "system" fn(GLenum);
type GlPixelStorei = unsafe extern "system" fn(GLenum, GLint);
/// `glReadPixels(x, y, width, height, format, type, dst)` — synchronous
/// readback of the current colour-attachment / default framebuffer pixels.
/// Used by the frame graph's per-pass thumbnail capture (ADR 0002 Q8).
type GlReadPixels = unsafe extern "system" fn(
    GLint,
    GLint,
    GLsizei,
    GLsizei,
    GLenum,
    GLenum,
    *mut c_void,
);

// State
#[allow(dead_code)]
type GlDepthFunc = unsafe extern "system" fn(GLenum);
#[allow(dead_code)]
type GlBlendFunc = unsafe extern "system" fn(GLenum, GLenum);
#[allow(dead_code)]
type GlBlendEquation = unsafe extern "system" fn(GLenum);

// ── Module-level GL helpers ───────────────────────────────────────────────────

/// Compile and link a GLSL program from vertex + fragment source.
///
/// Used during `OpenGLRenderer::new()` before `self` exists.  Panics on
/// compile or link error so problems surface immediately.
unsafe fn compile_gl_program(gl: &GlFns, vert_src: &str, frag_src: &str) -> GLuint {
    let vert = compile_gl_stage(gl, GL_VERTEX_SHADER, vert_src);
    let frag = compile_gl_stage(gl, GL_FRAGMENT_SHADER, frag_src);

    let program = (gl.create_program)();
    (gl.attach_shader)(program, vert);
    (gl.attach_shader)(program, frag);
    (gl.link_program)(program);

    let mut status: GLint = 0;
    (gl.get_programiv)(program, GL_LINK_STATUS, &mut status);
    if status == 0 {
        let mut log_len: GLint = 0;
        (gl.get_programiv)(program, GL_INFO_LOG_LENGTH, &mut log_len);
        let mut log = vec![0u8; log_len as usize];
        (gl.get_program_info_log)(
            program,
            log_len,
            null_mut(),
            log.as_mut_ptr() as *mut GLchar,
        );
        panic!(
            "[GL] Line program link error:\n{}",
            String::from_utf8_lossy(&log)
        );
    }

    (gl.delete_shader)(vert);
    (gl.delete_shader)(frag);
    program
}

unsafe fn compile_gl_stage(gl: &GlFns, kind: GLenum, src: &str) -> GLuint {
    let shader = (gl.create_shader)(kind);
    let ptr = src.as_ptr() as *const GLchar;
    let length = src.len() as GLint;
    (gl.shader_source)(shader, 1, &ptr, &length);
    (gl.compile_shader)(shader);

    let mut status: GLint = 0;
    (gl.get_shaderiv)(shader, GL_COMPILE_STATUS, &mut status);
    if status == 0 {
        let mut log_len: GLint = 0;
        (gl.get_shaderiv)(shader, GL_INFO_LOG_LENGTH, &mut log_len);
        let mut log = vec![0u8; log_len as usize];
        (gl.get_shader_info_log)(shader, log_len, null_mut(), log.as_mut_ptr() as *mut GLchar);
        let stage = if kind == GL_VERTEX_SHADER {
            "vertex"
        } else {
            "fragment"
        };
        panic!(
            "[GL] {} shader compile error:\n{}",
            stage,
            String::from_utf8_lossy(&log)
        );
    }
    shader
}

// ── GL function loader ────────────────────────────────────────────────────────

/// Load a core (opengl32.dll) or extension (wglGetProcAddress) function.
///
/// Core functions (like glClear) are in opengl32.dll;
/// modern functions (like glCreateShader) come from wglGetProcAddress.
/// The `opengl32` module handle must have been loaded once at startup and
/// passed here — it is NOT freed by this function.
fn terrain_patch_mesh_key(patch: &TerrainPatchItem) -> TerrainPatchMeshKey {
    TerrainPatchMeshKey {
        origin_x_bits: patch.origin.x.to_bits(),
        origin_z_bits: patch.origin.z.to_bits(),
        size_bits: patch.size.to_bits(),
        height_scale_bits: patch.height_scale.to_bits(),
        lod_level: patch.lod_level,
    }
}

fn terrain_patch_subdivisions(lod_level: u8) -> usize {
    match lod_level {
        0 => 16,
        1 => 8,
        _ => 4,
    }
}

fn terrain_patch_heightmap(height_scale: f32) -> Heightmap {
    Heightmap::new((height_scale * 0.25).max(1.0), 0.012, 4)
}

fn billboard_model_matrix(position: Vec3, size: f32, camera_position: Vec3) -> Mat4 {
    let to_camera = camera_position - position;
    let facing = if to_camera.length_sq() > 1e-8 {
        to_camera.normalize()
    } else {
        Vec3::Z
    };
    let reference_up = if facing.y.abs() > 0.95 {
        Vec3::Z
    } else {
        Vec3::Y
    };
    let right = reference_up.cross(facing).normalize();
    let up = facing.cross(right).normalize();
    let size = size.max(0.01);

    Mat4 {
        cols: [
            [right.x * size, right.y * size, right.z * size, 0.0],
            [facing.x * size, facing.y * size, facing.z * size, 0.0],
            [up.x * size, up.y * size, up.z * size, 0.0],
            [position.x, position.y, position.z, 1.0],
        ],
    }
}

fn build_terrain_patch_mesh(patch: &TerrainPatchItem) -> MeshData {
    let subdivisions = terrain_patch_subdivisions(patch.lod_level);
    let row_stride = subdivisions + 1;
    let step = patch.size / subdivisions as f32;
    let heightmap = terrain_patch_heightmap(patch.height_scale);

    let mut vertices = Vec::with_capacity(row_stride * row_stride);
    let mut indices = Vec::with_capacity(subdivisions * subdivisions * 6);

    for iz in 0..=subdivisions {
        for ix in 0..=subdivisions {
            let local_x = ix as f32 * step;
            let local_z = iz as f32 * step;
            let world_x = patch.origin.x + local_x;
            let world_z = patch.origin.z + local_z;
            let world_y = heightmap.sample(world_x, world_z);

            let hx0 = heightmap.sample(world_x - step, world_z);
            let hx1 = heightmap.sample(world_x + step, world_z);
            let hz0 = heightmap.sample(world_x, world_z - step);
            let hz1 = heightmap.sample(world_x, world_z + step);
            let tangent_x = Vec3::new(2.0 * step, hx1 - hx0, 0.0);
            let tangent_z = Vec3::new(0.0, hz1 - hz0, 2.0 * step);
            let normal = tangent_z.cross(tangent_x).normalize();

            // For the terrain grid, U increases along world +X, so the
            // surface tangent is the +X-derivative of the heightmap surface
            // (slope along x). Normalise to keep the TBN matrix orthonormal.
            let tangent_world = tangent_x.normalize();

            vertices.push(Vertex {
                pos: [world_x, world_y, world_z],
                normal: [normal.x, normal.y, normal.z],
                uv: [
                    local_x / patch.size.max(1.0),
                    1.0 - local_z / patch.size.max(1.0),
                ],
                tangent: [tangent_world.x, tangent_world.y, tangent_world.z, 1.0],
            });
        }
    }

    for iz in 0..subdivisions {
        for ix in 0..subdivisions {
            let tl = (iz * row_stride + ix) as u32;
            let tr = tl + 1;
            let bl = tl + row_stride as u32;
            let br = bl + 1;
            indices.extend_from_slice(&[tl, bl, br, tl, br, tr]);
        }
    }

    MeshData::from_raw(vertices, indices)
}

unsafe fn load_gl_fn(name: &str, opengl32: HMODULE) -> *const c_void {
    let cname = cstr(name);
    // Try wglGetProcAddress first (extensions / modern GL).
    let ptr = wglGetProcAddress(cname.as_ptr());
    if !ptr.is_null() && ptr as usize > 3 {
        return ptr;
    }
    // Fall back to opengl32.dll for legacy core functions.
    if opengl32.is_null() {
        return null_mut();
    }
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
    clear: GlClear,
    viewport: GlViewport,
    enable: GlEnable,
    disable: GlDisable,

    // VAO
    gen_vertex_arrays: GlGenVertexArrays,
    bind_vertex_array: GlBindVertexArray,
    delete_vertex_arrays: GlDeleteVertexArrays,

    // Buffer objects
    gen_buffers: GlGenBuffers,
    bind_buffer: GlBindBuffer,
    buffer_data: GlBufferData,
    delete_buffers: GlDeleteBuffers,
    vertex_attrib_pointer: GlVertexAttribPointer,
    enable_vertex_attrib_array: GlEnableVertexAttribArray,

    // Shaders
    create_shader: GlCreateShader,
    shader_source: GlShaderSource,
    compile_shader: GlCompileShader,
    get_shaderiv: GlGetShaderiv,
    get_shader_info_log: GlGetShaderInfoLog,
    delete_shader: GlDeleteShader,
    create_program: GlCreateProgram,
    attach_shader: GlAttachShader,
    link_program: GlLinkProgram,
    get_programiv: GlGetProgramiv,
    get_program_info_log: GlGetProgramInfoLog,
    use_program: GlUseProgram,
    delete_program: GlDeleteProgram,

    // Draw
    draw_elements: GlDrawElements,
    draw_arrays: GlDrawArrays,

    // Uniforms
    get_uniform_location: GlGetUniformLocation,
    uniform_matrix4fv: GlUniformMatrix4fv,
    uniform1i: GlUniform1i,
    uniform3fv: GlUniform3fv,
    uniform4fv: GlUniform4fv,

    // Textures
    gen_textures: GlGenTextures,
    delete_textures: GlDeleteTextures,
    bind_texture: GlBindTexture,
    tex_image2d: GlTexImage2D,
    tex_parameteri: GlTexParameteri,
    active_texture: GlActiveTexture,
    generate_mipmap: GlGenerateMipmap,
    pixel_storei: GlPixelStorei,
    read_pixels: GlReadPixels,

    // State
    blend_func: GlBlendFunc,
}

impl GlFns {
    unsafe fn load(opengl32: HMODULE) -> Self {
        // ── Core — always present ──────────────────────────────────────────────
        let clear_color = std::mem::transmute::<*const c_void, GlClearColor>(load_gl_fn(
            "glClearColor",
            opengl32,
        ));
        let clear = std::mem::transmute::<*const c_void, GlClear>(load_gl_fn("glClear", opengl32));
        let viewport =
            std::mem::transmute::<*const c_void, GlViewport>(load_gl_fn("glViewport", opengl32));
        let enable =
            std::mem::transmute::<*const c_void, GlEnable>(load_gl_fn("glEnable", opengl32));
        let disable =
            std::mem::transmute::<*const c_void, GlDisable>(load_gl_fn("glDisable", opengl32));

        // ── Modern GL — unwrap: a missing entry means the driver is broken ─────
        let gen_vertex_arrays = gl_fn!(opengl32, glGenVertexArrays, GlGenVertexArrays).unwrap();
        let bind_vertex_array = gl_fn!(opengl32, glBindVertexArray, GlBindVertexArray).unwrap();
        let delete_vertex_arrays =
            gl_fn!(opengl32, glDeleteVertexArrays, GlDeleteVertexArrays).unwrap();

        let gen_buffers = gl_fn!(opengl32, glGenBuffers, GlGenBuffers).unwrap();
        let bind_buffer = gl_fn!(opengl32, glBindBuffer, GlBindBuffer).unwrap();
        let buffer_data = gl_fn!(opengl32, glBufferData, GlBufferData).unwrap();
        let delete_buffers = gl_fn!(opengl32, glDeleteBuffers, GlDeleteBuffers).unwrap();
        let vertex_attrib_pointer =
            gl_fn!(opengl32, glVertexAttribPointer, GlVertexAttribPointer).unwrap();
        let enable_vertex_attrib_array = gl_fn!(
            opengl32,
            glEnableVertexAttribArray,
            GlEnableVertexAttribArray
        )
        .unwrap();

        let create_shader = gl_fn!(opengl32, glCreateShader, GlCreateShader).unwrap();
        let shader_source = gl_fn!(opengl32, glShaderSource, GlShaderSource).unwrap();
        let compile_shader = gl_fn!(opengl32, glCompileShader, GlCompileShader).unwrap();
        let get_shaderiv = gl_fn!(opengl32, glGetShaderiv, GlGetShaderiv).unwrap();
        let get_shader_info_log = gl_fn!(opengl32, glGetShaderInfoLog, GlGetShaderInfoLog).unwrap();
        let delete_shader = gl_fn!(opengl32, glDeleteShader, GlDeleteShader).unwrap();
        let create_program = gl_fn!(opengl32, glCreateProgram, GlCreateProgram).unwrap();
        let attach_shader = gl_fn!(opengl32, glAttachShader, GlAttachShader).unwrap();
        let link_program = gl_fn!(opengl32, glLinkProgram, GlLinkProgram).unwrap();
        let get_programiv = gl_fn!(opengl32, glGetProgramiv, GlGetProgramiv).unwrap();
        let get_program_info_log =
            gl_fn!(opengl32, glGetProgramInfoLog, GlGetProgramInfoLog).unwrap();
        let use_program = gl_fn!(opengl32, glUseProgram, GlUseProgram).unwrap();
        let delete_program = gl_fn!(opengl32, glDeleteProgram, GlDeleteProgram).unwrap();

        let draw_elements = gl_fn!(opengl32, glDrawElements, GlDrawElements).unwrap();
        let draw_arrays = gl_fn!(opengl32, glDrawArrays, GlDrawArrays).unwrap();

        let get_uniform_location =
            gl_fn!(opengl32, glGetUniformLocation, GlGetUniformLocation).unwrap();
        let uniform_matrix4fv = gl_fn!(opengl32, glUniformMatrix4fv, GlUniformMatrix4fv).unwrap();
        let uniform1i = gl_fn!(opengl32, glUniform1i, GlUniform1i).unwrap();
        let uniform3fv = gl_fn!(opengl32, glUniform3fv, GlUniform3fv).unwrap();
        let uniform4fv = gl_fn!(opengl32, glUniform4fv, GlUniform4fv).unwrap();

        // Texture functions are GL 1.1 core — fall back to opengl32.dll automatically.
        let gen_textures = gl_fn!(opengl32, glGenTextures, GlGenTextures).unwrap();
        let delete_textures = gl_fn!(opengl32, glDeleteTextures, GlDeleteTextures).unwrap();
        let bind_texture = gl_fn!(opengl32, glBindTexture, GlBindTexture).unwrap();
        let tex_image2d = gl_fn!(opengl32, glTexImage2D, GlTexImage2D).unwrap();
        let tex_parameteri = gl_fn!(opengl32, glTexParameteri, GlTexParameteri).unwrap();
        let active_texture = gl_fn!(opengl32, glActiveTexture, GlActiveTexture).unwrap();
        let generate_mipmap = gl_fn!(opengl32, glGenerateMipmap, GlGenerateMipmap).unwrap();
        let pixel_storei = gl_fn!(opengl32, glPixelStorei, GlPixelStorei).unwrap();
        let read_pixels = gl_fn!(opengl32, glReadPixels, GlReadPixels).unwrap();
        let blend_func = gl_fn!(opengl32, glBlendFunc, GlBlendFunc).unwrap();

        Self {
            clear_color,
            clear,
            viewport,
            enable,
            disable,
            gen_vertex_arrays,
            bind_vertex_array,
            delete_vertex_arrays,
            gen_buffers,
            bind_buffer,
            buffer_data,
            delete_buffers,
            vertex_attrib_pointer,
            enable_vertex_attrib_array,
            create_shader,
            shader_source,
            compile_shader,
            get_shaderiv,
            get_shader_info_log,
            delete_shader,
            create_program,
            attach_shader,
            link_program,
            get_programiv,
            get_program_info_log,
            use_program,
            delete_program,
            draw_elements,
            draw_arrays,
            get_uniform_location,
            uniform_matrix4fv,
            uniform1i,
            uniform3fv,
            uniform4fv,
            gen_textures,
            delete_textures,
            bind_texture,
            tex_image2d,
            tex_parameteri,
            active_texture,
            generate_mipmap,
            pixel_storei,
            read_pixels,
            blend_func,
        }
    }
}

// ── Uniform location cache ────────────────────────────────────────────────────

/// Cached `glGetUniformLocation` results for a single shader program.
///
/// Location queries are GL calls that can be slow; caching them per-program
/// reduces per-draw overhead to a single `HashMap` lookup.
///
/// A location of `-1` means the uniform is not active in that program (the
/// driver optimises it away or the shader doesn't use it) — callers must skip
/// setting it.
#[derive(Clone, Copy)]
struct CachedUniforms {
    mvp: GLint,
    model: GLint,
    color: GLint,
    light_dir: GLint,
    view_pos: GLint,
    ambient: GLint,
    texture: GLint,
    normal_map: GLint,
    metallic: GLint,
    roughness: GLint,
}

impl CachedUniforms {
    /// Query all known uniform locations from `program`.
    unsafe fn load(gl: &GlFns, program: GLuint) -> Self {
        macro_rules! loc {
            ($name:expr) => {
                (gl.get_uniform_location)(program, $name.as_ptr() as *const GLchar)
            };
        }
        Self {
            mvp: loc!(b"u_mvp\0"),
            model: loc!(b"u_model\0"),
            color: loc!(b"u_color\0"),
            light_dir: loc!(b"u_light_dir\0"),
            view_pos: loc!(b"u_view_pos\0"),
            ambient: loc!(b"u_ambient\0"),
            texture: loc!(b"u_texture\0"),
            normal_map: loc!(b"u_normal_map\0"),
            metallic: loc!(b"u_metallic\0"),
            roughness: loc!(b"u_roughness\0"),
        }
    }
}

// ── OpenGLRenderer ────────────────────────────────────────────────────────────

pub struct OpenGLRenderer {
    hwnd: HWND,
    hdc: HDC,
    hglrc: HGLRC,
    opengl32: HMODULE,
    width: u32,
    height: u32,
    gl: GlFns,
    /// Uniform location cache keyed by GL program ID.
    /// Populated lazily on first draw with each program.
    uniform_cache: HashMap<u32, CachedUniforms>,
    /// Compiled line-shader program (position + per-vertex colour).
    line_program: GLuint,
    /// VAO and VBO for batched debug line segments (updated each frame).
    line_vao: GLuint,
    line_vbo: GLuint,
    frame_mesh_cache: HashMap<MeshHandle, GpuMesh>,
    frame_texture_cache: HashMap<TextureHandle, GpuTexture>,
    frame_program_cache: HashMap<ShaderHandle, GLuint>,
    terrain_patch_mesh_cache: HashMap<TerrainPatchMeshKey, GpuMesh>,
    unit_plane_mesh: Option<GpuMesh>,
    /// Compiled scene frame graph (M9 — ADR 0002 Q8). Built once at
    /// construction and reused every frame; `Option` because
    /// [`Renderer::submit_frame`] takes the graph out via
    /// `Option::take`, executes it, then puts it back — Rust's borrow
    /// checker would reject `graph.execute(&mut self, ...)` on a plain
    /// field otherwise. If graph compile ever failed at construction,
    /// this is `None` and `submit_frame` falls back to direct
    /// method calls.
    scene_frame_graph: Option<super::FrameGraph>,
}

impl OpenGLRenderer {
    /// Create a WGL context on `hwnd`. Panics if the driver doesn't support GL.
    ///
    /// # Safety
    /// `hwnd` must be a valid, live Win32 window handle that outlives the
    /// returned renderer. Passing an invalid handle is undefined behaviour in
    /// the Win32 calls this makes (`GetDC`, `SetPixelFormat`, …).
    pub unsafe fn new(hwnd: HWND, width: u32, height: u32) -> Self {
        unsafe {
            let hdc = GetDC(hwnd);
            assert!(!hdc.is_null(), "GetDC failed");

            // Choose a pixel format that supports OpenGL + double buffering.
            let pfd = PIXELFORMATDESCRIPTOR {
                nSize: std::mem::size_of::<PIXELFORMATDESCRIPTOR>() as WORD,
                nVersion: 1,
                dwFlags: PFD_DRAW_TO_WINDOW | PFD_SUPPORT_OPENGL | PFD_DOUBLEBUFFER,
                iPixelType: PFD_TYPE_RGBA,
                cColorBits: 32,
                cDepthBits: 24,
                cStencilBits: 8,
                iLayerType: PFD_MAIN_PLANE,
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

            // Enable sRGB framebuffer encoding so the GPU writes the
            // gamma-correct sRGB representation of every linear-space
            // shader output to the colour buffer. Combined with
            // GL_SRGB8_ALPHA8 input textures (`OpenGLRenderer::upload_texture`
            // for `ColorSpace::Srgb`), this closes the linear-pipeline
            // loop: linear sample → linear math → ACES tonemap → linear
            // write → GPU-side linear→sRGB encode → display. See ADR
            // 0002 Q3.
            (gl.enable)(GL_FRAMEBUFFER_SRGB);

            // ── Compile line shader ────────────────────────────────────────────
            let line_program =
                compile_gl_program(&gl, opengl_shaders::LINE_VERT, opengl_shaders::LINE_FRAG);

            // ── Allocate persistent line VAO + VBO ────────────────────────────
            // Vertex layout: [x y z r g b a] (7 × f32 = 28 bytes per vertex)
            let mut line_vao: GLuint = 0;
            (gl.gen_vertex_arrays)(1, &mut line_vao);
            (gl.bind_vertex_array)(line_vao);

            let mut line_vbo: GLuint = 0;
            (gl.gen_buffers)(1, &mut line_vbo);
            (gl.bind_buffer)(GL_ARRAY_BUFFER, line_vbo);

            let stride = 7_i32 * 4; // 28 bytes
                                    // location 0 — position (vec3, offset 0)
            (gl.vertex_attrib_pointer)(0, 3, GL_FLOAT, GL_FALSE, stride, 0 as *const c_void);
            (gl.enable_vertex_attrib_array)(0);
            // location 1 — colour (vec4, offset 12)
            (gl.vertex_attrib_pointer)(1, 4, GL_FLOAT, GL_FALSE, stride, 12 as *const c_void);
            (gl.enable_vertex_attrib_array)(1);

            (gl.bind_vertex_array)(0);

            Self {
                hwnd,
                hdc,
                hglrc,
                opengl32,
                width,
                height,
                gl,
                uniform_cache: HashMap::new(),
                line_program,
                line_vao,
                line_vbo,
                frame_mesh_cache: HashMap::new(),
                frame_texture_cache: HashMap::new(),
                frame_program_cache: HashMap::new(),
                terrain_patch_mesh_cache: HashMap::new(),
                unit_plane_mesh: None,
                scene_frame_graph: build_scene_frame_graph().compile().ok(),
            }
        }
    }

    /// Returns the WGL rendering context handle (for extension setup).
    pub fn hglrc(&self) -> HGLRC {
        self.hglrc
    }
    /// Returns the device context handle.
    pub fn hdc(&self) -> HDC {
        self.hdc
    }

    fn cached_frame_mesh(&mut self, assets: &AssetStore, handle: MeshHandle) -> Option<GpuMesh> {
        if handle.is_null() {
            return None;
        }
        if let Some(mesh) = self.frame_mesh_cache.get(&handle) {
            return Some(*mesh);
        }

        let gpu_mesh = self.upload_mesh(assets.get::<MeshData>(handle)?);
        self.frame_mesh_cache.insert(handle, gpu_mesh);
        Some(gpu_mesh)
    }

    /// Evict every frame-cache entry whose texture handle's index
    /// matches `handle_index`. Used by the hot-reload dispatcher
    /// (M7.3) when an `AssetInvalidated { kind: Texture, .. }`
    /// event arrives — `AssetStore::replace` preserves the handle's
    /// generation, but the GPU resource behind the cached entry is
    /// the *previous* texture's bytes and must be re-uploaded.
    ///
    /// Returns the number of entries evicted (0 or 1 in practice;
    /// the cache is keyed by full handle).
    ///
    /// The previous `GpuTexture::id` is freed via `delete_texture`
    /// to avoid leaking GL texture objects across reloads. See
    /// ADR 0004 Q11 for the per-cache invalidation strategy.
    pub fn evict_texture_handle_by_index(&mut self, handle_index: u32) -> usize {
        let mut to_remove: Vec<TextureHandle> = self
            .frame_texture_cache
            .keys()
            .filter(|h| h.index == handle_index)
            .copied()
            .collect();
        let count = to_remove.len();
        for handle in to_remove.drain(..) {
            if let Some(gpu) = self.frame_texture_cache.remove(&handle) {
                self.delete_texture(gpu.id);
            }
        }
        count
    }

    /// Evict mesh-cache entries by handle index. Frees the GL VAO,
    /// VBO, and EBO so the next access re-uploads from the
    /// updated `MeshData`. See ADR 0004 Q11.
    pub fn evict_mesh_handle_by_index(&mut self, handle_index: u32) -> usize {
        let mut to_remove: Vec<MeshHandle> = self
            .frame_mesh_cache
            .keys()
            .filter(|h| h.index == handle_index)
            .copied()
            .collect();
        let count = to_remove.len();
        for handle in to_remove.drain(..) {
            if let Some(gpu) = self.frame_mesh_cache.remove(&handle) {
                self.delete_vao(gpu.vao, gpu.vbo, gpu.ebo);
            }
        }
        count
    }

    /// Evict shader-program-cache entries by handle index. Frees
    /// the GL program (which transitively invalidates the
    /// associated `uniform_cache` entries via `delete_program`).
    /// See ADR 0004 Q11.
    pub fn evict_shader_handle_by_index(&mut self, handle_index: u32) -> usize {
        let mut to_remove: Vec<ShaderHandle> = self
            .frame_program_cache
            .keys()
            .filter(|h| h.index == handle_index)
            .copied()
            .collect();
        let count = to_remove.len();
        for handle in to_remove.drain(..) {
            if let Some(program) = self.frame_program_cache.remove(&handle) {
                // delete_program also evicts uniform_cache for that
                // program ID — handle the cascade via the existing
                // method.
                self.delete_program(program);
            }
        }
        count
    }

    fn cached_frame_texture(
        &mut self,
        assets: &AssetStore,
        handle: TextureHandle,
    ) -> Option<GpuTexture> {
        if handle.is_null() {
            return None;
        }
        if let Some(texture) = self.frame_texture_cache.get(&handle) {
            return Some(*texture);
        }

        let gpu_texture = self.upload_texture(assets.get::<Texture>(handle)?);
        self.frame_texture_cache.insert(handle, gpu_texture);
        Some(gpu_texture)
    }

    fn cached_frame_program(
        &mut self,
        assets: &AssetStore,
        handle: ShaderHandle,
    ) -> Option<GLuint> {
        if handle.is_null() {
            return None;
        }
        if let Some(program) = self.frame_program_cache.get(&handle) {
            return Some(*program);
        }

        let shader = assets.get::<Shader>(handle)?;
        let program = self.compile_shader_program(&shader.vertex_source, &shader.fragment_source);
        self.frame_program_cache.insert(handle, program);
        Some(program)
    }

    fn cached_terrain_patch_mesh(&mut self, patch: &TerrainPatchItem) -> GpuMesh {
        let key = terrain_patch_mesh_key(patch);
        if let Some(mesh) = self.terrain_patch_mesh_cache.get(&key) {
            return *mesh;
        }

        let gpu_mesh = self.upload_mesh(&build_terrain_patch_mesh(patch));
        self.terrain_patch_mesh_cache.insert(key, gpu_mesh);
        gpu_mesh
    }

    fn cached_unit_plane_mesh(&mut self) -> GpuMesh {
        if let Some(mesh) = self.unit_plane_mesh {
            return mesh;
        }

        let mesh = self.upload_mesh(&MeshData::plane(8, 8));
        self.unit_plane_mesh = Some(mesh);
        mesh
    }

    /// Direct-call fallback for [`<Self as Renderer>::submit_frame`] when
    /// the scene frame graph failed to build at construction. Mirrors the
    /// pre-M9 render path so a compile failure never breaks the editor.
    fn submit_frame_direct(&mut self, assets: &AssetStore, frame: &RenderFrame) {
        if let Some(view) = frame.primary_view() {
            self.clear_and_setup_view(view);
            self.draw_submitted_view(assets, view);
            let vp = view.projection * view.view;
            self.render_debug_lines(&frame.debug_lines, &vp);
        } else {
            unsafe {
                (self.gl.viewport)(0, 0, self.width as GLsizei, self.height as GLsizei);
            }
            self.clear(Color::BLACK);
        }
        unsafe {
            (self.gl.viewport)(0, 0, self.width as GLsizei, self.height as GLsizei);
        }
        self.render_screen_quads(&frame.screen_quads);
    }

    /// Configure the OpenGL viewport rectangle from a [`RenderView`] and
    /// clear the framebuffer with its clear colour. Split out of
    /// [`Self::draw_submitted_view`] so the frame graph's
    /// `ClearAndViewportPass` (M9) can run this stage independently and
    /// the frame inspector captures a "cleared but empty" thumbnail
    /// before the scene draw layers on top.
    pub fn clear_and_setup_view(&mut self, view: &RenderView) {
        unsafe {
            (self.gl.viewport)(
                view.viewport.x as GLint,
                view.viewport.y as GLint,
                view.viewport.width as GLsizei,
                view.viewport.height as GLsizei,
            );
        }
        self.clear(view.clear_color);
    }

    /// Draw the 3D scene content for a [`RenderView`]: terrain, meshes,
    /// vegetation, water, and particle batches. Assumes the caller has
    /// already cleared and set the viewport via
    /// [`Self::clear_and_setup_view`]; the frame graph's
    /// `ClearAndViewportPass` fulfils that precondition when the scene
    /// is driven through `execute` / `execute_with_capture`, and
    /// [`<Self as Renderer>::submit_frame`] does it inline for callers
    /// that haven't migrated yet.
    pub fn draw_submitted_view(&mut self, assets: &AssetStore, view: &RenderView) {
        let light = view
            .lights
            .first()
            .copied()
            .unwrap_or_else(LightItem::default);
        for batch in &view.terrain_batches {
            self.draw_terrain_batch(assets, view, batch, light);
        }
        for item in &view.draw_items {
            self.draw_submitted_item(assets, view, item, light);
        }
        for batch in &view.vegetation_batches {
            self.draw_vegetation_batch(assets, view, batch, light);
        }
        for batch in &view.water_batches {
            self.draw_water_batch(assets, view, batch, light);
        }
        for batch in &view.particle_batches {
            self.draw_particle_batch(assets, view, batch, light);
        }
    }

    fn draw_submitted_item(
        &mut self,
        assets: &AssetStore,
        view: &RenderView,
        item: &DrawItem,
        light: LightItem,
    ) {
        let mesh = match self.cached_frame_mesh(assets, item.mesh) {
            Some(mesh) => mesh,
            None => return,
        };

        self.draw_material_mesh(
            assets,
            view,
            mesh,
            item.material,
            &item.model,
            light,
            Color::WHITE,
        );
    }

    fn draw_terrain_batch(
        &mut self,
        assets: &AssetStore,
        view: &RenderView,
        batch: &TerrainBatch,
        light: LightItem,
    ) {
        if batch.material.is_null() {
            return;
        }

        for patch in &batch.patches {
            let mesh = self.cached_terrain_patch_mesh(patch);
            self.draw_material_mesh(
                assets,
                view,
                mesh,
                batch.material,
                &Mat4::IDENTITY,
                light,
                Color::WHITE,
            );
        }
    }

    fn draw_vegetation_batch(
        &mut self,
        assets: &AssetStore,
        view: &RenderView,
        batch: &VegetationBatch,
        light: LightItem,
    ) {
        let mesh = match self.cached_frame_mesh(assets, batch.mesh) {
            Some(mesh) => mesh,
            None => return,
        };

        for model in &batch.instances {
            self.draw_material_mesh(
                assets,
                view,
                mesh,
                batch.material,
                model,
                light,
                Color::WHITE,
            );
        }
    }

    fn draw_water_batch(
        &mut self,
        assets: &AssetStore,
        view: &RenderView,
        batch: &WaterBatch,
        light: LightItem,
    ) {
        let Some(material) = batch.material else {
            return;
        };

        let mesh = match batch.mesh {
            Some(handle) => match self.cached_frame_mesh(assets, handle) {
                Some(mesh) => mesh,
                None => return,
            },
            None => self.cached_unit_plane_mesh(),
        };

        self.draw_material_mesh(
            assets,
            view,
            mesh,
            material,
            &batch.model,
            light,
            batch.fog_color,
        );
    }

    fn draw_particle_batch(
        &mut self,
        assets: &AssetStore,
        view: &RenderView,
        batch: &ParticleBatch,
        light: LightItem,
    ) {
        let Some(material) = batch.material else {
            return;
        };
        if batch.billboards.is_empty() {
            return;
        }

        let mesh = self.cached_unit_plane_mesh();
        unsafe {
            (self.gl.enable)(GL_BLEND);
            (self.gl.blend_func)(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA);
        }

        for billboard in &batch.billboards {
            let model =
                billboard_model_matrix(billboard.position, billboard.size, view.camera_position);
            self.draw_material_mesh(assets, view, mesh, material, &model, light, billboard.color);
        }

        unsafe {
            (self.gl.disable)(GL_BLEND);
        }
    }

    fn draw_material_mesh(
        &mut self,
        assets: &AssetStore,
        view: &RenderView,
        mesh: GpuMesh,
        material_handle: crate::assets::MaterialHandle,
        model: &Mat4,
        light: LightItem,
        tint: Color,
    ) {
        let material = match assets.get::<Material>(material_handle) {
            Some(material) => material,
            None => return,
        };
        let texture = match self.cached_frame_texture(assets, material.albedo) {
            Some(texture) => texture,
            None => return,
        };
        let program = match self.cached_frame_program(assets, material.shader) {
            Some(program) => program,
            None => return,
        };

        let mvp = view.projection * view.view * *model;
        // Internal frame-graph-driven draw path threads the material's
        // PBR factors through to the PBR shader. See ADR 0002 Q3.
        self.draw_mesh_textured(
            &mesh,
            &texture,
            program,
            &mvp,
            model,
            tint,
            light.direction,
            view.camera_position,
            light.ambient,
            material.metallic,
            material.roughness,
        );
    }

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

            let stride = std::mem::size_of::<Vertex>() as GLsizei; // 48 bytes

            // location 0 — position (vec3, byte offset 0)
            (gl.vertex_attrib_pointer)(0, 3, GL_FLOAT, GL_FALSE, stride, 0 as *const c_void);
            (gl.enable_vertex_attrib_array)(0);

            // location 1 — normal (vec3, byte offset 12)
            (gl.vertex_attrib_pointer)(1, 3, GL_FLOAT, GL_FALSE, stride, 12 as *const c_void);
            (gl.enable_vertex_attrib_array)(1);

            // location 2 — uv (vec2, byte offset 24)
            (gl.vertex_attrib_pointer)(2, 2, GL_FLOAT, GL_FALSE, stride, 24 as *const c_void);
            (gl.enable_vertex_attrib_array)(2);

            // location 3 — tangent (vec4: xyz tangent + w handedness sign,
            // byte offset 32). Shaders that don't read this attribute can
            // ignore it; binding it here keeps the VAO state consistent for
            // future normal-mapped material shaders. See ADR 0002 Q4.
            (gl.vertex_attrib_pointer)(3, 4, GL_FLOAT, GL_FALSE, stride, 32 as *const c_void);
            (gl.enable_vertex_attrib_array)(3);

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

            let vert = self.compile_stage(GL_VERTEX_SHADER, vert_src);
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
                    program,
                    log_len,
                    null_mut(),
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
        let ptr = src.as_ptr() as *const GLchar;
        let length = src.len() as GLint;
        (gl.shader_source)(shader, 1, &ptr, &length);
        (gl.compile_shader)(shader);

        let mut status: GLint = 0;
        (gl.get_shaderiv)(shader, GL_COMPILE_STATUS, &mut status);
        if status == 0 {
            let mut log_len: GLint = 0;
            (gl.get_shaderiv)(shader, GL_INFO_LOG_LENGTH, &mut log_len);
            let mut log = vec![0u8; log_len as usize];
            (gl.get_shader_info_log)(shader, log_len, null_mut(), log.as_mut_ptr() as *mut GLchar);
            let stage = if kind == GL_VERTEX_SHADER {
                "vertex"
            } else {
                "fragment"
            };
            let msg = String::from_utf8_lossy(&log);
            panic!("[GL] {} shader compile error:\n{}", stage, msg);
        }

        shader
    }

    // ── Texture upload ────────────────────────────────────────────────────────

    /// Upload RGBA8 pixel data to a new 2-D GL texture in linear colour
    /// space. Returns the GL texture ID.
    ///
    /// Mip-maps are generated automatically; linear filtering is set on all
    /// levels. Use [`Self::upload_texture_rgba8_with_internal_format`] when
    /// the source data is sRGB-encoded and the GPU should perform the
    /// sRGB→linear decode at sample time.
    pub fn upload_texture_rgba8(&self, width: u32, height: u32, data: &[u8]) -> u32 {
        self.upload_texture_rgba8_with_internal_format(
            width,
            height,
            data,
            GL_RGBA as GLint,
        )
    }

    /// Upload RGBA8 pixel data to a new 2-D GL texture with an explicit
    /// internal format.
    ///
    /// `internal_format` controls how the GPU stores and interprets the
    /// pixels. Two values are common:
    /// - `GL_RGBA` (linear; what [`Self::upload_texture_rgba8`] uses):
    ///   bytes are stored as-is and sampled as-is.
    /// - `GL_SRGB8_ALPHA8` (sRGB-encoded source): bytes are stored
    ///   as-is but **the GPU applies the sRGB→linear decode every time
    ///   the texture is sampled.** This is the correct way to handle
    ///   authored colour textures (`base_color`, albedo) in a PBR
    ///   pipeline; it eliminates an entire class of shader-side gamma
    ///   bugs at zero runtime cost. See ADR 0002 Q3.
    pub fn upload_texture_rgba8_with_internal_format(
        &self,
        width: u32,
        height: u32,
        data: &[u8],
        internal_format: GLint,
    ) -> u32 {
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
                0,
                internal_format,
                width as GLsizei,
                height as GLsizei,
                0,       // border (must be 0)
                GL_RGBA, // source format — we always upload RGBA byte-order
                GL_UNSIGNED_BYTE,
                data.as_ptr() as *const c_void,
            );

            (gl.generate_mipmap)(GL_TEXTURE_2D);

            (gl.tex_parameteri)(
                GL_TEXTURE_2D,
                GL_TEXTURE_MIN_FILTER,
                GL_LINEAR_MIPMAP_LINEAR,
            );
            (gl.tex_parameteri)(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_LINEAR);
            (gl.tex_parameteri)(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_REPEAT);
            (gl.tex_parameteri)(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_REPEAT);

            (gl.bind_texture)(GL_TEXTURE_2D, 0);

            tex
        }
    }

    // ── Draw calls ────────────────────────────────────────────────────────────

    /// Draw a VAO with a compiled shader program.
    ///
    /// Uniform locations are looked up once per program and cached, so
    /// repeated draws with the same program pay only a single `HashMap`
    /// lookup instead of a GL call per uniform.
    ///
    /// # Parameters
    /// - `vao`         — vertex array object ID (from [`Self::upload_mesh_raw`])
    /// - `index_count` — number of indices to draw
    /// - `program`     — GL shader program ID (from [`Self::compile_shader_program`])
    /// - `mvp`         — combined model-view-projection matrix
    /// - `model`       — model matrix (world-space normal transform)
    /// - `color`       — RGBA base colour / tint
    /// - `light_dir`   — world-space directional light direction
    /// - `view_pos`    — camera world-space position (for specular; ignored by flat shaders)
    /// - `ambient`     — ambient light intensity 0..1 (ignored by flat shaders)
    pub fn draw_vao(
        &mut self,
        vao: u32,
        index_count: u32,
        program: u32,
        mvp: &Mat4,
        model: &Mat4,
        color: Color,
        light_dir: Vec3,
        view_pos: Vec3,
        ambient: f32,
    ) {
        let uniforms = self.cached_uniforms(program);
        unsafe {
            let gl = &self.gl;
            (gl.use_program)(program);

            if uniforms.mvp >= 0 {
                (gl.uniform_matrix4fv)(
                    uniforms.mvp,
                    1,
                    GL_FALSE,
                    mvp.cols.as_ptr() as *const GLfloat,
                );
            }
            if uniforms.model >= 0 {
                (gl.uniform_matrix4fv)(
                    uniforms.model,
                    1,
                    GL_FALSE,
                    model.cols.as_ptr() as *const GLfloat,
                );
            }
            if uniforms.color >= 0 {
                (gl.uniform4fv)(
                    uniforms.color,
                    1,
                    [color.r, color.g, color.b, color.a].as_ptr(),
                );
            }
            if uniforms.light_dir >= 0 {
                (gl.uniform3fv)(
                    uniforms.light_dir,
                    1,
                    [light_dir.x, light_dir.y, light_dir.z].as_ptr(),
                );
            }
            if uniforms.view_pos >= 0 {
                (gl.uniform3fv)(
                    uniforms.view_pos,
                    1,
                    [view_pos.x, view_pos.y, view_pos.z].as_ptr(),
                );
            }
            if uniforms.ambient >= 0 {
                // glUniform1f — load the function pointer type inline
                type GlUniform1f = unsafe extern "system" fn(GLint, GLfloat);
                let ptr = load_gl_fn("glUniform1f", self.opengl32);
                if !ptr.is_null() {
                    let f = std::mem::transmute::<*const c_void, GlUniform1f>(ptr);
                    f(uniforms.ambient, ambient);
                }
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

    // ── Uniform cache helper ──────────────────────────────────────────────────

    /// Return (and lazily populate) the cached uniform locations for `program`.
    fn cached_uniforms(&mut self, program: u32) -> CachedUniforms {
        if !self.uniform_cache.contains_key(&program) {
            let cached = unsafe { CachedUniforms::load(&self.gl, program) };
            self.uniform_cache.insert(program, cached);
        }
        self.uniform_cache[&program]
    }

    /// Evict a program's cached uniform locations.  Call after
    /// [`delete_program`](Self::delete_program) so stale entries don't linger.
    pub fn invalidate_uniform_cache(&mut self, program: u32) {
        self.uniform_cache.remove(&program);
    }

    // ── Cleanup helpers ───────────────────────────────────────────────────────

    /// Upload a [`Texture`] to the GPU and return a [`GpuTexture`].
    ///
    /// The texture is converted to RGBA8 byte-order if it is not already,
    /// and the GPU's internal format is selected from
    /// `tex.color_space`:
    ///
    /// - [`ColorSpace::Srgb`]  → `GL_SRGB8_ALPHA8`. The GPU applies the
    ///   sRGB→linear decode at sample time. This is the correct behaviour
    ///   for any base-colour / albedo / authored-visual texture.
    /// - [`ColorSpace::Linear`] → `GL_RGBA`. Bytes pass through unmodified.
    ///   Required for any data-bearing texture (roughness, metallic, AO,
    ///   masks, height).
    ///
    /// See ADR 0002 Q3 for the discipline rationale.
    pub fn upload_texture(&self, tex: &Texture) -> GpuTexture {
        // Convert to RGBA8 byte-order if needed. The colour-space tag
        // applies regardless of source channel layout.
        let rgba: std::borrow::Cow<'_, [u8]> = match tex.format {
            TextureFormat::Rgba8 => std::borrow::Cow::Borrowed(&tex.data),
            TextureFormat::Rgb8 => {
                let mut out = Vec::with_capacity((tex.width * tex.height * 4) as usize);
                for chunk in tex.data.chunks_exact(3) {
                    out.push(chunk[0]);
                    out.push(chunk[1]);
                    out.push(chunk[2]);
                    out.push(255);
                }
                std::borrow::Cow::Owned(out)
            }
            TextureFormat::R8 => {
                let mut out = Vec::with_capacity((tex.width * tex.height * 4) as usize);
                for &b in &tex.data {
                    out.push(b);
                    out.push(b);
                    out.push(b);
                    out.push(255);
                }
                std::borrow::Cow::Owned(out)
            }
        };
        let internal_format = match tex.color_space {
            ColorSpace::Srgb => GL_SRGB8_ALPHA8,
            ColorSpace::Linear => GL_RGBA as GLint,
        };
        let id = self.upload_texture_rgba8_with_internal_format(
            tex.width,
            tex.height,
            &rgba,
            internal_format,
        );
        GpuTexture {
            id,
            width: tex.width,
            height: tex.height,
        }
    }

    /// Draw a textured mesh with Disney metallic-roughness PBR lighting.
    ///
    /// Binds `albedo` to unit 0, sets `u_texture`, threads matrices and
    /// PBR factors through cached uniform locations, and issues the
    /// indexed draw. The shader expected on `program` is the
    /// `TEXTURED_VERT` / `TEXTURED_FRAG` pair from
    /// [`crate::renderer::opengl_shaders`]; the BRDF math expects a
    /// linear-space `albedo` sample (which a sRGB-tagged
    /// [`crate::assets::Texture`] uploaded via [`Self::upload_texture`]
    /// gives automatically).
    ///
    /// `metallic` and `roughness` are scalar PBR factors in `[0, 1]`;
    /// see [`crate::assets::Material`] for the per-material defaults
    /// and ADR 0002 Q3 for the shading-model rationale.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_mesh_textured(
        &mut self,
        mesh: &GpuMesh,
        texture: &GpuTexture,
        program: u32,
        mvp: &Mat4,
        model: &Mat4,
        color: Color,
        light_dir: Vec3,
        view_pos: Vec3,
        ambient: f32,
        metallic: f32,
        roughness: f32,
    ) {
        let uniforms = self.cached_uniforms(program);
        unsafe {
            let gl = &self.gl;
            (gl.use_program)(program);

            (gl.active_texture)(GL_TEXTURE0);
            (gl.bind_texture)(GL_TEXTURE_2D, texture.id);

            if uniforms.texture >= 0 {
                (gl.uniform1i)(uniforms.texture, 0);
            }
            if uniforms.mvp >= 0 {
                (gl.uniform_matrix4fv)(
                    uniforms.mvp,
                    1,
                    GL_FALSE,
                    mvp.cols.as_ptr() as *const GLfloat,
                );
            }
            if uniforms.model >= 0 {
                (gl.uniform_matrix4fv)(
                    uniforms.model,
                    1,
                    GL_FALSE,
                    model.cols.as_ptr() as *const GLfloat,
                );
            }
            if uniforms.color >= 0 {
                (gl.uniform4fv)(
                    uniforms.color,
                    1,
                    [color.r, color.g, color.b, color.a].as_ptr(),
                );
            }
            if uniforms.light_dir >= 0 {
                (gl.uniform3fv)(
                    uniforms.light_dir,
                    1,
                    [light_dir.x, light_dir.y, light_dir.z].as_ptr(),
                );
            }
            if uniforms.view_pos >= 0 {
                (gl.uniform3fv)(
                    uniforms.view_pos,
                    1,
                    [view_pos.x, view_pos.y, view_pos.z].as_ptr(),
                );
            }
            // The `f32`-uniform GL function pointer is loaded lazily here
            // because GlFns doesn't yet stash it; resolved once per call,
            // shared across `ambient` / `metallic` / `roughness`. A future
            // cleanup commit can promote it into GlFns.
            type GlUniform1f = unsafe extern "system" fn(GLint, GLfloat);
            let uniform1f_ptr = load_gl_fn("glUniform1f", self.opengl32);
            let set_f32 = |loc: GLint, value: f32| {
                if loc >= 0 && !uniform1f_ptr.is_null() {
                    let f = std::mem::transmute::<*const c_void, GlUniform1f>(uniform1f_ptr);
                    f(loc, value);
                }
            };
            set_f32(uniforms.ambient, ambient);
            set_f32(uniforms.metallic, metallic);
            set_f32(uniforms.roughness, roughness);

            (gl.bind_vertex_array)(mesh.vao);
            (gl.draw_elements)(
                GL_TRIANGLES,
                mesh.index_count as GLsizei,
                GL_UNSIGNED_INT,
                null_mut(),
            );
            (gl.bind_vertex_array)(0);
            (gl.bind_texture)(GL_TEXTURE_2D, 0);
        }
    }

    /// Upload a [`NormalMap`] to the GPU and return a [`GpuTexture`] handle.
    ///
    /// Normal maps are *unconditionally linear* at the type level
    /// (ADR 0002 Q4). The GPU upload path uses `GL_RGBA` as the internal
    /// format — explicitly NOT `GL_SRGB8_ALPHA8` — so the GPU does not
    /// apply an sRGB→linear decode that would corrupt the encoded normal
    /// vectors. This is the type-system pin discipline from ADR 0002 Q3
    /// extended into the upload layer: the colour-space decision lives on
    /// the asset type and cannot be accidentally inverted at the GL call
    /// site.
    ///
    /// The on-disk bytes are already in the engine canonical OpenGL Y+
    /// convention; the `NormalMap::from_directx` path flips the green
    /// channel at construction time so the GPU only ever sees canonical
    /// data.
    pub fn upload_normal_map(&self, nm: &NormalMap) -> GpuTexture {
        // Reuse the texture upload path. The format conversion logic for
        // RGB8 / R8 → RGBA8 is identical to upload_texture and the on-disk
        // bytes are already in the engine canonical convention.
        let rgba: std::borrow::Cow<'_, [u8]> = match nm.format {
            TextureFormat::Rgba8 => std::borrow::Cow::Borrowed(&nm.data),
            TextureFormat::Rgb8 => {
                let mut out = Vec::with_capacity((nm.width * nm.height * 4) as usize);
                for chunk in nm.data.chunks_exact(3) {
                    out.push(chunk[0]);
                    out.push(chunk[1]);
                    out.push(chunk[2]);
                    out.push(255);
                }
                std::borrow::Cow::Owned(out)
            }
            TextureFormat::R8 => {
                // R8 normal map: encode `(R, 128, 255, 255)` so the unpacked
                // tangent-space normal points straight out of the surface
                // when the input is mid-grey, matching the convention for
                // single-channel height-derived normals before BC5
                // reconstruction is implemented.
                let mut out = Vec::with_capacity((nm.width * nm.height * 4) as usize);
                for &b in &nm.data {
                    out.push(b);
                    out.push(128);
                    out.push(255);
                    out.push(255);
                }
                std::borrow::Cow::Owned(out)
            }
        };
        let id = self.upload_texture_rgba8(nm.width, nm.height, &rgba);
        GpuTexture {
            id,
            width: nm.width,
            height: nm.height,
        }
    }

    /// Draw a textured + normal-mapped mesh with Disney metallic-roughness
    /// PBR lighting.
    ///
    /// Same as [`Self::draw_mesh_textured`] but additionally binds
    /// `normal_map` to texture unit 1 and sets `u_normal_map`. Use with the
    /// `TEXTURED_NORMAL_VERT` / `TEXTURED_NORMAL_FRAG` shader pair, which
    /// reads the location-3 tangent attribute, reconstructs a per-pixel
    /// TBN, samples the tangent-space normal, and runs the same Disney
    /// BRDF + ACES tonemap as the non-normal-mapped path. See ADR 0002 Q3
    /// + Q4.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_mesh_textured_normal_mapped(
        &mut self,
        mesh: &GpuMesh,
        albedo: &GpuTexture,
        normal_map: &GpuTexture,
        program: u32,
        mvp: &Mat4,
        model: &Mat4,
        color: Color,
        light_dir: Vec3,
        view_pos: Vec3,
        ambient: f32,
        metallic: f32,
        roughness: f32,
    ) {
        let uniforms = self.cached_uniforms(program);
        unsafe {
            let gl = &self.gl;
            (gl.use_program)(program);

            (gl.active_texture)(GL_TEXTURE0);
            (gl.bind_texture)(GL_TEXTURE_2D, albedo.id);
            (gl.active_texture)(GL_TEXTURE1);
            (gl.bind_texture)(GL_TEXTURE_2D, normal_map.id);

            if uniforms.texture >= 0 {
                (gl.uniform1i)(uniforms.texture, 0);
            }
            if uniforms.normal_map >= 0 {
                (gl.uniform1i)(uniforms.normal_map, 1);
            }
            if uniforms.mvp >= 0 {
                (gl.uniform_matrix4fv)(
                    uniforms.mvp,
                    1,
                    GL_FALSE,
                    mvp.cols.as_ptr() as *const GLfloat,
                );
            }
            if uniforms.model >= 0 {
                (gl.uniform_matrix4fv)(
                    uniforms.model,
                    1,
                    GL_FALSE,
                    model.cols.as_ptr() as *const GLfloat,
                );
            }
            if uniforms.color >= 0 {
                (gl.uniform4fv)(
                    uniforms.color,
                    1,
                    [color.r, color.g, color.b, color.a].as_ptr(),
                );
            }
            if uniforms.light_dir >= 0 {
                (gl.uniform3fv)(
                    uniforms.light_dir,
                    1,
                    [light_dir.x, light_dir.y, light_dir.z].as_ptr(),
                );
            }
            if uniforms.view_pos >= 0 {
                (gl.uniform3fv)(
                    uniforms.view_pos,
                    1,
                    [view_pos.x, view_pos.y, view_pos.z].as_ptr(),
                );
            }
            type GlUniform1f = unsafe extern "system" fn(GLint, GLfloat);
            let uniform1f_ptr = load_gl_fn("glUniform1f", self.opengl32);
            let set_f32 = |loc: GLint, value: f32| {
                if loc >= 0 && !uniform1f_ptr.is_null() {
                    let f = std::mem::transmute::<*const c_void, GlUniform1f>(uniform1f_ptr);
                    f(loc, value);
                }
            };
            set_f32(uniforms.ambient, ambient);
            set_f32(uniforms.metallic, metallic);
            set_f32(uniforms.roughness, roughness);

            (gl.bind_vertex_array)(mesh.vao);
            (gl.draw_elements)(
                GL_TRIANGLES,
                mesh.index_count as GLsizei,
                GL_UNSIGNED_INT,
                null_mut(),
            );
            (gl.bind_vertex_array)(0);

            // Unbind both texture units in reverse order.
            (gl.active_texture)(GL_TEXTURE1);
            (gl.bind_texture)(GL_TEXTURE_2D, 0);
            (gl.active_texture)(GL_TEXTURE0);
            (gl.bind_texture)(GL_TEXTURE_2D, 0);
        }
    }

    /// Synchronously read the current default-framebuffer's colour pixels
    /// into a freshly allocated RGBA8 buffer (4 bytes per pixel, top-down
    /// row order — OpenGL returns rows bottom-up, so the function flips
    /// them on the way out for callers that expect top-down image data
    /// like the editor frame inspector and screen captures).
    ///
    /// Used by the frame graph's per-pass thumbnail capture
    /// (see `FrameGraph::execute_with_capture`, ADR 0002 Q8).
    /// `glReadPixels` is synchronous and stalls the pipeline; this is
    /// intended for editor / debug capture flows, not the steady-state
    /// hot path.
    pub fn read_framebuffer_rgba8(&self) -> Vec<u8> {
        let w = self.width.max(1) as usize;
        let h = self.height.max(1) as usize;
        let mut bottom_up = vec![0u8; w * h * 4];
        unsafe {
            (self.gl.read_pixels)(
                0,
                0,
                w as GLsizei,
                h as GLsizei,
                GL_RGBA,
                GL_UNSIGNED_BYTE,
                bottom_up.as_mut_ptr() as *mut c_void,
            );
        }
        // Flip rows so the result is top-down.
        let row_bytes = w * 4;
        let mut top_down = vec![0u8; w * h * 4];
        for y in 0..h {
            let src = (h - 1 - y) * row_bytes;
            let dst = y * row_bytes;
            top_down[dst..dst + row_bytes].copy_from_slice(&bottom_up[src..src + row_bytes]);
        }
        top_down
    }

    /// Free a GPU texture previously created by [`Self::upload_texture`].
    pub fn free_texture(&self, tex: GpuTexture) {
        self.delete_texture(tex.id);
    }

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

    /// Delete a compiled shader program and evict its cached uniform locations.
    pub fn delete_program(&mut self, program: u32) {
        self.uniform_cache.remove(&program);
        unsafe {
            (self.gl.delete_program)(program);
        }
    }

    /// Delete a texture previously created by [`Self::upload_texture_rgba8`].
    pub fn delete_texture(&self, tex_id: u32) {
        unsafe {
            (self.gl.delete_textures)(1, &tex_id);
        }
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

    /// Draw a [`GpuMesh`] with a compiled shader program.
    ///
    /// Forwards all parameters to [`draw_vao`](Self::draw_vao).
    pub fn draw_mesh(
        &mut self,
        mesh: &GpuMesh,
        program: u32,
        mvp: &Mat4,
        model: &Mat4,
        color: Color,
        light_dir: Vec3,
        view_pos: Vec3,
        ambient: f32,
    ) {
        self.draw_vao(
            mesh.vao,
            mesh.index_count,
            program,
            mvp,
            model,
            color,
            light_dir,
            view_pos,
            ambient,
        );
    }

    /// Free all GPU resources owned by a GpuMesh.
    pub fn free_mesh(&self, mesh: GpuMesh) {
        self.delete_vao(mesh.vao, mesh.vbo, mesh.ebo);
    }
}

// ── Renderer trait impl ───────────────────────────────────────────────────────

impl Renderer for OpenGLRenderer {
    fn width(&self) -> u32 {
        self.width
    }
    fn height(&self) -> u32 {
        self.height
    }

    fn resize(&mut self, width: u32, height: u32) {
        self.width = width;
        self.height = height;
        unsafe {
            (self.gl.viewport)(0, 0, width as GLsizei, height as GLsizei);
        }
    }

    fn begin_frame(&mut self) {}

    fn clear(&mut self, color: Color) {
        unsafe {
            (self.gl.clear_color)(color.r, color.g, color.b, color.a);
            (self.gl.clear)(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);
        }
    }

    fn end_frame(&mut self) {
        unsafe {
            SwapBuffers(self.hdc);
        }
    }

    fn submit_frame(&mut self, assets: &AssetStore, frame: &RenderFrame) {
        // Route through the M9 scene frame graph so every editor frame
        // exercises the same pass topology the inspector captures.
        // See ADR 0002 Q8 (frame graph as a typed node graph) and ADR
        // 0003 Pillar 4 (built-in frame inspector) for the direction.
        //
        // The take/execute/put-back dance sidesteps a self borrow
        // conflict: `graph.execute(self, ...)` needs `&mut self`, but
        // `self.scene_frame_graph.execute(...)` also borrows `self`
        // via the field access. Taking the Option out gives the graph
        // an owned handle for the duration of the call.
        if let Some(graph) = self.scene_frame_graph.take() {
            graph.execute(self, assets, frame);
            self.scene_frame_graph = Some(graph);
        } else {
            self.submit_frame_direct(assets, frame);
        }
    }

    fn submit_frame_capturing(
        &mut self,
        assets: &AssetStore,
        frame: &RenderFrame,
        thumb_w: u32,
        thumb_h: u32,
    ) -> Vec<PassThumbnail> {
        // Inspector path: reuse the same graph, but ask for per-pass
        // thumbnails. glReadPixels stalls the GPU on every pass — see
        // `frame_graph::capture` for the performance posture.
        if let Some(graph) = self.scene_frame_graph.take() {
            let thumbs = graph.execute_with_capture(self, assets, frame, thumb_w, thumb_h);
            self.scene_frame_graph = Some(graph);
            thumbs
        } else {
            self.submit_frame_direct(assets, frame);
            Vec::new()
        }
    }

    fn render_debug_lines(&mut self, lines: &[DebugLine], vp: &Mat4) {
        if lines.is_empty() {
            return;
        }

        // Build flat vertex buffer: [x y z r g b a] × 2 per line.
        let mut verts: Vec<f32> = Vec::with_capacity(lines.len() * 14);
        for line in lines {
            verts.extend_from_slice(&[
                line.start.x,
                line.start.y,
                line.start.z,
                line.color.r,
                line.color.g,
                line.color.b,
                line.color.a,
            ]);
            verts.extend_from_slice(&[
                line.end.x,
                line.end.y,
                line.end.z,
                line.color.r,
                line.color.g,
                line.color.b,
                line.color.a,
            ]);
        }

        unsafe {
            let gl = &self.gl;

            (gl.bind_vertex_array)(self.line_vao);
            (gl.bind_buffer)(GL_ARRAY_BUFFER, self.line_vbo);
            (gl.buffer_data)(
                GL_ARRAY_BUFFER,
                (verts.len() * std::mem::size_of::<f32>()) as GLsizeiptr,
                verts.as_ptr() as *const c_void,
                GL_DYNAMIC_DRAW,
            );

            (gl.use_program)(self.line_program);

            // Upload VP matrix.
            let u_vp =
                (gl.get_uniform_location)(self.line_program, b"u_vp\0".as_ptr() as *const GLchar);
            if u_vp >= 0 {
                (gl.uniform_matrix4fv)(u_vp, 1, GL_FALSE, vp.cols.as_ptr() as *const GLfloat);
            }

            (gl.draw_arrays)(GL_LINES, 0, (lines.len() * 2) as GLsizei);
            (gl.bind_vertex_array)(0);
        }
    }

    fn render_screen_quads(&mut self, quads: &[ScreenQuad]) {
        if quads.is_empty() {
            return;
        }

        let width = self.width.max(1) as f32;
        let height = self.height.max(1) as f32;
        let mut verts: Vec<f32> = Vec::with_capacity(quads.len() * 42);

        for quad in quads {
            let x0 = quad.x;
            let y0 = quad.y;
            let x1 = quad.x + quad.w;
            let y1 = quad.y + quad.h;

            let nx0 = x0 / width * 2.0 - 1.0;
            let nx1 = x1 / width * 2.0 - 1.0;
            let ny0 = 1.0 - y0 / height * 2.0;
            let ny1 = 1.0 - y1 / height * 2.0;
            let z = -1.0;

            let color = [quad.color.r, quad.color.g, quad.color.b, quad.color.a];
            let p0 = [nx0, ny0, z];
            let p1 = [nx1, ny0, z];
            let p2 = [nx1, ny1, z];
            let p3 = [nx0, ny1, z];

            for position in [p0, p1, p2, p0, p2, p3] {
                verts.extend_from_slice(&[
                    position[0],
                    position[1],
                    position[2],
                    color[0],
                    color[1],
                    color[2],
                    color[3],
                ]);
            }
        }

        unsafe {
            let gl = &self.gl;

            (gl.bind_vertex_array)(self.line_vao);
            (gl.bind_buffer)(GL_ARRAY_BUFFER, self.line_vbo);
            (gl.buffer_data)(
                GL_ARRAY_BUFFER,
                (verts.len() * std::mem::size_of::<f32>()) as GLsizeiptr,
                verts.as_ptr() as *const c_void,
                GL_DYNAMIC_DRAW,
            );

            (gl.use_program)(self.line_program);

            let u_vp =
                (gl.get_uniform_location)(self.line_program, b"u_vp\0".as_ptr() as *const GLchar);
            if u_vp >= 0 {
                (gl.uniform_matrix4fv)(
                    u_vp,
                    1,
                    GL_FALSE,
                    Mat4::IDENTITY.cols.as_ptr() as *const GLfloat,
                );
            }

            (gl.draw_arrays)(GL_TRIANGLES, 0, (quads.len() * 6) as GLsizei);
            (gl.bind_vertex_array)(0);
        }
    }
}

// ── Drop ─────────────────────────────────────────────────────────────────────

impl Drop for OpenGLRenderer {
    fn drop(&mut self) {
        unsafe {
            let meshes: Vec<_> = self
                .frame_mesh_cache
                .drain()
                .map(|(_, mesh)| mesh)
                .collect();
            let terrain_meshes: Vec<_> = self
                .terrain_patch_mesh_cache
                .drain()
                .map(|(_, mesh)| mesh)
                .collect();
            let textures: Vec<_> = self
                .frame_texture_cache
                .drain()
                .map(|(_, texture)| texture)
                .collect();
            let programs: Vec<_> = self
                .frame_program_cache
                .drain()
                .map(|(_, program)| program)
                .collect();

            for mesh in meshes {
                self.delete_vao(mesh.vao, mesh.vbo, mesh.ebo);
            }
            for mesh in terrain_meshes {
                self.delete_vao(mesh.vao, mesh.vbo, mesh.ebo);
            }
            if let Some(mesh) = self.unit_plane_mesh.take() {
                self.delete_vao(mesh.vao, mesh.vbo, mesh.ebo);
            }
            for texture in textures {
                (self.gl.delete_textures)(1, &texture.id);
            }
            for program in programs {
                self.uniform_cache.remove(&program);
                (self.gl.delete_program)(program);
            }

            // Free line rendering resources.
            (self.gl.delete_buffers)(1, &self.line_vbo);
            (self.gl.delete_vertex_arrays)(1, &self.line_vao);
            (self.gl.delete_program)(self.line_program);

            wglMakeCurrent(null_mut(), null_mut());
            wglDeleteContext(self.hglrc);
            ReleaseDC(self.hwnd, self.hdc);
            if !self.opengl32.is_null() {
                FreeLibrary(self.opengl32);
            }
        }
    }
}

/// Build the M9 scene frame graph: the ordered pass sequence the
/// editor's real render + the inspector both go through. Kept as a
/// free function so it can be called from
/// [`OpenGLRenderer::new`] without touching partially-initialised
/// fields.
///
/// Pass order — matches the pre-M9 flat submit_frame:
/// 1. `ClearAndViewport`: viewport rectangle + clear color from the
///    primary view.
/// 2. `Scene`: 3D content (terrain, meshes, vegetation, water,
///    particles). Runs against the primary view.
/// 3. `DebugLines`: gizmos + editor overlays, in world space.
/// 4. `ScreenQuads`: native UI + inspector chrome, in pixel space.
fn build_scene_frame_graph() -> super::FrameGraphBuilder {
    let mut b = super::FrameGraphBuilder::new();
    b.add_pass(Box::new(super::ClearAndViewportPass::new("Clear")));
    b.add_pass(Box::new(super::ScenePass::new("Scene")));
    b.add_pass(Box::new(super::DebugLinesPass::new("DebugLines")));
    b.add_pass(Box::new(super::ScreenQuadsPass::new("ScreenQuads")));
    b
}

#[cfg(test)]
mod tests {
    use super::{billboard_model_matrix, build_terrain_patch_mesh, terrain_patch_subdivisions};
    use crate::math::Vec3;
    use crate::renderer::TerrainPatchItem;

    #[test]
    fn terrain_patch_mesh_builder_respects_lod() {
        let patch = TerrainPatchItem {
            origin: Vec3::new(32.0, 0.0, 48.0),
            size: 64.0,
            height_scale: 72.0,
            lod_level: 1,
        };

        let mesh = build_terrain_patch_mesh(&patch);
        let subdivisions = terrain_patch_subdivisions(patch.lod_level);
        assert_eq!(mesh.vertices.len(), (subdivisions + 1) * (subdivisions + 1));
        assert_eq!(mesh.indices.len(), subdivisions * subdivisions * 6);
    }

    #[test]
    fn billboard_model_faces_camera_and_keeps_translation() {
        let position = Vec3::new(10.0, 4.0, -2.0);
        let camera = Vec3::new(14.0, 6.0, 5.0);
        let model = billboard_model_matrix(position, 3.5, camera);
        let facing = Vec3::new(model.cols[1][0], model.cols[1][1], model.cols[1][2]).normalize();
        let expected = (camera - position).normalize();

        assert!(facing.dot(expected) > 0.999);
        assert_eq!(
            Vec3::new(model.cols[3][0], model.cols[3][1], model.cols[3][2]),
            position
        );
    }
}
