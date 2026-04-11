//! Component access declarations for parallel system scheduling.
//!
//! Each [`System`](super::system::System) can implement `access()` to return a
//! [`SystemAccess`] that describes which component types it reads and which it
//! writes.  [`super::schedule::SystemSchedule`] uses these declarations to
//! group non-conflicting systems into parallel batches.
//!
//! # Rules
//! Two systems **conflict** and must run sequentially if:
//! - Either declares `exclusive` access, OR
//! - One writes a type `T` that the other reads *or* writes.
//!
//! Two systems are **compatible** (can run in parallel) if none of their
//! write sets overlap with each other's read or write sets.
//!
//! # Example
//! ```ignore
//! impl System for MovementSystem {
//!     fn update(&mut self, world: &mut World, dt: f32) { /* … */ }
//!
//!     fn access(&self) -> SystemAccess {
//!         SystemAccess::new()
//!             .write::<Transform>()
//!             .read::<Velocity>()
//!     }
//! }
//! ```

use std::any::TypeId;

/// Describes the component types a system reads and writes each frame.
///
/// Build one with the builder methods [`read`](Self::read) and
/// [`write`](Self::write), or use [`exclusive`](Self::exclusive) for systems
/// that need sole access to the whole world.
#[derive(Clone, Default)]
pub struct SystemAccess {
    pub(crate) reads: Vec<TypeId>,
    pub(crate) writes: Vec<TypeId>,
    /// When `true` the system requires sole exclusive access to the entire
    /// world and cannot run concurrently with any other system.
    pub(crate) exclusive: bool,
}

impl SystemAccess {
    /// No declared reads or writes — the system will be placed into a batch
    /// with other non-conflicting systems.
    pub fn new() -> Self {
        Self::default()
    }

    /// Require exclusive access to the whole world.  Use this (or just accept
    /// the default from [`System::access`](super::system::System::access)) when
    /// a system accesses components that are not known at registration time.
    pub fn exclusive() -> Self {
        Self {
            exclusive: true,
            ..Default::default()
        }
    }

    /// Declare a shared (read-only) access to component type `T`.
    pub fn read<T: 'static>(mut self) -> Self {
        let tid = TypeId::of::<T>();
        if !self.reads.contains(&tid) {
            self.reads.push(tid);
        }
        self
    }

    /// Declare exclusive (read-write) access to component type `T`.
    pub fn write<T: 'static>(mut self) -> Self {
        let tid = TypeId::of::<T>();
        if !self.writes.contains(&tid) {
            self.writes.push(tid);
        }
        self
    }

    /// Returns `true` if running `self` and `other` concurrently would produce
    /// a write conflict on at least one component type.
    pub fn conflicts_with(&self, other: &SystemAccess) -> bool {
        if self.exclusive || other.exclusive {
            return true;
        }
        // self writes T  ⟹  other must not read or write T
        for &tid in &self.writes {
            if other.writes.contains(&tid) || other.reads.contains(&tid) {
                return true;
            }
        }
        // other writes T  ⟹  self must not read T  (write-write already caught above)
        for &tid in &other.writes {
            if self.reads.contains(&tid) {
                return true;
            }
        }
        false
    }
}
