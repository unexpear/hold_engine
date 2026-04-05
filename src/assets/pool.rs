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
    state:      SlotState<T>,
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
    free:  Vec<u32>, // recycled indices ready for reuse
}

impl<T> Default for AssetPool<T> {
    fn default() -> Self {
        Self { slots: Vec::new(), free: Vec::new() }
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
            self.slots.push(Slot { generation: 0, state: SlotState::Occupied(value) });
            Handle::new(index, 0)
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
            SlotState::Empty       => None,
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
            SlotState::Empty       => None,
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
            SlotState::Empty       => None,
        }
    }

    /// Returns `true` if `handle` points to a live asset in this pool.
    pub fn contains(&self, handle: Handle<T>) -> bool {
        self.get(handle).is_some()
    }

    /// Iterate over all live assets together with their handles.
    pub fn iter(&self) -> impl Iterator<Item = (Handle<T>, &T)> {
        self.slots.iter().enumerate().filter_map(|(i, slot)| {
            match &slot.state {
                SlotState::Occupied(v) => {
                    Some((Handle::new(i as u32, slot.generation), v))
                }
                SlotState::Empty => None,
            }
        })
    }
}
