use std::fmt;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;

/// A typed, opaque handle into an [`crate::assets::AssetPool`].
///
/// Exactly 16 bytes (two `u32`s + zero-sized marker).  Cheap to copy and
/// store; the generation field makes stale handles detectable.
///
/// All trait implementations are written by hand so that they never add
/// spurious bounds on `T` (e.g. `T: Debug`, `T: Clone`, etc.).
pub struct Handle<T> {
    pub(crate) index: u32,
    pub(crate) generation: u32,
    _marker: PhantomData<T>,
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

impl<T> PartialEq for Handle<T> {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.index == other.index && self.generation == other.generation
    }
}

impl<T> Eq for Handle<T> {}

impl<T> Hash for Handle<T> {
    #[inline]
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.index.hash(state);
        self.generation.hash(state);
    }
}

impl<T> Handle<T> {
    /// Construct a handle from raw parts.  Prefer using [`AssetPool::insert`].
    #[inline]
    pub(crate) fn new(index: u32, generation: u32) -> Self {
        Self {
            index,
            generation,
            _marker: PhantomData,
        }
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

    /// Index of the slot this handle points to in the owning
    /// [`AssetPool<T>`]. Combined with [`Self::generation`] it
    /// uniquely identifies a live asset.
    ///
    /// Public so external integrations (e.g. the hot-reload
    /// dispatcher's downstream cache eviction in user code) can
    /// match handles by index when generation is preserved across
    /// `AssetStore::replace` (ADR 0004 Q6).
    #[inline]
    pub fn index(&self) -> u32 {
        self.index
    }

    /// Generation counter at the moment the handle was issued. A
    /// handle with a stale generation no longer resolves through
    /// the asset store; `AssetStore::replace` deliberately does
    /// *not* bump the generation, so handles obtained before the
    /// reload remain valid.
    #[inline]
    pub fn generation(&self) -> u32 {
        self.generation
    }
}
