//! Convenience re-exports for quick starts.
//!
//! ```rust
//! use hold::prelude::*;
//! ```

pub use crate::{
    // ECS core
    Entity, World, System, SystemSchedule,
    Transform, GlobalTransform, TransformPropagateSystem,

    // Camera
    Camera, CameraMode, CameraSystem, ActiveCamera,

    // Lighting
    DirectionalLight,

    // Rendering
    renderer::{Color, Renderer},
    renderer::software::SoftwareRenderer,

    // Math
    math::{Mat4, Quat, Vec2, Vec3, Vec4},

    // Input
    input::{self, InputState, MouseButton, vk},

    // Engine core
    engine::{App, Engine},

    // Assets
    MeshData, Vertex, GpuMesh, GpuTexture, Handle, AssetStore,
    Texture, TextureFormat, TexError,
    load_bmp, load_tga, load_texture,

    // Gameplay components
    Stats, Inventory, ItemId, ItemStack,
    DialogueTree, DialogueNode, DialogueChoice, ActiveDialogue, NodeId,
    Quest, QuestId, QuestLog, QuestObjective, QuestState,

    // World
    Heightmap, Terrain, TerrainChunk,
    LodTerrain, LodChunk, LodMesh, LodLevel, ChunkTier,
    PlayerController,
    Skybox,

    // Events
    EventBus, EventQueue, AppExit, WindowResized,

    // Hierarchy
    Parent, Children,
};
