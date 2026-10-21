use crate::ecs::components::{RigidBody, SphereCollider, Transform, Velocity};
use crate::ecs::{Entity, System, SystemAccess, World};
use crate::math::{Quat, Vec3};

/// Basic motion integrator plus naive sphere/sphere collision resolution.
pub struct PhysicsSystem {
    pub gravity: Vec3,
}

impl PhysicsSystem {
    pub fn new(gravity: Vec3) -> Self {
        Self { gravity }
    }
}

impl Default for PhysicsSystem {
    fn default() -> Self {
        Self::new(Vec3::new(0.0, -9.81, 0.0))
    }
}

impl System for PhysicsSystem {
    fn access(&self) -> SystemAccess {
        SystemAccess::new()
            .write::<Transform>()
            .write::<Velocity>()
            .read::<RigidBody>()
            .read::<SphereCollider>()
    }

    fn update(&mut self, world: &mut World, dt: f32) {
        let bodies: Vec<Entity> = world
            .query_with::<Velocity, Transform>()
            .map(|(entity, _)| entity)
            .collect();

        for entity in bodies {
            integrate_body(world, entity, self.gravity, dt);
        }

        resolve_sphere_collisions(world);
    }
}

fn integrate_body(world: &mut World, entity: Entity, gravity: Vec3, dt: f32) {
    let Some(velocity_snapshot) = world.get::<Velocity>(entity).copied() else {
        return;
    };
    let Some((position, rotation)) = world
        .get::<Transform>(entity)
        .map(|transform| (transform.position, transform.rotation))
    else {
        return;
    };

    let body = world.get::<RigidBody>(entity).copied();
    let mut next_velocity = velocity_snapshot;

    if let Some(body) = body {
        next_velocity.linear += gravity * body.gravity_scale * dt;
        next_velocity.linear = apply_damping(next_velocity.linear, body.linear_damping, dt);
        next_velocity.angular = apply_damping(next_velocity.angular, body.angular_damping, dt);
    }

    let next_position = position + next_velocity.linear * dt;
    let next_rotation = if next_velocity.angular.length_sq() > 1e-8 {
        (Quat::from_euler_xyz(
            next_velocity.angular.x * dt,
            next_velocity.angular.y * dt,
            next_velocity.angular.z * dt,
        ) * rotation)
            .normalize()
    } else {
        rotation
    };

    if let Some(velocity) = world.get_mut::<Velocity>(entity) {
        *velocity = next_velocity;
    }
    if let Some(transform) = world.get_mut::<Transform>(entity) {
        transform.position = next_position;
        transform.rotation = next_rotation;
    }
}

fn apply_damping(value: Vec3, damping: f32, dt: f32) -> Vec3 {
    let factor = (1.0 - damping.max(0.0) * dt.max(0.0)).max(0.0);
    value * factor
}

#[derive(Clone, Copy)]
struct ColliderState {
    entity: Entity,
    radius: f32,
    dynamic: bool,
    restitution: f32,
}

fn resolve_sphere_collisions(world: &mut World) {
    let colliders: Vec<ColliderState> = world
        .query::<SphereCollider>()
        .filter(|(entity, _)| world.has_component::<Transform>(*entity))
        .map(|(entity, collider)| ColliderState {
            entity,
            radius: collider.radius.max(0.001),
            dynamic: world.has_component::<Velocity>(entity),
            restitution: world
                .get::<RigidBody>(entity)
                .map(|body| body.restitution)
                .unwrap_or(0.5),
        })
        .collect();

    for i in 0..colliders.len() {
        for j in i + 1..colliders.len() {
            let a = colliders[i];
            let b = colliders[j];

            if !a.dynamic && !b.dynamic {
                continue;
            }

            let Some(a_pos) = world.get::<Transform>(a.entity).map(|t| t.position) else {
                continue;
            };
            let Some(b_pos) = world.get::<Transform>(b.entity).map(|t| t.position) else {
                continue;
            };

            let delta = b_pos - a_pos;
            let dist_sq = delta.length_sq();
            let radius_sum = a.radius + b.radius;
            if dist_sq >= radius_sum * radius_sum {
                continue;
            }

            let dist = dist_sq.sqrt();
            let normal = if dist > 1e-6 { delta / dist } else { Vec3::X };
            let penetration = radius_sum - dist;

            match (a.dynamic, b.dynamic) {
                (true, true) => {
                    apply_position_shift(world, a.entity, -normal * (penetration * 0.5));
                    apply_position_shift(world, b.entity, normal * (penetration * 0.5));
                    resolve_dynamic_pair(
                        world,
                        a.entity,
                        b.entity,
                        normal,
                        (a.restitution + b.restitution) * 0.5,
                    );
                }
                (true, false) => {
                    apply_position_shift(world, a.entity, -normal * penetration);
                    resolve_against_static(world, a.entity, normal, a.restitution);
                }
                (false, true) => {
                    apply_position_shift(world, b.entity, normal * penetration);
                    resolve_against_static(world, b.entity, -normal, b.restitution);
                }
                (false, false) => {}
            }
        }
    }
}

fn apply_position_shift(world: &mut World, entity: Entity, delta: Vec3) {
    if let Some(transform) = world.get_mut::<Transform>(entity) {
        transform.position += delta;
    }
}

fn resolve_against_static(
    world: &mut World,
    entity: Entity,
    normal_to_static: Vec3,
    restitution: f32,
) {
    let Some(current) = world.get::<Velocity>(entity).copied() else {
        return;
    };

    let approach_speed = current.linear.dot(normal_to_static);
    if approach_speed <= 0.0 {
        return;
    }

    let mut next = current;
    next.linear = current.linear - normal_to_static * ((1.0 + restitution) * approach_speed);

    if let Some(velocity) = world.get_mut::<Velocity>(entity) {
        *velocity = next;
    }
}

fn resolve_dynamic_pair(
    world: &mut World,
    entity_a: Entity,
    entity_b: Entity,
    normal_a_to_b: Vec3,
    restitution: f32,
) {
    let Some(velocity_a) = world.get::<Velocity>(entity_a).copied() else {
        return;
    };
    let Some(velocity_b) = world.get::<Velocity>(entity_b).copied() else {
        return;
    };

    let relative_speed = (velocity_a.linear - velocity_b.linear).dot(normal_a_to_b);
    if relative_speed <= 0.0 {
        return;
    }

    let impulse = normal_a_to_b * ((1.0 + restitution) * relative_speed * 0.5);

    if let Some(velocity) = world.get_mut::<Velocity>(entity_a) {
        velocity.linear -= impulse;
    }
    if let Some(velocity) = world.get_mut::<Velocity>(entity_b) {
        velocity.linear += impulse;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn physics_integrates_gravity_and_angular_velocity() {
        let mut world = World::new();
        let entity = world.spawn();
        world.add_component(entity, Transform::from_position(Vec3::new(0.0, 10.0, 0.0)));
        world.add_component(
            entity,
            Velocity::from_angular(Vec3::new(0.0, 1.0, 0.0)).with_linear(Vec3::ZERO),
        );
        world.add_component(entity, RigidBody::dynamic());

        let mut system = PhysicsSystem::default();
        system.update(&mut world, 0.5);

        let velocity = world.get::<Velocity>(entity).unwrap();
        let transform = world.get::<Transform>(entity).unwrap();
        assert!((velocity.linear.y - (-4.905)).abs() < 0.01);
        assert!((transform.position.y - 7.5475).abs() < 0.02);
        assert!(transform.rotation != Quat::IDENTITY);
    }

    #[test]
    fn physics_resolves_dynamic_sphere_overlap() {
        let mut world = World::new();
        let a = world.spawn();
        world.add_component(a, Transform::from_position(Vec3::new(-0.4, 0.0, 0.0)));
        world.add_component(a, Velocity::from_linear(Vec3::new(1.0, 0.0, 0.0)));
        world.add_component(a, SphereCollider::new(0.5));

        let b = world.spawn();
        world.add_component(b, Transform::from_position(Vec3::new(0.4, 0.0, 0.0)));
        world.add_component(b, Velocity::from_linear(Vec3::new(-1.0, 0.0, 0.0)));
        world.add_component(b, SphereCollider::new(0.5));

        let mut system = PhysicsSystem::default();
        system.update(&mut world, 0.0);

        let a_pos = world.get::<Transform>(a).unwrap().position;
        let b_pos = world.get::<Transform>(b).unwrap().position;
        let a_vel = world.get::<Velocity>(a).unwrap().linear;
        let b_vel = world.get::<Velocity>(b).unwrap().linear;

        assert!(a_pos.distance(b_pos) >= 0.999);
        assert!(a_vel.x <= 0.0);
        assert!(b_vel.x >= 0.0);
    }
}
