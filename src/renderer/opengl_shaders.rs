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

// ── Debug-line shaders ────────────────────────────────────────────────────────

/// Vertex shader for batched debug line segments.
///
/// Vertex layout: position (vec3) at location 0, per-vertex RGBA (vec4) at
/// location 1.  No normals or UVs — lines are unlit and coloured per vertex.
pub const LINE_VERT: &str = r#"
#version 330 core
layout(location = 0) in vec3 a_pos;
layout(location = 1) in vec4 a_color;
uniform mat4 u_vp;
out vec4 v_color;
void main() {
    gl_Position = u_vp * vec4(a_pos, 1.0);
    v_color = a_color;
}
"#;

/// Fragment shader for batched debug line segments.
pub const LINE_FRAG: &str = r#"
#version 330 core
in  vec4 v_color;
out vec4 frag_color;
void main() { frag_color = v_color; }
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

/// Textured fragment shader — Disney metallic-roughness PBR lighting,
/// with one directional light and an ACES tonemap before output.
///
/// The albedo sampler is bound to a texture uploaded with
/// `GL_SRGB8_ALPHA8`, so the value returned by `texture()` is already
/// linear (the GPU did the sRGB→linear decode at sample time, ADR
/// 0002 Q3). All BRDF math runs in linear space; the final ACES
/// tonemap returns linear-mapped values that the framebuffer's sRGB
/// encode (via `GL_FRAMEBUFFER_SRGB`) writes out display-correct.
///
/// `u_color` is a linear-space tint applied to base colour.
/// `u_metallic` and `u_roughness` are scalar PBR factors in `[0, 1]`.
pub const TEXTURED_FRAG: &str = r#"
#version 330 core
in  vec3 v_pos_world;
in  vec3 v_normal_world;
in  vec2 v_uv;

out vec4 frag_color;

uniform sampler2D u_texture;
uniform vec4      u_color;       // linear tint (multiplied into base colour)
uniform vec3      u_light_dir;   // direction TOWARD the light, world space
uniform vec3      u_view_pos;
uniform float     u_ambient;
uniform float     u_metallic;    // [0, 1]
uniform float     u_roughness;   // [0, 1]

const float PI = 3.14159265358979;

// --- Disney metallic-roughness BRDF helpers ---

// GGX / Trowbridge-Reitz normal distribution. `alpha = roughness^2`.
float distribution_ggx(vec3 N, vec3 H, float alpha) {
    float a2 = alpha * alpha;
    float n_dot_h = max(dot(N, H), 0.0);
    float d = (n_dot_h * n_dot_h) * (a2 - 1.0) + 1.0;
    return a2 / (PI * d * d);
}

// Smith / Schlick-GGX geometry term, height-correlated approximation.
float geometry_schlick_ggx(float n_dot_v, float k) {
    return n_dot_v / (n_dot_v * (1.0 - k) + k);
}
float geometry_smith(vec3 N, vec3 V, vec3 L, float alpha) {
    // Direct-lighting remap: k = (alpha + 1)^2 / 8.
    float k = (alpha + 1.0) * (alpha + 1.0) / 8.0;
    float n_dot_v = max(dot(N, V), 0.0);
    float n_dot_l = max(dot(N, L), 0.0);
    return geometry_schlick_ggx(n_dot_v, k) * geometry_schlick_ggx(n_dot_l, k);
}

// Schlick approximation of the Fresnel term.
vec3 fresnel_schlick(float cos_theta, vec3 f0) {
    return f0 + (vec3(1.0) - f0) * pow(clamp(1.0 - cos_theta, 0.0, 1.0), 5.0);
}

// Narkowicz ACES tonemap fit. Compresses HDR linear input to LDR linear
// output ready for the framebuffer's sRGB encode. See ADR 0002 Q3.
vec3 aces_tonemap(vec3 x) {
    const float a = 2.51;
    const float b = 0.03;
    const float c = 2.43;
    const float d = 0.59;
    const float e = 0.14;
    return clamp((x * (a * x + b)) / (x * (c * x + d) + e), 0.0, 1.0);
}

void main() {
    // GPU-decoded sRGB → linear; multiply by the linear tint.
    vec4 albedo_sample = texture(u_texture, v_uv);
    vec3 base_color = albedo_sample.rgb * u_color.rgb;
    float alpha_out = albedo_sample.a * u_color.a;

    vec3 N = normalize(v_normal_world);
    vec3 L = normalize(u_light_dir);
    vec3 V = normalize(u_view_pos - v_pos_world);
    vec3 H = normalize(L + V);

    float n_dot_l = max(dot(N, L), 0.0);

    // Disney convention: F0 is 0.04 for dielectrics, base_color for metals.
    vec3 f0 = mix(vec3(0.04), base_color, u_metallic);

    float alpha = u_roughness * u_roughness;

    float D = distribution_ggx(N, H, alpha);
    float G = geometry_smith(N, V, L, alpha);
    vec3  F = fresnel_schlick(max(dot(H, V), 0.0), f0);

    // Cook-Torrance specular.
    float n_dot_v = max(dot(N, V), 0.0);
    vec3 specular = (D * G) * F / max(4.0 * n_dot_v * n_dot_l, 1e-4);

    // Diffuse term: energy-conserved Lambertian. Metals have no
    // diffuse contribution, so kD scales by (1 - metallic).
    vec3 kD = (vec3(1.0) - F) * (1.0 - u_metallic);
    vec3 diffuse = kD * base_color / PI;

    // Single directional light contribution.
    vec3 direct = (diffuse + specular) * n_dot_l;

    // Cheap ambient: linear in base_color, scaled by the ambient
    // factor. Not physically based — replaces with image-based
    // lighting when M5.5 lands. Documented limitation; ADR 0002 Q3.
    vec3 ambient = u_ambient * base_color;

    vec3 hdr_linear = ambient + direct;
    vec3 ldr_linear = aces_tonemap(hdr_linear);

    frag_color = vec4(ldr_linear, alpha_out);
}
"#;

// ── Textured + normal-mapped lit shaders ──────────────────────────────────────
//
// The first reference shader pair to consume the location-3 tangent attribute
// (see [`crate::assets::mesh::Vertex`]) and a typed [`crate::assets::NormalMap`]
// asset. The fragment shader reconstructs a per-pixel TBN matrix from the
// interpolated vertex normal + tangent, samples a tangent-space normal map,
// and uses the perturbed world-space normal in Blinn-Phong lighting.
//
// Bitangent is reconstructed in the fragment shader as
// `cross(N, T) * v_tangent_w` so that the handedness sign packed into
// `tangent.w` (per the MikkTSpace convention; see ADR 0002 Q4) is honoured.
// This avoids paying a 3-float varying for the bitangent and keeps the math
// numerically stable through the perspective-correct interpolation.
//
// Sampler bindings:
//   - `u_texture`    → albedo, texture unit 0
//   - `u_normal_map` → normal map, texture unit 1

/// Vertex shader for textured + normal-mapped lit meshes.
///
/// Vertex layout (matches [`crate::assets::mesh::Vertex`]):
/// - location 0 — position  (vec3)
/// - location 1 — normal    (vec3)
/// - location 2 — UV coords (vec2)
/// - location 3 — tangent   (vec4 = xyz tangent + w handedness sign)
pub const TEXTURED_NORMAL_VERT: &str = r#"
#version 330 core
layout(location = 0) in vec3 a_pos;
layout(location = 1) in vec3 a_normal;
layout(location = 2) in vec2 a_uv;
layout(location = 3) in vec4 a_tangent;

uniform mat4 u_mvp;
uniform mat4 u_model;

out vec3 v_pos_world;
out vec3 v_normal_world;
out vec2 v_uv;
out vec3 v_tangent_world;
out float v_tangent_w;

void main() {
    vec4 world_pos  = u_model * vec4(a_pos, 1.0);
    gl_Position     = u_mvp   * vec4(a_pos, 1.0);
    v_pos_world     = world_pos.xyz;

    // Normal matrix: transpose of the inverse of the upper-3x3 of u_model.
    // Handles non-uniform scale correctly.
    mat3 normal_matrix = mat3(transpose(inverse(u_model)));
    v_normal_world  = normal_matrix * a_normal;
    // Tangent transforms as a direction by the same normal matrix.
    v_tangent_world = normal_matrix * a_tangent.xyz;
    v_tangent_w     = a_tangent.w;
    v_uv            = a_uv;
}
"#;

/// Fragment shader for textured + normal-mapped Disney metallic-roughness
/// PBR meshes. Same shading model as [`TEXTURED_FRAG`]; the difference is
/// per-pixel normal perturbation from a tangent-space normal map.
///
/// The normal map sampler is bound to a [`crate::assets::NormalMap`]
/// asset uploaded with `GL_RGBA` (linear, NOT sRGB — normal maps encode
/// vectors, not display colours; see ADR 0002 Q4 + Q3). The TBN matrix
/// is reconstructed per-pixel from the interpolated vertex normal +
/// tangent + handedness sign, so the shader works correctly across
/// mirrored UV islands.
pub const TEXTURED_NORMAL_FRAG: &str = r#"
#version 330 core
in  vec3 v_pos_world;
in  vec3 v_normal_world;
in  vec2 v_uv;
in  vec3 v_tangent_world;
in  float v_tangent_w;

out vec4 frag_color;

uniform sampler2D u_texture;     // albedo, texture unit 0
uniform sampler2D u_normal_map;  // tangent-space normal map, texture unit 1
uniform vec4      u_color;
uniform vec3      u_light_dir;
uniform vec3      u_view_pos;
uniform float     u_ambient;
uniform float     u_metallic;
uniform float     u_roughness;

const float PI = 3.14159265358979;

float distribution_ggx(vec3 N, vec3 H, float alpha) {
    float a2 = alpha * alpha;
    float n_dot_h = max(dot(N, H), 0.0);
    float d = (n_dot_h * n_dot_h) * (a2 - 1.0) + 1.0;
    return a2 / (PI * d * d);
}

float geometry_schlick_ggx(float n_dot_v, float k) {
    return n_dot_v / (n_dot_v * (1.0 - k) + k);
}
float geometry_smith(vec3 N, vec3 V, vec3 L, float alpha) {
    float k = (alpha + 1.0) * (alpha + 1.0) / 8.0;
    float n_dot_v = max(dot(N, V), 0.0);
    float n_dot_l = max(dot(N, L), 0.0);
    return geometry_schlick_ggx(n_dot_v, k) * geometry_schlick_ggx(n_dot_l, k);
}

vec3 fresnel_schlick(float cos_theta, vec3 f0) {
    return f0 + (vec3(1.0) - f0) * pow(clamp(1.0 - cos_theta, 0.0, 1.0), 5.0);
}

vec3 aces_tonemap(vec3 x) {
    const float a = 2.51;
    const float b = 0.03;
    const float c = 2.43;
    const float d = 0.59;
    const float e = 0.14;
    return clamp((x * (a * x + b)) / (x * (c * x + d) + e), 0.0, 1.0);
}

void main() {
    vec4 albedo_sample = texture(u_texture, v_uv);
    vec3 base_color = albedo_sample.rgb * u_color.rgb;
    float alpha_out = albedo_sample.a * u_color.a;

    // Re-orthonormalise the interpolated TBN at the fragment.
    vec3 N_geometric = normalize(v_normal_world);
    vec3 T = normalize(v_tangent_world);
    T = normalize(T - dot(T, N_geometric) * N_geometric);
    vec3 B = cross(N_geometric, T) * v_tangent_w;

    // Sample, unpack from [0,1] to [-1,1], transform tangent → world.
    vec3 n_tan = texture(u_normal_map, v_uv).xyz * 2.0 - 1.0;
    vec3 N = normalize(T * n_tan.x + B * n_tan.y + N_geometric * n_tan.z);

    vec3 L = normalize(u_light_dir);
    vec3 V = normalize(u_view_pos - v_pos_world);
    vec3 H = normalize(L + V);

    float n_dot_l = max(dot(N, L), 0.0);

    vec3 f0 = mix(vec3(0.04), base_color, u_metallic);
    float alpha = u_roughness * u_roughness;

    float D = distribution_ggx(N, H, alpha);
    float G = geometry_smith(N, V, L, alpha);
    vec3  F = fresnel_schlick(max(dot(H, V), 0.0), f0);

    float n_dot_v = max(dot(N, V), 0.0);
    vec3 specular = (D * G) * F / max(4.0 * n_dot_v * n_dot_l, 1e-4);

    vec3 kD = (vec3(1.0) - F) * (1.0 - u_metallic);
    vec3 diffuse = kD * base_color / PI;

    vec3 direct = (diffuse + specular) * n_dot_l;
    vec3 ambient = u_ambient * base_color;

    vec3 hdr_linear = ambient + direct;
    vec3 ldr_linear = aces_tonemap(hdr_linear);

    frag_color = vec4(ldr_linear, alpha_out);
}
"#;
