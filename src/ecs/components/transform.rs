use crate::math::{Mat4, Quat, Vec3};

pub struct Transform {
    pub position: Vec3,
    pub rotation: Quat,
    pub scale:    Vec3,
}

impl Transform {
    pub const IDENTITY: Self = Self {
        position: Vec3::ZERO,
        rotation: Quat::IDENTITY,
        scale:    Vec3::ONE,
    };

    pub fn from_position(pos: Vec3) -> Self {
        Self { position: pos, ..Self::IDENTITY }
    }

    pub fn from_rotation(rot: Quat) -> Self {
        Self { rotation: rot, ..Self::IDENTITY }
    }

    /// Compute the local TRS matrix: T * R * S.
    pub fn local_matrix(&self) -> Mat4 {
        Mat4::translate(self.position) * self.rotation.to_mat4() * Mat4::scale(self.scale)
    }
}

impl Default for Transform {
    fn default() -> Self { Self::IDENTITY }
}
