mod mat4;
mod quat;
mod vec2;
mod vec3;
mod vec4;

pub use mat4::Mat4;
pub use quat::Quat;
pub use vec2::Vec2;
pub use vec3::Vec3;
pub use vec4::Vec4;

pub const PI: f32 = std::f32::consts::PI;
pub const TAU: f32 = std::f32::consts::TAU;

#[inline]
pub fn deg_to_rad(deg: f32) -> f32 {
    deg * (PI / 180.0)
}
#[inline]
pub fn rad_to_deg(rad: f32) -> f32 {
    rad * (180.0 / PI)
}
#[inline]
pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}
#[inline]
pub fn clamp(v: f32, lo: f32, hi: f32) -> f32 {
    v.clamp(lo, hi)
}
