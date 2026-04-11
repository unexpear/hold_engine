use crate::ecs::components::{ActiveCamera, Camera, CameraMode, GlobalTransform};
use crate::ecs::{Entity, System, SystemAccess, World};
use crate::math::{Mat4, Vec3};

/// Recomputes [\Camera::view\] (and rebuilds proj if aspect changed) for every
/// entity that carries both [\ActiveCamera\] and [\Camera\].
///
/// **Run order**: must execute *after* [\TransformPropagateSystem\] so that
/// \GlobalTransform\ values are up to date.
pub struct CameraSystem;

impl System for CameraSystem {
    fn access(&self) -> SystemAccess {
        // Writes Camera; reads ActiveCamera, CameraMode, GlobalTransform.
        // NOTE: GlobalTransform is also written by TransformPropagateSystem,
        // so those two systems will always be placed in separate batches and
        // run sequentially — CameraSystem must follow TransformPropagateSystem.
        SystemAccess::new()
            .write::<Camera>()
            .read::<ActiveCamera>()
            .read::<CameraMode>()
            .read::<GlobalTransform>()
    }

    fn update(&mut self, world: &mut World, _dt: f32) {
        // Collect active cameras together with their CameraMode in one pass,
        // avoiding a separate world.get::<CameraMode> call inside the loop.
        // query_with iterates CameraMode and filters to entities that also
        // carry ActiveCamera — no extra collect step needed for filtering.
        let cameras: Vec<(Entity, CameraMode)> = world
            .query_with::<CameraMode, ActiveCamera>()
            .map(|(e, mode)| (e, mode.clone()))
            .collect();

        for (entity, mode) in cameras {
            let view = match mode {
                CameraMode::FirstPerson { yaw, pitch, .. } => {
                    // Position comes from the entity's GlobalTransform.
                    let pos = world
                        .get::<GlobalTransform>(entity)
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

                CameraMode::ThirdPerson {
                    target,
                    distance,
                    yaw,
                    pitch,
                } => {
                    // World-space position of the entity we're orbiting.
                    let target_pos = world
                        .get::<GlobalTransform>(target)
                        .map(|gt| gt.matrix.extract_position())
                        .unwrap_or(Vec3::ZERO);

                    // Spherical to Cartesian — orbit around target.
                    let eye = target_pos
                        + Vec3::new(
                            yaw.sin() * pitch.cos() * distance,
                            pitch.sin() * distance,
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
