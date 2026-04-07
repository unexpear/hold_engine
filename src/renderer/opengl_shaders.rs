//! Embedded GLSL shader sources for the OpenGL renderer backend.
//!
//! All shaders target GLSL 3.30 core (OpenGL 3.3), which is the minimum
//! version the engine requires.  No file I/O is needed — the strings are
//! baked into the binary at compile time.
//!
//! # Uniform conventions (shared across all lit shaders)
//!
//! | Uniform         | Type      | Description                              |
//! |-----------------|-----------|------------------------------------------|
//! | `u_mvp`         | `mat4`    | Model-view-projection matrix             |
//! | `u_model`       | `mat4`    | Model matrix (world-space normals)       |
//! | `u_color`       | `vec4`    | Base RGBA colour / tint                  |
//! | `u_light_dir`   | `vec3`    | World-space directional light direction  |
//! | `u_view_pos`    | `vec3`    | Camera/eye world-space position          |
//! | `u_ambient`     | `float`   | Ambient light intensity (0..1)           |
//! | `u_texture`     | `sampler2D` | Albedo texture (textured shaders only) |

// ── Lit mesh shaders (Blinn-Phong: ambient + diffuse + specular) ──────────────

/// Vertex shader for lit meshes.
///
/// Outputs world-space position and normal for per-fragment lighting, plus UVs.
///
/// Vertex layout (matches [`crate::assets::mesh::Vertex`]):
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

out vec3 v_pos_world;
out vec3 v_normal_world;
out vec2 v_uv;

void main() {
    vec4 world_pos  = u_model * vec4(a_pos, 1.0);
    gl_Position     = u_mvp   * vec4(a_pos, 1.0);
    v_pos_world     = world_pos.xyz;
    // Normal matrix: transpose of the inverse of the upper-3x3 of u_model.
    // Handles non-uniform scale correctly.
    v_normal_world  = mat3(transpose(inverse(u_model))) * a_normal;
    v_uv            = a_uv;
}
"#;

/// Fragment shader for lit meshes — Blinn-Phong with one directional light.
///
/// Combines ambient + diffuse + specular.  The specular term uses the
/// half-vector (Blinn) for a rounder, cheaper highlight than Phong.
pub const MESH_FRAG: &str = r#"
#version 330 core
in  vec3 v_pos_world;
in  vec3 v_normal_world;
in  vec2 v_uv;

out vec4 frag_color;

uniform vec4  u_color;       // base RGBA
uniform vec3  u_light_dir;   // world-space, pointing *toward* the light
uniform vec3  u_view_pos;    // camera world position (for specular)
uniform float u_ambient;     // ambient intensity  (default: 0.15)

void main() {
    vec3 N = normalize(v_normal_world);
    vec3 L = normalize(u_light_dir);
    vec3 V = normalize(u_view_pos - v_pos_world);
    vec3 H = normalize(L + V);  // Blinn-Phong halfway vector

    float amb  = max(u_ambient, 0.0);
    float diff = max(dot(N, L), 0.0);
    float spec = pow(max(dot(N, H), 0.0), 32.0) * (1.0 - amb);

    float light = clamp(amb + (1.0 - amb) * diff + 0.15 * spec, 0.0, 1.0);
    frag_color  = vec4(u_color.rgb * light, u_color.a);
}
"#;

// ── Flat / unlit shaders ──────────────────────────────────────────────────────

/// Flat vertex shader — passes position through MVP; normals and UVs ignored.
pub const FLAT_VERT: &str = r#"
#version 330 core
layout(location = 0) in vec3 a_pos;
layout(location = 1) in vec3 a_normal;
layout(location = 2) in vec2 a_uv;
uniform mat4 u_mvp;
void main() { gl_Position = u_mvp * vec4(a_pos, 1.0); }
"#;

/// Flat fragment shader — outputs a solid colour with no lighting.
pub const FLAT_FRAG: &str = r#"
#version 330 core
out vec4 frag_color;
uniform vec4 u_color;
void main() { frag_color = u_color; }
"#;

// ── Textured lit shaders ──────────────────────────────────────────────────────

/// Vertex shader for textured lit meshes.  Same layout as [`MESH_VERT`].
pub const TEXTURED_VERT: &str = r#"
#version 330 core
layout(location = 0) in vec3 a_pos;
layout(location = 1) in vec3 a_normal;
layout(location = 2) in vec2 a_uv;

uniform mat4 u_mvp;
uniform mat4 u_model;

out vec3 v_pos_world;
out vec3 v_normal_world;
out vec2 v_uv;

void main() {
    vec4 world_pos  = u_model * vec4(a_pos, 1.0);
    gl_Position     = u_mvp   * vec4(a_pos, 1.0);
    v_pos_world     = world_pos.xyz;
    v_normal_world  = mat3(transpose(inverse(u_model))) * a_normal;
    v_uv            = a_uv;
}
"#;

/// Textured fragment shader — samples albedo, then applies Blinn-Phong lighting.
///
/// `u_color` acts as a tint/multiplier on top of the texture sample.
pub const TEXTURED_FRAG: &str = r#"
#version 330 core
in  vec3 v_pos_world;
in  vec3 v_normal_world;
in  vec2 v_uv;

out vec4 frag_color;

uniform sampler2D u_texture;
uniform vec4      u_color;      // tint / multiplier
uniform vec3      u_light_dir;
uniform vec3      u_view_pos;
uniform float     u_ambient;

void main() {
    vec4 albedo = texture(u_texture, v_uv) * u_color;

    vec3 N = normalize(v_normal_world);
    vec3 L = normalize(u_light_dir);
    vec3 V = normalize(u_view_pos - v_pos_world);
    vec3 H = normalize(L + V);

    float amb  = max(u_ambient, 0.0);
    float diff = max(dot(N, L), 0.0);
    float spec = pow(max(dot(N, H), 0.0), 32.0) * (1.0 - amb);

    float light = clamp(amb + (1.0 - amb) * diff + 0.15 * spec, 0.0, 1.0);
    frag_color  = vec4(albedo.rgb * light, albedo.a);
}
"#;
