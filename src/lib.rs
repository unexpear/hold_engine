pub mod assets;
pub mod ecs;
pub mod editor;
pub mod engine;
pub mod events;
pub mod input;
pub mod lod;
pub mod math;
pub mod platform;
pub mod player;
pub mod prelude;
pub mod renderer;
pub mod scene;
pub mod skybox;
pub mod terrain;

// Convenient re-exports at the crate root.
pub use assets::{
    load_bmp, load_texture, load_tga, AssetStore, GpuMesh, GpuTexture, Handle, MeshData, ObjError,
    TexError, Texture, TextureFormat, Vertex,
};
pub use ecs::{
    ActiveCamera, ActiveDialogue, Camera, CameraMode, CameraSystem, Children, DialogueChoice,
    DialogueNode, DialogueTree, DirectionalLight, Entity, GlobalTransform, Inventory, ItemId,
    ItemStack, Mut, NodeId, Parent, Query2, Query2Mut, Query3, Query3Mut, Quest, QuestId, QuestLog,
    QuestObjective, QuestState, Stats, System, SystemAccess, SystemSchedule, Transform,
    TransformPropagateSystem, World,
};
pub use engine::{App, Engine};
pub use events::{AppExit, EventBus, EventQueue, WindowResized};
pub use input::{InputState, MouseButton};
pub use lod::{ChunkTier, LodChunk, LodLevel, LodMesh, LodTerrain};
pub use math::{Mat4, Quat, Vec2, Vec3, Vec4};
pub use player::PlayerController;
pub use renderer::{BackendKind, Color, DebugLine, Renderer};
pub use scene::{
    load_scene, parse_scene, save_scene, serialize_scene, spawn_scene, CameraDef, EntityDef,
    LightDef, SceneData, SceneError, StatsDef, TransformDef,
};
pub use skybox::Skybox;
pub use terrain::{Heightmap, Terrain, TerrainChunk};

pub use editor::{
    apply_scene_document_to_world, generate_draw_list, load_and_apply_scene_from_hsc,
    load_and_apply_scene_from_project, load_project, load_scene_doc, load_scene_doc_from_hsc,
    save_project, save_scene_doc, save_scene_doc_to_hsc, EditorApplyError, EditorAxis,
    EditorCamera, EditorCommand, EditorCommandError, EditorDrawList, EditorEntity, EditorEvent,
    EditorInspectorCursor, EditorIoError, EditorPipelineError, EditorPlayState, EditorRuntimeHost,
    EditorSceneSession, EditorSessionError, InspectorComponent, InspectorField, InspectorValue,
    Project, SceneBridgeError, SceneDocument,
};
