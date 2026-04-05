use std::any::{Any, TypeId};
use std::collections::HashMap;
use super::entity::Entity;
use super::system::System;

// One sparse map per component type: entity → boxed component.
type ComponentMap = HashMap<Entity, Box<dyn Any>>;

pub struct World {
    next_id: u64,
    components: HashMap<TypeId, ComponentMap>,
    systems: Vec<Box<dyn System>>,
}

impl World {
    pub fn new() -> Self {
        Self {
            next_id: 0,
            components: HashMap::new(),
            systems: Vec::new(),
        }
    }

    // ── Entities ─────────────────────────────────────────────────────────────

    /// Allocate a new entity with no components.
    pub fn spawn(&mut self) -> Entity {
        let e = Entity::new(self.next_id);
        self.next_id += 1;
        e
    }

    /// Remove an entity and all of its components.
    pub fn despawn(&mut self, entity: Entity) {
        for storage in self.components.values_mut() {
            storage.remove(&entity);
        }
    }

    // ── Components ────────────────────────────────────────────────────────────

    pub fn add_component<C: Any>(&mut self, entity: Entity, component: C) {
        self.components
            .entry(TypeId::of::<C>())
            .or_default()
            .insert(entity, Box::new(component));
    }

    pub fn remove_component<C: Any>(&mut self, entity: Entity) {
        if let Some(storage) = self.components.get_mut(&TypeId::of::<C>()) {
            storage.remove(&entity);
        }
    }

    pub fn has_component<C: Any>(&self, entity: Entity) -> bool {
        self.components
            .get(&TypeId::of::<C>())
            .map_or(false, |s| s.contains_key(&entity))
    }

    pub fn get<C: Any>(&self, entity: Entity) -> Option<&C> {
        self.components
            .get(&TypeId::of::<C>())?
            .get(&entity)?
            .downcast_ref::<C>()
    }

    pub fn get_mut<C: Any>(&mut self, entity: Entity) -> Option<&mut C> {
        self.components
            .get_mut(&TypeId::of::<C>())?
            .get_mut(&entity)?
            .downcast_mut::<C>()
    }

    // ── Queries ───────────────────────────────────────────────────────────────

    /// All entities that have component C, with a shared reference.
    pub fn query<C: Any>(&self) -> Vec<(Entity, &C)> {
        match self.components.get(&TypeId::of::<C>()) {
            None => Vec::new(),
            Some(storage) => storage
                .iter()
                .filter_map(|(&e, c)| c.downcast_ref::<C>().map(|c| (e, c)))
                .collect(),
        }
    }

    /// All entities that have component C, with an exclusive reference.
    pub fn query_mut<C: Any>(&mut self) -> Vec<(Entity, &mut C)> {
        match self.components.get_mut(&TypeId::of::<C>()) {
            None => Vec::new(),
            Some(storage) => storage
                .iter_mut()
                .filter_map(|(&e, c)| c.downcast_mut::<C>().map(|c| (e, c)))
                .collect(),
        }
    }

    /// Entities that have BOTH component A and component B.
    /// Returns owned copies of the entity IDs and immutable references.
    pub fn query2<A: Any, B: Any>(&self) -> Vec<(Entity, &A, &B)> {
        let a_storage = match self.components.get(&TypeId::of::<A>()) {
            Some(s) => s,
            None => return Vec::new(),
        };
        let b_storage = match self.components.get(&TypeId::of::<B>()) {
            Some(s) => s,
            None => return Vec::new(),
        };
        a_storage
            .iter()
            .filter_map(|(&e, a)| {
                let a = a.downcast_ref::<A>()?;
                let b = b_storage.get(&e)?.downcast_ref::<B>()?;
                Some((e, a, b))
            })
            .collect()
    }

    // ── Systems ───────────────────────────────────────────────────────────────

    pub fn add_system<S: System>(&mut self, system: S) {
        self.systems.push(Box::new(system));
    }

    /// Run all registered systems. Uses `take` to avoid borrow conflicts.
    pub fn update(&mut self, dt: f32) {
        let mut systems = std::mem::take(&mut self.systems);
        for system in &mut systems {
            system.update(self, dt);
        }
        self.systems = systems;
    }
}

impl Default for World {
    fn default() -> Self { Self::new() }
}
