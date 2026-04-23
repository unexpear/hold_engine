use crate::assets::MaterialHandle;
use crate::ecs::components::{
    Particle, ParticleEmitter, ParticleSource, RigidBody, Transform, Velocity,
};
use crate::ecs::{Entity, System, World};
use crate::math::{Quat, Vec3};
use crate::renderer::{DebugLine, ParticleBatch, ParticleBillboard};

/// Spawns runtime particle entities from [`ParticleEmitter`] components.
pub struct ParticleEmitSystem;

impl System for ParticleEmitSystem {
    fn update(&mut self, world: &mut World, dt: f32) {
        let emitters: Vec<Entity> = world
            .query_with::<ParticleEmitter, Transform>()
            .map(|(entity, _)| entity)
            .collect();

        for emitter_entity in emitters {
            let Some((origin, rotation)) = world
                .get::<Transform>(emitter_entity)
                .map(|transform| (transform.position, transform.rotation))
            else {
                continue;
            };

            let alive_particles = world
                .query::<ParticleSource>()
                .filter(|(_, source)| source.emitter == emitter_entity)
                .count() as u32;

            let spawn_count = {
                let Some(emitter) = world.get_mut::<ParticleEmitter>(emitter_entity) else {
                    continue;
                };
                emitter.take_spawn_count(dt, alive_particles)
            };

            let mut plans = Vec::with_capacity(spawn_count as usize);
            for _ in 0..spawn_count {
                if let Some(plan) = sample_particle(world, emitter_entity, origin, rotation) {
                    plans.push(plan);
                }
            }

            for plan in plans {
                let entity = world.spawn();
                world.add_component(entity, Transform::from_position(plan.position));
                world.add_component(entity, Velocity::from_linear(plan.velocity));
                world.add_component(
                    entity,
                    RigidBody::particle(plan.gravity_scale, plan.linear_damping),
                );
                world.add_component(entity, plan.particle);
                world.add_component(
                    entity,
                    ParticleSource {
                        emitter: emitter_entity,
                    },
                );
            }
        }
    }
}

/// Ages particles and despawns them when their lifetime runs out.
pub struct ParticleLifetimeSystem;

impl System for ParticleLifetimeSystem {
    fn update(&mut self, world: &mut World, dt: f32) {
        let particles: Vec<Entity> = world
            .query::<Particle>()
            .map(|(entity, _)| entity)
            .collect();
        let mut expired = Vec::new();

        for entity in particles {
            if let Some(particle) = world.get_mut::<Particle>(entity) {
                particle.age += dt.max(0.0);
                if particle.age >= particle.lifetime {
                    expired.push(entity);
                }
            }
        }

        for entity in expired {
            world.despawn(entity);
        }
    }
}

/// Build a backend-neutral particle debug overlay.
pub fn generate_particle_debug_lines(world: &World) -> Vec<DebugLine> {
    let mut lines = Vec::new();

    for (entity, particle) in world.query::<Particle>() {
        let Some(position) = world
            .get::<Transform>(entity)
            .map(|transform| transform.position)
        else {
            continue;
        };

        let color = particle.current_color();
        let half_extent = (particle.current_size() * 0.5).max(0.01);
        lines.push(DebugLine::new(
            position - Vec3::new(half_extent, 0.0, 0.0),
            position + Vec3::new(half_extent, 0.0, 0.0),
            color,
        ));
        lines.push(DebugLine::new(
            position - Vec3::new(0.0, half_extent, 0.0),
            position + Vec3::new(0.0, half_extent, 0.0),
            color,
        ));
        lines.push(DebugLine::new(
            position - Vec3::new(0.0, 0.0, half_extent),
            position + Vec3::new(0.0, 0.0, half_extent),
            color,
        ));
    }

    lines
}

/// Build a backend-neutral particle billboard batch from live particle entities.
pub fn generate_particle_batch(
    world: &World,
    material: Option<MaterialHandle>,
) -> Option<ParticleBatch> {
    let mut billboards = Vec::new();

    for (entity, particle) in world.query::<Particle>() {
        let Some(position) = world
            .get::<Transform>(entity)
            .map(|transform| transform.position)
        else {
            continue;
        };

        billboards.push(ParticleBillboard {
            position,
            size: particle.current_size().max(0.01),
            color: particle.current_color(),
        });
    }

    if billboards.is_empty() {
        None
    } else {
        Some(ParticleBatch {
            material,
            billboards,
            soft_edges: true,
        })
    }
}

struct ParticleSpawn {
    position: Vec3,
    velocity: Vec3,
    gravity_scale: f32,
    linear_damping: f32,
    particle: Particle,
}

fn sample_particle(
    world: &mut World,
    emitter_entity: Entity,
    origin: Vec3,
    rotation: Quat,
) -> Option<ParticleSpawn> {
    let emitter = world.get_mut::<ParticleEmitter>(emitter_entity)?;
    let base_dir = if emitter.direction.length_sq() > 1e-6 {
        emitter.direction.normalize()
    } else {
        Vec3::Y
    };

    let spread = emitter.sample_in_unit_sphere() * emitter.spread.max(0.0);
    let local_dir = (base_dir + spread).normalize();
    let world_dir = rotation.rotate_vec3(local_dir);
    let speed = emitter
        .sample_range(emitter.speed_min, emitter.speed_max)
        .max(0.0);
    let lifetime = emitter
        .sample_range(emitter.lifetime_min, emitter.lifetime_max)
        .max(0.05);
    let offset = emitter.sample_in_unit_sphere() * emitter.spawn_radius.max(0.0);

    Some(ParticleSpawn {
        position: origin + offset,
        velocity: world_dir * speed,
        gravity_scale: emitter.gravity_scale,
        linear_damping: emitter.linear_damping,
        particle: Particle::new(
            lifetime,
            emitter.start_color,
            emitter.end_color,
            emitter.start_size,
            emitter.end_size,
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::renderer::Color;

    #[test]
    fn emitter_spawns_particles_and_honors_cap() {
        let mut world = World::new();
        let emitter_entity = world.spawn();
        world.add_component(emitter_entity, Transform::from_position(Vec3::ZERO));
        let mut emitter = ParticleEmitter::spark_fountain();
        emitter.rate_per_second = 10.0;
        emitter.max_particles = 3;
        emitter.seed = 123;
        world.add_component(emitter_entity, emitter);

        let mut system = ParticleEmitSystem;
        system.update(&mut world, 0.5);

        assert_eq!(world.query::<Particle>().count(), 3);
        assert_eq!(world.query::<ParticleSource>().count(), 3);
    }

    #[test]
    fn lifetime_system_despawns_expired_particles() {
        let mut world = World::new();
        let entity = world.spawn();
        world.add_component(entity, Transform::from_position(Vec3::ZERO));
        let mut particle = Particle::new(1.0, Color::WHITE, Color::RED, 0.5, 0.0);
        particle.age = 0.9;
        world.add_component(entity, particle);

        let mut system = ParticleLifetimeSystem;
        system.update(&mut world, 0.2);

        assert_eq!(world.query::<Particle>().count(), 0);
    }

    #[test]
    fn debug_lines_reflect_particle_color_and_size() {
        let mut world = World::new();
        let entity = world.spawn();
        world.add_component(entity, Transform::from_position(Vec3::ZERO));
        let mut particle = Particle::new(1.0, Color::RED, Color::BLUE, 2.0, 0.0);
        particle.age = 0.5;
        world.add_component(entity, particle);

        let lines = generate_particle_debug_lines(&world);
        assert_eq!(lines.len(), 3);
        for line in &lines {
            assert_eq!(line.color, Color::new(0.5, 0.0, 0.5, 1.0));
        }
        assert_eq!(lines[0].start.x, -0.5);
        assert_eq!(lines[0].end.x, 0.5);
    }

    #[test]
    fn particle_batch_reflects_particle_color_and_size() {
        let mut world = World::new();
        let entity = world.spawn();
        world.add_component(entity, Transform::from_position(Vec3::new(2.0, 3.0, 4.0)));
        let mut particle = Particle::new(1.0, Color::RED, Color::BLUE, 2.0, 0.5);
        particle.age = 0.5;
        world.add_component(entity, particle);

        let batch = generate_particle_batch(&world, None).unwrap();
        assert_eq!(batch.billboards.len(), 1);
        assert_eq!(batch.billboards[0].position, Vec3::new(2.0, 3.0, 4.0));
        assert_eq!(batch.billboards[0].color, Color::new(0.5, 0.0, 0.5, 1.0));
        assert_eq!(batch.billboards[0].size, 1.25);
        assert!(batch.soft_edges);
    }
}
