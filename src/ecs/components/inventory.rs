//! Item / inventory ECS components.

/// Unique identifier for an item type.
///
/// Wrap a plain u32 so game code can define item catalogues however it likes
/// (e.g. a HashMap<ItemId, ItemDef>) without baking string names into the
/// component itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ItemId(pub u32);

/// An entry inside an [Inventory].
#[derive(Debug, Clone)]
pub struct ItemStack {
    pub id: ItemId,
    pub quantity: u32,
    pub max_stack: u32,
}

impl ItemStack {
    pub fn new(id: ItemId, quantity: u32, max_stack: u32) -> Self {
        Self {
            id,
            quantity,
            max_stack,
        }
    }

    /// Single-item stack with stack limit of 1 (e.g. a weapon).
    pub fn single(id: ItemId) -> Self {
        Self::new(id, 1, 1)
    }

    /// Stackable item with a default max of 99.
    pub fn stackable(id: ItemId, quantity: u32) -> Self {
        Self::new(id, quantity, 99)
    }

    /// Can more of this item be added?
    #[inline]
    pub fn can_add(&self, n: u32) -> bool {
        self.quantity + n <= self.max_stack
    }
}

/// Fixed-size inventory attached to an entity.
pub struct Inventory {
    pub slots: Vec<Option<ItemStack>>,
}

impl Inventory {
    /// Create an inventory with size empty slots.
    pub fn new(size: usize) -> Self {
        Self {
            slots: vec![None; size],
        }
    }

    /// Total number of slots (occupied + empty).
    pub fn capacity(&self) -> usize {
        self.slots.len()
    }

    /// Number of slots that contain an item.
    pub fn occupied(&self) -> usize {
        self.slots.iter().filter(|s| s.is_some()).count()
    }

    /// Try to add quantity of id to the inventory.
    ///
    /// First tries to merge into an existing stack of the same item, then
    /// falls back to an empty slot.  Returns the leftover quantity that
    /// could not be stored (0 = everything fit).
    pub fn add(&mut self, id: ItemId, mut quantity: u32, max_stack: u32) -> u32 {
        // Merge into existing stacks first.
        for slot in self.slots.iter_mut() {
            if quantity == 0 {
                break;
            }
            if let Some(stack) = slot {
                if stack.id == id {
                    let room = stack.max_stack - stack.quantity;
                    let take = quantity.min(room);
                    stack.quantity += take;
                    quantity -= take;
                }
            }
        }
        // Fill empty slots.
        for slot in self.slots.iter_mut() {
            if quantity == 0 {
                break;
            }
            if slot.is_none() {
                let take = quantity.min(max_stack);
                *slot = Some(ItemStack::new(id, take, max_stack));
                quantity -= take;
            }
        }
        quantity
    }

    /// Remove up to quantity of id.  Returns how many were actually removed.
    pub fn remove(&mut self, id: ItemId, mut quantity: u32) -> u32 {
        let mut removed = 0u32;
        for slot in self.slots.iter_mut() {
            if quantity == 0 {
                break;
            }
            if let Some(stack) = slot {
                if stack.id == id {
                    let take = quantity.min(stack.quantity);
                    stack.quantity -= take;
                    quantity -= take;
                    removed += take;
                    if stack.quantity == 0 {
                        *slot = None;
                    }
                }
            }
        }
        removed
    }

    /// Count total quantity of an item across all stacks.
    pub fn count(&self, id: ItemId) -> u32 {
        self.slots
            .iter()
            .filter_map(|s| s.as_ref())
            .filter(|s| s.id == id)
            .map(|s| s.quantity)
            .sum()
    }

    /// Does the inventory contain at least n of the given item?
    pub fn has(&self, id: ItemId, n: u32) -> bool {
        self.count(id) >= n
    }

    /// Get the stack in slot index (bounds-checked).
    pub fn get(&self, index: usize) -> Option<&ItemStack> {
        self.slots.get(index).and_then(|s| s.as_ref())
    }

    /// Clear a specific slot.  Returns whatever was there.
    pub fn take(&mut self, index: usize) -> Option<ItemStack> {
        self.slots.get_mut(index).and_then(|s| s.take())
    }
}
