use std::any::{Any, TypeId};
use std::collections::HashMap;

use crate::assets::{AssetPool, Handle};

/// Top-level asset registry.
///
/// Maintains one [`AssetPool<T>`] per asset type, stored as type-erased
/// `Box<dyn Any>` values keyed by [`TypeId`].  All pool access is
/// zero-overhead when the type is known at the call site.
pub struct AssetStore {
    pools: HashMap<TypeId, Box<dyn Any>>,
}

impl Default for AssetStore {
    fn default() -> Self {
        Self {
            pools: HashMap::new(),
        }
    }
}

impl AssetStore {
    /// Create an empty store.
    pub fn new() -> Self {
        Self::default()
    }

    // ------------------------------------------------------------------
    // Public API
    // ------------------------------------------------------------------

    /// Insert `value` into the pool for `T`, creating the pool if needed.
    /// Returns a [`Handle<T>`] that can be used to retrieve the asset later.
    pub fn insert<T: 'static>(&mut self, value: T) -> Handle<T> {
        self.get_pool_mut::<T>().insert(value)
    }

    /// Remove the asset identified by `handle`.
    ///
    /// Returns `Some(value)` on success, `None` if the handle is stale or the
    /// pool for `T` does not exist.
    pub fn remove<T: 'static>(&mut self, handle: Handle<T>) -> Option<T> {
        self.pools
            .get_mut(&TypeId::of::<T>())?
            .downcast_mut::<AssetPool<T>>()?
            .remove(handle)
    }

    /// Replace the asset under `handle` with `value`, preserving identity.
    ///
    /// Returns the previous value, or `None` if the handle is stale, null,
    /// or no pool for `T` exists. The handle remains valid after the call;
    /// every existing clone of it continues to resolve, now to the new
    /// value.
    ///
    /// This is the load-bearing primitive for hot-reload. Unlike
    /// `remove + insert`, the handle's generation does not change, so
    /// downstream consumers (ECS components, materials, scene
    /// serialisations) continue to work without rewiring. See
    /// `docs/adr/0004-hot-reload-architecture.md` Q6, Q10 and
    /// [`AssetPool::replace`].
    ///
    /// If no pool for `T` has been created (no asset of type `T` was ever
    /// inserted), returns `None`. This case is benign — there can be no
    /// live handles of a non-existent pool — but the caller may want to
    /// surface it as an "unknown asset type" diagnostic rather than treat
    /// it as a successful no-op.
    pub fn replace<T: 'static>(&mut self, handle: Handle<T>, value: T) -> Option<T> {
        self.pools
            .get_mut(&TypeId::of::<T>())?
            .downcast_mut::<AssetPool<T>>()?
            .replace(handle, value)
    }

    /// Borrow the asset identified by `handle`.
    pub fn get<T: 'static>(&self, handle: Handle<T>) -> Option<&T> {
        self.pools
            .get(&TypeId::of::<T>())?
            .downcast_ref::<AssetPool<T>>()?
            .get(handle)
    }

    /// Mutably borrow the asset identified by `handle`.
    pub fn get_mut<T: 'static>(&mut self, handle: Handle<T>) -> Option<&mut T> {
        self.pools
            .get_mut(&TypeId::of::<T>())?
            .downcast_mut::<AssetPool<T>>()?
            .get_mut(handle)
    }

    /// Direct access to the underlying pool for `T`, useful for iteration.
    ///
    /// Returns `None` if no assets of type `T` have been inserted yet.
    pub fn pool<T: 'static>(&self) -> Option<&AssetPool<T>> {
        self.pools
            .get(&TypeId::of::<T>())?
            .downcast_ref::<AssetPool<T>>()
    }

    /// Mutable access to the underlying pool for `T`.
    ///
    /// Returns `None` if no assets of type `T` have been inserted yet.
    pub fn pool_mut<T: 'static>(&mut self) -> Option<&mut AssetPool<T>> {
        self.pools
            .get_mut(&TypeId::of::<T>())?
            .downcast_mut::<AssetPool<T>>()
    }

    // ------------------------------------------------------------------
    // Internal helpers
    // ------------------------------------------------------------------

    /// Get (or lazily create) the pool for `T`.
    fn get_pool_mut<T: 'static>(&mut self) -> &mut AssetPool<T> {
        self.pools
            .entry(TypeId::of::<T>())
            .or_insert_with(|| Box::new(AssetPool::<T>::new()))
            .downcast_mut::<AssetPool<T>>()
            .expect("type-erased pool downcast should always succeed")
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// AssetStore::replace delegates to the pool and surfaces the
    /// previous value to the caller. End-to-end happy path.
    #[test]
    fn replace_returns_previous_and_swaps_value() {
        let mut store = AssetStore::new();
        let handle = store.insert(10u32);
        let previous = store.replace(handle, 99u32);
        assert_eq!(previous, Some(10));
        assert_eq!(store.get(handle), Some(&99));
    }

    /// Identity preservation through the store wrapper. ECS components
    /// that hold a `Handle<T>` continue to resolve after replace —
    /// without this, hot-reload would mean rewiring every consumer.
    #[test]
    fn replace_preserves_handle_through_store() {
        let mut store = AssetStore::new();
        let handle = store.insert(String::from("v1"));
        let _ = store.replace(handle, String::from("v2"));
        assert_eq!(store.get(handle), Some(&"v2".to_string()));
    }

    /// Replace on a type with no registered pool returns None instead
    /// of panicking. Callers that want "unknown asset type" diagnostics
    /// must surface this case explicitly.
    #[test]
    fn replace_on_unknown_type_returns_none() {
        let mut store = AssetStore::new();
        // No `u32` pool has ever been created.
        let bogus: Handle<u32> = Handle::new(0, 0);
        assert_eq!(store.replace(bogus, 1u32), None);
    }

    /// Stale handles get rejected at the store layer too — the pool's
    /// generational check fires through the type-erased boundary.
    #[test]
    fn replace_on_stale_handle_returns_none_at_store() {
        let mut store = AssetStore::new();
        let handle = store.insert(7i32);
        let _ = store.remove(handle);
        assert_eq!(store.replace(handle, 8i32), None);
    }

    /// A different type's handle pointing at a numerically equal slot
    /// must NOT match. The store keeps separate pools per `TypeId`;
    /// replace<u32> must never touch an i32 pool even if the indices
    /// collide. Pillar 4 (debuggability) — silent cross-type bugs
    /// would be brutal to track down.
    #[test]
    fn replace_does_not_cross_type_boundaries() {
        let mut store = AssetStore::new();
        let u_handle = store.insert(100u32);
        // Insert an i32 with a coincident index.
        let _i_handle = store.insert(-1i32);
        // Replace via the u32 handle goes only to the u32 pool.
        let prev = store.replace(u_handle, 200u32);
        assert_eq!(prev, Some(100));
        // The i32 we inserted is untouched.
        assert_eq!(store.pool::<i32>().unwrap().iter().count(), 1);
    }
}
