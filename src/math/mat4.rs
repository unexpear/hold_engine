use std::ops::Mul;
use super::{Vec3, Vec4};

/// Column-major 4x4 matrix. `cols[c][r]` = element at column c, row r.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Mat4 {
    pub cols: [[f32; 4]; 4],
}

impl Mat4 {
    pub const IDENTITY: Self = Self {
        cols: [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ],
    };

    pub const ZERO: Self = Self { cols: [[0.0; 4]; 4] };

    #[inline] pub fn identity() -> Self { Self::IDENTITY }
    #[inline] pub fn zero() -> Self { Self::ZERO }

    // ── Transforms ──────────────────────────────────────────────────────────

    pub fn translate(v: Vec3) -> Self {
        let mut m = Self::identity();
        m.cols[3][0] = v.x;
        m.cols[3][1] = v.y;
        m.cols[3][2] = v.z;
        m
    }

    pub fn scale(v: Vec3) -> Self {
        let mut m = Self::zero();
        m.cols[0][0] = v.x;
        m.cols[1][1] = v.y;
        m.cols[2][2] = v.z;
        m.cols[3][3] = 1.0;
        m
    }

    pub fn scale_uniform(s: f32) -> Self { Self::scale(Vec3::splat(s)) }

    pub fn rotate_x(angle: f32) -> Self {
        let (s, c) = angle.sin_cos();
        let mut m = Self::identity();
        m.cols[1][1] = c;  m.cols[2][1] = -s;
        m.cols[1][2] = s;  m.cols[2][2] =  c;
        m
    }

    pub fn rotate_y(angle: f32) -> Self {
        let (s, c) = angle.sin_cos();
        let mut m = Self::identity();
        m.cols[0][0] =  c;  m.cols[2][0] = s;
        m.cols[0][2] = -s;  m.cols[2][2] = c;
        m
    }

    pub fn rotate_z(angle: f32) -> Self {
        let (s, c) = angle.sin_cos();
        let mut m = Self::identity();
        m.cols[0][0] = c;  m.cols[1][0] = -s;
        m.cols[0][1] = s;  m.cols[1][1] =  c;
        m
    }

    /// Rotate around an arbitrary axis (must be normalised).
    pub fn rotate_axis(axis: Vec3, angle: f32) -> Self {
        let (s, c) = angle.sin_cos();
        let t = 1.0 - c;
        let Vec3 { x, y, z } = axis;
        let mut m = Self::identity();
        m.cols[0][0] = t*x*x + c;    m.cols[1][0] = t*x*y - s*z; m.cols[2][0] = t*x*z + s*y;
        m.cols[0][1] = t*x*y + s*z;  m.cols[1][1] = t*y*y + c;   m.cols[2][1] = t*y*z - s*x;
        m.cols[0][2] = t*x*z - s*y;  m.cols[1][2] = t*y*z + s*x; m.cols[2][2] = t*z*z + c;
        m
    }

    // ── View / Projection ────────────────────────────────────────────────────

    /// Right-handed look-at view matrix.
    pub fn look_at(eye: Vec3, center: Vec3, up: Vec3) -> Self {
        let f = (center - eye).normalize();
        let s = f.cross(up).normalize();
        let u = s.cross(f);
        let mut m = Self::identity();
        m.cols[0][0] = s.x;   m.cols[1][0] = s.y;   m.cols[2][0] = s.z;
        m.cols[0][1] = u.x;   m.cols[1][1] = u.y;   m.cols[2][1] = u.z;
        m.cols[0][2] = -f.x;  m.cols[1][2] = -f.y;  m.cols[2][2] = -f.z;
        m.cols[3][0] = -s.dot(eye);
        m.cols[3][1] = -u.dot(eye);
        m.cols[3][2] =  f.dot(eye);
        m
    }

    /// Symmetric perspective projection (OpenGL convention, depth -1..1).
    pub fn perspective_gl(fov_y: f32, aspect: f32, near: f32, far: f32) -> Self {
        let inv_tan = 1.0 / (fov_y * 0.5).tan();
        let mut m = Self::zero();
        m.cols[0][0] = inv_tan / aspect;
        m.cols[1][1] = inv_tan;
        m.cols[2][2] = -(far + near) / (far - near);
        m.cols[2][3] = -1.0;
        m.cols[3][2] = -(2.0 * far * near) / (far - near);
        m
    }

    /// Symmetric perspective projection (Vulkan/DX convention, depth 0..1).
    pub fn perspective_vk(fov_y: f32, aspect: f32, near: f32, far: f32) -> Self {
        let inv_tan = 1.0 / (fov_y * 0.5).tan();
        let mut m = Self::zero();
        m.cols[0][0] = inv_tan / aspect;
        m.cols[1][1] = -inv_tan; // flip Y for Vulkan
        m.cols[2][2] = far / (near - far);
        m.cols[2][3] = -1.0;
        m.cols[3][2] = -(far * near) / (far - near);
        m
    }

    /// Orthographic projection (OpenGL convention).
    pub fn ortho(left: f32, right: f32, bottom: f32, top: f32, near: f32, far: f32) -> Self {
        let mut m = Self::identity();
        m.cols[0][0] = 2.0 / (right - left);
        m.cols[1][1] = 2.0 / (top - bottom);
        m.cols[2][2] = -2.0 / (far - near);
        m.cols[3][0] = -(right + left) / (right - left);
        m.cols[3][1] = -(top + bottom) / (top - bottom);
        m.cols[3][2] = -(far + near) / (far - near);
        m
    }

    // ── Vector operations ─────────────────────────────────────────────────────

    pub fn mul_vec4(&self, v: Vec4) -> Vec4 {
        let c = &self.cols;
        Vec4::new(
            c[0][0]*v.x + c[1][0]*v.y + c[2][0]*v.z + c[3][0]*v.w,
            c[0][1]*v.x + c[1][1]*v.y + c[2][1]*v.z + c[3][1]*v.w,
            c[0][2]*v.x + c[1][2]*v.y + c[2][2]*v.z + c[3][2]*v.w,
            c[0][3]*v.x + c[1][3]*v.y + c[2][3]*v.z + c[3][3]*v.w,
        )
    }

    /// Transform a point (w=1): applies translation.
    #[inline] pub fn transform_point(&self, v: Vec3) -> Vec4 {
        self.mul_vec4(Vec4::from_vec3(v, 1.0))
    }

    /// Transform a direction (w=0): ignores translation.
    #[inline] pub fn transform_dir(&self, v: Vec3) -> Vec3 {
        self.mul_vec4(Vec4::from_vec3(v, 0.0)).truncate()
    }

    /// Inverse for rigid-body matrices (rotation + translation only, no scale).
    /// O(1) and numerically stable — use this for camera view matrices.
    pub fn inverse_rigid(&self) -> Self {
        let tx = self.cols[3][0];
        let ty = self.cols[3][1];
        let tz = self.cols[3][2];
        Self {
            cols: [
                [self.cols[0][0], self.cols[1][0], self.cols[2][0], 0.0],
                [self.cols[0][1], self.cols[1][1], self.cols[2][1], 0.0],
                [self.cols[0][2], self.cols[1][2], self.cols[2][2], 0.0],
                [
                    -(self.cols[0][0]*tx + self.cols[1][0]*ty + self.cols[2][0]*tz),
                    -(self.cols[0][1]*tx + self.cols[1][1]*ty + self.cols[2][1]*tz),
                    -(self.cols[0][2]*tx + self.cols[1][2]*ty + self.cols[2][2]*tz),
                    1.0,
                ],
            ],
        }
    }

    /// Extract the translation component of a TRS matrix.
    #[inline]
    pub fn extract_position(&self) -> Vec3 {
        Vec3::new(self.cols[3][0], self.cols[3][1], self.cols[3][2])
    }

    pub fn transpose(&self) -> Self {
        let c = &self.cols;
        Self {
            cols: [
                [c[0][0], c[1][0], c[2][0], c[3][0]],
                [c[0][1], c[1][1], c[2][1], c[3][1]],
                [c[0][2], c[1][2], c[2][2], c[3][2]],
                [c[0][3], c[1][3], c[2][3], c[3][3]],
            ],
        }
    }
}

impl Mul for Mat4 {
    type Output = Self;
    fn mul(self, rhs: Self) -> Self {
        let mut out = Self::zero();
        for col in 0..4 {
            for row in 0..4 {
                out.cols[col][row] = (0..4).map(|k| self.cols[k][row] * rhs.cols[col][k]).sum();
            }
        }
        out
    }
}

impl std::fmt::Display for Mat4 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for row in 0..4 {
            write!(f, "[ ")?;
            for col in 0..4 {
                write!(f, "{:8.3} ", self.cols[col][row])?;
            }
            writeln!(f, "]")?;
        }
        Ok(())
    }
}
