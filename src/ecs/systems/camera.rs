use crate::ecs::{System, World};
use crate::ecs::components::{ActiveCamera, Camera, CameraMode, GlobalTransform};
use crate::math::{Vec3, Mat4};

/// Recomputes [\Camera::view\] (and rebuilds proj if aspect changed) for every
/// entity that carries both [\ActiveCamera\] and [\Camera\].
///
/// **Run order**: must execute *after* [\TransformPropagateSystem\] so that
/// \GlobalTransform\ values are up to date.
pub struct CameraSystem;

impl System for CameraSystem {
    fn update(&mut self, world: &mut World, _dt: f32) {
        // Collect active camera entities first so we release the borrow.
        let cameras: Vec<_> = world
            .query::<ActiveCamera>()
            .map(|(e, _)| e)
            .collect();

        for entity in cameras {
            let mode = match world.get::<CameraMode>(entity) {
                Some(m) => m.clone(),
                None    => continue,
            };

            let view = match mode {
                CameraMode::FirstPerson { yaw, pitch, .. } => {
                    // Position comes from the entity's GlobalTransform.
                    let pos = world.get::<GlobalTransform>(entity)
                        .map(|gt| gt.matrix.extract_position())
                        .unwrap_or(Vec3::ZERO);

                    // Forward vector from yaw/pitch (looking along -Z at yaw=0).
                    let forward = Vec3::new(
                        yaw.sin() * pitch.cos(),
                        pitch.sin(),
                        -(yaw.cos() * pitch.cos()),
                    );

                    Mat4::look_at(pos, pos + forward, Vec3::Y)
                }

                CameraMode::ThirdPerson { target, distance, yaw, pitch } => {
                    // World-space position of the entity we're orbiting.
                    let target_pos = world
                        .get::<GlobalTransform>(target)
                        .map(|gt| gt.matrix.extract_position())
                        .unwrap_or(Vec3::ZERO);

                    // Spherical to Cartesian — orbit around target.    
                    let eye = target_pos + Vec3::new(
                        yaw.sin() * pitch.cos() * distance,
                        pitch.sin()             * distance,
                        yaw.cos() * pitch.cos() * distance,
                    );

                    Mat4::look_at(eye, target_pos, Vec3::Y)
                }
            };

            if let Some(cam) = world.get_mut::<Camera>(entity) {        
                cam.view = view;
            }
        }
    }
}
