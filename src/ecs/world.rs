use std::any::{Any, TypeId};
use std::collections::HashMap;

use super::entity::Entity;
use super::query::{Query2, Query2Mut, Query3, Query3Mut};
use super::sparse_set::{SparseSet, SparseSetIter, SparseSetIterMut};

// ── EntityMeta ────────────────────────────────────────────────────────────────

struct EntityMeta {
    generation: u32,
    alive:      bool,
}

// ── AnyStorage trait ──────────────────────────────────────────────────────────

/// Object-safe trait so `World` can erase the component type yet still call
/// `remove_entity` when an entity is despawned.
trait AnyStorage: 'static {
    fn remove_entity(&mut self, entity: Entity);
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

impl<T: 'static> AnyStorage for SparseSet<T> {
    fn remove_entity(&mut self, entity: Entity) {
        self.remove(entity);
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

// ── World ─────────────────────────────────────────────────────────────────────

/// The ECS world.  Owns all entity metadata and component storages.
///
/// Systems are **not** stored here; use [`super::schedule::SystemSchedule`]
/// for that.
pub struct World {
    entities:   Vec<EntityMeta>,
    free_list:  Vec<u32>,
    components: HashMap<TypeId, Box<dyn AnyStorage>>,
}

impl World {
    pub fn new() -> Self {
        Self {
            entities:   Vec::new(),
            free_list:  Vec::new(),
            components: HashMap::new(),
        }
    }

    // ── Entity lifecycle ──────────────────────────────────────────────────────

    /// Allocate a new entity (reusing a recycled slot when possible).
    pub fn spawn(&mut self) -> Entity {
        if let Some(index) = self.free_list.pop() {
            let meta = &mut self.entities[index as usize];
            // Bump generation so old handles become invalid.
            // Saturate at u32::MAX — slot becomes permanently dead rather than
            // silently wrapping and aliasing a generation-0 handle.
            meta.generation = meta.generation.saturating_add(1);
            if meta.generation == u32::MAX {
                // Slot exhausted; leave it dead and fall through to allocating a new one.
                self.entities.push(EntityMeta { generation: 0, alive: true });
                let new_index = (self.entities.len() - 1) as u32;
                return Entity::new(new_index, 0);
            }
            meta.alive = true;
            Entity::new(index, meta.generation)
        } else {
            let index = self.entities.len() as u32;
            self.entities.push(EntityMeta { generation: 0, alive: true });
            Entity::new(index, 0)
        }
    }

    /// Destroy an entity: checks the generation so stale handles are ignored.
    pub fn despawn(&mut self, entity: Entity) {
        if !self.is_alive(entity) {
            return;
        }
        // Remove from every component storage.
        for storage in self.components.values_mut() {
            storage.remove_entity(entity);
        }
        let meta = &mut self.entities[entity.index as usize];
        meta.alive = false;
        self.free_list.push(entity.index);
    }

    /// Returns `true` if the handle refers to a currently-alive entity.
    pub fn is_alive(&self, entity: Entity) -> bool {
        let idx = entity.index as usize;
        idx < self.entities.len()
            && self.entities[idx].alive
            && self.entities[idx].generation == entity.generation
    }

    // ── Components ────────────────────────────────────────────────────────────

    /// Attach (or replace) a component on an entity.
    pub fn add_component<C: 'static>(&mut self, entity: Entity, component: C) {
        if !self.is_alive(entity) {
            return;
        }
        self.storage_mut::<C>().insert(entity, component);
    }

    /// Detach a component from an entity (no-op if absent).
    pub fn remove_component<C: 'static>(&mut self, entity: Entity) {
        if let Some(storage) = self.components.get_mut(&TypeId::of::<C>()) {
            if let Some(set) = storage.as_any_mut().downcast_mut::<SparseSet<C>>() {
                set.remove(entity);
            }
        }
    }

    pub fn has_component<C: 'static>(&self, entity: Entity) -> bool {
        self.get::<C>(entity).is_some()
    }

    /// Shared reference to a component, with generation check.
    pub fn get<C: 'static>(&self, entity: Entity) -> Option<&C> {
        if !self.is_alive(entity) {
            return None;
        }
        self.storage_ref::<C>()?.get(entity)
    }

    /// Exclusive reference to a component, with generation check.
    pub fn get_mut<C: 'static>(&mut self, entity: Entity) -> Option<&mut C> {
        if !self.is_alive(entity) {
            return None;
        }
        self.storage_mut::<C>().get_mut(entity)
    }

    // ── Queries ───────────────────────────────────────────────────────────────

    /// Iterate every entity that has component `C`.
    ///
    /// Iterates the dense array directly — no Vec allocation, no indirection.
    ///
    /// **Multi-component queries:** for now, compose with `get`:
    /// ```ignore
    /// world.query::<A>()
    ///      .filter_map(|(e, a)| world.get::<B>(e).map(|b| (e, a, b)))
    /// ```
    /// This is still O(|A|) with O(1) sparse lookups into `B`, but it does extra
    /// random-access probing compared with dedicated join queries.
    /// Archetype/table-style joins are the correct long-term fix.
    pub fn query<C: 'static>(&self) -> SparseSetIter<'_, C> {
        match self.storage_ref::<C>() {
            Some(set) => set.iter(),
            None      => SparseSetIter::empty(),
        }
    }

    /// Mutable variant of [`query`].
    pub fn query_mut<C: 'static>(&mut self) -> SparseSetIterMut<'_, C> {
        self.storage_mut::<C>().iter_mut()
    }

    // ── Multi-component queries ───────────────────────────────────────────────

    /// Borrow two component storages immutably for joint iteration.
    ///
    /// Returns `None` if either type has no storage yet (no entity carries it).
    ///
    /// ```ignore
    /// if let Some(q) = world.query2::<Transform, Camera>() {
    ///     for (entity, transform, camera) in q.iter() { ... }
    /// }
    /// ```
    pub fn query2<A: 'static, B: 'static>(&self) -> Option<Query2<'_, A, B>> {
        let a = self.storage_ref::<A>()?;
        let b = self.storage_ref::<B>()?;
        Some(Query2 { a, b })
    }

    /// Borrow three component storages immutably for joint iteration.
    ///
    /// Returns `None` if any of the three types has no storage yet.
    pub fn query3<A: 'static, B: 'static, C: 'static>(&self) -> Option<Query3<'_, A, B, C>> {
        let a = self.storage_ref::<A>()?;
        let b = self.storage_ref::<B>()?;
        let c = self.storage_ref::<C>()?;
        Some(Query3 { a, b, c })
    }

    /// Borrow `A` mutably and `B` immutably for joint iteration.
    ///
    /// Returns `None` if either storage is absent.
    ///
    /// # Panics
    /// Panics if `A` and `B` are the same type — a type cannot be borrowed
    /// mutably and immutably at the same time.
    ///
    /// ```ignore
    /// if let Some(mut q) = world.query2_mut::<Transform, Velocity>() {
    ///     for (_e, transform, velocity) in q.iter_mut() {
    ///         transform.position = transform.position + velocity.0 * dt;
    ///     }
    /// }
    /// ```
    pub fn query2_mut<A: 'static, B: 'static>(&mut self) -> Option<Query2Mut<'_, A, B>> {
        assert!(
            TypeId::of::<A>() != TypeId::of::<B>(),
            "query2_mut: A and B must be distinct component types"
        );

        // We need &mut SparseSet<A> and &SparseSet<B> simultaneously.
        // Since they live under different TypeId keys they are distinct heap
        // allocations (each Box<dyn AnyStorage> is its own object).
        // We acquire raw pointers sequentially — each HashMap borrow ends at
        // the semicolon — then reconstruct typed references.
        //
        // SAFETY: a_ptr and b_ptr point to non-overlapping memory (different
        // TypeIds → different HashMap entries → different Box allocations).
        // The returned Query2Mut is lifetime-bound to `&mut self`, preventing
        // the World from being accessed while the query lives.
        let a_ptr = self.components
            .get_mut(&TypeId::of::<A>())?
            .as_any_mut()
            .downcast_mut::<SparseSet<A>>()?
            as *mut SparseSet<A>;

        let b_ptr = self.components
            .get(&TypeId::of::<B>())?
            .as_any()
            .downcast_ref::<SparseSet<B>>()?
            as *const SparseSet<B>;

        Some(Query2Mut {
            a: unsafe { &mut *a_ptr },
            b: unsafe { &*b_ptr },
        })
    }

    /// Borrow `A` mutably and `B`, `C` immutably for joint iteration.
    ///
    /// Returns `None` if any storage is absent.
    ///
    /// # Panics
    /// Panics if any two of `A`, `B`, `C` are the same type.
    pub fn query3_mut<A: 'static, B: 'static, C: 'static>(
        &mut self,
    ) -> Option<Query3Mut<'_, A, B, C>> {
        let a_tid = TypeId::of::<A>();
        let b_tid = TypeId::of::<B>();
        let c_tid = TypeId::of::<C>();
        assert!(
            a_tid != b_tid && a_tid != c_tid && b_tid != c_tid,
            "query3_mut: A, B, and C must all be distinct component types"
        );

        // Same safety argument as query2_mut — three distinct TypeIds mean
        // three non-overlapping Box allocations.
        let a_ptr = self.components
            .get_mut(&a_tid)?
            .as_any_mut()
            .downcast_mut::<SparseSet<A>>()?
            as *mut SparseSet<A>;

        let b_ptr = self.components
            .get(&b_tid)?
            .as_any()
            .downcast_ref::<SparseSet<B>>()?
            as *const SparseSet<B>;

        let c_ptr = self.components
            .get(&c_tid)?
            .as_any()
            .downcast_ref::<SparseSet<C>>()?
            as *const SparseSet<C>;

        Some(Query3Mut {
            a: unsafe { &mut *a_ptr },
            b: unsafe { &*b_ptr },
            c: unsafe { &*c_ptr },
        })
    }

    // ── Filter helpers ────────────────────────────────────────────────────────

    /// Iterate entities that have `A` and **also** carry `B` (without reading `B`).
    ///
    /// ```ignore
    /// // All entities that have both a Transform and an ActiveCamera marker.
    /// for (entity, transform) in world.query_with::<Transform, ActiveCamera>() { ... }
    /// ```
    pub fn query_with<A: 'static, B: 'static>(
        &self,
    ) -> impl Iterator<Item = (Entity, &A)> + '_ {
        let b_set = self.storage_ref::<B>();
        self.query::<A>().filter(move |(e, _)| {
            b_set.map_or(false, |s| s.contains(*e))
        })
    }

    /// Iterate entities that have `A` but do **not** carry `B`.
    ///
    /// ```ignore
    /// // Root entities: have Transform but no Parent.
    /// for (entity, transform) in world.query_without::<Transform, Parent>() { ... }
    /// ```
    pub fn query_without<A: 'static, B: 'static>(
        &self,
    ) -> impl Iterator<Item = (Entity, &A)> + '_ {
        let b_set = self.storage_ref::<B>();
        self.query::<A>().filter(move |(e, _)| {
            b_set.map_or(true, |s| !s.contains(*e))
        })
    }

    // ── Hierarchy helpers ─────────────────────────────────────────────────────

    /// Attach `child` to `parent`, maintaining both Parent and Children components.
    pub fn set_parent(&mut self, child: Entity, parent: Entity) {
        use super::components::{Children, Parent};
        if child == parent || !self.is_alive(child) || !self.is_alive(parent) {
            return;
        }
        // Remove from old parent if any.
        if let Some(old_parent) = self.get::<Parent>(child).map(|p| p.entity) {
            if let Some(children) = self.get_mut::<Children>(old_parent) {
                children.list.retain(|&e| e != child);
            }
        }
        self.add_component(child, Parent { entity: parent });
        // Add child to new parent's Children (or create it).
        let has_children = self.get::<Children>(parent).is_some();
        if has_children {
            if let Some(children) = self.get_mut::<Children>(parent) {
                children.list.push(child);
            }
        } else {
            self.add_component(parent, Children { list: vec![child] });
        }
    }

    /// Detach `child` from its parent.
    pub fn remove_parent(&mut self, child: Entity) {
        use super::components::{Children, Parent};
        if let Some(old_parent) = self.get::<Parent>(child).map(|p| p.entity) {
            if let Some(children) = self.get_mut::<Children>(old_parent) {
                children.list.retain(|&e| e != child);
            }
            self.remove_component::<Parent>(child);
        }
    }

    // ── Internal helpers ──────────────────────────────────────────────────────

    /// Get or create the `SparseSet<C>` for component type `C`.
    fn storage_mut<C: 'static>(&mut self) -> &mut SparseSet<C> {
        let entry = self.components
            .entry(TypeId::of::<C>())
            .or_insert_with(|| Box::new(SparseSet::<C>::new()));

        entry
            .as_any_mut()
            .downcast_mut::<SparseSet<C>>()
            .expect("component storage downcast should match its TypeId")
    }

    /// Read-only access to `SparseSet<C>`, or `None` if no component of that
    /// type has ever been registered.
    fn storage_ref<C: 'static>(&self) -> Option<&SparseSet<C>> {
        let entry = self.components.get(&TypeId::of::<C>())?;

        entry.as_any().downcast_ref::<SparseSet<C>>()
    }
}

impl Default for World {
    fn default() -> Self {
        Self::new()
    }
}
