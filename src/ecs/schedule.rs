//! System scheduling with automatic parallel batch execution.
//!
//! [`SystemSchedule`] runs registered systems each frame.  Systems that
//! implement [`System::access`] with non-overlapping component declarations are
//! grouped into **parallel batches** and executed concurrently using
//! [`std::thread::scope`].  Systems that return
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
//! System B — writes Camera,    reads GlobalTransform ┘  → parallel batch
//! System C — exclusive                               → sequential step
//! System D — writes Stats,     reads Transform  ┐
//! System E — writes Inventory                   ┘  → parallel batch
//! ```
//!
//! # Safety contract
//!
//! When two systems run in parallel the scheduler gives each a raw `&mut World`
//! pointer.  Because each `Box<dyn AnyStorage>` inside `World` is a separate
//! heap allocation, concurrent access to **different** component types is
//! memory-safe.  **You are responsible** for ensuring that each system only
//! touches the component types it declared — accessing an undeclared type that
//! another parallel system is writing is a data race.

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
        Self { systems: Vec::new() }
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
    /// 2. Computes parallel batches from access declarations.
    /// 3. Runs each batch: single-system batches run directly; multi-system
    ///    batches are dispatched with [`std::thread::scope`].
    pub fn run(&mut self, world: &mut World, dt: f32) {
        world.advance_tick();

        // Compute batch ranges once per frame — O(n²) in the number of
        // systems, negligible for typical counts (< 100).
        let batches = compute_batches(&self.systems);

        for range in batches {
            let slice = &mut self.systems[range];
            if slice.len() == 1 {
                slice[0].0.update(world, dt);
            } else {
                run_parallel(slice, world, dt);
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

// ── Parallel dispatch ─────────────────────────────────────────────────────────

/// Run every system in `slice` concurrently using [`std::thread::scope`].
///
/// # Safety
///
/// The caller (i.e. `compute_batches`) has verified that no two systems in
/// `slice` have a write conflict on any component TypeId.  Each component
/// storage (`SparseSet<T>`) is a distinct heap allocation accessed through a
/// separate `Box<dyn AnyStorage>` entry in the `World`'s HashMap, so
/// concurrent access to different TypeIds cannot alias.
///
/// Systems are trusted to access only the component types they declared.
fn run_parallel(
    slice: &mut [(Box<dyn System>, SystemAccess)],
    world: &mut World,
    dt: f32,
) {
    // Cast raw pointers to `usize` so closures are `Send` — `usize` is always
    // `Send`.  The safety argument is in the function-level doc comment.
    let world_raw = world as *mut World as usize;

    std::thread::scope(|scope| {
        for (system, _access) in slice.iter_mut() {
            // Convert both pointers to usize so the closures are Send.
            let sys_raw = system as *mut Box<dyn System> as usize;
            let w_raw   = world_raw;

            scope.spawn(move || {
                // SAFETY: see function-level comment.
                let w = unsafe { &mut *(w_raw   as *mut World) };
                let s = unsafe { &mut *(sys_raw as *mut Box<dyn System>) };
                s.update(w, dt);
            });
        }
    });
    // thread::scope guarantees all threads have finished here.
}
