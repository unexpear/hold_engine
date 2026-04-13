use super::{Mat4, Vec3};
use std::ops::Mul;

/// Unit quaternion for rotation. Stored as (x, y, z, w).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Quat {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub w: f32,
}

impl Quat {
    pub const IDENTITY: Self = Self {
        x: 0.0,
        y: 0.0,
        z: 0.0,
        w: 1.0,
    };

    #[inline]
    pub fn new(x: f32, y: f32, z: f32, w: f32) -> Self {
        Self { x, y, z, w }
    }

    /// Build a rotation quaternion from an axis (need not be normalised) and angle in radians.
    pub fn from_axis_angle(axis: Vec3, angle_rad: f32) -> Self {
        let axis = axis.normalize();
        let half = angle_rad * 0.5;
        let (s, c) = half.sin_cos();
        Self {
            x: axis.x * s,
            y: axis.y * s,
            z: axis.z * s,
            w: c,
        }
    }

    /// Build from Euler angles (radians), applied in Z → Y → X order (extrinsic XYZ).
    pub fn from_euler_xyz(x: f32, y: f32, z: f32) -> Self {
        Self::from_axis_angle(Vec3::Z, z)
            * Self::from_axis_angle(Vec3::Y, y)
            * Self::from_axis_angle(Vec3::X, x)
    }

    /// Convert this quaternion into Euler angles in radians.
    ///
    /// Returns `(x, y, z)` matching [`Self::from_euler_xyz`].
    pub fn to_euler_xyz(self) -> Vec3 {
        let q = self.normalize();

        let sinr_cosp = 2.0 * (q.w * q.x + q.y * q.z);
        let cosr_cosp = 1.0 - 2.0 * (q.x * q.x + q.y * q.y);
        let x = sinr_cosp.atan2(cosr_cosp);

        let sinp = 2.0 * (q.w * q.y - q.z * q.x);
        let y = if sinp.abs() >= 1.0 {
            sinp.signum() * std::f32::consts::FRAC_PI_2
        } else {
            sinp.asin()
        };

        let siny_cosp = 2.0 * (q.w * q.z + q.x * q.y);
        let cosy_cosp = 1.0 - 2.0 * (q.y * q.y + q.z * q.z);
        let z = siny_cosp.atan2(cosy_cosp);

        Vec3::new(x, y, z)
    }

    #[inline]
    pub fn length_sq(&self) -> f32 {
        self.x * self.x + self.y * self.y + self.z * self.z + self.w * self.w
    }

    #[inline]
    pub fn normalize(self) -> Self {
        let len = self.length_sq().sqrt();
        if len > 1e-10 {
            let inv = 1.0 / len;
            Self {
                x: self.x * inv,
                y: self.y * inv,
                z: self.z * inv,
                w: self.w * inv,
            }
        } else {
            Self::IDENTITY
        }
    }

    #[inline]
    pub fn conjugate(self) -> Self {
        Self {
            x: -self.x,
            y: -self.y,
            z: -self.z,
            w: self.w,
        }
    }

    #[inline]
    pub fn dot(self, o: Self) -> f32 {
        self.x * o.x + self.y * o.y + self.z * o.z + self.w * o.w
    }

    /// Hamilton product.
    pub fn hamilton(self, rhs: Self) -> Self {
        Self {
            x: self.w * rhs.x + self.x * rhs.w + self.y * rhs.z - self.z * rhs.y,
            y: self.w * rhs.y - self.x * rhs.z + self.y * rhs.w + self.z * rhs.x,
            z: self.w * rhs.z + self.x * rhs.y - self.y * rhs.x + self.z * rhs.w,
            w: self.w * rhs.w - self.x * rhs.x - self.y * rhs.y - self.z * rhs.z,
        }
    }

    /// Rotate a vector by this quaternion: q * (0, v) * q*.
    pub fn rotate_vec3(self, v: Vec3) -> Vec3 {
        let qv = Self {
            x: v.x,
            y: v.y,
            z: v.z,
            w: 0.0,
        };
        let result = self.hamilton(qv).hamilton(self.conjugate());
        Vec3::new(result.x, result.y, result.z)
    }

    /// Build a 4x4 rotation matrix from this unit quaternion (standard formula).
    pub fn to_mat4(self) -> Mat4 {
        let Self { x, y, z, w } = self;
        let x2 = x + x;
        let y2 = y + y;
        let z2 = z + z;
        let xx = x * x2;
        let xy = x * y2;
        let xz = x * z2;
        let yy = y * y2;
        let yz = y * z2;
        let zz = z * z2;
        let wx = w * x2;
        let wy = w * y2;
        let wz = w * z2;

        Mat4 {
            cols: [
                [1.0 - (yy + zz), xy + wz, xz - wy, 0.0],
                [xy - wz, 1.0 - (xx + zz), yz + wx, 0.0],
                [xz + wy, yz - wx, 1.0 - (xx + yy), 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ],
        }
    }

    /// Spherical linear interpolation. Handles antipodal case (dot < 0).
    pub fn slerp(self, other: Self, t: f32) -> Self {
        let mut dot = self.dot(other);
        // If dot < 0, negate one quaternion to take the shorter arc.
        let other = if dot < 0.0 {
            dot = -dot;
            Self {
                x: -other.x,
                y: -other.y,
                z: -other.z,
                w: -other.w,
            }
        } else {
            other
        };

        if dot > 0.9995 {
            // Quaternions are very close — linear interpolation is safe.
            return Self {
                x: self.x + t * (other.x - self.x),
                y: self.y + t * (other.y - self.y),
                z: self.z + t * (other.z - self.z),
                w: self.w + t * (other.w - self.w),
            }
            .normalize();
        }

        let theta_0 = dot.acos();
        let theta = theta_0 * t;
        let sin_theta_0 = theta_0.sin();
        let s0 = (theta_0 - theta).sin() / sin_theta_0;
        let s1 = theta.sin() / sin_theta_0;

        Self {
            x: s0 * self.x + s1 * other.x,
            y: s0 * self.y + s1 * other.y,
            z: s0 * self.z + s1 * other.z,
            w: s0 * self.w + s1 * other.w,
        }
    }
}

impl Mul<Quat> for Quat {
    type Output = Self;
    fn mul(self, rhs: Self) -> Self {
        self.hamilton(rhs)
    }
}

impl std::fmt::Display for Quat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "({}, {}, {}, {})", self.x, self.y, self.z, self.w)
    }
}

#[cfg(test)]
mod tests {
    use super::Quat;

    #[test]
    fn euler_roundtrip_matches_from_euler_xyz() {
        let source = Quat::from_euler_xyz(0.35, -0.62, 0.48).normalize();
        let euler = source.to_euler_xyz();
        let roundtrip = Quat::from_euler_xyz(euler.x, euler.y, euler.z).normalize();

        assert!(
            source.dot(roundtrip).abs() > 0.9999,
            "quat roundtrip drifted too far"
        );
    }
}
