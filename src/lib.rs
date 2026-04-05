pub mod ecs;
pub mod engine;
pub mod input;
pub mod math;
pub mod platform;
pub mod renderer;

// Convenient re-exports at the crate root.
pub use ecs::{Entity, System, World};
pub use engine::Engine;
pub use input::{InputState, MouseButton};
pub use math::{Mat4, Vec2, Vec3, Vec4};
pub use renderer::{BackendKind, Color, Renderer};
