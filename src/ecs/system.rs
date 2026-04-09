use super::access::SystemAccess;
use super::world::World;

/// Trait implemented by every game system.
///
/// Systems are registered with [`super::schedule::SystemSchedule`] and invoked
/// each frame via `SystemSchedule::run`.
///
/// # Parallel execution
///
/// Implement [`access`](Self::access) to declare which component types your
/// system reads and writes.  The scheduler uses these declarations to group
/// non-conflicting systems into parallel batches.  The default returns
/// [`SystemAccess::exclusive`], which is always safe but prevents any
/// parallelism.
///
/// ```ignore
/// impl System for VelocitySystem {
///     fn update(&mut self, world: &mut World, dt: f32) { /* … */ }
///
///     fn access(&self) -> SystemAccess {
///         SystemAccess::new()
///             .write::<Transform>()
///             .read::<Velocity>()
///     }
/// }
/// ```
///
/// # Safety contract for parallel systems
///
/// When the scheduler runs two systems in parallel it gives each a raw
/// `&mut World`.  The borrow-checker cannot enforce that your system only
/// accesses the components it declared.  **It is your responsibility** to
/// ensure systems that declare non-exclusive access only touch the TypeIds
/// they listed — otherwise you risk data races.
pub trait System: Send + 'static {
    fn update(&mut self, world: &mut World, dt: f32);

    /// Declare this system's component access for parallel scheduling.
    ///
    /// Returns [`SystemAccess::exclusive`] by default.
    fn access(&self) -> SystemAccess {
        SystemAccess::exclusive()
    }
}
