use super::Vec3;
use std::ops::{Add, Div, Mul, Neg, Sub};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Vec4 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub w: f32,
}

impl Vec4 {
    pub const ZERO: Self = Self {
        x: 0.0,
        y: 0.0,
        z: 0.0,
        w: 0.0,
    };
    pub const ONE: Self = Self {
        x: 1.0,
        y: 1.0,
        z: 1.0,
        w: 1.0,
    };

    #[inline]
    pub fn new(x: f32, y: f32, z: f32, w: f32) -> Self {
        Self { x, y, z, w }
    }
    #[inline]
    pub fn from_vec3(v: Vec3, w: f32) -> Self {
        Self::new(v.x, v.y, v.z, w)
    }
    #[inline]
    pub fn truncate(self) -> Vec3 {
        Vec3::new(self.x, self.y, self.z)
    }

    #[inline]
    pub fn dot(self, o: Self) -> f32 {
        self.x * o.x + self.y * o.y + self.z * o.z + self.w * o.w
    }
    #[inline]
    pub fn length_sq(self) -> f32 {
        self.dot(self)
    }
    #[inline]
    pub fn length(self) -> f32 {
        self.length_sq().sqrt()
    }
    #[inline]
    pub fn normalize(self) -> Self {
        let len = self.length();
        if len > 1e-10 {
            self * (1.0 / len)
        } else {
            Self::ZERO
        }
    }
    /// Perspective divide: divide xyz by w to get NDC coordinates.
    #[inline]
    pub fn perspective_divide(self) -> Vec3 {
        Vec3::new(self.x / self.w, self.y / self.w, self.z / self.w)
    }
}

impl Add for Vec4 {
    type Output = Self;
    fn add(self, r: Self) -> Self {
        Self::new(self.x + r.x, self.y + r.y, self.z + r.z, self.w + r.w)
    }
}
impl Sub for Vec4 {
    type Output = Self;
    fn sub(self, r: Self) -> Self {
        Self::new(self.x - r.x, self.y - r.y, self.z - r.z, self.w - r.w)
    }
}
impl Mul<f32> for Vec4 {
    type Output = Self;
    fn mul(self, s: f32) -> Self {
        Self::new(self.x * s, self.y * s, self.z * s, self.w * s)
    }
}
impl Mul<Vec4> for f32 {
    type Output = Vec4;
    fn mul(self, v: Vec4) -> Vec4 {
        v * self
    }
}
impl Div<f32> for Vec4 {
    type Output = Self;
    fn div(self, s: f32) -> Self {
        self * (1.0 / s)
    }
}
impl Neg for Vec4 {
    type Output = Self;
    fn neg(self) -> Self {
        Self::new(-self.x, -self.y, -self.z, -self.w)
    }
}

impl std::fmt::Display for Vec4 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "({:.3}, {:.3}, {:.3}, {:.3})",
            self.x, self.y, self.z, self.w
        )
    }
}
