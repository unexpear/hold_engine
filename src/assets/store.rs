use std::any::{Any, TypeId};
use std::collections::HashMap;

use crate::assets::{AssetPool, Handle};

/// Top-level asset registry.
///
/// Maintains one [`AssetPool<T>`] per asset type, stored as type-erased
/// `Box<dyn Any>` values keyed by [`TypeId`].  All pool access is
/// zero-overhead when the type is known at the call site.
#[derive(Default)]
pub struct AssetStore {
    pools: HashMap<TypeId, Box<dyn Any>>,
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
