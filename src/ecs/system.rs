use super::world::World;

/// A system processes components every frame.
/// Systems are stored in the World and run via `world.update(dt)`.
pub trait System: 'static {
    fn update(&mut self, world: &mut World, dt: f32);
}
