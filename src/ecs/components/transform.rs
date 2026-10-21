use crate::math::{Mat4, Quat, Vec3};

pub struct Transform {
    pub position: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
}

impl Transform {
    pub const IDENTITY: Self = Self {
        position: Vec3::ZERO,
        rotation: Quat::IDENTITY,
        scale: Vec3::ONE,
    };

    pub fn from_position(pos: Vec3) -> Self {
        Self {
            position: pos,
            ..Self::IDENTITY
        }
    }

    pub fn from_rotation(rot: Quat) -> Self {
        Self {
            rotation: rot,
            ..Self::IDENTITY
        }
    }

    /// Compute the local TRS matrix: T * R * S.
    pub fn local_matrix(&self) -> Mat4 {
        Mat4::translate(self.position) * self.rotation.to_mat4() * Mat4::scale(self.scale)
    }

    // ── Builder helpers ───────────────────────────────────────────────────────

    #[inline]
    pub fn with_position(mut self, pos: Vec3) -> Self {
        self.position = pos;
        self
    }
    #[inline]
    pub fn with_rotation(mut self, rot: Quat) -> Self {
        self.rotation = rot;
        self
    }
    #[inline]
    pub fn with_scale(mut self, scale: Vec3) -> Self {
        self.scale = scale;
        self
    }
    #[inline]
    pub fn with_uniform_scale(mut self, s: f32) -> Self {
        self.scale = Vec3::splat(s);
        self
    }

    /// Build a Transform whose local -Z axis points from `eye` toward `target`.
    pub fn look_at(eye: Vec3, target: Vec3, up: Vec3) -> Self {
        let f = (target - eye).normalize(); // forward (world direction)
        let s = f.cross(up).normalize(); // right
        let u = s.cross(f); // reorthogonalised up

        // Rotation matrix: columns are the local axes expressed in world space.
        // Local +X → s, Local +Y → u, Local -Z → f  ⟹  Local +Z → -f.
        let (r00, r01, r02) = (s.x, u.x, -f.x);
        let (r10, r11, r12) = (s.y, u.y, -f.y);
        let (r20, r21, r22) = (s.z, u.z, -f.z);

        // Shepperd method: extract quaternion from 3×3 rotation matrix.
        let trace = r00 + r11 + r22;
        let rotation = if trace > 0.0 {
            let sq = (trace + 1.0).sqrt() * 2.0; // sq = 4w
            Quat::new(
                (r21 - r12) / sq,
                (r02 - r20) / sq,
                (r10 - r01) / sq,
                0.25 * sq,
            )
        } else if r00 > r11 && r00 > r22 {
            let sq = (1.0 + r00 - r11 - r22).sqrt() * 2.0; // sq = 4x
            Quat::new(
                0.25 * sq,
                (r01 + r10) / sq,
                (r02 + r20) / sq,
                (r21 - r12) / sq,
            )
        } else if r11 > r22 {
            let sq = (1.0 + r11 - r00 - r22).sqrt() * 2.0; // sq = 4y
            Quat::new(
                (r01 + r10) / sq,
                0.25 * sq,
                (r12 + r21) / sq,
                (r02 - r20) / sq,
            )
        } else {
            let sq = (1.0 + r22 - r00 - r11).sqrt() * 2.0; // sq = 4z
            Quat::new(
                (r02 + r20) / sq,
                (r12 + r21) / sq,
                0.25 * sq,
                (r10 - r01) / sq,
            )
        };

        Self {
            position: eye,
            rotation,
            scale: Vec3::ONE,
        }
    }

    /// Transform a point by this transform (applies scale, rotation, translation).
    #[inline]
    pub fn transform_point(&self, p: Vec3) -> Vec3 {
        self.local_matrix().transform_point(p).truncate()
    }

    /// Transform a direction by this transform (applies rotation only, ignores position and scale).
    #[inline]
    pub fn transform_dir(&self, d: Vec3) -> Vec3 {
        self.rotation.rotate_vec3(d)
    }
}

impl Default for Transform {
    fn default() -> Self {
        Self::IDENTITY
    }
}
