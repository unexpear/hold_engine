use crate::ecs::Entity;

pub struct Parent {
    pub entity: Entity,
}
pub struct Children {
    pub list: Vec<Entity>,
}

impl Children {
    pub fn new() -> Self {
        Self { list: Vec::new() }
    }
}

impl Default for Children {
    fn default() -> Self {
        Self::new()
    }
}
