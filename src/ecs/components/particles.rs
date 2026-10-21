use crate::ecs::entity::Entity;
use crate::math::Vec3;
use crate::renderer::Color;

/// Runtime state for an individual particle.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Particle {
    pub age: f32,
    pub lifetime: f32,
    pub start_color: Color,
    pub end_color: Color,
    pub start_size: f32,
    pub end_size: f32,
}

impl Particle {
    pub fn new(
        lifetime: f32,
        start_color: Color,
        end_color: Color,
        start_size: f32,
        end_size: f32,
    ) -> Self {
        Self {
            age: 0.0,
            lifetime: lifetime.max(0.001),
            start_color,
            end_color,
            start_size: start_size.max(0.001),
            end_size: end_size.max(0.0),
        }
    }

    pub fn normalized_age(&self) -> f32 {
        (self.age / self.lifetime).clamp(0.0, 1.0)
    }

    pub fn current_color(&self) -> Color {
        self.start_color.lerp(self.end_color, self.normalized_age())
    }

    pub fn current_size(&self) -> f32 {
        self.start_size + (self.end_size - self.start_size) * self.normalized_age()
    }
}

/// Tracks which emitter authored a particle so caps can be enforced per-source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParticleSource {
    pub emitter: Entity,
}

/// Config and runtime state for a particle emitter.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ParticleEmitter {
    pub emitting: bool,
    pub rate_per_second: f32,
    pub max_particles: u32,
    pub spawn_radius: f32,
    pub direction: Vec3,
    pub spread: f32,
    pub speed_min: f32,
    pub speed_max: f32,
    pub lifetime_min: f32,
    pub lifetime_max: f32,
    pub gravity_scale: f32,
    pub linear_damping: f32,
    pub start_size: f32,
    pub end_size: f32,
    pub start_color: Color,
    pub end_color: Color,
    pub accumulator: f32,
    pub seed: u32,
}

impl ParticleEmitter {
    pub fn spark_fountain() -> Self {
        Self {
            emitting: true,
            rate_per_second: 40.0,
            max_particles: 160,
            spawn_radius: 0.15,
            direction: Vec3::Y,
            spread: 0.45,
            speed_min: 2.5,
            speed_max: 5.5,
            lifetime_min: 0.4,
            lifetime_max: 1.1,
            gravity_scale: 0.9,
            linear_damping: 1.6,
            start_size: 0.18,
            end_size: 0.02,
            start_color: Color::new(1.0, 0.85, 0.25, 1.0),
            end_color: Color::new(1.0, 0.2, 0.05, 0.0),
            accumulator: 0.0,
            seed: 1,
        }
    }

    pub fn with_emitting(mut self, emitting: bool) -> Self {
        self.emitting = emitting;
        self
    }

    pub(crate) fn take_spawn_count(&mut self, dt: f32, alive_particles: u32) -> u32 {
        if !self.emitting || self.max_particles == 0 || self.rate_per_second <= 0.0 {
            return 0;
        }

        self.accumulator += self.rate_per_second * dt.max(0.0);
        let requested = self.accumulator.floor() as u32;
        let capacity = self.max_particles.saturating_sub(alive_particles);
        let spawn_count = requested.min(capacity);

        if requested > capacity {
            self.accumulator = self.accumulator.fract();
        } else {
            self.accumulator -= spawn_count as f32;
        }

        spawn_count
    }

    pub(crate) fn sample_range(&mut self, a: f32, b: f32) -> f32 {
        if (a - b).abs() < 1e-6 {
            return a;
        }
        let (min, max) = if a < b { (a, b) } else { (b, a) };
        min + (max - min) * self.next_f32()
    }

    pub(crate) fn sample_in_unit_sphere(&mut self) -> Vec3 {
        for _ in 0..8 {
            let sample = Vec3::new(
                self.next_f32_signed(),
                self.next_f32_signed(),
                self.next_f32_signed(),
            );
            if sample.length_sq() <= 1.0 {
                return sample;
            }
        }
        Vec3::ZERO
    }

    fn next_u32(&mut self) -> u32 {
        self.seed = self
            .seed
            .wrapping_mul(1_664_525)
            .wrapping_add(1_013_904_223);
        self.seed
    }

    fn next_f32(&mut self) -> f32 {
        self.next_u32() as f32 / u32::MAX as f32
    }

    fn next_f32_signed(&mut self) -> f32 {
        self.next_f32() * 2.0 - 1.0
    }
}
