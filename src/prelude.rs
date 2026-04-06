//! Convenience re-exports for quick starts.
//!
//! ```rust
//! use hold::prelude::*;
//! ```

pub use crate::{
    // Engine core
    engine::{App, Engine},

    // Input
    input::{self, vk, InputState, MouseButton},

    load_bmp,
    load_scene,
    load_texture,

    load_tga,
    // Math
    math::{Mat4, Quat, Vec2, Vec3, Vec4},

    parse_scene,
    renderer::software::SoftwareRenderer,

    // Rendering
    renderer::{Color, Renderer},
    save_scene,

    serialize_scene,
    spawn_scene,
    ActiveCamera,

    ActiveDialogue,
    AppExit,
    AssetStore,
    // Camera
    Camera,
    CameraMode,
    CameraSystem,
    Children,

    ChunkTier,
    DialogueChoice,
    DialogueNode,
    DialogueTree,
    // Lighting
    DirectionalLight,

    EditorEntity,
    // ECS core
    Entity,
    EntityDef,
    // Events
    EventBus,
    EventQueue,
    GlobalTransform,
    GpuMesh,
    GpuTexture,
    Handle,
    // World
    Heightmap,
    InspectorComponent,
    InspectorField,
    InspectorValue,
    Inventory,
    ItemId,
    ItemStack,
    LodChunk,
    LodLevel,
    LodMesh,
    LodTerrain,
    // Assets
    MeshData,
    NodeId,
    // Hierarchy
    Parent,
    PlayerController,
    // Editor
    Project,
    Query2,
    Query2Mut,
    Query3,
    Query3Mut,

    Quest,
    QuestId,
    QuestLog,
    QuestObjective,
    QuestState,

    // Scene
    SceneData,
    SceneDocument,
    SceneError,
    Skybox,

    // Gameplay components
    Stats,
    System,
    SystemSchedule,
    Terrain,
    TerrainChunk,
    TexError,
    Texture,
    TextureFormat,
    Transform,
    TransformPropagateSystem,
    Vertex,
    WindowResized,

    World,
};
