#![allow(dead_code)]
use super::entity::Entity;

/// Sentinel value stored in `sparse` when an entity has no component.
const ABSENT: u32 = u32::MAX;

/// A sparse-set storage for a single component type `T`.
///
/// Layout
/// ------
/// * `sparse`   — indexed by `entity.index`; value is the position in `dense`
///                (or `ABSENT` if not present).
/// * `dense`    — tightly packed component values; cache-friendly for iteration.
/// * `entities` — parallel to `dense`; maps a dense index back to its Entity.
///
/// All three arrays are kept in sync on every insert/remove.
pub struct SparseSet<T> {
    sparse:   Vec<u32>,
    dense:    Vec<T>,
    entities: Vec<Entity>,
}

impl<T> SparseSet<T> {
    pub fn new() -> Self {
        Self {
            sparse:   Vec::new(),
            dense:    Vec::new(),
            entities: Vec::new(),
        }
    }

    // ── Core operations ───────────────────────────────────────────────────────

    /// Insert (or overwrite) a component for `entity`.
    pub fn insert(&mut self, entity: Entity, value: T) {
        let idx = entity.index as usize;

        // Grow sparse array if needed, filling gaps with ABSENT.
        if idx >= self.sparse.len() {
            self.sparse.resize(idx + 1, ABSENT);
        }

        let slot = self.sparse[idx];
        if slot != ABSENT {
            // Overwrite existing entry.
            self.dense[slot as usize] = value;
            self.entities[slot as usize] = entity;
        } else {
            // Append to dense arrays and record position in sparse.
            let dense_pos = self.dense.len() as u32;
            self.sparse[idx] = dense_pos;
            self.dense.push(value);
            self.entities.push(entity);
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
        }

        self.dense.pop();
        self.entities.pop();
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

    // ── Iteration ─────────────────────────────────────────────────────────────

    /// Iterate all `(Entity, &T)` pairs in dense order (cache-friendly).
    pub fn iter(&self) -> SparseSetIter<'_, T> {
        SparseSetIter {
            entities: &self.entities,
            dense:    &self.dense,
            pos:      0,
        }
    }

    /// Iterate all `(Entity, &mut T)` pairs in dense order.
    pub fn iter_mut(&mut self) -> SparseSetIterMut<'_, T> {
        SparseSetIterMut {
            entities: &self.entities,
            dense:    self.dense.as_mut_slice(),
            pos:      0,
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

// ── Concrete iterator types (avoids lifetime trouble with `impl Iterator`) ─────

pub struct SparseSetIter<'a, T> {
    entities: &'a [Entity],
    dense:    &'a [T],
    pos:      usize,
}

impl<'a, T> SparseSetIter<'a, T> {
    /// An iterator that yields nothing (used when no storage exists for a type).
    pub fn empty() -> Self {
        Self { entities: &[], dense: &[], pos: 0 }
    }
}

impl<'a, T> Iterator for SparseSetIter<'a, T> {
    type Item = (Entity, &'a T);

    fn next(&mut self) -> Option<Self::Item> {
        if self.pos >= self.dense.len() {
            return None;
        }
        let entity = self.entities[self.pos];
        let value  = &self.dense[self.pos];
        self.pos += 1;
        Some((entity, value))
    }
}

pub struct SparseSetIterMut<'a, T> {
    entities: &'a [Entity],
    dense:    &'a mut [T],
    pos:      usize,
}

impl<'a, T> Iterator for SparseSetIterMut<'a, T> {
    type Item = (Entity, &'a mut T);

    fn next(&mut self) -> Option<Self::Item> {
        if self.pos >= self.dense.len() {
            return None;
        }
        let entity = self.entities[self.pos];
        // SAFETY: we advance `pos` by 1 each call, so we never alias the same
        // mutable slot.  The lifetime transmute is needed because Rust's borrow
        // checker cannot see through the split-borrow here without unsafe.
        let value: &'a mut T = unsafe {
            &mut *(self.dense.as_mut_ptr().add(self.pos))
        };
        self.pos += 1;
        Some((entity, value))
    }
}
