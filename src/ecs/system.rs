use super::world::World;

/// Trait implemented by every game system.
///
/// Systems are registered with [`super::schedule::SystemSchedule`] and invoked
/// each frame via `SystemSchedule::run`.
pub trait System: 'static {
    fn update(&mut self, world: &mut World, dt: f32);
}
