pub mod assets;
pub mod ecs;
pub mod engine;
pub mod events;
pub mod input;
pub mod math;
pub mod platform;
pub mod player;
pub mod renderer;
pub mod skybox;
pub mod terrain;

// Convenient re-exports at the crate root.
pub use assets::{AssetStore, GpuMesh, Handle, MeshData, ObjError, Vertex};
pub use ecs::{ActiveCamera, Camera, CameraMode, CameraSystem, Children, DirectionalLight, Entity,
              GlobalTransform, Parent, System, SystemSchedule, Transform, TransformPropagateSystem, World};
pub use engine::Engine;
pub use events::{AppExit, EventBus, EventQueue, WindowResized};
pub use input::{InputState, MouseButton};
pub use math::{Mat4, Quat, Vec2, Vec3, Vec4};
pub use player::PlayerController;
pub use renderer::{BackendKind, Color, Renderer};
pub use skybox::Skybox;
pub use terrain::{Heightmap, Terrain, TerrainChunk};
