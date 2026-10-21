use super::system::System;
use super::world::World;

/// Owns and runs a collection of systems.
///
/// Kept separate from [`World`] so systems can receive `&mut World` without
/// borrow-checker gymnastics.
pub struct SystemSchedule {
    systems: Vec<Box<dyn System>>,
}

impl SystemSchedule {
    pub fn new() -> Self {
        Self { systems: Vec::new() }
    }

    /// Register a system to be run each frame.
    pub fn add_system<S: System>(&mut self, system: S) {
        self.systems.push(Box::new(system));
    }

    /// Run all registered systems in insertion order.
    pub fn run(&mut self, world: &mut World, dt: f32) {
        for system in &mut self.systems {
            system.update(world, dt);
        }
    }
}

impl Default for SystemSchedule {
    fn default() -> Self {
        Self::new()
    }
}
