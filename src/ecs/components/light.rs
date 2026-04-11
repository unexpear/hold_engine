use crate::math::Vec3;
use crate::renderer::Color;

/// A directional light (e.g. the sun).
///
/// The direction vector points toward the light source (the opposite
/// of the direction rays travel). This matches the GLSL u_light_dir convention.
pub struct DirectionalLight {
    /// Direction toward the light (normalised).
    pub direction: Vec3,
    /// Light colour.
    pub color: Color,
    /// Diffuse intensity multiplier (0.0 = off, 1.0 = full).
    pub intensity: f32,
    /// Ambient floor -- minimum brightness even in full shadow (0.0 .. 1.0).
    pub ambient: f32,
}

impl DirectionalLight {
    /// Convenient constructor that normalises the direction for you.
    pub fn sun(direction: Vec3, color: Color) -> Self {
        Self {
            direction: direction.normalize(),
            color,
            intensity: 1.0,
            ambient: 0.15,
        }
    }

    /// Compute the lit colour for a surface with the given normal and base colour.
    ///
    /// Uses Lambert (N dot L) diffuse plus a constant ambient term.
    /// The result is clamped to [0, 1] per channel.
    #[inline]
    pub fn shade(&self, normal: Vec3, base_color: Color) -> Color {
        let n_dot_l = normal.dot(self.direction).max(0.0);
        let diffuse = n_dot_l * self.intensity;
        let total = (self.ambient + diffuse).min(1.0);
        Color {
            r: (base_color.r * self.color.r * total).min(1.0),
            g: (base_color.g * self.color.g * total).min(1.0),
            b: (base_color.b * self.color.b * total).min(1.0),
            a: base_color.a,
        }
    }
}
