//! Change-detection wrapper returned by [`super::world::World::get_tracked_mut`].
//!
//! `Mut<T>` behaves like `&mut T` through `Deref`/`DerefMut`, but it only
//! records the current frame tick on the component's `changed_tick` when
//! the value is actually accessed mutably — avoiding false positives from
//! systems that hold a `Mut` but never write through it.
//!
//! # Example
//! ```ignore
//! // Only marks the component changed if the guard is written through.
//! if let Some(mut hp) = world.get_tracked_mut::<Stats>(player) {
//!     if hp.health < 100 { hp.health += 5; }  // ← marks changed only here
//! }
//!
//! // Systems that want to react to changes check against their last-run tick:
//! let since = self.last_run_tick;
//! for (entity, stats) in world.query_changed_since::<Stats>(since) { ... }
//! self.last_run_tick = world.tick();
//! ```

use std::ops::{Deref, DerefMut};

/// A mutable reference to a component that lazily marks the component as
/// changed (updates its `changed_tick`) only when accessed through
/// [`DerefMut`].
///
/// Obtain one via [`super::world::World::get_tracked_mut`].
pub struct Mut<'a, T> {
    pub(crate) value: &'a mut T,
    pub(crate) changed_tick: &'a mut u32,
    pub(crate) current_tick: u32,
}

impl<'a, T> Mut<'a, T> {
    /// Unconditionally mark this component as changed without accessing the
    /// value (useful with interior-mutability patterns).
    #[inline]
    pub fn set_changed(&mut self) {
        *self.changed_tick = self.current_tick;
    }

    /// Returns `true` if the component has been modified since `since_tick`.
    #[inline]
    pub fn is_changed_since(&self, since_tick: u32) -> bool {
        *self.changed_tick > since_tick
    }

    /// The tick on which this component was last changed.
    #[inline]
    pub fn last_changed_tick(&self) -> u32 {
        *self.changed_tick
    }

    /// Unwrap into a plain `&mut T`, marking the component changed immediately.
    #[inline]
    pub fn into_inner(mut self) -> &'a mut T {
        self.set_changed();
        self.value
    }
}

impl<T> Deref for Mut<'_, T> {
    type Target = T;
    #[inline]
    fn deref(&self) -> &T {
        self.value
    }
}

impl<T> DerefMut for Mut<'_, T> {
    /// Access the value mutably.  This is the point at which `changed_tick`
    /// is set to `current_tick` — if you never dereference mutably, the
    /// component is not considered changed this frame.
    #[inline]
    fn deref_mut(&mut self) -> &mut T {
        *self.changed_tick = self.current_tick;
        self.value
    }
}
