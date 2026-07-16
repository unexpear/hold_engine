pub mod assets;
pub mod determinism;
pub mod ecs;
pub mod editor;
pub mod engine;
pub mod events;
pub mod hot_reload;
pub mod input;
pub mod lod;
pub mod math;
pub mod platform;
pub mod player;
pub mod prelude;
pub mod renderer;
pub mod runtime_frame;
pub mod runtime_slice;
pub mod scene;
pub mod skybox;
pub mod terrain;

// Convenient re-exports at the crate root.
pub use assets::{
    compile_asset_from_source, compile_material_asset, compile_mesh_asset, compile_texture_asset,
    compiled_asset_path_for_logical_name, decode_compiled_asset, decode_compiled_material_payload,
    decode_compiled_mesh_payload, decode_compiled_texture_payload, encode_compiled_asset,
    encode_compiled_material_payload, encode_compiled_mesh_payload,
    encode_compiled_texture_payload, load_bmp, load_texture, load_tga,
    parse_compiled_material_source, resolve_source_asset_path, source_asset_candidates,
    source_asset_is_stale, write_compiled_asset, AssetCompilerError, AssetStore, CompiledAssetBlob,
    CompiledAssetDependency, CompiledAssetError, CompiledAssetKind, CompiledAssetMetadata,
    CompiledAssetRuntimeError, CompiledAssetRuntimeLoader, CompiledMaterialPayload, GpuMesh,
    compute_ao_per_vertex, compute_curvature_per_vertex, AssetInvalidated, AssetKind, ColorSpace,
    GpuTexture, Handle, InvalidationBus, Material, MaterialHandle, MeshData, MeshHandle,
    MeshMapAO, MeshMapAOHandle, MeshMapCurvature, MeshMapCurvatureHandle, MeshMapError, NormalMap,
    NormalMapConvention, NormalMapHandle, ObjError, Shader, ShaderHandle, SubscriberId, TexError,
    Texture, TextureFormat, TextureHandle, Vertex, COMPILED_ASSET_MAGIC, COMPILED_ASSET_VERSION,
};
pub use hot_reload::{
    canonicalise_for_index, default_compile_fn, Coalescer, CompileErr, CompileFn, CompileOk,
    CompileQueue, CompileQueueConfig, CompileQueueError, CompileResult, CompileTarget,
    DispatchError, DispatchSummary, DroppedResult, ExtensionFilter, FileChange, FileChangeKind,
    FileWatcher, Generation, RawFileEvent, ReloadDispatcher, SceneReloadResult, WatcherConfig,
    WatcherError,
};
pub use determinism::{
    decode_input_recording, encode_input_recording, frame_digest_from_components,
    record_and_replay_frame_digests, DeterministicHasher, FrameDigest, InputRecorder,
    InputRecording, InputRecordingError, InputReplayer, RecordedFrame, RecordedInput,
    INPUT_RECORDING_FORMAT_VERSION, INPUT_RECORDING_MAGIC,
};
pub use ecs::{
    generate_particle_batch, generate_particle_debug_lines, ActiveCamera, ActiveDialogue,
    AnimationGraphState, AnimationState, AudioEmitter, Breakable, Camera, CameraMode, CameraSystem,
    CharacterController, Children, DialogueChoice, DialogueNode, DialogueTree, DirectionalLight,
    Entity, GlobalTransform, Health, Inventory, ItemId, ItemStack, MaterialRef, Mut, NavAgent,
    NodeId, Parent, Particle, ParticleEmitSystem, ParticleEmitter, ParticleLifetimeSystem,
    ParticleSource, PatrolRoute, PhysicsSystem, Query2, Query2Mut, Query3, Query3Mut, Quest,
    QuestId, QuestLog, QuestObjective, QuestState, RigidBody, SkeletonPose, SphereCollider,
    StaticMesh, Stats, System, SystemAccess, SystemSchedule, TerrainChunkRef, Transform,
    TransformPropagateSystem, VegetationInstance, VehicleController, Velocity, Weapon, WheelState,
    World,
};
pub use engine::{App, Engine};
pub use events::{AppExit, EventBus, EventQueue, WindowResized};
pub use input::{InputState, MouseButton};
pub use lod::{ChunkTier, LodChunk, LodLevel, LodMesh, LodTerrain};
pub use math::{Mat4, Quat, Vec2, Vec3, Vec4};
pub use player::PlayerController;
pub use renderer::{
    BackendCapabilities, BackendKind, BackendMaturity, Color, DebugLine, DrawItem, FogState,
    LightItem, ParticleBatch, ParticleBillboard, PassThumbnail, RenderFrame, RenderView,
    RenderViewport, Renderer, ScreenQuad, TerrainBatch, TerrainPatchItem, VegetationBatch,
    WaterBatch, DEFAULT_THUMBNAIL_SIZE,
};
pub use runtime_frame::{build_runtime_slice_frame, RuntimeSliceBindings, RuntimeSliceViewConfig};
pub use runtime_slice::{
    BreakablePropDef, PatrolRouteDef, RuntimeSliceAuthoring, SpawnPointDef, SpawnPointKind,
    StaticMeshActorDef, TerrainAuthoring, TerrainPaintLayer, VegetationPaintStroke,
    VegetationPrototypeDef, VehicleSpawnerDef, WaterVolumeDef,
};
pub use scene::{
    load_scene, parse_scene, save_scene, serialize_scene, spawn_scene, CameraDef, EntityDef,
    LightDef, SceneData, SceneError, StatsDef, TransformDef,
};
pub use skybox::Skybox;
pub use terrain::{Heightmap, Terrain, TerrainChunk};

pub use editor::{
    apply_scene_document_to_world, build_native_ui, build_viewport_overlay,
    decode_editor_command_protocol, decode_editor_event_protocol, encode_editor_command_protocol,
    encode_editor_event_protocol, generate_draw_list, load_and_apply_scene_from_hsc,
    load_and_apply_scene_from_project, load_project, load_scene_doc, load_scene_doc_from_hsc,
    save_project, save_scene_doc, save_scene_doc_to_hsc, DispatchApplyReport, EditorApplyError,
    EditorAxis, EditorBridgeServer, EditorCamera, EditorCommand, EditorCommandError,
    EditorDrawList, EditorEntity, EditorEvent, EditorInspectorCursor, EditorIoError,
    EditorNativeUi, EditorNativeUiState, EditorPipelineError, EditorPlayState,
    EditorProtocolError, EditorReplayState, EditorRuntimeHost, EditorSceneSession,
    EditorSessionError, EditorUiActionLogEntry, EditorUiActionTone, EditorUiCommandPalette,
    EditorUiHitRegion, EditorUiPaletteEntry, EditorUiPaletteEntrySource, EditorUiPanel,
    EditorUiPanelRegion, EditorViewportGizmoHandle, EditorViewportGizmoHandleId,
    EditorViewportGizmoMode, EditorViewportOverlay, FrameInspectorPass, FrameInspectorState,
    InspectorComponent, InspectorField, InspectorValue, Project, ReplayMode, ReplayTickOutcome,
    SceneBridgeError, SceneDocument, SceneReloadOutcome, AutosaveScheduler,
    recovery_path_for_scene, AUTOSAVE_IDLE_DEBOUNCE_SECS, AUTOSAVE_MAX_INTERVAL_SECS,
    AUTOSAVE_RETRY_BACKOFF_SECS,
};
