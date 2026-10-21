/// A lightweight handle to an entity.
///
/// The 64-bit layout packs `index` (which slot in the entity table) and
/// `generation` (how many times that slot has been recycled) so that stale
/// handles to despawned entities can be detected cheaply.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Entity {
    pub index: u32,
    pub generation: u32,
}

impl Entity {
    #[inline]
    pub fn new(index: u32, generation: u32) -> Self {
        Self { index, generation }
    }
}

impl std::fmt::Display for Entity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Entity({}, gen={})", self.index, self.generation)
    }
}
