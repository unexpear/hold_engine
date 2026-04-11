//! Procedural cube-mapped skybox rendered per-pixel.
//!
//! For each pixel the renderer casts a view-direction ray, picks the dominant
//! cube-map face, and evaluates a procedural colour function.  No texture
//! images are needed -- the sky, sun disc, horizon haze and ground are all
//! computed analytically.

use crate::math::{Mat4, Vec3};
use crate::renderer::software::SoftwareRenderer;
use crate::renderer::{Color, Renderer};

/// Procedural skybox that writes directly into the software framebuffer.
pub struct Skybox {
    /// Normalised direction to the sun (world-space).
    pub sun_dir: Vec3,
    /// Zenith (overhead) colour.
    pub zenith: Color,
    /// Horizon colour.
    pub horizon: Color,
    /// Ground (below-horizon) colour.
    pub ground: Color,
    /// Sun disc angular radius in radians.
    pub sun_radius: f32,
    /// Sun disc colour.
    pub sun_color: Color,
}

impl Skybox {
    /// Sensible daytime defaults.
    pub fn day(sun_dir: Vec3) -> Self {
        Self {
            sun_dir: sun_dir.normalize(),
            zenith: Color::rgb(0.20, 0.35, 0.85),
            horizon: Color::rgb(0.55, 0.70, 0.95),
            ground: Color::rgb(0.25, 0.22, 0.20),
            sun_radius: 0.04,
            sun_color: Color::rgb(1.0, 0.95, 0.8),
        }
    }

    /// Render the skybox into the software renderer's framebuffer.
    ///
    /// Call after clear and before any geometry so that
    /// the depth buffer is still at +inf and geometry will overdraw the sky.
    pub fn draw(&self, renderer: &mut SoftwareRenderer, view: &Mat4, proj: &Mat4) {
        let w = renderer.width() as usize;
        let h = renderer.height() as usize;

        // Build view-rotation-only (strip translation).
        let mut rot_view = *view;
        rot_view.cols[3][0] = 0.0;
        rot_view.cols[3][1] = 0.0;
        rot_view.cols[3][2] = 0.0;
        rot_view.cols[3][3] = 1.0;
        // Zero any non-zero in the w-row of cols 0..2 (shouldn't be, but safe).
        rot_view.cols[0][3] = 0.0;
        rot_view.cols[1][3] = 0.0;
        rot_view.cols[2][3] = 0.0;

        let vp = *proj * rot_view;
        let inv = mat4_inverse(&vp);

        let fw = w as f32;
        let fh = h as f32;

        for py in 0..h {
            for px in 0..w {
                let ndc_x = (px as f32 + 0.5) / fw * 2.0 - 1.0;
                let ndc_y = (py as f32 + 0.5) / fh * -2.0 + 1.0;

                let dir = unproject(&inv, ndc_x, ndc_y);
                let color = self.sample(dir);
                renderer.draw_pixel(px as i32, py as i32, color);
            }
        }
    }

    /// Sample the procedural sky for a given world-space direction.
    fn sample(&self, dir: Vec3) -> Color {
        let up = dir.y.clamp(-1.0, 1.0);
        let sky = if up >= 0.0 {
            let t = 1.0 - up;
            let t = t * t;
            self.zenith.lerp(self.horizon, t)
        } else {
            let t = (-up).min(1.0);
            self.horizon.lerp(self.ground, t)
        };

        // Sun disc.
        let cos_angle = dir.dot(self.sun_dir).clamp(-1.0, 1.0);
        let angle = cos_angle.acos();
        if angle < self.sun_radius {
            let rim = angle / self.sun_radius;
            let blend = 1.0 - rim * rim;
            return sky.lerp(self.sun_color, blend.clamp(0.0, 1.0));
        }

        // Atmospheric glow around the sun.
        let glow = (cos_angle.max(0.0)).powf(128.0) * 0.6;
        if glow > 0.001 {
            return sky.lerp(self.sun_color, glow.min(1.0));
        }

        sky
    }
}

// ---------------------------------------------------------------------------

/// Unproject an NDC pixel into a normalised world-space direction.
fn unproject(inv_vp: &Mat4, ndc_x: f32, ndc_y: f32) -> Vec3 {
    let c = &inv_vp.cols;
    // Multiply inv_vp * (ndc_x, ndc_y, 1.0, 1.0)
    let x = c[0][0] * ndc_x + c[1][0] * ndc_y + c[2][0] + c[3][0];
    let y = c[0][1] * ndc_x + c[1][1] * ndc_y + c[2][1] + c[3][1];
    let z = c[0][2] * ndc_x + c[1][2] * ndc_y + c[2][2] + c[3][2];
    let w = c[0][3] * ndc_x + c[1][3] * ndc_y + c[2][3] + c[3][3];
    let inv_w = if w.abs() > 1e-12 { 1.0 / w } else { 1.0 };
    Vec3::new(x * inv_w, y * inv_w, z * inv_w).normalize()
}

/// Full 4x4 matrix inverse via cofactors.
/// Input uses column-major cols[col][row]; we flatten to row-major for the
/// cofactor algebra and then pack back.
fn mat4_inverse(m: &Mat4) -> Mat4 {
    // Flatten to row-major s[row*4+col].
    let c = &m.cols;
    let s: [f32; 16] = [
        c[0][0], c[1][0], c[2][0], c[3][0], c[0][1], c[1][1], c[2][1], c[3][1], c[0][2], c[1][2],
        c[2][2], c[3][2], c[0][3], c[1][3], c[2][3], c[3][3],
    ];

    let mut inv = [0.0_f32; 16];

    inv[0] = s[5] * s[10] * s[15] - s[5] * s[11] * s[14] - s[9] * s[6] * s[15]
        + s[9] * s[7] * s[14]
        + s[13] * s[6] * s[11]
        - s[13] * s[7] * s[10];
    inv[1] = -s[1] * s[10] * s[15] + s[1] * s[11] * s[14] + s[9] * s[2] * s[15]
        - s[9] * s[3] * s[14]
        - s[13] * s[2] * s[11]
        + s[13] * s[3] * s[10];
    inv[2] = s[1] * s[6] * s[15] - s[1] * s[7] * s[14] - s[5] * s[2] * s[15]
        + s[5] * s[3] * s[14]
        + s[13] * s[2] * s[7]
        - s[13] * s[3] * s[6];
    inv[3] = -s[1] * s[6] * s[11] + s[1] * s[7] * s[10] + s[5] * s[2] * s[11]
        - s[5] * s[3] * s[10]
        - s[9] * s[2] * s[7]
        + s[9] * s[3] * s[6];

    inv[4] = -s[4] * s[10] * s[15] + s[4] * s[11] * s[14] + s[8] * s[6] * s[15]
        - s[8] * s[7] * s[14]
        - s[12] * s[6] * s[11]
        + s[12] * s[7] * s[10];
    inv[5] = s[0] * s[10] * s[15] - s[0] * s[11] * s[14] - s[8] * s[2] * s[15]
        + s[8] * s[3] * s[14]
        + s[12] * s[2] * s[11]
        - s[12] * s[3] * s[10];
    inv[6] = -s[0] * s[6] * s[15] + s[0] * s[7] * s[14] + s[4] * s[2] * s[15]
        - s[4] * s[3] * s[14]
        - s[12] * s[2] * s[7]
        + s[12] * s[3] * s[6];
    inv[7] = s[0] * s[6] * s[11] - s[0] * s[7] * s[10] - s[4] * s[2] * s[11]
        + s[4] * s[3] * s[10]
        + s[8] * s[2] * s[7]
        - s[8] * s[3] * s[6];

    inv[8] = s[4] * s[9] * s[15] - s[4] * s[11] * s[13] - s[8] * s[5] * s[15]
        + s[8] * s[7] * s[13]
        + s[12] * s[5] * s[11]
        - s[12] * s[7] * s[9];
    inv[9] = -s[0] * s[9] * s[15] + s[0] * s[11] * s[13] + s[8] * s[1] * s[15]
        - s[8] * s[3] * s[13]
        - s[12] * s[1] * s[11]
        + s[12] * s[3] * s[9];
    inv[10] = s[0] * s[5] * s[15] - s[0] * s[7] * s[13] - s[4] * s[1] * s[15]
        + s[4] * s[3] * s[13]
        + s[12] * s[1] * s[7]
        - s[12] * s[3] * s[5];
    inv[11] = -s[0] * s[5] * s[11] + s[0] * s[7] * s[9] + s[4] * s[1] * s[11]
        - s[4] * s[3] * s[9]
        - s[8] * s[1] * s[7]
        + s[8] * s[3] * s[5];

    inv[12] = -s[4] * s[9] * s[14] + s[4] * s[10] * s[13] + s[8] * s[5] * s[14]
        - s[8] * s[6] * s[13]
        - s[12] * s[5] * s[10]
        + s[12] * s[6] * s[9];
    inv[13] = s[0] * s[9] * s[14] - s[0] * s[10] * s[13] - s[8] * s[1] * s[14]
        + s[8] * s[2] * s[13]
        + s[12] * s[1] * s[10]
        - s[12] * s[2] * s[9];
    inv[14] = -s[0] * s[5] * s[14] + s[0] * s[6] * s[13] + s[4] * s[1] * s[14]
        - s[4] * s[2] * s[13]
        - s[12] * s[1] * s[6]
        + s[12] * s[2] * s[5];
    inv[15] = s[0] * s[5] * s[10] - s[0] * s[6] * s[9] - s[4] * s[1] * s[10]
        + s[4] * s[2] * s[9]
        + s[8] * s[1] * s[6]
        - s[8] * s[2] * s[5];

    let det = s[0] * inv[0] + s[1] * inv[4] + s[2] * inv[8] + s[3] * inv[12];
    if det.abs() < 1e-12 {
        return Mat4::IDENTITY;
    }
    let inv_det = 1.0 / det;
    for v in &mut inv {
        *v *= inv_det;
    }

    // Pack back to column-major.
    Mat4 {
        cols: [
            [inv[0], inv[4], inv[8], inv[12]],
            [inv[1], inv[5], inv[9], inv[13]],
            [inv[2], inv[6], inv[10], inv[14]],
            [inv[3], inv[7], inv[11], inv[15]],
        ],
    }
}
