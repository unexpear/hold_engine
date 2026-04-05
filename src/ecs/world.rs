use std::any::{Any, TypeId};
use std::collections::HashMap;

use super::entity::Entity;
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
            meta.generation += 1;
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
    /// This is O(|A| × log(|B|)) rather than O(|A| ∩ |B|).  Archetype tables
    /// are the correct long-term fix.
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

    // ── Hierarchy helpers ─────────────────────────────────────────────────────

    /// Attach `child` to `parent`, maintaining both Parent and Children components.
    pub fn set_parent(&mut self, child: Entity, parent: Entity) {
        use super::components::{Children, Parent};
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
