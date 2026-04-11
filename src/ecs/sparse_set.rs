use super::change::Mut;
use super::entity::Entity;

/// Sentinel value stored in `sparse` when an entity has no component.
const ABSENT: u32 = u32::MAX;

/// Hard cap on entity indices to prevent accidental gigabyte allocations.
/// 4 million entities is well above any practical usage while keeping the
/// sparse array under 16 MB.
const MAX_ENTITY_INDEX: usize = 4_000_000;

/// A sparse-set storage for a single component type `T`.
///
/// Layout
/// ------
/// * `sparse`        — indexed by `entity.index`; value is the position in `dense`
///                     (or `ABSENT` if not present).
/// * `dense`         — tightly packed component values; cache-friendly for iteration.
/// * `entities`      — parallel to `dense`; maps a dense index back to its Entity.
/// * `added_ticks`   — parallel to `dense`; the world tick when the component was
///                     first attached to that entity.
/// * `changed_ticks` — parallel to `dense`; the world tick when the component was
///                     last written (via [`insert`](Self::insert) or a [`Mut`] deref).
///
/// All five arrays are kept in sync on every insert/remove.
pub struct SparseSet<T> {
    sparse: Vec<u32>,
    dense: Vec<T>,
    entities: Vec<Entity>,
    added_ticks: Vec<u32>,
    changed_ticks: Vec<u32>,
}

impl<T> SparseSet<T> {
    pub fn new() -> Self {
        Self {
            sparse: Vec::new(),
            dense: Vec::new(),
            entities: Vec::new(),
            added_ticks: Vec::new(),
            changed_ticks: Vec::new(),
        }
    }

    // ── Core operations ───────────────────────────────────────────────────────

    /// Insert (or overwrite) a component for `entity`.
    ///
    /// * On a **new** insert both `added_tick` and `changed_tick` are set to `tick`.
    /// * On an **overwrite** only `changed_tick` is updated; `added_tick` stays at
    ///   the original insertion tick.
    pub fn insert(&mut self, entity: Entity, value: T, tick: u32) {
        let idx = entity.index as usize;
        assert!(
            idx < MAX_ENTITY_INDEX,
            "entity index {} exceeds MAX_ENTITY_INDEX",
            idx
        );

        // Grow sparse array if needed, filling gaps with ABSENT.
        if idx >= self.sparse.len() {
            self.sparse.resize(idx + 1, ABSENT);
        }

        let slot = self.sparse[idx];
        if slot != ABSENT {
            // Overwrite — keep added_tick, update changed_tick.
            let s = slot as usize;
            self.dense[s] = value;
            self.entities[s] = entity;
            self.changed_ticks[s] = tick;
        } else {
            // New insert — set both ticks.
            let dense_pos = self.dense.len() as u32;
            self.sparse[idx] = dense_pos;
            self.dense.push(value);
            self.entities.push(entity);
            self.added_ticks.push(tick);
            self.changed_ticks.push(tick);
        }
    }

    /// Remove the component for `entity`. Returns `true` if it was present.
    ///
    /// Uses swap-remove so the dense array stays packed — O(1).
    pub fn remove(&mut self, entity: Entity) -> bool {
        let idx = entity.index as usize;
        if idx >= self.sparse.len() {
            return false;
        }
        let slot = self.sparse[idx];
        if slot == ABSENT {
            return false;
        }
        let slot = slot as usize;

        // Mark the removed entity as absent in sparse.
        self.sparse[idx] = ABSENT;

        let last = self.dense.len() - 1;
        if slot != last {
            // Swap the last dense element into the vacated slot.
            let moved_entity = self.entities[last];
            self.sparse[moved_entity.index as usize] = slot as u32;
            self.dense.swap(slot, last);
            self.entities.swap(slot, last);
            self.added_ticks.swap(slot, last);
            self.changed_ticks.swap(slot, last);
        }

        self.dense.pop();
        self.entities.pop();
        self.added_ticks.pop();
        self.changed_ticks.pop();
        true
    }

    pub fn contains(&self, entity: Entity) -> bool {
        let idx = entity.index as usize;
        idx < self.sparse.len() && self.sparse[idx] != ABSENT
    }

    pub fn get(&self, entity: Entity) -> Option<&T> {
        let idx = entity.index as usize;
        if idx >= self.sparse.len() {
            return None;
        }
        let slot = self.sparse[idx];
        if slot == ABSENT {
            None
        } else {
            Some(&self.dense[slot as usize])
        }
    }

    pub fn get_mut(&mut self, entity: Entity) -> Option<&mut T> {
        let idx = entity.index as usize;
        if idx >= self.sparse.len() {
            return None;
        }
        let slot = self.sparse[idx];
        if slot == ABSENT {
            None
        } else {
            Some(&mut self.dense[slot as usize])
        }
    }

    /// Return a [`Mut<T>`] guard that tracks the `changed_tick` when written through.
    pub fn get_tracked_mut(&mut self, entity: Entity, current_tick: u32) -> Option<Mut<'_, T>> {
        let idx = entity.index as usize;
        if idx >= self.sparse.len() {
            return None;
        }
        let slot = self.sparse[idx];
        if slot == ABSENT {
            return None;
        }
        let s = slot as usize;
        Some(Mut {
            value: &mut self.dense[s],
            changed_tick: &mut self.changed_ticks[s],
            current_tick,
        })
    }

    // ── Tick queries ──────────────────────────────────────────────────────────

    /// Iterate components whose `added_tick` is **strictly greater than** `since`.
    ///
    /// Pass `system.last_run_tick` to see components added since the system
    /// last ran.
    pub fn iter_added_since(&self, since: u32) -> SparseSetTickIter<'_, T> {
        SparseSetTickIter {
            entities: &self.entities,
            dense: &self.dense,
            ticks: &self.added_ticks,
            since,
            pos: 0,
        }
    }

    /// Iterate components whose `changed_tick` is **strictly greater than** `since`.
    ///
    /// A component is "changed" if it was inserted or if a [`Mut`] guard was
    /// dereferenced mutably since the given tick.
    pub fn iter_changed_since(&self, since: u32) -> SparseSetTickIter<'_, T> {
        SparseSetTickIter {
            entities: &self.entities,
            dense: &self.dense,
            ticks: &self.changed_ticks,
            since,
            pos: 0,
        }
    }

    // ── Iteration ─────────────────────────────────────────────────────────────

    /// Iterate all `(Entity, &T)` pairs in dense order (cache-friendly).
    pub fn iter(&self) -> SparseSetIter<'_, T> {
        SparseSetIter {
            entities: &self.entities,
            dense: &self.dense,
            pos: 0,
        }
    }

    /// Iterate all `(Entity, &mut T)` pairs in dense order.
    pub fn iter_mut(&mut self) -> SparseSetIterMut<'_, T> {
        SparseSetIterMut {
            entities: self.entities.iter(),
            dense: self.dense.iter_mut(),
        }
    }

    /// Number of components stored.
    pub fn len(&self) -> usize {
        self.dense.len()
    }

    pub fn is_empty(&self) -> bool {
        self.dense.is_empty()
    }
}

impl<T> Default for SparseSet<T> {
    fn default() -> Self {
        Self::new()
    }
}

// ── SparseSetIter (all components) ────────────────────────────────────────────

pub struct SparseSetIter<'a, T> {
    entities: &'a [Entity],
    dense: &'a [T],
    pos: usize,
}

impl<'a, T> SparseSetIter<'a, T> {
    /// An iterator that yields nothing (used when no storage exists for a type).
    pub fn empty() -> Self {
        Self {
            entities: &[],
            dense: &[],
            pos: 0,
        }
    }
}

impl<'a, T> Iterator for SparseSetIter<'a, T> {
    type Item = (Entity, &'a T);

    fn next(&mut self) -> Option<Self::Item> {
        if self.pos >= self.dense.len() {
            return None;
        }
        let entity = self.entities[self.pos];
        let value = &self.dense[self.pos];
        self.pos += 1;
        Some((entity, value))
    }
}

// ── SparseSetIterMut (all components, mutable) ────────────────────────────────

pub struct SparseSetIterMut<'a, T> {
    entities: std::slice::Iter<'a, Entity>,
    dense: std::slice::IterMut<'a, T>,
}

impl<'a, T> Iterator for SparseSetIterMut<'a, T> {
    type Item = (Entity, &'a mut T);

    fn next(&mut self) -> Option<Self::Item> {
        let entity = *self.entities.next()?;
        let value = self.dense.next()?;
        Some((entity, value))
    }
}

// ── SparseSetTickIter (tick-filtered, immutable) ──────────────────────────────

/// Iterator yielding `(Entity, &T)` for components whose tick is `> since`.
///
/// Used by [`SparseSet::iter_added_since`] and [`SparseSet::iter_changed_since`].
pub struct SparseSetTickIter<'a, T> {
    entities: &'a [Entity],
    dense: &'a [T],
    ticks: &'a [u32],
    since: u32,
    pos: usize,
}

impl<'a, T> SparseSetTickIter<'a, T> {
    pub fn empty() -> Self {
        Self {
            entities: &[],
            dense: &[],
            ticks: &[],
            since: 0,
            pos: 0,
        }
    }
}

impl<'a, T> Iterator for SparseSetTickIter<'a, T> {
    type Item = (Entity, &'a T);

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if self.pos >= self.dense.len() {
                return None;
            }
            let tick = self.ticks[self.pos];
            let entity = self.entities[self.pos];
            let value = &self.dense[self.pos];
            self.pos += 1;
            if tick > self.since {
                return Some((entity, value));
            }
        }
    }
}
