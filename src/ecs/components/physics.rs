use crate::math::Vec3;

/// Per-entity linear and angular velocity.
///
/// `linear` is expressed in world units per second.
/// `angular` is expressed as XYZ Euler radians per second.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Velocity {
    pub linear: Vec3,
    pub angular: Vec3,
}

impl Velocity {
    pub const ZERO: Self = Self {
        linear: Vec3::ZERO,
        angular: Vec3::ZERO,
    };

    pub fn from_linear(linear: Vec3) -> Self {
        Self {
            linear,
            ..Self::ZERO
        }
    }

    pub fn from_angular(angular: Vec3) -> Self {
        Self {
            angular,
            ..Self::ZERO
        }
    }

    pub fn with_linear(mut self, linear: Vec3) -> Self {
        self.linear = linear;
        self
    }

    pub fn with_angular(mut self, angular: Vec3) -> Self {
        self.angular = angular;
        self
    }
}

impl Default for Velocity {
    fn default() -> Self {
        Self::ZERO
    }
}

/// Simple rigid-body tuning for gravity, damping, and collision bounce.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RigidBody {
    pub gravity_scale: f32,
    pub linear_damping: f32,
    pub angular_damping: f32,
    pub restitution: f32,
}

impl RigidBody {
    pub fn dynamic() -> Self {
        Self::default()
    }

    pub fn particle(gravity_scale: f32, linear_damping: f32) -> Self {
        Self {
            gravity_scale,
            linear_damping,
            angular_damping: 0.0,
            restitution: 0.0,
        }
    }

    pub fn with_gravity_scale(mut self, gravity_scale: f32) -> Self {
        self.gravity_scale = gravity_scale;
        self
    }

    pub fn with_linear_damping(mut self, linear_damping: f32) -> Self {
        self.linear_damping = linear_damping;
        self
    }

    pub fn with_angular_damping(mut self, angular_damping: f32) -> Self {
        self.angular_damping = angular_damping;
        self
    }

    pub fn with_restitution(mut self, restitution: f32) -> Self {
        self.restitution = restitution.clamp(0.0, 1.0);
        self
    }
}

impl Default for RigidBody {
    fn default() -> Self {
        Self {
            gravity_scale: 1.0,
            linear_damping: 0.0,
            angular_damping: 0.0,
            restitution: 0.5,
        }
    }
}

/// Naive sphere collider used by the first-pass physics system.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SphereCollider {
    pub radius: f32,
}

impl SphereCollider {
    pub fn new(radius: f32) -> Self {
        Self {
            radius: radius.max(0.001),
        }
    }
}
