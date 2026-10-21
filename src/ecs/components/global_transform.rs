use crate::math::Mat4;

/// The world-space transform matrix, computed each frame by TransformPropagateSystem.
/// Read-only from game code — only the propagation system writes this.
pub struct GlobalTransform {
    pub matrix: Mat4,
}

impl GlobalTransform {
    pub fn identity() -> Self {
        Self {
            matrix: Mat4::IDENTITY,
        }
    }
}

impl Default for GlobalTransform {
    fn default() -> Self {
        Self::identity()
    }
}
