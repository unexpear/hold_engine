use std::fmt;
use std::marker::PhantomData;

/// A typed, opaque handle into an [`crate::assets::AssetPool`].
///
/// Exactly 16 bytes (two `u32`s + zero-sized marker).  Cheap to copy and
/// store; the generation field makes stale handles detectable.
///
/// All trait implementations are written by hand so that they never add
/// spurious bounds on `T` (e.g. `T: Debug`, `T: Clone`, etc.).
#[derive(PartialEq, Eq, Hash)]
pub struct Handle<T> {
    pub(crate) index:      u32,
    pub(crate) generation: u32,
    _marker:               PhantomData<T>,
}

impl<T> Clone for Handle<T> {
    #[inline]
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for Handle<T> {}

impl<T> fmt::Debug for Handle<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Handle(index={}, gen={})", self.index, self.generation)
    }
}

impl<T> Handle<T> {
    /// Construct a handle from raw parts.  Prefer using [`AssetPool::insert`].
    #[inline]
    pub(crate) fn new(index: u32, generation: u32) -> Self {
        Self { index, generation, _marker: PhantomData }
    }

    /// Sentinel "null" handle — never points to a valid asset.
    #[inline]
    pub fn null() -> Self {
        Self::new(u32::MAX, 0)
    }

    /// Returns `true` when this handle was constructed with [`Handle::null`].
    #[inline]
    pub fn is_null(&self) -> bool {
        self.index == u32::MAX
    }
}
