//! Multi-component query types for [`super::world::World`].
//!
//! Obtain query borrows through the `World::query2`, `World::query3`,
//! `World::query2_mut`, and `World::query3_mut` methods.
//!
//! # Examples
//!
//! ```ignore
//! // Read two components together — no collect() needed.
//! if let Some(q) = world.query2::<Transform, Camera>() {
//!     for (entity, transform, camera) in q.iter() {
//!         println!("{:?} fov={}", entity, camera.fov);
//!     }
//! }
//!
//! // Write one component while reading another.
//! if let Some(mut q) = world.query2_mut::<Transform, Velocity>() {
//!     for (_entity, transform, velocity) in q.iter_mut() {
//!         transform.position = transform.position + velocity.0 * dt;
//!     }
//! }
//!
//! // Optional second component — iterate A, pair with B when present.
//! if let Some(q) = world.query2::<Stats, Inventory>() {
//!     for (entity, stats, maybe_inv) in q.iter_opt_b() {
//!         // maybe_inv: Option<&Inventory>
//!     }
//! }
//!
//! // Filter without reading — entities that have Transform but not Parent.
//! for (entity, transform) in world.query_without::<Transform, Parent>() { }
//! ```

use super::entity::Entity;
use super::sparse_set::SparseSet;

// ── Query2 — immutable borrow of two component storages ───────────────────────

/// Immutable borrow of two component storages for joint iteration.
///
/// Created by [`super::world::World::query2`].
pub struct Query2<'w, A: 'static, B: 'static> {
    pub(super) a: &'w SparseSet<A>,
    pub(super) b: &'w SparseSet<B>,
}

impl<'w, A: 'static, B: 'static> Query2<'w, A, B> {
    /// Iterate every entity that has **both** `A` and `B`.
    ///
    /// Iterates the `A` storage and performs O(1) lookups into `B` — O(|A|) total.
    pub fn iter(&self) -> impl Iterator<Item = (Entity, &'w A, &'w B)> + '_ {
        let b = self.b;
        self.a
            .iter()
            .filter_map(move |(e, a)| b.get(e).map(|bv| (e, a, bv)))
    }

    /// Iterate every entity that has `A`, pairing with `B` if present.
    ///
    /// Useful for optional relationships — the component `B` may or may not
    /// exist on any given entity.
    pub fn iter_opt_b(&self) -> impl Iterator<Item = (Entity, &'w A, Option<&'w B>)> + '_ {
        let b = self.b;
        self.a.iter().map(move |(e, a)| (e, a, b.get(e)))
    }

    /// Number of components in the `A` storage (upper bound for matched entities).
    #[inline]
    pub fn len(&self) -> usize {
        self.a.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.a.is_empty()
    }
}

// ── Query3 — immutable borrow of three component storages ─────────────────────

/// Immutable borrow of three component storages for joint iteration.
///
/// Created by [`super::world::World::query3`].
pub struct Query3<'w, A: 'static, B: 'static, C: 'static> {
    pub(super) a: &'w SparseSet<A>,
    pub(super) b: &'w SparseSet<B>,
    pub(super) c: &'w SparseSet<C>,
}

impl<'w, A: 'static, B: 'static, C: 'static> Query3<'w, A, B, C> {
    /// Iterate every entity that has **all three** components.
    pub fn iter(&self) -> impl Iterator<Item = (Entity, &'w A, &'w B, &'w C)> + '_ {
        let b = self.b;
        let c = self.c;
        self.a
            .iter()
            .filter_map(move |(e, a)| b.get(e).and_then(|bv| c.get(e).map(|cv| (e, a, bv, cv))))
    }

    /// Iterate `A` and `B` together, pairing with `C` if present.
    pub fn iter_opt_c(&self) -> impl Iterator<Item = (Entity, &'w A, &'w B, Option<&'w C>)> + '_ {
        let b = self.b;
        let c = self.c;
        self.a
            .iter()
            .filter_map(move |(e, a)| b.get(e).map(|bv| (e, a, bv, c.get(e))))
    }
}

// ── Query2Mut — mutable A, immutable B ───────────────────────────────────────

/// Mutable borrow of `A` and immutable borrow of `B` for joint iteration.
///
/// Created by [`super::world::World::query2_mut`].
///
/// Use [`iter_mut`](Self::iter_mut) to get `&mut A` alongside `&B`.
/// Use [`iter`](Self::iter) when you only need shared access to both.
pub struct Query2Mut<'w, A: 'static, B: 'static> {
    pub(super) a: &'w mut SparseSet<A>,
    pub(super) b: &'w SparseSet<B>,
}

impl<'w, A: 'static, B: 'static> Query2Mut<'w, A, B> {
    /// Iterate every entity with both `A` (mutable) and `B` (shared).
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (Entity, &mut A, &B)> + '_ {
        let b: &SparseSet<B> = self.b;
        self.a
            .iter_mut()
            .filter_map(move |(e, a)| b.get(e).map(|bv| (e, a, bv)))
    }

    /// Iterate every entity with both `A` (shared) and `B` (shared).
    pub fn iter(&self) -> impl Iterator<Item = (Entity, &A, &B)> + '_ {
        let b = self.b;
        self.a
            .iter()
            .filter_map(move |(e, a)| b.get(e).map(|bv| (e, a, bv)))
    }

    /// Iterate `A` (mutable), pairing with `B` if present.
    pub fn iter_mut_opt_b(&mut self) -> impl Iterator<Item = (Entity, &mut A, Option<&B>)> + '_ {
        let b: &SparseSet<B> = self.b;
        self.a.iter_mut().map(move |(e, a)| (e, a, b.get(e)))
    }
}

// ── Query3Mut — mutable A, immutable B and C ─────────────────────────────────

/// Mutable borrow of `A` and immutable borrows of `B` and `C`.
///
/// Created by [`super::world::World::query3_mut`].
pub struct Query3Mut<'w, A: 'static, B: 'static, C: 'static> {
    pub(super) a: &'w mut SparseSet<A>,
    pub(super) b: &'w SparseSet<B>,
    pub(super) c: &'w SparseSet<C>,
}

impl<'w, A: 'static, B: 'static, C: 'static> Query3Mut<'w, A, B, C> {
    /// Iterate every entity with all three components; `A` mutable.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (Entity, &mut A, &B, &C)> + '_ {
        let b: &SparseSet<B> = self.b;
        let c: &SparseSet<C> = self.c;
        self.a
            .iter_mut()
            .filter_map(move |(e, a)| b.get(e).and_then(|bv| c.get(e).map(|cv| (e, a, bv, cv))))
    }

    /// Iterate every entity with all three components; all shared.
    pub fn iter(&self) -> impl Iterator<Item = (Entity, &A, &B, &C)> + '_ {
        let b = self.b;
        let c = self.c;
        self.a
            .iter()
            .filter_map(move |(e, a)| b.get(e).and_then(|bv| c.get(e).map(|cv| (e, a, bv, cv))))
    }

    /// Iterate `A` (mutable) and `B` (shared), pairing `C` if present.
    pub fn iter_mut_opt_c(
        &mut self,
    ) -> impl Iterator<Item = (Entity, &mut A, &B, Option<&C>)> + '_ {
        let b: &SparseSet<B> = self.b;
        let c: &SparseSet<C> = self.c;
        self.a
            .iter_mut()
            .filter_map(move |(e, a)| b.get(e).map(|bv| (e, a, bv, c.get(e))))
    }
}
