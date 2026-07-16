//! Native windowed editor shell for the M2 milestone.
//!
//! Usage:
//!   cargo run --example editor_shell
//!   cargo run --example editor_shell -- --scene path/to/scene.hsc
//!   cargo run --example editor_shell -- --backend opengl

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use hold::{
    assets::{
        compile_asset_from_source, compiled_asset_path_for_logical_name,
        parse_compiled_material_source, resolve_source_asset_path, source_asset_is_stale,
        write_compiled_asset, CompiledAssetKind, CompiledAssetRuntimeLoader,
    },
    build_native_ui, build_runtime_slice_frame, build_viewport_overlay,
    decode_editor_event_protocol, encode_editor_command_protocol,
    engine::App,
    hot_reload::{
        default_compile_fn, CompileQueue, CompileQueueConfig, CompileTarget, ExtensionFilter,
        FileWatcher, ReloadDispatcher, WatcherConfig,
    },
    input::{vk, InputState, MouseButton},
    math::Mat4,
    renderer::{
        opengl_shaders, BackendKind, OpenGLRenderer, RenderViewport, Renderer, SoftwareRenderer,
    },
    ActiveCamera, AssetStore, Camera, DirectionalLight, EditorAxis, EditorBridgeServer,
    EditorCommand, EditorEvent, EditorNativeUiState, EditorPlayState, EditorProtocolError,
    EditorUiActionLogEntry, EditorUiActionTone, EditorUiCommandPalette, EditorUiPaletteEntry,
    EditorUiPaletteEntrySource, EditorUiPanel, EditorViewportGizmoHandle,
    EditorViewportGizmoHandleId, EditorViewportGizmoMode, Engine, GlobalTransform, RenderFrame,
    RuntimeSliceBindings, RuntimeSliceViewConfig, SceneData, Shader, Transform,
};

/// Owns the hot-reload pipeline for the editor shell. The four
/// pieces — file watcher, compile queue, reload dispatcher, plus
/// the source-root path for diagnostic messages — are bundled here
/// so they share a single Drop-order ledger and so the editor's
/// per-frame tick has one place to drain.
///
/// The ordering on construction is load-bearing:
/// `FileWatcher` → `CompileQueue` → `ReloadDispatcher` chained via
/// two mpsc channels (file changes and compile results). On Drop,
/// the dispatcher's receiver disappears first, the queue's worker
/// notices the closed channel and exits, then the watcher's worker
/// gets its stop signal and exits. Threads join in their owning
/// types' Drop impls; nothing leaks.
///
/// See `docs/adr/0004-hot-reload-architecture.md` Q3 + Q10.
struct HotReloadPipeline {
    /// Kept alive for its Drop impl — the watcher thread runs until
    /// the FileWatcher is dropped. Field is read on shutdown only.
    _watcher: FileWatcher,
    /// Kept alive for its Drop impl — see _watcher.
    _queue: CompileQueue,
    /// The dispatcher is the only piece the per-frame tick touches.
    dispatcher: ReloadDispatcher,
    /// Source root the watcher is observing. Surfaced in startup
    /// diagnostics so developers can see "hot-reload watching
    /// C:\\projects\\foo\\source" in the launch log.
    source_root: PathBuf,
    /// Logical names the dispatcher is registered for. Telemetry +
    /// diagnostic display only; the dispatcher is the source of
    /// truth for actual reload routing.
    registered_logical_names: Vec<String>,
}

const DEFAULT_SCENE: &str = "examples/demo.hsc";
const EDITOR_FOV_DEG: f32 = 60.0;
const DELTA_SMALL: f32 = 0.1;
const DELTA_LARGE: f32 = 1.0;
const ACTION_LOG_LIMIT: usize = 64;
const PARTICLE_MATERIAL_LOGICAL_NAME: &str = "materials/particles/default";

fn compiled_root_for_host(host: &hold::EditorRuntimeHost) -> PathBuf {
    if let Some(project) = host.project() {
        return PathBuf::from(&project.assets_dir).join("compiled");
    }
    if let Some(source_path) = host.session().source_path() {
        if let Some(parent) = source_path.parent() {
            return parent.join("compiled");
        }
    }
    PathBuf::from("assets").join("compiled")
}

fn source_root_for_host(host: &hold::EditorRuntimeHost) -> PathBuf {
    if let Some(project) = host.project() {
        return PathBuf::from(&project.assets_dir).join("source");
    }
    if let Some(source_path) = host.session().source_path() {
        if let Some(parent) = source_path.parent() {
            return parent.join("source");
        }
    }
    PathBuf::from("assets").join("source")
}

fn compiled_asset_exists(compiled_root: &Path, logical_name: &str) -> bool {
    compiled_asset_path_for_logical_name(compiled_root, logical_name).exists()
}

/// Stand up the hot-reload pipeline for an `EditorRuntimeHost`.
/// Returns `None` when the source root the scene was opened from
/// doesn't resolve to a real on-disk directory — typical for
/// synthetic test scenes that ship from inside the repo's
/// `examples/` tree without a project. The shell still runs in
/// that case; the hot-reload panel just stays empty.
///
/// The pipeline is `FileWatcher` → `CompileQueue` → `ReloadDispatcher`
/// chained via two mpsc channels. Drop order on shutdown:
/// dispatcher's receiver disappears first → queue worker exits →
/// watcher worker exits. All threads join in their owning types'
/// Drop impls.
///
/// **What gets registered for reload?** v1 registers the scene
/// source itself for hot-reload. Mesh / texture / material /
/// shader registration is a follow-up — those need scene
/// introspection to discover which compiled assets the scene
/// actually references, plus existing handles to bind them to.
/// Scene reload is the headline Pillar 2 feature so it lands
/// first; expanding the target set is mechanical once a scene
/// → handle map is plumbed.
///
/// See `docs/adr/0004-hot-reload-architecture.md` Q3, Q10, Q14.
fn build_hot_reload_pipeline(host: &hold::EditorRuntimeHost) -> Option<HotReloadPipeline> {
    // Discover the source root the watcher should observe. Mirrors
    // `source_root_for_host` for consistency with the rest of the
    // shell's path resolution, but resolves to None when the path
    // doesn't exist — the watcher's CreateFileW would fail anyway.
    let source_root = source_root_for_host(host);
    if !source_root.exists() {
        return None;
    }

    // The scene's .hsc source path is the only target we register
    // in v1. If the host has no source path (in-memory-only scene)
    // we fall back to None — there's nothing to watch yet.
    let scene_source_path = host.session().source_path()?.to_path_buf();
    let scene_logical_name = scene_logical_name_for_host(host, &scene_source_path);
    let target = CompileTarget::new(
        scene_logical_name.clone(),
        CompiledAssetKind::Scene,
        scene_source_path.clone(),
    );

    // Wire the channels: watcher → queue → dispatcher.
    let (tx_changes, rx_changes) = std::sync::mpsc::channel();
    let (tx_results, rx_results) = std::sync::mpsc::channel();

    let watcher_config = WatcherConfig {
        root: source_root.clone(),
        extensions: ExtensionFilter::default_engine_extensions(),
        coalesce_window: std::time::Duration::from_millis(50),
    };

    // The watcher gets started first because its constructor opens
    // the watch root synchronously (CreateFileW). If the directory
    // is unreadable we catch the error here rather than silently
    // failing later — Pillar 4 (debuggability): never silently
    // disable a feature.
    let watcher = match FileWatcher::start_with_sender(watcher_config, tx_changes) {
        Ok(w) => w,
        Err(err) => {
            eprintln!("[editor] hot-reload disabled: watcher start failed: {err}");
            return None;
        }
    };

    let queue = match CompileQueue::start_with_sender(
        CompileQueueConfig {
            file_changes: rx_changes,
            targets: vec![target],
            compile_fn: default_compile_fn(),
        },
        tx_results,
    ) {
        Ok(q) => q,
        Err(err) => {
            eprintln!("[editor] hot-reload disabled: compile queue start failed: {err}");
            // Watcher will be dropped here; thread exits cleanly.
            return None;
        }
    };

    let mut dispatcher = ReloadDispatcher::new(rx_results);
    dispatcher.register_scene(scene_logical_name.clone(), scene_source_path);

    Some(HotReloadPipeline {
        _watcher: watcher,
        _queue: queue,
        dispatcher,
        source_root,
        registered_logical_names: vec![scene_logical_name],
    })
}

/// Best-effort logical-name derivation for the active scene.
/// Strips the project-relative source-root prefix when possible so
/// the dispatcher's logical name matches the convention used by
/// other compiled assets (`levels/level01` rather than an absolute
/// path). Falls back to the file stem if no project relativization
/// is available.
fn scene_logical_name_for_host(host: &hold::EditorRuntimeHost, scene_path: &Path) -> String {
    let source_root = source_root_for_host(host);
    if let Ok(rel) = scene_path.strip_prefix(&source_root) {
        // Drop the extension to match the "logical name = path
        // without extension" convention used elsewhere in the
        // pipeline.
        let mut rel_pb = PathBuf::from(rel);
        rel_pb.set_extension("");
        return rel_pb
            .to_string_lossy()
            .replace('\\', "/")
            .trim_start_matches('/')
            .to_string();
    }
    scene_path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "scene".to_string())
}

fn axis_slot(axis: EditorAxis) -> usize {
    match axis {
        EditorAxis::X => 0,
        EditorAxis::Y => 1,
        EditorAxis::Z => 2,
    }
}

struct Args {
    scene: PathBuf,
    backend: BackendKind,
    backend_info: bool,
    self_test_seconds: Option<f32>,
}

impl Args {
    fn parse() -> Self {
        let mut args = std::env::args().skip(1).peekable();
        let mut scene = PathBuf::from(DEFAULT_SCENE);
        let mut backend = BackendKind::Software;
        let mut backend_info = false;
        let mut self_test_seconds = None;

        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--scene" => {
                    if let Some(path) = args.next() {
                        scene = PathBuf::from(path);
                    }
                }
                "--backend" => {
                    if let Some(name) = args.next() {
                        backend = match name.as_str() {
                            "opengl" | "gl" => BackendKind::OpenGL,
                            "vulkan" | "vk" => BackendKind::Vulkan,
                            _ => BackendKind::Software,
                        };
                    }
                }
                "--backend-info" => backend_info = true,
                "--self-test" => self_test_seconds = Some(1.0),
                "--self-test-seconds" => {
                    if let Some(raw) = args.next() {
                        if let Ok(seconds) = raw.parse::<f32>() {
                            self_test_seconds = Some(seconds.max(0.0));
                        }
                    }
                }
                _ => {}
            }
        }

        Self {
            scene,
            backend,
            backend_info,
            self_test_seconds,
        }
    }
}

fn yes_no(value: bool) -> &'static str {
    if value {
        "yes"
    } else {
        "no"
    }
}

fn print_backend_info() {
    println!("[backend] capability snapshot");
    for backend in BackendKind::ALL {
        let capabilities = backend.capabilities();
        println!(
            "[backend] {} | maturity={} | shell={} | overlays={} | mesh_upload={} | textured_mesh={}",
            backend,
            capabilities.maturity,
            yes_no(capabilities.native_editor_shell_ready),
            yes_no(capabilities.editor_overlay_ready()),
            yes_no(capabilities.mesh_upload),
            yes_no(capabilities.textured_mesh),
        );
        println!("[backend]   summary: {}", capabilities.summary);
        println!("[backend]   gaps: {}", capabilities.gaps);
    }
}

struct EditorShellApp {
    server: EditorBridgeServer,
    backend: BackendKind,
    runtime_assets: AssetStore,
    runtime_loader: CompiledAssetRuntimeLoader,
    runtime_bindings: RuntimeSliceBindings,
    runtime_shader: hold::ShaderHandle,
    particle_material: Option<hold::MaterialHandle>,
    asset_errors_reported: HashSet<String>,
    status: String,
    ui_state: EditorNativeUiState,
    gizmo_mode: EditorViewportGizmoMode,
    hovered_entity: Option<u64>,
    hovered_handle: Option<EditorViewportGizmoHandleId>,
    viewport_drag: Option<ViewportDragState>,
    palette: CommandPaletteState,
    action_log: Vec<EditorUiActionLogEntry>,
    assistant_suggestions: Vec<EditorAssistantSuggestion>,
    self_test_remaining: Option<f32>,
    /// Decides when to checkpoint unsaved work to the recovery sidecar
    /// (M11). Pure timing; the write itself goes through the host.
    autosave: hold::AutosaveScheduler,
    /// Hot-reload pipeline. `None` when the source root the
    /// scene was opened from doesn't exist on disk (e.g.
    /// running from a synthetic test scene under `examples/`
    /// without a project root). The shell still runs without
    /// hot-reload — the panel just stays empty.
    hot_reload: Option<HotReloadPipeline>,
}

#[derive(Debug, Clone)]
struct ViewportDragState {
    handle: EditorViewportGizmoHandle,
    residual_deltas: [f32; 3],
}

#[derive(Debug, Clone, Default)]
struct CommandPaletteState {
    open: bool,
    query: String,
    selected_index: usize,
}

#[derive(Debug, Clone)]
struct EditorAssistantSuggestion {
    title: String,
    detail: String,
    commands: Vec<EditorCommand>,
}

impl EditorShellApp {
    fn new(
        server: EditorBridgeServer,
        backend: BackendKind,
        self_test_seconds: Option<f32>,
    ) -> Self {
        let mut runtime_assets = AssetStore::new();
        let runtime_shader = runtime_assets.insert(Shader {
            name: "runtime-slice-preview".into(),
            vertex_source: opengl_shaders::TEXTURED_VERT.into(),
            fragment_source: opengl_shaders::TEXTURED_FRAG.into(),
        });
        let compiled_root = compiled_root_for_host(server.host());
        // Build the hot-reload pipeline before moving `server` into
        // the struct literal — the pipeline build inspects the
        // host to discover the source root + scene path, but
        // doesn't keep a reference past construction.
        let hot_reload = build_hot_reload_pipeline(server.host());
        let mut app = Self {
            server,
            backend,
            runtime_assets,
            runtime_loader: CompiledAssetRuntimeLoader::new(compiled_root),
            runtime_bindings: RuntimeSliceBindings::default(),
            runtime_shader,
            particle_material: None,
            asset_errors_reported: HashSet::new(),
            status: String::new(),
            ui_state: EditorNativeUiState::default(),
            gizmo_mode: EditorViewportGizmoMode::Translate,
            hovered_entity: None,
            hovered_handle: None,
            viewport_drag: None,
            palette: CommandPaletteState::default(),
            action_log: Vec::new(),
            assistant_suggestions: Vec::new(),
            self_test_remaining: self_test_seconds,
            autosave: hold::AutosaveScheduler::default(),
            hot_reload,
        };
        app.push_action_log(
            EditorUiActionTone::Suggestion,
            "Shell ready",
            "Command palette suggestions stay explicit and user-triggered",
        );
        if let Some(seconds) = self_test_seconds {
            app.push_action_log(
                EditorUiActionTone::Suggestion,
                "Self-test armed",
                format!("Automatic shutdown after {:.2}s", seconds),
            );
        }
        // Surface hot-reload status as a startup log entry so
        // developers see "hot-reload watching <path>" or "hot-reload
        // not started: <reason>" in the action log on launch — same
        // surface the panel renders so there's one place to look.
        match &app.hot_reload {
            Some(pipeline) => {
                let names = if pipeline.registered_logical_names.is_empty() {
                    "(no targets)".to_string()
                } else {
                    pipeline.registered_logical_names.join(", ")
                };
                app.push_action_log(
                    EditorUiActionTone::Suggestion,
                    "Hot-reload watching",
                    format!(
                        "{} | targets: {}",
                        pipeline.source_root.display(),
                        names
                    ),
                );
            }
            None => {
                app.push_action_log(
                    EditorUiActionTone::Suggestion,
                    "Hot-reload disabled",
                    "Source root unavailable for this scene; reload pipeline not started",
                );
            }
        }

        // Crash-recovery probe (M11). A leftover recovery sidecar means unsaved
        // work never reached the source file. We PROMPT rather than auto-load:
        // silently replacing the scene the user just opened would be its own
        // kind of data loss. The palette exposes Recover / Discard.
        if let Some(path) = app.server.host().pending_recovery() {
            app.push_action_log(
                EditorUiActionTone::Error,
                "Unsaved work recovered",
                format!(
                    "A previous session ended with unsaved changes ({}). \
                     Open the command palette: Recover Autosave, or Discard Recovery.",
                    path.display()
                ),
            );
        }

        app.refresh_status();
        app
    }

    fn refresh_status(&mut self) {
        let scene_name = self.server.host().session().scene().name.clone();
        let backend = match self.backend {
            BackendKind::Software => "SW",
            BackendKind::OpenGL => "GL",
            BackendKind::Vulkan => "VK",
        };
        let play_state = match self.server.host().play_state() {
            EditorPlayState::Editing => "EDIT",
            EditorPlayState::Playing => "PLAY",
            EditorPlayState::Paused => "PAUSED",
            EditorPlayState::ReplayingDeterministic => "REPLAY",
        };
        let dirty = if self.server.host().is_dirty() {
            "*"
        } else {
            ""
        };
        let sync_state = if self.server.host().world_requires_rebuild() {
            "REBUILD"
        } else {
            "SYNC"
        };
        let history_state = format!(
            "U:{} R:{}",
            if self.server.host().can_undo() {
                "Y"
            } else {
                "N"
            },
            if self.server.host().can_redo() {
                "Y"
            } else {
                "N"
            }
        );

        let cursor_str = match self.server.host().cursor() {
            Some(cursor) => {
                let axis = cursor
                    .axis
                    .map(|axis| format!(".{}", axis))
                    .unwrap_or_default();
                format!(
                    " [{}] {}.{}{}",
                    cursor.entity_id, cursor.component_name, cursor.field_name, axis
                )
            }
            None => " (no selection)".to_string(),
        };

        self.status = format!(
            "{}{} | {} | {} | {} | {} | {}{}",
            scene_name,
            dirty,
            backend,
            play_state,
            self.gizmo_mode,
            sync_state,
            history_state,
            cursor_str
        );
    }

    fn report_asset_error_once(&mut self, key: impl Into<String>, detail: impl Into<String>) {
        let key = key.into();
        if !self.asset_errors_reported.insert(key) {
            return;
        }

        let detail = detail.into();
        self.push_action_log(
            EditorUiActionTone::Error,
            "Asset load failed",
            detail.clone(),
        );
        eprintln!("[editor] {detail}");
    }

    fn clear_runtime_asset_bindings(&mut self) {
        let compiled_root = self.runtime_loader.compiled_root().to_path_buf();
        self.runtime_loader = CompiledAssetRuntimeLoader::new(compiled_root);
        self.runtime_bindings = RuntimeSliceBindings::default();
        self.particle_material = None;
    }

    fn ensure_compiled_texture_from_source(
        &mut self,
        source_root: &Path,
        compiled_root: &Path,
        logical_name: &str,
    ) -> bool {
        let Some(source_path) =
            resolve_source_asset_path(source_root, logical_name, CompiledAssetKind::Texture)
        else {
            self.report_asset_error_once(
                format!("missing-source:texture:{logical_name}"),
                format!(
                    "missing texture source for '{}' under {}",
                    logical_name,
                    source_root.display()
                ),
            );
            return false;
        };

        let needs_compile = match source_asset_is_stale(compiled_root, &source_path, logical_name) {
            Ok(value) => value,
            Err(err) => {
                self.report_asset_error_once(
                    format!("stale-check:texture:{logical_name}"),
                    format!(
                        "failed to stat texture source '{}': {err}",
                        source_path.display()
                    ),
                );
                return false;
            }
        };
        if !needs_compile {
            return false;
        }

        match compile_asset_from_source(CompiledAssetKind::Texture, &source_path, logical_name)
            .and_then(|blob| write_compiled_asset(compiled_root, &blob).map(|_| blob))
        {
            Ok(_) => true,
            Err(err) => {
                self.report_asset_error_once(
                    format!("compile:texture:{logical_name}"),
                    format!(
                        "failed to compile texture '{}' from {}: {err}",
                        logical_name,
                        source_path.display()
                    ),
                );
                false
            }
        }
    }

    fn ensure_compiled_material_from_source(
        &mut self,
        source_root: &Path,
        compiled_root: &Path,
        logical_name: &str,
    ) -> bool {
        let Some(source_path) =
            resolve_source_asset_path(source_root, logical_name, CompiledAssetKind::Material)
        else {
            self.report_asset_error_once(
                format!("missing-source:material:{logical_name}"),
                format!(
                    "missing material source for '{}' under {}",
                    logical_name,
                    source_root.display()
                ),
            );
            return false;
        };

        let material_source = match fs::read_to_string(&source_path) {
            Ok(source) => source,
            Err(err) => {
                self.report_asset_error_once(
                    format!("read-source:material:{logical_name}"),
                    format!(
                        "failed to read material source '{}' from {}: {err}",
                        logical_name,
                        source_path.display()
                    ),
                );
                return false;
            }
        };
        let payload = match parse_compiled_material_source(&material_source) {
            Ok(payload) => payload,
            Err(err) => {
                self.report_asset_error_once(
                    format!("parse-source:material:{logical_name}"),
                    format!(
                        "failed to parse material source '{}' from {}: {err}",
                        logical_name,
                        source_path.display()
                    ),
                );
                return false;
            }
        };

        let mut changed = false;
        changed |= self.ensure_compiled_texture_from_source(
            source_root,
            compiled_root,
            &payload.albedo_logical_name,
        );
        if let Some(texture) = payload.normal_logical_name.as_deref() {
            changed |=
                self.ensure_compiled_texture_from_source(source_root, compiled_root, texture);
        }
        if let Some(texture) = payload.specular_gloss_logical_name.as_deref() {
            changed |=
                self.ensure_compiled_texture_from_source(source_root, compiled_root, texture);
        }
        if !compiled_asset_exists(compiled_root, &payload.albedo_logical_name) {
            return changed;
        }
        if let Some(texture) = payload.normal_logical_name.as_deref() {
            if !compiled_asset_exists(compiled_root, texture) {
                return changed;
            }
        }
        if let Some(texture) = payload.specular_gloss_logical_name.as_deref() {
            if !compiled_asset_exists(compiled_root, texture) {
                return changed;
            }
        }

        let needs_compile = match source_asset_is_stale(compiled_root, &source_path, logical_name) {
            Ok(value) => value,
            Err(err) => {
                self.report_asset_error_once(
                    format!("stale-check:material:{logical_name}"),
                    format!(
                        "failed to stat material source '{}': {err}",
                        source_path.display()
                    ),
                );
                return changed;
            }
        };
        if !needs_compile {
            return changed;
        }

        match compile_asset_from_source(CompiledAssetKind::Material, &source_path, logical_name)
            .and_then(|blob| write_compiled_asset(compiled_root, &blob).map(|_| blob))
        {
            Ok(_) => true,
            Err(err) => {
                self.report_asset_error_once(
                    format!("compile:material:{logical_name}"),
                    format!(
                        "failed to compile material '{}' from {}: {err}",
                        logical_name,
                        source_path.display()
                    ),
                );
                changed
            }
        }
    }

    fn ensure_compiled_mesh_from_source(
        &mut self,
        source_root: &Path,
        compiled_root: &Path,
        logical_name: &str,
    ) -> bool {
        let Some(source_path) =
            resolve_source_asset_path(source_root, logical_name, CompiledAssetKind::Mesh)
        else {
            self.report_asset_error_once(
                format!("missing-source:mesh:{logical_name}"),
                format!(
                    "missing mesh source for '{}' under {}",
                    logical_name,
                    source_root.display()
                ),
            );
            return false;
        };

        let needs_compile = match source_asset_is_stale(compiled_root, &source_path, logical_name) {
            Ok(value) => value,
            Err(err) => {
                self.report_asset_error_once(
                    format!("stale-check:mesh:{logical_name}"),
                    format!(
                        "failed to stat mesh source '{}': {err}",
                        source_path.display()
                    ),
                );
                return false;
            }
        };
        if !needs_compile {
            return false;
        }

        match compile_asset_from_source(CompiledAssetKind::Mesh, &source_path, logical_name)
            .and_then(|blob| write_compiled_asset(compiled_root, &blob).map(|_| blob))
        {
            Ok(_) => true,
            Err(err) => {
                self.report_asset_error_once(
                    format!("compile:mesh:{logical_name}"),
                    format!(
                        "failed to compile mesh '{}' from {}: {err}",
                        logical_name,
                        source_path.display()
                    ),
                );
                false
            }
        }
    }

    fn sync_runtime_slice_source_assets(&mut self, scene: &SceneData) {
        let source_root = source_root_for_host(self.server.host());
        let compiled_root = self.runtime_loader.compiled_root().to_path_buf();
        let mut materials = HashSet::<String>::new();
        let mut meshes = HashSet::<String>::new();

        for layer in &scene.runtime_slice.terrain.paint_layers {
            if let Some(material) = layer.material.as_deref() {
                materials.insert(material.to_string());
            }
        }

        for prototype in &scene.runtime_slice.vegetation_prototypes {
            if let Some(mesh) = prototype.mesh.as_deref() {
                meshes.insert(mesh.to_string());
            }
            if let Some(material) = prototype.material.as_deref() {
                materials.insert(material.to_string());
            }
        }

        for actor in &scene.runtime_slice.static_mesh_actors {
            if let Some(mesh) = actor.mesh.as_deref() {
                meshes.insert(mesh.to_string());
            }
            if let Some(material) = actor.material.as_deref() {
                materials.insert(material.to_string());
            }
        }

        for volume in &scene.runtime_slice.water_volumes {
            if let Some(material) = volume.material.as_deref() {
                materials.insert(material.to_string());
            }
        }

        for spawner in &scene.runtime_slice.vehicle_spawners {
            if let Some(mesh) = spawner.mesh.as_deref() {
                meshes.insert(mesh.to_string());
            }
            if let Some(material) = spawner.material.as_deref() {
                materials.insert(material.to_string());
            }
        }

        for prop in &scene.runtime_slice.breakable_props {
            if let Some(mesh) = prop.intact_mesh.as_deref() {
                meshes.insert(mesh.to_string());
            }
            if let Some(mesh) = prop.broken_mesh.as_deref() {
                meshes.insert(mesh.to_string());
            }
            if let Some(material) = prop.material.as_deref() {
                materials.insert(material.to_string());
            }
        }

        materials.insert(PARTICLE_MATERIAL_LOGICAL_NAME.to_string());

        let mut any_recompiled = false;
        for material in materials {
            any_recompiled |=
                self.ensure_compiled_material_from_source(&source_root, &compiled_root, &material);
        }
        for mesh in meshes {
            any_recompiled |=
                self.ensure_compiled_mesh_from_source(&source_root, &compiled_root, &mesh);
        }

        if any_recompiled {
            self.clear_runtime_asset_bindings();
            self.push_action_log(
                EditorUiActionTone::Event,
                "Compiled Assets Updated",
                format!(
                    "Rebuilt runtime slice assets from {}",
                    source_root.display()
                ),
            );
        }
    }

    fn ensure_compiled_material_binding(&mut self, logical_name: &str) {
        if self
            .runtime_bindings
            .resolve_material(logical_name)
            .is_some()
        {
            return;
        }

        match self.runtime_loader.ensure_material(
            &mut self.runtime_assets,
            logical_name,
            self.runtime_shader,
        ) {
            Ok(material) => self.runtime_bindings.bind_material(logical_name, material),
            Err(err) => self.report_asset_error_once(
                format!("load-material:{logical_name}"),
                format!("failed to load compiled material '{logical_name}': {err}"),
            ),
        }
    }

    fn ensure_compiled_mesh_binding(&mut self, logical_name: &str) {
        if self.runtime_bindings.resolve_mesh(logical_name).is_some() {
            return;
        }

        match self
            .runtime_loader
            .ensure_mesh(&mut self.runtime_assets, logical_name)
        {
            Ok(mesh) => self.runtime_bindings.bind_mesh(logical_name, mesh),
            Err(err) => self.report_asset_error_once(
                format!("load-mesh:{logical_name}"),
                format!("failed to load compiled mesh '{logical_name}': {err}"),
            ),
        }
    }

    fn ensure_particle_material(&mut self) -> Option<hold::MaterialHandle> {
        if let Some(material) = self.particle_material {
            return Some(material);
        }

        match self.runtime_loader.ensure_material(
            &mut self.runtime_assets,
            PARTICLE_MATERIAL_LOGICAL_NAME,
            self.runtime_shader,
        ) {
            Ok(material) => {
                self.particle_material = Some(material);
                Some(material)
            }
            Err(err) => {
                self.report_asset_error_once(
                    "load-material:particles/default",
                    format!(
                        "failed to load compiled material '{}': {err}",
                        PARTICLE_MATERIAL_LOGICAL_NAME
                    ),
                );
                None
            }
        }
    }

    fn sync_runtime_slice_compiled_assets(&mut self, scene: &SceneData) {
        for layer in &scene.runtime_slice.terrain.paint_layers {
            if let Some(material) = layer.material.as_deref() {
                self.ensure_compiled_material_binding(material);
            }
        }

        for prototype in &scene.runtime_slice.vegetation_prototypes {
            if let Some(mesh) = prototype.mesh.as_deref() {
                self.ensure_compiled_mesh_binding(mesh);
            }
            if let Some(material) = prototype.material.as_deref() {
                self.ensure_compiled_material_binding(material);
            }
        }

        for actor in &scene.runtime_slice.static_mesh_actors {
            if let Some(mesh) = actor.mesh.as_deref() {
                self.ensure_compiled_mesh_binding(mesh);
            }
            if let Some(material) = actor.material.as_deref() {
                self.ensure_compiled_material_binding(material);
            }
        }

        for volume in &scene.runtime_slice.water_volumes {
            if let Some(material) = volume.material.as_deref() {
                self.ensure_compiled_material_binding(material);
            }
        }

        for spawner in &scene.runtime_slice.vehicle_spawners {
            if let Some(mesh) = spawner.mesh.as_deref() {
                self.ensure_compiled_mesh_binding(mesh);
            }
            if let Some(material) = spawner.material.as_deref() {
                self.ensure_compiled_material_binding(material);
            }
        }

        for prop in &scene.runtime_slice.breakable_props {
            if let Some(mesh) = prop.intact_mesh.as_deref() {
                self.ensure_compiled_mesh_binding(mesh);
            }
            if let Some(mesh) = prop.broken_mesh.as_deref() {
                self.ensure_compiled_mesh_binding(mesh);
            }
            if let Some(material) = prop.material.as_deref() {
                self.ensure_compiled_material_binding(material);
            }
        }
    }

    fn active_render_state(&self, aspect: f32) -> (Mat4, Mat4, hold::Vec3) {
        match self.server.host().play_state() {
            // Replay uses the editor camera too (recorded simulation
            // drives gameplay state, the editor frames the view).
            EditorPlayState::Editing
            | EditorPlayState::Paused
            | EditorPlayState::ReplayingDeterministic => {
                let camera = &self.server.host().editor_camera;
                (
                    camera.view_matrix(),
                    Mat4::perspective_gl(EDITOR_FOV_DEG.to_radians(), aspect, 0.1, 1000.0),
                    camera.position,
                )
            }
            EditorPlayState::Playing => self
                .server
                .host()
                .world()
                .query::<ActiveCamera>()
                .filter_map(|(entity, _)| {
                    let camera = self.server.host().world().get::<Camera>(entity)?;
                    let position = self
                        .server
                        .host()
                        .world()
                        .get::<GlobalTransform>(entity)
                        .map(|global| global.matrix.extract_position())
                        .or_else(|| {
                            self.server
                                .host()
                                .world()
                                .get::<Transform>(entity)
                                .map(|transform| transform.position)
                        })
                        .unwrap_or(hold::Vec3::ZERO);
                    Some((camera.view, camera.proj, position))
                })
                .next()
                .unwrap_or((
                    Mat4::IDENTITY,
                    Mat4::perspective_gl(EDITOR_FOV_DEG.to_radians(), aspect, 0.1, 1000.0),
                    hold::Vec3::ZERO,
                )),
        }
    }

    fn active_light(&self) -> hold::LightItem {
        self.server
            .host()
            .world()
            .query::<DirectionalLight>()
            .map(|(_, light)| hold::LightItem {
                direction: light.direction,
                color: light.color,
                intensity: light.intensity,
                ambient: light.ambient,
            })
            .next()
            .unwrap_or_default()
    }

    fn apply_drag_deltas(&mut self, drag: &mut ViewportDragState, dx: i32, dy: i32) {
        for (axis, delta) in drag.handle.world_deltas_for_mouse_delta(dx, dy) {
            let slot = axis_slot(axis);
            let total_delta = drag.residual_deltas[slot] + delta;
            if total_delta.abs() >= 0.0005 {
                drag.residual_deltas[slot] = 0.0;
                self.dispatch_silent(EditorCommand::SetAxis(axis));
                self.dispatch_silent(EditorCommand::ApplyDelta(total_delta));
            } else {
                drag.residual_deltas[slot] = total_delta;
            }
        }
    }

    fn push_action_log(
        &mut self,
        tone: EditorUiActionTone,
        label: impl Into<String>,
        detail: impl Into<String>,
    ) {
        self.action_log.push(EditorUiActionLogEntry {
            label: label.into(),
            detail: detail.into(),
            tone,
        });
        let overflow = self.action_log.len().saturating_sub(ACTION_LOG_LIMIT);
        if overflow > 0 {
            self.action_log.drain(0..overflow);
        }
    }

    fn open_palette(&mut self) {
        self.palette.open = true;
        self.palette.selected_index = 0;
    }

    fn close_palette(&mut self) {
        self.palette.open = false;
        self.palette.query.clear();
        self.palette.selected_index = 0;
    }

    fn palette_matches(query: &str, title: &str, detail: &str, keywords: &str) -> bool {
        if query.trim().is_empty() {
            return true;
        }

        let haystack = format!("{title} {detail} {keywords}").to_ascii_lowercase();
        query
            .split_whitespace()
            .all(|term| haystack.contains(&term.to_ascii_lowercase()))
    }

    fn build_palette_model(&self) -> Option<EditorUiCommandPalette> {
        if !self.palette.open {
            return None;
        }

        let host = self.server.host();
        let play_state = host.play_state();
        let slice = &host.session().scene().runtime_slice;
        let query = self.palette.query.trim().to_string();
        let mut entries = Vec::new();

        let mut push_entry = |title: &str,
                              detail: &str,
                              keywords: &str,
                              enabled: bool,
                              commands: Vec<EditorCommand>,
                              source: EditorUiPaletteEntrySource| {
            if Self::palette_matches(&query, title, detail, keywords) {
                entries.push(EditorUiPaletteEntry {
                    title: title.to_string(),
                    detail: detail.to_string(),
                    source,
                    enabled,
                    commands,
                });
            }
        };

        push_entry(
            "Save Scene",
            "Write the current .hsc file",
            "save write disk scene",
            play_state == EditorPlayState::Editing && host.session().source_path().is_some(),
            vec![EditorCommand::Save],
            EditorUiPaletteEntrySource::BuiltIn,
        );

        // Crash recovery (M11). Only offered while a sidecar actually exists,
        // so the palette does not advertise a no-op.
        let has_recovery = host.pending_recovery().is_some();
        push_entry(
            "Recover Autosave",
            "Load unsaved work from a previous session (still unsaved afterwards)",
            "recover autosave crash restore unsaved",
            has_recovery && play_state == EditorPlayState::Editing,
            vec![EditorCommand::RecoverAutosave],
            EditorUiPaletteEntrySource::BuiltIn,
        );
        push_entry(
            "Discard Recovery",
            "Delete the recovery file and keep the scene as it is on disk",
            "discard recovery autosave delete",
            has_recovery,
            vec![EditorCommand::DiscardAutosave],
            EditorUiPaletteEntrySource::BuiltIn,
        );
        push_entry(
            "Undo Last Edit",
            "Restore the previous authored snapshot",
            "undo revert back history",
            play_state == EditorPlayState::Editing && host.can_undo(),
            vec![EditorCommand::Undo],
            EditorUiPaletteEntrySource::BuiltIn,
        );
        push_entry(
            "Redo Last Edit",
            "Reapply the last undone authored snapshot",
            "redo forward history",
            play_state == EditorPlayState::Editing && host.can_redo(),
            vec![EditorCommand::Redo],
            EditorUiPaletteEntrySource::BuiltIn,
        );
        push_entry(
            "Reload Source Scene",
            "Reload authored .hsc from disk",
            "reload source disk reopen",
            play_state == EditorPlayState::Editing && host.session().source_path().is_some(),
            vec![EditorCommand::ReloadSource],
            EditorUiPaletteEntrySource::BuiltIn,
        );
        push_entry(
            "Patch World",
            "Apply supported in-place runtime updates",
            "patch sync runtime world",
            true,
            vec![EditorCommand::ForcePatch],
            EditorUiPaletteEntrySource::BuiltIn,
        );
        push_entry(
            "Rebuild World",
            "Deterministically rebuild runtime from SceneDocument",
            "rebuild sync runtime world full",
            true,
            vec![EditorCommand::ForceRebuild],
            EditorUiPaletteEntrySource::BuiltIn,
        );
        push_entry(
            "Enter Play Mode",
            "Start the runtime simulation from editing",
            "play run simulate",
            play_state == EditorPlayState::Editing,
            vec![EditorCommand::EnterPlayMode],
            EditorUiPaletteEntrySource::BuiltIn,
        );
        push_entry(
            "Pause Play Mode",
            "Freeze the running simulation",
            "pause freeze play",
            play_state == EditorPlayState::Playing,
            vec![EditorCommand::PausePlayMode],
            EditorUiPaletteEntrySource::BuiltIn,
        );
        push_entry(
            "Resume Play Mode",
            "Resume from paused simulation state",
            "resume continue play",
            play_state == EditorPlayState::Paused,
            vec![EditorCommand::ResumePlayMode],
            EditorUiPaletteEntrySource::BuiltIn,
        );
        push_entry(
            "Stop Play Mode",
            "Restore the authored scene snapshot",
            "stop play restore authored",
            play_state != EditorPlayState::Editing,
            vec![EditorCommand::StopPlayMode],
            EditorUiPaletteEntrySource::BuiltIn,
        );
        push_entry(
            "Select Next Entity",
            "Advance the hierarchy selection",
            "entity next hierarchy select",
            true,
            vec![EditorCommand::SelectNextEntity],
            EditorUiPaletteEntrySource::BuiltIn,
        );
        push_entry(
            "Select Previous Entity",
            "Move back through the hierarchy selection",
            "entity previous hierarchy select",
            true,
            vec![EditorCommand::SelectPrevEntity],
            EditorUiPaletteEntrySource::BuiltIn,
        );
        push_entry(
            "Select Next Component",
            "Advance component focus for the current entity",
            "component next inspector select",
            true,
            vec![EditorCommand::SelectNextComponent],
            EditorUiPaletteEntrySource::BuiltIn,
        );
        push_entry(
            "Select Previous Component",
            "Move back through components for the current entity",
            "component previous inspector select",
            true,
            vec![EditorCommand::SelectPrevComponent],
            EditorUiPaletteEntrySource::BuiltIn,
        );
        push_entry(
            "Select Next Field",
            "Advance field focus in the inspector",
            "field next inspector select",
            true,
            vec![EditorCommand::SelectNextField],
            EditorUiPaletteEntrySource::BuiltIn,
        );
        push_entry(
            "Select Previous Field",
            "Move back through inspector fields",
            "field previous inspector select",
            true,
            vec![EditorCommand::SelectPrevField],
            EditorUiPaletteEntrySource::BuiltIn,
        );
        push_entry(
            "Set Axis X",
            "Focus X for Vec3 edits",
            "axis x vector",
            true,
            vec![EditorCommand::SetAxis(EditorAxis::X)],
            EditorUiPaletteEntrySource::BuiltIn,
        );
        push_entry(
            "Set Axis Y",
            "Focus Y for Vec3 edits",
            "axis y vector",
            true,
            vec![EditorCommand::SetAxis(EditorAxis::Y)],
            EditorUiPaletteEntrySource::BuiltIn,
        );
        push_entry(
            "Set Axis Z",
            "Focus Z for Vec3 edits",
            "axis z vector",
            true,
            vec![EditorCommand::SetAxis(EditorAxis::Z)],
            EditorUiPaletteEntrySource::BuiltIn,
        );
        push_entry(
            "Nudge Field Positive",
            "Apply +0.1 to the current numeric field",
            "nudge increase plus delta",
            play_state != EditorPlayState::Playing,
            vec![EditorCommand::ApplyDelta(DELTA_SMALL)],
            EditorUiPaletteEntrySource::BuiltIn,
        );
        push_entry(
            "Nudge Field Negative",
            "Apply -0.1 to the current numeric field",
            "nudge decrease minus delta",
            play_state != EditorPlayState::Playing,
            vec![EditorCommand::ApplyDelta(-DELTA_SMALL)],
            EditorUiPaletteEntrySource::BuiltIn,
        );

        let has_sand_layer = slice
            .terrain
            .paint_layers
            .iter()
            .any(|layer| layer.name == "sand");
        let has_palm_prototype = slice
            .vegetation_prototypes
            .iter()
            .any(|prototype| prototype.name == "palm");
        let has_route_a = slice
            .patrol_routes
            .iter()
            .any(|route| route.name == "route_a");
        let has_jeep_spawner = slice
            .vehicle_spawners
            .iter()
            .any(|spawner| spawner.name == "jeep");

        push_entry(
            "Set Demo Island Terrain",
            "Apply the authored island terrain envelope",
            "runtime slice terrain island outdoor vertical slice",
            play_state != EditorPlayState::Playing,
            vec![EditorCommand::SetTerrainSettings {
                size_meters: hold::Vec3::new(1024.0, 256.0, 1024.0),
                chunk_size_meters: 64,
                height_scale_meters: 80.0,
            }],
            EditorUiPaletteEntrySource::BuiltIn,
        );
        push_entry(
            if has_sand_layer {
                "Refresh Sand Terrain Layer"
            } else {
                "Add Sand Terrain Layer"
            },
            "Upsert the terrain paint layer used by the island slice",
            "runtime slice terrain layer sand paint material",
            play_state != EditorPlayState::Playing,
            vec![EditorCommand::UpsertTerrainPaintLayer {
                name: "sand".into(),
                material: Some("materials/sand".into()),
                tiling: 6.0,
                weightmap_channel: 0,
            }],
            EditorUiPaletteEntrySource::BuiltIn,
        );
        push_entry(
            "Paint Palm Vegetation Cluster",
            "Stamp the authored foliage stroke for the demo island",
            "runtime slice vegetation foliage palm paint",
            play_state != EditorPlayState::Playing && has_palm_prototype,
            vec![EditorCommand::PaintVegetation {
                prototype: "palm".into(),
                center: hold::Vec3::new(120.0, 0.0, 180.0),
                radius: 80.0,
                density: 0.9,
            }],
            EditorUiPaletteEntrySource::BuiltIn,
        );
        push_entry(
            if has_route_a {
                "Refresh Patrol Route A"
            } else {
                "Add Patrol Route A"
            },
            "Upsert the authored enemy patrol spline",
            "runtime slice patrol ai route",
            play_state != EditorPlayState::Playing,
            vec![EditorCommand::UpsertPatrolRoute {
                name: "route_a".into(),
                points: vec![
                    hold::Vec3::new(96.0, 0.0, 96.0),
                    hold::Vec3::new(160.0, 0.0, 128.0),
                    hold::Vec3::new(128.0, 0.0, 192.0),
                ],
                looped: true,
            }],
            EditorUiPaletteEntrySource::BuiltIn,
        );
        push_entry(
            if has_jeep_spawner {
                "Refresh Jeep Spawner"
            } else {
                "Add Jeep Spawner"
            },
            "Upsert the authored vehicle spawn point",
            "runtime slice vehicle jeep spawn",
            play_state != EditorPlayState::Playing,
            vec![EditorCommand::UpsertVehicleSpawner {
                name: "jeep".into(),
                mesh: Some("meshes/jeep".into()),
                material: Some("materials/jeep".into()),
                position: hold::Vec3::new(56.0, 0.0, 56.0),
                facing_yaw_deg: 90.0,
            }],
            EditorUiPaletteEntrySource::BuiltIn,
        );

        for suggestion in &self.assistant_suggestions {
            push_entry(
                &suggestion.title,
                &suggestion.detail,
                "ai suggest assist preview explicit",
                true,
                suggestion.commands.clone(),
                EditorUiPaletteEntrySource::Suggested,
            );
        }

        let selected_index = self
            .palette
            .selected_index
            .min(entries.len().saturating_sub(1));
        let helper_text = if self.assistant_suggestions.is_empty() {
            "Enter runs the highlighted action. Future AI suggestions will appear here only as explicit commands you can review first.".to_string()
        } else {
            format!(
                "Enter runs the highlighted action. {} AI suggestion(s) are loaded as explicit previewable actions.",
                self.assistant_suggestions.len()
            )
        };

        Some(EditorUiCommandPalette {
            query,
            helper_text,
            empty_text: "NO MATCHING ACTIONS".into(),
            entries,
            selected_index,
        })
    }

    fn move_palette_selection(&mut self, dir: i32, palette: Option<&EditorUiCommandPalette>) {
        let Some(palette) = palette else {
            self.palette.selected_index = 0;
            return;
        };
        if palette.entries.is_empty() {
            self.palette.selected_index = 0;
            return;
        }
        let len = palette.entries.len() as i32;
        self.palette.selected_index =
            ((self.palette.selected_index as i32 + dir).rem_euclid(len)) as usize;
    }

    fn collect_palette_text_input(input: &InputState) -> String {
        let mut out = String::new();
        for (key, ch) in [
            (b'A' as u32, 'a'),
            (b'B' as u32, 'b'),
            (b'C' as u32, 'c'),
            (b'D' as u32, 'd'),
            (b'E' as u32, 'e'),
            (b'F' as u32, 'f'),
            (b'G' as u32, 'g'),
            (b'H' as u32, 'h'),
            (b'I' as u32, 'i'),
            (b'J' as u32, 'j'),
            (b'K' as u32, 'k'),
            (b'L' as u32, 'l'),
            (b'M' as u32, 'm'),
            (b'N' as u32, 'n'),
            (b'O' as u32, 'o'),
            (b'P' as u32, 'p'),
            (b'Q' as u32, 'q'),
            (b'R' as u32, 'r'),
            (b'S' as u32, 's'),
            (b'T' as u32, 't'),
            (b'U' as u32, 'u'),
            (b'V' as u32, 'v'),
            (b'W' as u32, 'w'),
            (b'X' as u32, 'x'),
            (b'Y' as u32, 'y'),
            (b'Z' as u32, 'z'),
        ] {
            if input.key_pressed(key) {
                out.push(ch);
            }
        }
        if input.key_pressed(vk::SPACE) && !out.ends_with(' ') {
            out.push(' ');
        }
        out
    }

    fn process_palette_input(
        &mut self,
        input: &InputState,
        ui: &hold::EditorNativeUi,
        palette: Option<&EditorUiCommandPalette>,
    ) -> bool {
        if !self.palette.open {
            return false;
        }

        let (mx, my) = input.mouse_pos();
        if input.mouse_pressed(MouseButton::Left) {
            if ui.panel_at(mx, my) != Some(EditorUiPanel::CommandPalette) {
                self.close_palette();
                return true;
            }
            if let Some(commands) = ui.commands_at(mx, my) {
                self.dispatch_many(commands);
                self.close_palette();
                return true;
            }
        }

        if input.key_pressed(vk::ESCAPE) {
            self.close_palette();
            return true;
        }
        if input.key_pressed(vk::UP) {
            self.move_palette_selection(-1, palette);
            return true;
        }
        if input.key_pressed(vk::DOWN) {
            self.move_palette_selection(1, palette);
            return true;
        }
        if input.key_pressed(vk::ENTER) {
            if let Some(palette) = palette {
                if let Some(entry) = palette.entries.get(self.palette.selected_index) {
                    if entry.enabled {
                        self.dispatch_many(entry.commands.clone());
                    }
                }
            }
            self.close_palette();
            return true;
        }
        if input.key_pressed(vk::BACKSPACE) {
            self.palette.query.pop();
            self.palette.selected_index = 0;
            return true;
        }

        let typed = Self::collect_palette_text_input(input);
        if !typed.is_empty() {
            self.palette.query.push_str(&typed);
            self.palette.selected_index = 0;
            return true;
        }

        true
    }

    fn describe_command(command: &EditorCommand) -> (String, String) {
        match command {
            EditorCommand::Save => ("Command Save".into(), "Write authored scene to disk".into()),
            EditorCommand::Undo => (
                "Command Undo".into(),
                "Restore previous authored snapshot".into(),
            ),
            EditorCommand::Redo => (
                "Command Redo".into(),
                "Reapply undone authored snapshot".into(),
            ),
            EditorCommand::ReloadSource => (
                "Command Reload".into(),
                "Reload source scene from disk".into(),
            ),
            EditorCommand::ForcePatch => ("Command Patch".into(), "Try in-place world sync".into()),
            EditorCommand::ForceRebuild => (
                "Command Rebuild".into(),
                "Rebuild runtime world from document".into(),
            ),
            EditorCommand::RecoverAutosave => (
                "Command Recover".into(),
                "Load unsaved work from the autosave recovery file".into(),
            ),
            EditorCommand::DiscardAutosave => (
                "Command Discard Recovery".into(),
                "Delete the recovery file and keep the on-disk scene".into(),
            ),
            EditorCommand::EnterPlayMode => ("Command Play".into(), "Enter play mode".into()),
            EditorCommand::PausePlayMode => {
                ("Command Pause".into(), "Pause runtime simulation".into())
            }
            EditorCommand::ResumePlayMode => {
                ("Command Resume".into(), "Resume paused simulation".into())
            }
            EditorCommand::StopPlayMode => (
                "Command Stop".into(),
                "Stop play and restore authored scene".into(),
            ),
            EditorCommand::SelectNextEntity => ("Command Next Entity".into(), String::new()),
            EditorCommand::SelectPrevEntity => ("Command Prev Entity".into(), String::new()),
            EditorCommand::SelectNextComponent => ("Command Next Component".into(), String::new()),
            EditorCommand::SelectPrevComponent => ("Command Prev Component".into(), String::new()),
            EditorCommand::SelectNextField => ("Command Next Field".into(), String::new()),
            EditorCommand::SelectPrevField => ("Command Prev Field".into(), String::new()),
            EditorCommand::SelectEntity { entity_id } => {
                ("Command Select Entity".into(), format!("E{entity_id}"))
            }
            EditorCommand::SelectComponent { component_name, .. } => {
                ("Command Select Component".into(), component_name.clone())
            }
            EditorCommand::SelectField { field_name, .. } => {
                ("Command Select Field".into(), field_name.clone())
            }
            EditorCommand::SetAxis(axis) => ("Command Set Axis".into(), axis.to_string()),
            EditorCommand::ApplyDelta(delta) => {
                ("Command Apply Delta".into(), format!("{delta:+.2}"))
            }
            EditorCommand::SetTerrainSettings { .. } => (
                "Command Terrain Settings".into(),
                "Update runtime-slice terrain bounds".into(),
            ),
            EditorCommand::UpsertTerrainPaintLayer { name, .. } => {
                ("Command Terrain Layer".into(), name.clone())
            }
            EditorCommand::RemoveTerrainPaintLayer { name } => {
                ("Command Remove Terrain Layer".into(), name.clone())
            }
            EditorCommand::PaintVegetation { prototype, .. } => {
                ("Command Paint Vegetation".into(), prototype.clone())
            }
            EditorCommand::EraseVegetation { prototype, .. } => (
                "Command Erase Vegetation".into(),
                prototype.clone().unwrap_or_else(|| "all".into()),
            ),
            EditorCommand::UpsertPatrolRoute { name, .. } => {
                ("Command Patrol Route".into(), name.clone())
            }
            EditorCommand::RemovePatrolRoute { name } => {
                ("Command Remove Patrol Route".into(), name.clone())
            }
            EditorCommand::UpsertVehicleSpawner { name, .. } => {
                ("Command Vehicle Spawner".into(), name.clone())
            }
            EditorCommand::RemoveVehicleSpawner { name } => {
                ("Command Remove Vehicle Spawner".into(), name.clone())
            }
        }
    }

    fn describe_event(event: &EditorEvent) -> (String, String, EditorUiActionTone) {
        match event {
            EditorEvent::SelectionChanged(cursor) => (
                "Event Selection".into(),
                format!(
                    "E{} {}.{}",
                    cursor.entity_id, cursor.component_name, cursor.field_name
                ),
                EditorUiActionTone::Event,
            ),
            EditorEvent::FieldEdited { cursor, .. } => (
                "Event Field Edited".into(),
                format!("{}.{}", cursor.component_name, cursor.field_name),
                EditorUiActionTone::Event,
            ),
            EditorEvent::PlayModeChanged(state) => (
                "Event Play State".into(),
                format!("{state:?}"),
                EditorUiActionTone::Event,
            ),
            EditorEvent::PatchApplied => (
                "Event Patch Applied".into(),
                "World synced in place".into(),
                EditorUiActionTone::Event,
            ),
            EditorEvent::PatchRequiresRebuild { reason } => (
                "Event Rebuild Required".into(),
                reason.clone(),
                EditorUiActionTone::Error,
            ),
            EditorEvent::WorldRebuilt => (
                "Event World Rebuilt".into(),
                "Runtime entity map refreshed".into(),
                EditorUiActionTone::Event,
            ),
            EditorEvent::Saved => (
                "Event Scene Saved".into(),
                "Authored scene written to disk".into(),
                EditorUiActionTone::Event,
            ),
            EditorEvent::UndoApplied => (
                "Event Undo Applied".into(),
                "Previous authored snapshot restored".into(),
                EditorUiActionTone::Event,
            ),
            EditorEvent::RedoApplied => (
                "Event Redo Applied".into(),
                "Later authored snapshot restored".into(),
                EditorUiActionTone::Event,
            ),
            EditorEvent::Reloaded => (
                "Event Scene Reloaded".into(),
                "Authored scene reloaded from disk".into(),
                EditorUiActionTone::Event,
            ),
            EditorEvent::RuntimeSliceUpdated { path } => (
                "Event Runtime Slice Updated".into(),
                path.clone(),
                EditorUiActionTone::Event,
            ),
            EditorEvent::AutosaveRecovered { path } => (
                "Event Autosave Recovered".into(),
                format!("Unsaved work restored from {path}; still unsaved — press Ctrl+S"),
                EditorUiActionTone::Suggestion,
            ),
            EditorEvent::AutosaveDiscarded => (
                "Event Autosave Discarded".into(),
                "Recovery file deleted; on-disk scene kept".into(),
                EditorUiActionTone::Event,
            ),
            EditorEvent::AssetInvalidated {
                kind,
                handle_index,
                logical_name,
            } => (
                "Event Asset Hot-Reloaded".into(),
                format!("{} #{} -> {}", kind, handle_index, logical_name),
                EditorUiActionTone::Event,
            ),
            EditorEvent::CompileFailed {
                kind,
                logical_name,
                error,
            } => (
                "Event Compile Failed".into(),
                format!("{} {}: {}", kind, logical_name, error),
                EditorUiActionTone::Error,
            ),
            EditorEvent::FrameCaptured {
                generation,
                passes,
            } => (
                "Event Frame Captured".into(),
                format!("gen={} passes={}", generation, passes.len()),
                EditorUiActionTone::Event,
            ),
        }
    }

    fn process_input(&mut self, input: &InputState, width: u32, height: u32, vp: &Mat4) {
        let ctrl = input.key(vk::CTRL);
        let shift = input.key(vk::SHIFT);
        let palette_model = self.build_palette_model();
        let ui = build_native_ui(
            self.server.host(),
            width,
            height,
            &mut self.ui_state,
            &self.action_log,
            palette_model.as_ref(),
        );
        let viewport = build_viewport_overlay(
            self.server.host(),
            vp,
            width,
            height,
            self.gizmo_mode,
            self.hovered_entity,
            self.hovered_handle,
        );
        let (mx, my) = input.mouse_pos();
        let wheel = input.mouse_wheel();
        let pointer_over_panel = ui.panel_at(mx, my).is_some();

        if ctrl && input.key_pressed(vk::K) {
            if self.palette.open {
                self.close_palette();
            } else {
                self.open_palette();
            }
            return;
        }

        if self.process_palette_input(input, &ui, palette_model.as_ref()) {
            self.refresh_status();
            return;
        }

        if pointer_over_panel {
            self.hovered_entity = None;
            self.hovered_handle = None;
        } else {
            self.hovered_handle = viewport.pick_gizmo_handle(mx, my).map(|handle| handle.id());
            self.hovered_entity = if self.hovered_handle.is_some() {
                None
            } else {
                viewport.pick_entity(mx, my)
            };
        }

        if input.mouse_released(MouseButton::Left) {
            self.viewport_drag = None;
        }

        if wheel.abs() > f32::EPSILON {
            if let Some(panel) = ui.panel_at(mx, my) {
                let notches = wheel.abs().round().max(1.0) as i32;
                let lines = if wheel > 0.0 {
                    -3 * notches
                } else {
                    3 * notches
                };
                self.ui_state.scroll_lines(panel, lines);
            }
        }

        if input.key_pressed(vk::F1) {
            self.gizmo_mode = EditorViewportGizmoMode::Translate;
            self.refresh_status();
            println!("[editor] gizmo mode -> {}", self.gizmo_mode);
            println!("[editor] {}", self.status);
            return;
        }
        if input.key_pressed(vk::F2) {
            self.gizmo_mode = EditorViewportGizmoMode::Rotate;
            self.refresh_status();
            println!("[editor] gizmo mode -> {}", self.gizmo_mode);
            println!("[editor] {}", self.status);
            return;
        }
        if input.key_pressed(vk::F3) {
            self.gizmo_mode = EditorViewportGizmoMode::Scale;
            self.refresh_status();
            println!("[editor] gizmo mode -> {}", self.gizmo_mode);
            println!("[editor] {}", self.status);
            return;
        }

        if let Some(mut drag) = self.viewport_drag.take() {
            if self.server.host().play_state() == EditorPlayState::Playing
                || !input.mouse_button(MouseButton::Left)
            {
                self.hovered_handle = None;
            } else {
                let (dx, dy) = input.mouse_delta();
                self.apply_drag_deltas(&mut drag, dx, dy);
                self.viewport_drag = Some(drag);
                return;
            }
        }

        if input.mouse_pressed(MouseButton::Left) {
            if let Some(commands) = ui.commands_at(mx, my) {
                self.dispatch_many(commands);
                return;
            }

            if let Some(handle) = viewport.pick_gizmo_handle(mx, my) {
                self.dispatch_many(handle.activation_commands());
                self.hovered_handle = Some(handle.id());
                self.viewport_drag = Some(ViewportDragState {
                    handle,
                    residual_deltas: [0.0; 3],
                });
                return;
            }

            if let Some(entity_id) = viewport.pick_entity(mx, my) {
                self.dispatch(EditorCommand::SelectEntity { entity_id });
                return;
            }
        }

        if input.key_pressed(vk::ESCAPE) {
            std::process::exit(0);
        }

        if ctrl && input.key_pressed(vk::S) {
            self.dispatch(EditorCommand::Save);
            return;
        }

        if ctrl && input.key_pressed(vk::Z) {
            self.dispatch(EditorCommand::Undo);
            return;
        }

        if ctrl && input.key_pressed(vk::Y) {
            self.dispatch(EditorCommand::Redo);
            return;
        }

        if ctrl && input.key_pressed(vk::P) {
            self.dispatch(EditorCommand::ForcePatch);
            return;
        }

        if ctrl && input.key_pressed(vk::R) {
            self.dispatch(EditorCommand::ForceRebuild);
            return;
        }

        if input.key_pressed(vk::F7) {
            self.dispatch(EditorCommand::ReloadSource);
            return;
        }

        // F10: toggle the M9 frame inspector (ADR 0002 Q8, ADR 0003
        // Pillar 4). When on, per-frame render uses
        // `submit_frame_capturing` and pushes thumbnails to the host's
        // FrameInspectorState so the native UI panel renders them.
        // When off, capture stops and the panel disappears; the last
        // capture stays in state so the user can compare across
        // toggles by disabling capture mid-inspection.
        if input.key_pressed(vk::F10) {
            self.toggle_frame_inspector();
            return;
        }

        // F11 / F12: live record/replay (ADR 0005 — M8.0 wire-up).
        // F11 toggles recording; F12 starts replay against the most
        // recently completed recording (or surfaces a "no recording"
        // message if none exists). The handlers consult the host's
        // replay state machine for transitions and the action log
        // for user-visible feedback. F-keys win even during replay
        // (apply_recorded preserves the OS-captured F-key state for
        // this frame because we read input *before* the per-frame
        // replay tick overwrites it — see tick_replay below).
        if input.key_pressed(vk::F11) {
            self.toggle_recording();
            return;
        }
        if input.key_pressed(vk::F12) {
            self.toggle_replay();
            return;
        }

        if input.key_pressed(vk::F8) {
            match self.server.host().play_state() {
                EditorPlayState::Editing => self.dispatch(EditorCommand::EnterPlayMode),
                EditorPlayState::Playing | EditorPlayState::Paused => {
                    self.dispatch(EditorCommand::StopPlayMode)
                }
                // Replay mode: F8 is a no-op. Replay control comes from
                // the determinism harness UI (M8), not the play-mode
                // shortcuts.
                EditorPlayState::ReplayingDeterministic => {}
            }
            return;
        }

        if input.key_pressed(vk::F9) {
            match self.server.host().play_state() {
                EditorPlayState::Playing => self.dispatch(EditorCommand::PausePlayMode),
                EditorPlayState::Paused => self.dispatch(EditorCommand::ResumePlayMode),
                EditorPlayState::Editing => {}
                EditorPlayState::ReplayingDeterministic => {}
            }
            return;
        }

        if input.key_pressed(vk::TAB) {
            if shift {
                self.dispatch(EditorCommand::SelectPrevEntity);
            } else {
                self.dispatch(EditorCommand::SelectNextEntity);
            }
        }

        if input.key_pressed(vk::LBRACKET) {
            self.dispatch(EditorCommand::SelectPrevComponent);
        }
        if input.key_pressed(vk::RBRACKET) {
            self.dispatch(EditorCommand::SelectNextComponent);
        }

        if input.key_pressed(vk::SEMICOLON) {
            self.dispatch(EditorCommand::SelectPrevField);
        }
        if input.key_pressed(vk::APOSTROPHE) {
            self.dispatch(EditorCommand::SelectNextField);
        }

        if input.key_pressed(vk::KEY_1) {
            self.dispatch(EditorCommand::SetAxis(EditorAxis::X));
        }
        if input.key_pressed(vk::KEY_2) {
            self.dispatch(EditorCommand::SetAxis(EditorAxis::Y));
        }
        if input.key_pressed(vk::KEY_3) {
            self.dispatch(EditorCommand::SetAxis(EditorAxis::Z));
        }

        let step = if shift { DELTA_LARGE } else { DELTA_SMALL };
        if input.key_pressed(vk::MINUS) {
            self.dispatch(EditorCommand::ApplyDelta(-step));
        }
        if input.key_pressed(vk::EQUALS) {
            self.dispatch(EditorCommand::ApplyDelta(step));
        }
    }

    fn dispatch_many(&mut self, commands: Vec<EditorCommand>) {
        self.dispatch_many_internal(commands, true);
    }

    fn dispatch_many_internal(&mut self, commands: Vec<EditorCommand>, log: bool) {
        let mut all_events = Vec::new();
        for command in commands {
            if log {
                let (label, detail) = Self::describe_command(&command);
                self.push_action_log(EditorUiActionTone::Command, label, detail);
            }
            match self.dispatch_protocol(command) {
                Ok(events) => all_events.extend(events),
                Err(err) => {
                    self.print_error(&err, log);
                    break;
                }
            }
        }

        if !all_events.is_empty() {
            self.handle_events(&all_events, log);
        }
    }

    fn dispatch(&mut self, command: EditorCommand) {
        self.dispatch_internal(command, true);
    }

    fn dispatch_silent(&mut self, command: EditorCommand) {
        self.dispatch_internal(command, false);
    }

    fn dispatch_internal(&mut self, command: EditorCommand, log: bool) {
        if log {
            let (label, detail) = Self::describe_command(&command);
            self.push_action_log(EditorUiActionTone::Command, label, detail);
        }
        match self.dispatch_protocol(command) {
            Ok(events) => self.handle_events(&events, log),
            Err(err) => self.print_error(&err, log),
        }
    }

    fn dispatch_protocol(
        &mut self,
        command: EditorCommand,
    ) -> Result<Vec<EditorEvent>, EditorProtocolError> {
        let message = encode_editor_command_protocol(&command);
        let encoded_events = self.server.execute_message(&message)?;
        encoded_events
            .iter()
            .map(|event| decode_editor_event_protocol(event))
            .collect()
    }

    fn handle_events(&mut self, events: &[EditorEvent], log: bool) {
        for event in events {
            if log {
                let (label, detail, tone) = Self::describe_event(event);
                self.push_action_log(tone, label, detail);
            }
            if log {
                match event {
                    EditorEvent::SelectionChanged(cursor) => {
                        let axis = cursor
                            .axis
                            .map(|axis| format!(" axis={}", axis))
                            .unwrap_or_default();
                        println!(
                            "[editor] selected entity {} - {} - {}{}",
                            cursor.entity_id, cursor.component_name, cursor.field_name, axis
                        );
                    }
                    EditorEvent::FieldEdited { cursor, new_value } => {
                        println!(
                            "[editor] {}.{} = {:?}",
                            cursor.component_name, cursor.field_name, new_value
                        );
                    }
                    EditorEvent::PlayModeChanged(state) => {
                        println!("[editor] play state -> {:?}", state);
                    }
                    EditorEvent::PatchApplied => {
                        println!("[editor] world patched (in-place)");
                    }
                    EditorEvent::PatchRequiresRebuild { reason } => {
                        println!("[editor] patch requires rebuild: {reason} (press Ctrl+R)");
                    }
                    EditorEvent::WorldRebuilt => {
                        println!("[editor] world rebuilt");
                    }
                    EditorEvent::Saved => {
                        println!("[editor] scene saved");
                    }
                    EditorEvent::UndoApplied => {
                        println!("[editor] undo restored the previous authored snapshot");
                    }
                    EditorEvent::RedoApplied => {
                        println!("[editor] redo restored the later authored snapshot");
                    }
                    EditorEvent::Reloaded => {
                        println!("[editor] scene reloaded from source");
                    }
                    EditorEvent::RuntimeSliceUpdated { path } => {
                        println!("[editor] runtime slice updated: {path}");
                    }
                    EditorEvent::AssetInvalidated {
                        kind,
                        handle_index,
                        logical_name,
                    } => {
                        println!(
                            "[editor] hot-reload applied: {} #{} -> {}",
                            kind, handle_index, logical_name
                        );
                    }
                    EditorEvent::CompileFailed {
                        kind,
                        logical_name,
                        error,
                    } => {
                        println!(
                            "[editor] compile failed for {} {}: {}",
                            kind, logical_name, error
                        );
                    }
                    EditorEvent::FrameCaptured {
                        generation,
                        passes,
                    } => {
                        println!(
                            "[editor] frame captured gen={} passes={}",
                            generation,
                            passes.len()
                        );
                    }
                    EditorEvent::AutosaveRecovered { path } => {
                        println!("[editor] recovered unsaved work from {path}");
                    }
                    EditorEvent::AutosaveDiscarded => {
                        println!("[editor] discarded autosave recovery file");
                    }
                }
            }
        }

        self.refresh_status();
        if log {
            println!("[editor] {}", self.status);
        }
    }

    fn print_error(&mut self, err: &EditorProtocolError, log: bool) {
        if log {
            self.push_action_log(
                EditorUiActionTone::Error,
                "Command Rejected",
                err.to_string(),
            );
        }
        eprintln!("[editor] {err}");
    }

    /// Per-frame hot-reload tick. Drains the dispatcher's compile
    /// results (which were produced asynchronously on the worker
    /// thread since the last frame), routes them through the
    /// host's `apply_dispatch_summary` — which updates the
    /// AssetStore for asset reloads, drives
    /// `EditorSceneSession::reload_scene_document` for scenes, and
    /// records each outcome on the Hot-Reload Status panel.
    ///
    /// Skips silently when no pipeline is wired (test scene without
    /// a project root) — the editor stays usable, the status panel
    /// just stays empty.
    ///
    // ── Live record/replay (ADR 0005 — M8.0) ─────────────────────────────

    /// F11 handler: toggle the replay state machine between
    /// `Normal` and `Recording`. If currently replaying, stops
    /// the replay first (replay → Normal → Recording in that
    /// order). Surfaces the transition on the editor's action log
    /// with the captured frame count so the developer can confirm
    /// "I just recorded N frames."
    fn toggle_recording(&mut self) {
        // Reading via `replay()` keeps the pre-transition state
        // mode visible for the log message; mutation goes through
        // `replay_mut()` below.
        let was_recording = self.server.host().replay().is_recording();
        let was_replaying = self.server.host().replay().is_replaying();

        if was_recording {
            let frames = self.server.host_mut().replay_mut().stop_recording();
            self.push_action_log(
                EditorUiActionTone::Suggestion,
                "Recording stopped",
                format!("Captured {frames} frames; press F12 to replay."),
            );
        } else {
            // Cancel any in-flight replay first (start_recording
            // is the user's "fresh state" signal — replay isn't a
            // valid sibling state).
            if was_replaying {
                self.server.host_mut().replay_mut().stop_replay();
                // Restoring play_state to Editing happens implicitly:
                // the next App::update tick observes the new replay
                // mode and updates play_state accordingly via
                // `sync_play_state_with_replay`.
            }
            // v1: seed always 0. A future "seeded RNG" milestone
            // will pull this from the host's RNG state.
            self.server.host_mut().replay_mut().start_recording(0);
            self.push_action_log(
                EditorUiActionTone::Suggestion,
                "Recording started",
                "F11 again to stop. Inputs are captured per-frame.",
            );
        }
    }

    /// F12 handler: toggle the replay state machine between
    /// `Normal` and `Replaying`. If no recording exists, surfaces
    /// a "no recording — press F11 first" message rather than
    /// silently failing.
    fn toggle_replay(&mut self) {
        let was_replaying = self.server.host().replay().is_replaying();
        if was_replaying {
            self.server.host_mut().replay_mut().stop_replay();
            self.push_action_log(
                EditorUiActionTone::Suggestion,
                "Replay cancelled",
                "Press F12 again to replay from the start.",
            );
            return;
        }

        // Not currently replaying. Try to start.
        let started = self.server.host_mut().replay_mut().start_replay();
        if started {
            // Surface the count for confirmation. last_recording
            // is guaranteed Some(_) when start_replay returns true.
            let count = self
                .server
                .host()
                .replay()
                .last_recording()
                .map(|r| r.len())
                .unwrap_or(0);
            self.push_action_log(
                EditorUiActionTone::Suggestion,
                "Replay started",
                format!("Replaying {count} frames; F12 to cancel."),
            );
        } else {
            self.push_action_log(
                EditorUiActionTone::Error,
                "No recording available",
                "Press F11 to start recording first; then F12 to replay.",
            );
        }
    }

    /// Bidirectional sync between the editor's play_state and
    /// the replay state machine's mode. Called once per frame
    /// from `App::update` after `tick_replay`.
    ///
    /// - When replay just started, set play_state to
    ///   `ReplayingDeterministic` so the hot-reload dispatcher's
    ///   existing replay-lock observes it (ADR 0004 Q7).
    /// - When replay just ended (auto-finished or user-cancelled),
    ///   restore play_state to `Editing` so reloads resume and
    ///   the editor's "REC" / "REPLAY" indicator clears.
    ///
    /// Idempotent: setting the same state repeatedly is a no-op.
    /// Recording does NOT touch play_state — recording is a
    /// passive observer of live play, so the user's existing
    /// Editing/Playing/Paused choice is preserved.
    fn sync_play_state_with_replay(&mut self) {
        let replaying = self.server.host().replay().is_replaying();
        let play_state = self.server.host().play_state();
        match (replaying, play_state) {
            (true, EditorPlayState::ReplayingDeterministic) => {} // already synced
            (true, _) => {
                // Just entered replay: lock to ReplayingDeterministic.
                let _previous = self
                    .server
                    .host_mut()
                    .session_mut()
                    .enter_replay_mode();
            }
            (false, EditorPlayState::ReplayingDeterministic) => {
                // Just exited replay: restore to Editing.
                self.server.host_mut().session_mut().exit_replay_mode();
            }
            (false, _) => {} // not replaying, not in replay state — nothing to do
        }
    }

    /// Per-frame tick for the replay state machine. Called from
    /// `App::update` after the F-key handlers and before the
    /// gameplay update so the rest of the frame sees consistent
    /// input state.
    ///
    /// Returns the effective input + dt the rest of the frame
    /// should consume:
    /// - In `Normal` mode: the live OS input + raw dt unchanged.
    /// - In `Recording` mode: live OS input + raw dt, plus a
    ///   side-effect of appending this frame to the recorder.
    /// - In `Replaying` mode: a synthesized [`InputState`]
    ///   reconstructed from the recorded frame, plus the
    ///   recorded `dt`. When the recording exhausts, the state
    ///   machine returns to `Normal` automatically and the
    ///   action log gets a "replay finished" entry.
    ///
    /// The returned `InputState` is owned (cloned from OS input
    /// or freshly constructed for replay) — the caller can pass
    /// it to gameplay logic without disturbing the engine's
    /// underlying `&InputState`.
    fn tick_replay(&mut self, os_input: &InputState, raw_dt: f32) -> (InputState, f32) {
        // Recording is a passive observer: capture and pass live
        // input through unchanged.
        if self.server.host().replay().is_recording() {
            self.server
                .host_mut()
                .replay_mut()
                .tick_recording(os_input, raw_dt);
            return (os_input.clone(), raw_dt);
        }

        // Replay drives input + dt from the recording.
        if self.server.host().replay().is_replaying() {
            let mut replayed = InputState::new();
            let outcome = self
                .server
                .host_mut()
                .replay_mut()
                .tick_replay(&mut replayed);
            match outcome {
                hold::ReplayTickOutcome::Replayed { dt } => {
                    return (replayed, dt);
                }
                hold::ReplayTickOutcome::Finished => {
                    // Recording exhausted on this tick. Surface
                    // the transition; pass live OS input through
                    // for this frame so the editor stays
                    // responsive immediately rather than waiting
                    // one frame.
                    self.push_action_log(
                        EditorUiActionTone::Suggestion,
                        "Replay finished",
                        "Reached end of recording; back to normal mode.",
                    );
                    return (os_input.clone(), raw_dt);
                }
                hold::ReplayTickOutcome::NotReplaying => {
                    // Defensive: shouldn't happen given the
                    // outer is_replaying check, but tolerate it.
                    return (os_input.clone(), raw_dt);
                }
            }
        }

        (os_input.clone(), raw_dt)
    }

    /// F10 handler: flip the M9 frame-inspector enable flag on the
    /// host, log the transition to the editor's action log so the
    /// developer sees the toggle land. When enabled, the next frame's
    /// render goes through `submit_frame_capturing`; when disabled,
    /// the panel disappears (the last capture stays in state — an
    /// on-purpose "compare against previous" affordance).
    ///
    /// See ADR 0002 Q8 (frame inspector) and ADR 0003 Pillar 4
    /// (debuggability).
    fn toggle_frame_inspector(&mut self) {
        let now_enabled = self
            .server
            .host_mut()
            .frame_inspector_mut()
            .toggle_enabled();
        let (label, detail) = if now_enabled {
            (
                "Frame inspector on",
                "Per-pass thumbnails will land at the bottom each frame; press F10 to disable.",
            )
        } else {
            (
                "Frame inspector off",
                "Capture paused; the last capture stays in state until F10 re-enables.",
            )
        };
        self.push_action_log(EditorUiActionTone::Suggestion, label, detail);
    }

    /// Per-frame autosave tick (M11). Asks the scheduler whether enough idle
    /// time has passed since the last edit (or whether the interval floor has
    /// elapsed during continuous editing), and if so checkpoints the scene to
    /// its recovery sidecar.
    ///
    /// Never touches the source file and never clears the dirty flag — the
    /// user still has unsaved work. A failed autosave is logged and the
    /// scheduler is NOT reset, so it retries on the next opportunity rather
    /// than silently giving up on the user's data.
    fn tick_autosave(&mut self, dt: f32) {
        let host = self.server.host();
        let has_work = host.has_unsaved_authored_work();
        let can_autosave = host.play_state() == EditorPlayState::Editing;
        let revision = host.edit_revision();
        if !self.autosave.tick(dt, has_work, can_autosave, revision) {
            return;
        }

        match self.server.host().autosave_to_recovery() {
            Ok(Some(path)) => {
                self.autosave.note_saved();
                self.push_action_log(
                    EditorUiActionTone::Suggestion,
                    "Autosaved",
                    format!("Recovery checkpoint written to {}", path.display()),
                );
            }
            Ok(None) => {
                // Untitled scene: no recovery target. Advance the watermark so
                // we do not ask again until a materially different edit.
                self.autosave.note_saved();
            }
            Err(err) => {
                // note_failed() arms a back-off so a persistently-failing write
                // (read-only recovery dir, a locked file) is retried after a
                // delay rather than re-attempted every single frame. Losing an
                // autosave silently is exactly what this milestone prevents.
                self.autosave.note_failed();
                self.push_action_log(
                    EditorUiActionTone::Error,
                    "Autosave failed",
                    format!("{err}"),
                );
            }
        }
    }

    /// Called from `App::update` at the very top of every frame
    /// so reloads land before the editor camera tick, input
    /// processing, and the renderer; the rest of the frame sees
    /// post-reload state.
    fn tick_hot_reload(&mut self) {
        let Some(pipeline) = self.hot_reload.as_mut() else {
            return;
        };
        let play_state = self.server.host().play_state();
        let summary = pipeline
            .dispatcher
            .dispatch_with_play_state(&mut self.runtime_assets, play_state);
        // Skip the work if the dispatch was a no-op. Avoids
        // pushing a "no activity" log line every idle frame.
        let has_work = !summary.applied.is_empty()
            || !summary.scenes_reloaded.is_empty()
            || !summary.errors.is_empty();
        if !has_work {
            return;
        }

        let report = self.server.host_mut().apply_dispatch_summary(summary);

        // Surface the summary on the action log too so developers
        // see "hot-reload: 1 scene diffed, 0 assets, 0 errors"
        // without having to look at the corner panel. The status
        // panel always carries the per-asset detail; the action
        // log is the chronological narrative of what happened.
        if report.has_activity() {
            let detail = format!(
                "{} assets | scenes: {} diffed / {} no-change / {} rebuilt / {} failed | {} errors",
                report.assets_applied,
                report.scenes_diffed,
                report.scenes_no_change,
                report.scenes_rebuilt,
                report.scenes_failed,
                report.errors_recorded,
            );
            let tone = if report.scenes_failed > 0 || report.errors_recorded > 0 {
                EditorUiActionTone::Error
            } else {
                EditorUiActionTone::Suggestion
            };
            self.push_action_log(tone, "Hot-reload tick", detail);
        }
    }
}

impl App for EditorShellApp {
    fn update(
        &mut self,
        world: &mut hold::World,
        renderer: &mut dyn Renderer,
        input: &InputState,
        dt: f32,
    ) {
        // Hot-reload tick (ADR 0004 Q10). Runs before any other
        // per-frame work so reloads land at frame boundary and the
        // rest of the update sees the post-reload state. Replay
        // mode is gated inside dispatch_with_play_state — passing
        // the host's current play_state lets the dispatcher buffer
        // reloads while replay is recording, per Q7.
        self.tick_hot_reload();

        // Autosave tick (M11, ADR 0003 Pillar 3). Debounced and dirty-gated;
        // is_dirty() is already Editing-only, so play/replay can never
        // checkpoint transient simulation state as authored work.
        self.tick_autosave(dt);

        // Live record/replay tick (ADR 0005 — M8.0). Captures
        // input when recording, synthesizes input from the
        // recording when replaying. Returns the effective
        // (input, dt) the simulation portion of the frame should
        // see — `process_input` below still uses raw OS input
        // for F-key handling so F12-to-cancel-replay always
        // works regardless of what the recording captured.
        let (effective_input, effective_dt) = self.tick_replay(input, dt);
        // Sync the editor's play_state to ReplayingDeterministic
        // when replay is active so the hot-reload dispatcher's
        // existing replay-lock kicks in (ADR 0004 Q7 +
        // ADR 0005 Q1 interaction).
        self.sync_play_state_with_replay();
        self.server
            .host_mut()
            .update_editor_camera(&effective_input, effective_dt);

        let height = renderer.height().max(1) as f32;
        let aspect = renderer.width() as f32 / height;
        let input_vp: Mat4 = self.server.host().active_view_proj(EDITOR_FOV_DEG, aspect);
        // process_input deliberately reads RAW OS input — F-key
        // shortcuts (F11/F12 record/replay, F8 play, etc.) must
        // win over any captured F-key state in the recording.
        // Without this carve-out, a recording that included an
        // F12 press would re-trigger replay forever on every
        // playback, and the user could never escape replay mode.
        self.process_input(input, renderer.width(), renderer.height(), &input_vp);
        let vp: Mat4 = self.server.host().active_view_proj(EDITOR_FOV_DEG, aspect);

        let mut draw_list = hold::generate_draw_list(
            self.server.host().world(),
            self.server.host().session().last_world_map(),
            self.server.host().session().scene(),
            self.server.host().cursor().as_ref(),
        );
        let viewport = build_viewport_overlay(
            self.server.host(),
            &vp,
            renderer.width(),
            renderer.height(),
            self.gizmo_mode,
            self.hovered_entity,
            self.hovered_handle,
        );
        draw_list.lines.extend(viewport.debug_lines.iter().copied());
        renderer.render_debug_lines(&draw_list.lines, &vp);
        renderer.render_screen_quads(&viewport.quads);

        let palette_model = self.build_palette_model();
        let ui = build_native_ui(
            self.server.host(),
            renderer.width(),
            renderer.height(),
            &mut self.ui_state,
            &self.action_log,
            palette_model.as_ref(),
        );

        let scene = self
            .server
            .host()
            .session()
            .scene()
            .to_scene_data()
            .unwrap_or_else(|_| SceneData {
                name: self.server.host().session().scene().name.clone(),
                runtime_slice: hold::RuntimeSliceAuthoring::default(),
                entities: Vec::new(),
            });
        self.sync_runtime_slice_source_assets(&scene);
        self.sync_runtime_slice_compiled_assets(&scene);
        let particle_material = self.ensure_particle_material();
        let particle_batch =
            hold::generate_particle_batch(self.server.host().world(), particle_material);

        let (view, projection, camera_position) = self.active_render_state(aspect);
        let mut frame: RenderFrame = build_runtime_slice_frame(
            &scene,
            &self.runtime_bindings,
            RuntimeSliceViewConfig {
                viewport: RenderViewport {
                    x: 0,
                    y: 0,
                    width: renderer.width(),
                    height: renderer.height(),
                },
                clear_color: hold::Color::new(0.10, 0.10, 0.12, 1.0),
                view,
                projection,
                camera_position,
                light: self.active_light(),
                fog: None,
                draw_authoring_helpers: true,
            },
        );
        if let (Some(view), Some(batch)) = (frame.primary_view_mut(), particle_batch) {
            view.particle_batches.push(batch);
        }
        frame.debug_lines.extend(draw_list.lines.drain(..));
        frame
            .debug_lines
            .extend(viewport.debug_lines.iter().copied());
        frame.screen_quads.extend(viewport.quads.iter().copied());
        frame.screen_quads.extend(ui.quads.iter().copied());
        // M9: when the inspector is armed, route rendering through
        // `submit_frame_capturing` so per-pass thumbnails come back
        // and land in the host's FrameInspectorState. Otherwise take
        // the cheap path. See ADR 0002 Q8 + ADR 0003 Pillar 4.
        if self.server.host().frame_inspector().is_enabled() {
            let thumbs = renderer.submit_frame_capturing(
                &self.runtime_assets,
                &frame,
                hold::DEFAULT_THUMBNAIL_SIZE.0,
                hold::DEFAULT_THUMBNAIL_SIZE.1,
            );
            if !thumbs.is_empty() {
                let _generation = self
                    .server
                    .host_mut()
                    .frame_inspector_mut()
                    .push_capture(thumbs);
                // NB: `EditorEvent::FrameCaptured` is defined and
                // codec-tested for the bridge, but there is no
                // out-of-band emit path on `EditorBridgeServer` yet —
                // events flow as return values from `execute`. When
                // the async emit lands, generation + FrameInspectorPass
                // metadata should ship through it. Tracked as an M9
                // follow-up alongside the textured-quad thumbnail
                // rendering.
            }
        } else {
            renderer.submit_frame(&self.runtime_assets, &frame);
        }

        if let Some(remaining) = &mut self.self_test_remaining {
            *remaining -= dt;
            if *remaining <= 0.0 {
                self.self_test_remaining = None;
                world.events.emit(hold::AppExit);
            }
        }
    }

    fn window_title(&self) -> Option<String> {
        Some(format!("Hold Editor - {}", self.status))
    }
}

fn main() {
    let args = Args::parse();

    if args.backend_info {
        print_backend_info();
        return;
    }

    let backend_caps = args.backend.capabilities();
    if !backend_caps.native_editor_shell_ready {
        eprintln!(
            "error: backend '{}' is {} for the native editor shell",
            args.backend, backend_caps.maturity
        );
        eprintln!("error: {}", backend_caps.gaps);
        std::process::exit(1);
    }

    let server = match EditorBridgeServer::open_hsc(&args.scene) {
        Ok(server) => server,
        Err(err) => {
            eprintln!("error: failed to open '{}': {err}", args.scene.display());
            std::process::exit(1);
        }
    };

    println!(
        "[editor] opened '{}' ({} entities) | backend={:?}",
        args.scene.display(),
        server.host().session().scene().entities.len(),
        args.backend
    );
    println!(
        "[editor] backend '{}' | maturity={} | overlays={} | mesh_upload={} | textured_mesh={}",
        args.backend,
        backend_caps.maturity,
        yes_no(backend_caps.editor_overlay_ready()),
        yes_no(backend_caps.mesh_upload),
        yes_no(backend_caps.textured_mesh),
    );
    println!("[editor] {}", backend_caps.summary);

    let title = format!(
        "Hold Editor - {} | {:?}",
        args.scene.display(),
        args.backend
    );
    let engine = Engine::new(&title, 1280, 720);
    let app = EditorShellApp::new(server, args.backend, args.self_test_seconds);

    println!("[editor] {}", app.status);
    if let Some(seconds) = args.self_test_seconds {
        println!(
            "[editor] self-test mode armed; automatic shutdown after {:.2}s",
            seconds
        );
    }
    println!(
        "[editor] Tab/S+Tab=entity  [/]=component  ;/'=field  1/2/3=axis  -/==delta  F1/F2/F3=move/rotate/scale  F7=reload  F8=play  F9=pause  F11=record  F12=replay  Ctrl+K=palette  Ctrl+S/Z/Y/P/R=save/undo/redo/patch/rebuild  Esc=quit"
    );
    println!("[editor] Left-click viewport=select  drag gizmo handle=edit selected Transform  history panel tracks explicit commands and events");

    match args.backend {
        BackendKind::Software => {
            let (hwnd, w, h) = (
                engine.window.hwnd,
                engine.window.width(),
                engine.window.height(),
            );
            // SAFETY: `hwnd` is the live window handle owned by `engine.window`.
            let renderer = unsafe { SoftwareRenderer::new(hwnd, w, h) };
            engine.run_app(renderer, app);
        }
        BackendKind::OpenGL => {
            let (hwnd, w, h) = (
                engine.window.hwnd,
                engine.window.width(),
                engine.window.height(),
            );
            // SAFETY: `hwnd` is the live window handle owned by `engine.window`.
            let renderer = unsafe { OpenGLRenderer::new(hwnd, w, h) };
            engine.run_app(renderer, app);
        }
        BackendKind::Vulkan => {
            let (hwnd, hinstance, w, h) = (
                engine.window.hwnd,
                engine.window.hinstance,
                engine.window.width(),
                engine.window.height(),
            );
            // SAFETY: `hwnd`/`hinstance` are the live window handle and module
            // instance owned by `engine.window`.
            let renderer = unsafe { hold::renderer::VulkanRenderer::new(hwnd, hinstance, w, h) };
            engine.run_app(renderer, app);
        }
    }
}
