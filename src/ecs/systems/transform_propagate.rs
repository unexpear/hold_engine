use crate::ecs::components::{Children, GlobalTransform, Parent, Transform};
use crate::ecs::{System, SystemAccess, World};
use crate::math::Mat4;

pub struct TransformPropagateSystem;

impl System for TransformPropagateSystem {
    fn access(&self) -> SystemAccess {
        // Reads Transform, Children, Parent; writes GlobalTransform.
        SystemAccess::new()
            .write::<GlobalTransform>()
            .read::<Transform>()
            .read::<Children>()
            .read::<Parent>()
    }

    fn update(&mut self, world: &mut World, _dt: f32) {
        // Root entities: have Transform but no Parent.
        // query_without iterates the Transform storage and skips any entity
        // that also has a Parent — no intermediate Vec needed for the filter.
        let mut stack: Vec<(crate::ecs::Entity, Mat4)> = world
            .query_without::<Transform, Parent>()
            .map(|(e, t)| (e, t.local_matrix()))
            .collect();

        while let Some((entity, world_matrix)) = stack.pop() {
            // Write GlobalTransform.
            world.add_component(
                entity,
                GlobalTransform {
                    matrix: world_matrix,
                },
            );

            // Push children onto stack.
            let child_list: Vec<_> = world
                .get::<Children>(entity)
                .map(|c| c.list.clone())
                .unwrap_or_default();
            for child in child_list {
                let local = world
                    .get::<Transform>(child)
                    .map(|t| t.local_matrix())
                    .unwrap_or(Mat4::IDENTITY);
                stack.push((child, world_matrix * local));
            }
        }
    }
}
