use crate::assets::{MaterialHandle, MeshHandle};
use crate::math::{Mat4, Vec3};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StaticMesh {
    pub mesh: MeshHandle,
}

impl Default for StaticMesh {
    fn default() -> Self {
        Self {
            mesh: MeshHandle::null(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MaterialRef {
    pub material: MaterialHandle,
}

impl Default for MaterialRef {
    fn default() -> Self {
        Self {
            material: MaterialHandle::null(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerrainChunkRef {
    pub chunk_x: i32,
    pub chunk_z: i32,
    pub lod_level: u8,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VegetationInstance {
    pub prototype: String,
    pub wind_weight: f32,
}

impl VegetationInstance {
    pub fn new(prototype: impl Into<String>) -> Self {
        Self {
            prototype: prototype.into(),
            wind_weight: 1.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CharacterController {
    pub radius: f32,
    pub height: f32,
    pub move_speed: f32,
    pub jump_speed: f32,
    pub grounded: bool,
}

impl Default for CharacterController {
    fn default() -> Self {
        Self {
            radius: 0.4,
            height: 1.8,
            move_speed: 6.0,
            jump_speed: 6.5,
            grounded: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Weapon {
    pub damage: f32,
    pub range: f32,
    pub cooldown_seconds: f32,
    pub spread_deg: f32,
    pub projectile_speed: f32,
}

impl Default for Weapon {
    fn default() -> Self {
        Self {
            damage: 25.0,
            range: 80.0,
            cooldown_seconds: 0.2,
            spread_deg: 1.5,
            projectile_speed: 180.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Health {
    pub current: f32,
    pub max: f32,
}

impl Health {
    pub fn new(max: f32) -> Self {
        let max = max.max(1.0);
        Self { current: max, max }
    }

    pub fn apply_damage(&mut self, amount: f32) {
        self.current = (self.current - amount.max(0.0)).clamp(0.0, self.max);
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Breakable {
    pub intact_mesh: MeshHandle,
    pub broken_mesh: MeshHandle,
    pub break_threshold: f32,
    pub break_impulse: f32,
}

impl Breakable {
    pub fn is_broken(&self, current_health: f32) -> bool {
        current_health <= self.break_threshold
    }
}

impl Default for Breakable {
    fn default() -> Self {
        Self {
            intact_mesh: MeshHandle::null(),
            broken_mesh: MeshHandle::null(),
            break_threshold: 0.0,
            break_impulse: 15.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AudioEmitter {
    pub cue: String,
    pub gain: f32,
    pub min_distance: f32,
    pub max_distance: f32,
    pub looping: bool,
}

impl AudioEmitter {
    pub fn new(cue: impl Into<String>) -> Self {
        Self {
            cue: cue.into(),
            gain: 1.0,
            min_distance: 1.0,
            max_distance: 35.0,
            looping: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NavAgent {
    pub radius: f32,
    pub height: f32,
    pub max_speed: f32,
    pub stopping_distance: f32,
}

impl Default for NavAgent {
    fn default() -> Self {
        Self {
            radius: 0.45,
            height: 1.8,
            max_speed: 4.5,
            stopping_distance: 1.2,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PatrolRoute {
    pub points: Vec<Vec3>,
    pub looped: bool,
}

impl PatrolRoute {
    pub fn new(points: Vec<Vec3>) -> Self {
        Self {
            points,
            looped: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VehicleController {
    pub max_speed: f32,
    pub acceleration: f32,
    pub brake_deceleration: f32,
    pub steering_angle_deg: f32,
    pub handbrake_multiplier: f32,
}

impl Default for VehicleController {
    fn default() -> Self {
        Self {
            max_speed: 35.0,
            acceleration: 12.0,
            brake_deceleration: 18.0,
            steering_angle_deg: 32.0,
            handbrake_multiplier: 1.6,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WheelState {
    pub steer_deg: f32,
    pub suspension_compression: f32,
    pub drive: bool,
    pub brake: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SkeletonPose {
    pub joints: Vec<Mat4>,
}

impl SkeletonPose {
    pub fn identity(joint_count: usize) -> Self {
        Self {
            joints: vec![Mat4::IDENTITY; joint_count],
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AnimationState {
    pub clip: String,
    pub time_seconds: f32,
    pub playback_rate: f32,
    pub looping: bool,
}

impl AnimationState {
    pub fn new(clip: impl Into<String>) -> Self {
        Self {
            clip: clip.into(),
            time_seconds: 0.0,
            playback_rate: 1.0,
            looping: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AnimationGraphState {
    pub state: String,
    pub previous_state: String,
    pub blend_alpha: f32,
}

impl AnimationGraphState {
    pub fn new(state: impl Into<String>) -> Self {
        let state = state.into();
        Self {
            previous_state: state.clone(),
            state,
            blend_alpha: 1.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::math::Vec3;

    use super::{
        Breakable, CharacterController, Health, PatrolRoute, SkeletonPose, VehicleController,
    };

    #[test]
    fn runtime_slice_defaults_are_safe_bootstrap_values() {
        let controller = CharacterController::default();
        let vehicle = VehicleController::default();

        assert!(controller.height > controller.radius);
        assert!(controller.move_speed > 0.0);
        assert!(vehicle.max_speed > 0.0);
        assert!(vehicle.acceleration > 0.0);
    }

    #[test]
    fn health_and_breakable_thresholds_track_damage() {
        let mut health = Health::new(120.0);
        let breakable = Breakable {
            break_threshold: 35.0,
            ..Breakable::default()
        };

        health.apply_damage(40.0);
        assert!(!breakable.is_broken(health.current));

        health.apply_damage(60.0);
        assert!(breakable.is_broken(health.current));
        assert!(breakable.intact_mesh.is_null());
        assert!(breakable.broken_mesh.is_null());
    }

    #[test]
    fn patrol_route_and_pose_helpers_preserve_authored_counts() {
        let route = PatrolRoute::new(vec![Vec3::ZERO, Vec3::new(5.0, 0.0, 3.0)]);
        let pose = SkeletonPose::identity(6);

        assert_eq!(route.points.len(), 2);
        assert!(route.looped);
        assert_eq!(pose.joints.len(), 6);
    }
}
