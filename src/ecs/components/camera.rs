use crate::ecs::entity::Entity;
use crate::math::{deg_to_rad, Mat4};

/// Perspective camera component.
///
/// Attach to an entity alongside [\CameraMode\] and [\ActiveCamera\].  
/// [\CameraSystem\] recomputes \iew\ and \proj\ every frame — renderers
/// just read \cam.view\ and \cam.proj\.
pub struct Camera {
    pub fov: f32, // vertical FOV in radians
    pub near: f32,
    pub far: f32,
    pub aspect: f32, // width / height — call set_aspect on window resize
    /// World-to-camera matrix. Written by CameraSystem each frame.     
    pub view: Mat4,
    /// Projection matrix. Rebuilt when fov/aspect/near/far change.     
    pub proj: Mat4,
}

impl Camera {
    pub fn perspective(fov_deg: f32, aspect: f32, near: f32, far: f32) -> Self {
        let fov = deg_to_rad(fov_deg);
        Self {
            fov,
            near,
            far,
            aspect,
            view: Mat4::IDENTITY,
            proj: Mat4::perspective_gl(fov, aspect, near, far),
        }
    }

    /// Recompute the projection matrix after the window is resized.    
    pub fn set_aspect(&mut self, aspect: f32) {
        self.aspect = aspect;
        self.proj = Mat4::perspective_gl(self.fov, aspect, self.near, self.far);
    }
}

/// How the camera derives its view matrix each frame.
#[derive(Clone)]
pub enum CameraMode {
    /// FPS camera: position comes from the entity's Transform, orientation
    /// from yaw/pitch.
    ///
    /// * \yaw\   — horizontal angle in radians (0 = looking along −Z)
    /// * \pitch\ — vertical angle in radians   (clamped ±89°)
    /// * \move_speed\       — units per second for WASD movement
    /// * \look_sensitivity\ — radians per pixel of mouse delta
    FirstPerson {
        yaw: f32,
        pitch: f32,
        move_speed: f32,
        look_sensitivity: f32,
    },

    /// Eye orbits \	arget\ at \distance\ metres, steered by \yaw\/\pitch\.
    ///
    /// * \yaw\   — horizontal angle in radians (0 = behind the target on +Z axis)
    /// * \pitch\ — vertical angle in radians   (0 = level, clamped to avoid flip)
    ThirdPerson {
        target: Entity,
        distance: f32,
        yaw: f32,
        pitch: f32,
    },
}

impl CameraMode {
    /// Convenient constructor for a first-person camera with sensible defaults.
    pub fn first_person() -> Self {
        Self::FirstPerson {
            yaw: 0.0,
            pitch: 0.0,
            move_speed: 5.0,
            look_sensitivity: 0.003,
        }
    }
}

/// Marker component — the entity carrying this is the active render camera.
/// Only one entity should have this at a time.
pub struct ActiveCamera;
