//! System scheduling with automatic parallel batch execution.
//!
//! [`SystemSchedule`] runs registered systems each frame.  Systems that
//! implement [`System::access`] with non-overlapping component declarations are
//! grouped into batches for future scheduling policy decisions. Systems that return
//! [`SystemAccess::exclusive`](super::access::SystemAccess::exclusive) (the
//! default) always run alone in their own sequential step.
//!
//! # How batching works
//!
//! The scheduler processes the system list in registration order and greedily
//! extends the current batch as long as no access conflict exists:
//!
//! ```text
//! System A — writes Transform, reads Velocity   ┐
//! System B — writes Camera,    reads GlobalTransform ┘  → same batch
//! System C — exclusive                               → sequential step
//! System D — writes Stats,     reads Transform  ┐
//! System E — writes Inventory                   ┘  → same batch
//! ```
//!
//! # Safety
//!
//! `System::update` takes `&mut World`, so a system may mutate global world
//! state beyond component storages (`spawn`, `despawn`, event queues, etc.).
//! To keep the API safe without introducing an unsafe contract, execution is
//! currently serialized even within non-conflicting batches.

use std::ops::Range;

use super::access::SystemAccess;
use super::system::System;
use super::world::World;

// ── SystemSchedule ────────────────────────────────────────────────────────────

/// Owns and runs a collection of systems, grouping non-conflicting ones into
/// parallel batches each frame.
pub struct SystemSchedule {
    /// Paired `(system, access)` so we never need a second lookup.
    systems: Vec<(Box<dyn System>, SystemAccess)>,
}

impl SystemSchedule {
    pub fn new() -> Self {
        Self {
            systems: Vec::new(),
        }
    }

    /// Register a system.  Its [`access`](System::access) descriptor is
    /// captured once here and reused every frame for batch computation.
    pub fn add_system<S: System>(&mut self, system: S) {
        let access = system.access();
        self.systems.push((Box::new(system), access));
    }

    /// Run all systems for one frame.
    ///
    /// 1. Advances the world tick.
    /// 2. Computes non-conflicting batches from access declarations.
    /// 3. Runs each batch sequentially.
    pub fn run(&mut self, world: &mut World, dt: f32) {
        world.advance_tick();

        // Compute batch ranges once per frame — O(n²) in the number of
        // systems, negligible for typical counts (< 100).
        let batches = compute_batches(&self.systems);

        for range in batches {
            let slice = &mut self.systems[range];
            for (system, _access) in slice.iter_mut() {
                system.update(world, dt);
            }
        }
    }
}

impl Default for SystemSchedule {
    fn default() -> Self {
        Self::new()
    }
}

// ── Batch computation ─────────────────────────────────────────────────────────

/// Partition `systems` into maximal contiguous groups where no two members
/// have a write conflict.  Exclusive systems always form a singleton group.
fn compute_batches(systems: &[(Box<dyn System>, SystemAccess)]) -> Vec<Range<usize>> {
    let mut batches = Vec::new();
    let mut i = 0;

    while i < systems.len() {
        if systems[i].1.exclusive {
            batches.push(i..i + 1);
            i += 1;
            continue;
        }

        // Try to grow the batch greedily.
        let start = i;
        let mut batch: Vec<&SystemAccess> = vec![&systems[i].1];
        i += 1;

        while i < systems.len() {
            let candidate = &systems[i].1;
            if candidate.exclusive {
                break;
            }
            // Conflict with any existing batch member → start a new batch.
            if batch.iter().any(|a| a.conflicts_with(candidate)) {
                break;
            }
            batch.push(candidate);
            i += 1;
        }

        batches.push(start..start + batch.len());
    }

    batches
}
