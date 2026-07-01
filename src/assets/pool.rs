use crate::assets::Handle;

// ---------------------------------------------------------------------------
// Internal slot representation
// ---------------------------------------------------------------------------

enum SlotState<T> {
    Empty,
    Occupied(T),
}

struct Slot<T> {
    generation: u32,
    state: SlotState<T>,
}

// ---------------------------------------------------------------------------
// AssetPool
// ---------------------------------------------------------------------------

/// A generational arena that owns assets of a single type `T`.
///
/// Handles returned by [`AssetPool::insert`] remain valid until the asset is
/// explicitly removed.  After removal the slot's generation is bumped so that
/// any old handles referring to that slot will be rejected.
pub struct AssetPool<T> {
    slots: Vec<Slot<T>>,
    free: Vec<u32>, // recycled indices ready for reuse
}

impl<T> Default for AssetPool<T> {
    fn default() -> Self {
        Self {
            slots: Vec::new(),
            free: Vec::new(),
        }
    }
}

impl<T> AssetPool<T> {
    /// Create an empty pool.
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert `value` into the pool and return a [`Handle`] to it.
    pub fn insert(&mut self, value: T) -> Handle<T> {
        if let Some(index) = self.free.pop() {
            let slot = &mut self.slots[index as usize];
            // The generation was already bumped when the previous occupant was
            // removed, so we can use it directly.
            slot.state = SlotState::Occupied(value);
            Handle::new(index, slot.generation)
        } else {
            let index = self.slots.len() as u32;
            self.slots.push(Slot {
                generation: 0,
                state: SlotState::Occupied(value),
            });
            Handle::new(index, 0)
        }
    }

    /// Replace the asset at `handle` with `value` without changing the
    /// slot's generation or invalidating any outstanding clone of `handle`.
    ///
    /// Returns the previous value, or `None` if the handle is stale, null,
    /// out-of-range, or pointing at an empty slot. The caller is
    /// responsible for any cleanup (GPU resource release, file handle
    /// drop, etc.) on the returned value.
    ///
    /// This is the load-bearing primitive for hot-reload. Unlike `remove +
    /// insert`, identity is preserved: every existing reference to
    /// `handle` continues to resolve, now to the new value. See
    /// `docs/adr/0004-hot-reload-architecture.md` Q6 + Q10.
    pub fn replace(&mut self, handle: Handle<T>, value: T) -> Option<T> {
        if handle.is_null() {
            return None;
        }
        let slot = self.slots.get_mut(handle.index as usize)?;
        if slot.generation != handle.generation {
            return None;
        }
        let old_state = std::mem::replace(&mut slot.state, SlotState::Occupied(value));
        match old_state {
            SlotState::Occupied(v) => Some(v),
            // Generation matched but slot was empty: the slot was removed
            // and the generation hasn't been recycled yet. Defensive
            // None — the caller's handle is effectively stale and the
            // freshly-installed value is now orphaned in the slot. In
            // practice this branch is unreachable because remove() bumps
            // the generation, but explicit handling keeps the function
            // total.
            SlotState::Empty => None,
        }
    }

    /// Remove the asset pointed to by `handle`.
    ///
    /// Returns `Some(value)` on success, or `None` if the handle is stale /
    /// null / out-of-range.
    pub fn remove(&mut self, handle: Handle<T>) -> Option<T> {
        if handle.is_null() {
            return None;
        }
        let slot = self.slots.get_mut(handle.index as usize)?;
        if slot.generation != handle.generation {
            return None;
        }
        let old_state = std::mem::replace(&mut slot.state, SlotState::Empty);
        // Bump the generation so stale handles to this slot become invalid.
        slot.generation = slot.generation.wrapping_add(1);
        self.free.push(handle.index);
        match old_state {
            SlotState::Occupied(v) => Some(v),
            SlotState::Empty => None,
        }
    }

    /// Return a shared reference to the asset, or `None` for stale/null handles.
    pub fn get(&self, handle: Handle<T>) -> Option<&T> {
        if handle.is_null() {
            return None;
        }
        let slot = self.slots.get(handle.index as usize)?;
        if slot.generation != handle.generation {
            return None;
        }
        match &slot.state {
            SlotState::Occupied(v) => Some(v),
            SlotState::Empty => None,
        }
    }

    /// Return a mutable reference to the asset, or `None` for stale/null handles.
    pub fn get_mut(&mut self, handle: Handle<T>) -> Option<&mut T> {
        if handle.is_null() {
            return None;
        }
        let slot = self.slots.get_mut(handle.index as usize)?;
        if slot.generation != handle.generation {
            return None;
        }
        match &mut slot.state {
            SlotState::Occupied(v) => Some(v),
            SlotState::Empty => None,
        }
    }

    /// Returns `true` if `handle` points to a live asset in this pool.
    pub fn contains(&self, handle: Handle<T>) -> bool {
        self.get(handle).is_some()
    }

    /// Iterate over all live assets together with their handles.
    pub fn iter(&self) -> impl Iterator<Item = (Handle<T>, &T)> {
        self.slots
            .iter()
            .enumerate()
            .filter_map(|(i, slot)| match &slot.state {
                SlotState::Occupied(v) => Some((Handle::new(i as u32, slot.generation), v)),
                SlotState::Empty => None,
            })
    }

    /// Iterate mutably over all live assets together with their handles.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (Handle<T>, &mut T)> {
        self.slots.iter_mut().enumerate().filter_map(|(i, slot)| {
            let gen = slot.generation;
            match &mut slot.state {
                SlotState::Occupied(v) => Some((Handle::new(i as u32, gen), v)),
                SlotState::Empty => None,
            }
        })
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Replace returns the previous value to the caller. The caller can
    /// then drop or release the old value (closing GPU resources, etc.).
    /// Without this, hot-reload would leak the previous asset's
    /// resources every time something reloaded.
    #[test]
    fn replace_returns_previous_value() {
        let mut pool: AssetPool<u32> = AssetPool::new();
        let handle = pool.insert(42);
        let previous = pool.replace(handle, 99);
        assert_eq!(previous, Some(42));
        assert_eq!(pool.get(handle), Some(&99));
    }

    /// Identity preservation: the handle returned by `insert` continues
    /// to resolve to the new value after `replace`. This is the property
    /// hot-reload depends on (ADR 0004 Q6).
    #[test]
    fn replace_preserves_handle_identity() {
        let mut pool: AssetPool<String> = AssetPool::new();
        let handle = pool.insert("first".into());
        let _ = pool.replace(handle, "second".into());
        // Same handle still works, generation unchanged.
        assert_eq!(pool.get(handle), Some(&"second".to_string()));
        assert!(pool.contains(handle));
    }

    /// Replace must not bump the slot's generation; outstanding clones
    /// of the handle (held by the editor inspector, ECS components,
    /// material structs) must keep working.
    #[test]
    fn replace_does_not_bump_generation() {
        let mut pool: AssetPool<i32> = AssetPool::new();
        let original = pool.insert(1);
        let _ = pool.replace(original, 2);
        let _ = pool.replace(original, 3);
        // Generation never changed; the handle clone is still valid.
        assert_eq!(pool.get(original), Some(&3));
    }

    /// A handle whose owning slot was removed is stale — replace must
    /// reject it, not resurrect the slot. Resurrection would corrupt
    /// the recycled-index pool.
    #[test]
    fn replace_on_stale_handle_returns_none() {
        let mut pool: AssetPool<u32> = AssetPool::new();
        let handle = pool.insert(7);
        let _ = pool.remove(handle);
        let outcome = pool.replace(handle, 9);
        assert_eq!(outcome, None);
        // Stale handle still doesn't resolve.
        assert_eq!(pool.get(handle), None);
    }

    /// The null handle is the explicit "no asset" placeholder; replace
    /// must reject it like every other accessor.
    #[test]
    fn replace_on_null_handle_returns_none() {
        let mut pool: AssetPool<u32> = AssetPool::new();
        let null: Handle<u32> = Handle::null();
        assert_eq!(pool.replace(null, 1), None);
    }

    /// Replace doesn't allocate a new slot — the slot count stays the
    /// same. The recycled-index free list is also untouched. This
    /// pins the "in-place mutation" semantic at the storage level.
    #[test]
    fn replace_does_not_grow_pool() {
        let mut pool: AssetPool<u32> = AssetPool::new();
        let h = pool.insert(0);
        let live_before = pool.iter().count();
        let free_before = pool.free.len();
        let _ = pool.replace(h, 1);
        let _ = pool.replace(h, 2);
        let _ = pool.replace(h, 3);
        assert_eq!(pool.iter().count(), live_before);
        assert_eq!(pool.free.len(), free_before);
    }
}
