use crate::ecs::{System, World};
use crate::ecs::components::{Children, GlobalTransform, Parent, Transform};
use crate::math::Mat4;

pub struct TransformPropagateSystem;

impl System for TransformPropagateSystem {
    fn update(&mut self, world: &mut World, _dt: f32) {
        // Collect all entities that have a Transform component.
        // We collect first so the borrow on `world` ends before we call `world.get`.
        let with_transform: Vec<_> = world
            .query::<Transform>()
            .map(|(e, _)| e)
            .collect();

        // Filter down to roots: entities with Transform but no Parent.
        let roots: Vec<_> = with_transform
            .into_iter()
            .filter(|&e| world.get::<Parent>(e).is_none())
            .collect();

        // Use an explicit stack to avoid recursion.
        let mut stack: Vec<(crate::ecs::Entity, Mat4)> = roots
            .into_iter()
            .map(|e| {
                let local = world.get::<Transform>(e)
                    .map(|t| t.local_matrix())
                    .unwrap_or(Mat4::IDENTITY);
                (e, local)
            })
            .collect();

        while let Some((entity, world_matrix)) = stack.pop() {
            // Write GlobalTransform.
            world.add_component(entity, GlobalTransform { matrix: world_matrix });

            // Push children onto stack.
            if let Some(children) = world.get::<Children>(entity) {
                let child_list = children.list.clone();
                for child in child_list {
                    let local = world.get::<Transform>(child)
                        .map(|t| t.local_matrix())
                        .unwrap_or(Mat4::IDENTITY);
                    stack.push((child, world_matrix * local));
                }
            }
        }
    }
}
