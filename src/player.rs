//! Player controller -- capsule movement, gravity, terrain collision.

use crate::lod::LodTerrain;
use crate::math::Vec3;

/// First-person player with capsule collision against the heightmap terrain.
pub struct PlayerController {
    /// Centre of the capsule in world space.
    pub position: Vec3,
    /// Current velocity (m/s).
    pub velocity: Vec3,
    /// Capsule radius.
    pub radius: f32,
    /// Capsule total height (tip to tip).
    pub height: f32,
    pub move_speed: f32,
    pub jump_speed: f32,
    pub gravity: f32,
    pub yaw: f32,
    pub pitch: f32,
    pub look_sensitivity: f32,
    pub grounded: bool,
}

impl PlayerController {
    pub fn new(position: Vec3) -> Self {
        Self {
            position,
            velocity: Vec3::ZERO,
            radius: 0.4,
            height: 1.8,
            move_speed: 8.0,
            jump_speed: 8.0,
            gravity: 20.0,
            yaw: 0.0,
            pitch: 0.0,
            look_sensitivity: 0.003,
            grounded: false,
        }
    }

    /// Eye position -- near the top of the capsule.
    pub fn eye_position(&self) -> Vec3 {
        Vec3::new(
            self.position.x,
            self.position.y + self.height * 0.5 - 0.1,
            self.position.z,
        )
    }

    /// Forward direction on the XZ plane (derived from yaw).
    pub fn forward(&self) -> Vec3 {
        Vec3::new(self.yaw.sin(), 0.0, -self.yaw.cos())
    }

    /// Right direction on the XZ plane.
    pub fn right(&self) -> Vec3 {
        Vec3::new(self.yaw.cos(), 0.0, self.yaw.sin())
    }

    /// Call once per frame.
    ///
    /// * move_input -- x = strafe right, z = forward (each -1..1)
    /// * look_delta -- raw mouse delta in pixels (dx, dy)
    /// * jump       -- true the frame the jump key was *pressed*
    /// * sprint     -- true while the sprint key is *held*
    pub fn update(
        &mut self,
        dt: f32,
        move_input: Vec3,
        look_delta: (f32, f32),
        jump: bool,
        sprint: bool,
        terrain: &LodTerrain,
    ) {
        let half_pi = std::f32::consts::FRAC_PI_2 - 0.01;

        // Mouse look
        self.yaw += look_delta.0 * self.look_sensitivity;
        self.pitch -= look_delta.1 * self.look_sensitivity;
        self.pitch = self.pitch.clamp(-half_pi, half_pi);

        // Horizontal movement
        let forward = self.forward();
        let right = self.right();
        let wish = forward * move_input.z + right * move_input.x;
        let wish_len = wish.length();
        if wish_len > 0.0 {
            let speed = if sprint {
                self.move_speed * 2.5
            } else {
                self.move_speed
            };
            let step = wish * (speed * dt / wish_len);
            self.position.x += step.x;
            self.position.z += step.z;
        }

        // Gravity
        self.velocity.y -= self.gravity * dt;

        // Jump
        if jump && self.grounded {
            self.velocity.y = self.jump_speed;
            self.grounded = false;
        }

        // Integrate vertical velocity
        self.position.y += self.velocity.y * dt;

        // Terrain collision (capsule foot)
        let foot = self.position.y - self.height * 0.5;
        let ground = self.sample_ground(terrain);
        if foot < ground {
            self.position.y = ground + self.height * 0.5;
            if self.velocity.y < 0.0 {
                self.velocity.y = 0.0;
            }
            self.grounded = true;
        } else {
            self.grounded = (foot - ground) < 0.05;
        }
    }

    /// Sample terrain at five points under the capsule footprint and return
    /// the maximum height so slopes and ridges do not let us clip through.
    fn sample_ground(&self, terrain: &LodTerrain) -> f32 {
        let x = self.position.x;
        let z = self.position.z;
        let r = self.radius;
        terrain
            .height_at(x, z)
            .max(terrain.height_at(x + r, z))
            .max(terrain.height_at(x - r, z))
            .max(terrain.height_at(x, z + r))
            .max(terrain.height_at(x, z - r))
    }
}
