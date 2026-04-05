//! Embedded GLSL shader sources for the OpenGL renderer backend.
//!
//! All shaders target GLSL 3.30 core (OpenGL 3.3), which is the minimum
//! version the engine requires.  No file I/O is needed — the strings are
//! baked into the binary at compile time.

/// Default mesh vertex shader.
///
/// Vertex layout (matches [`super::opengl::Vertex`]):
/// - location 0 — position  (vec3)
/// - location 1 — normal    (vec3)
/// - location 2 — UV coords (vec2)
pub const MESH_VERT: &str = r#"
#version 330 core
layout(location = 0) in vec3 a_pos;
layout(location = 1) in vec3 a_normal;
layout(location = 2) in vec2 a_uv;
uniform mat4 u_mvp;
uniform mat4 u_model;
out vec3 v_normal_world;
out vec2 v_uv;
void main() {
    gl_Position  = u_mvp * vec4(a_pos, 1.0);
    v_normal_world = mat3(transpose(inverse(u_model))) * a_normal;
    v_uv = a_uv;
}
"#;

/// Default mesh fragment shader — Blinn-Phong with a single directional light.
pub const MESH_FRAG: &str = r#"
#version 330 core
in  vec3 v_normal_world;
in  vec2 v_uv;
out vec4 frag_color;
uniform vec3 u_light_dir;   // world-space, pointing toward light
uniform vec4 u_color;       // base RGBA color
void main() {
    float diff = max(dot(normalize(v_normal_world), normalize(u_light_dir)), 0.0);
    frag_color  = u_color * (0.15 + 0.85 * diff);
}
"#;

/// Flat unlit vertex shader — passes position through MVP; ignores normals/UVs.
pub const FLAT_VERT: &str = r#"
#version 330 core
layout(location = 0) in vec3 a_pos;
layout(location = 1) in vec3 a_normal;
layout(location = 2) in vec2 a_uv;
uniform mat4 u_mvp;
void main() { gl_Position = u_mvp * vec4(a_pos, 1.0); }
"#;

/// Flat unlit fragment shader — outputs a uniform colour with no lighting.
pub const FLAT_FRAG: &str = r#"
#version 330 core
out vec4 frag_color;
uniform vec4 u_color;
void main() { frag_color = u_color; }
"#;
