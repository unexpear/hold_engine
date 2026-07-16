//! OpenGL-only textured sample scene.
//!
//! Proves a minimal asset-store path for:
//! - `MeshData` handles
//! - `Texture` handles
//! - `Material` handles referencing a shared shader
//! - OpenGL mesh upload + textured draw calls
//!
//! Usage:
//!   cargo run --example textured_scene
//!   cargo run --example textured_scene -- --self-test-seconds 1
//!
//! Controls:
//!   Esc        -- quit
//!   F5         -- toggle orbit pause

use std::collections::HashMap;
use std::f32::consts::{PI, TAU};
use std::path::PathBuf;
use std::time::Duration;

use hold::assets::{Material, Shader};
use hold::input::vk;
use hold::renderer::{
    opengl_shaders, ClearPass, DrawTexturedNormalMappedPass, DrawTexturedNormalMappedPassParams,
    DrawTexturedPass, DrawTexturedPassParams, FrameGraph, OpenGLRenderer, Pass, DEFAULT_THUMBNAIL_SIZE,
};
use hold::{
    AppExit, AssetStore, ColorSpace, Color, CompileQueue, CompileQueueConfig, CompileResult,
    CompileTarget, Engine, ExtensionFilter, FileWatcher, GpuMesh, GpuTexture, Handle, InputState,
    Mat4, MeshData, NormalMap, ReloadDispatcher, Renderer, Texture, TextureFormat, Vec3,
    WatcherConfig, World, default_compile_fn,
};
use hold::assets::{AssetKind, CompiledAssetKind};

const WINDOW_TITLE: &str = "Hold Engine - Textured Scene";
const FLOOR_TINT: Color = Color::new(0.95, 0.98, 1.0, 1.0);
const CRATE_TINT: Color = Color::new(1.0, 0.98, 0.94, 1.0);
const CLEAR_COLOR: Color = Color::new(0.08, 0.10, 0.12, 1.0);

struct Args {
    self_test_seconds: Option<f32>,
}

impl Args {
    fn parse() -> Self {
        let mut args = std::env::args().skip(1);
        let mut self_test_seconds = None;

        while let Some(arg) = args.next() {
            match arg.as_str() {
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

        Self { self_test_seconds }
    }
}

struct TexturedScene {
    assets: AssetStore,
    floor_mesh: Handle<MeshData>,
    cube_mesh: Handle<MeshData>,
    floor_material: Handle<Material>,
    cube_material: Handle<Material>,
    gpu: Option<GpuScene>,
    orbit_angle: f32,
    orbit_paused: bool,
    self_test_remaining: Option<f32>,
    /// When `true`, the next frame goes through
    /// `FrameGraph::execute_with_capture` instead of `execute` and the
    /// resulting per-pass thumbnails are summarised on stdout. Set by
    /// pressing F6, by the self-test path one frame before exit, and
    /// cleared after the capture frame so subsequent frames return to
    /// the cheap path. See ADR 0002 Q8.
    capture_next_frame: bool,
    /// Live file-system watcher; `None` until the user presses F7 to
    /// arm it. When armed, every frame the example drains coalesced
    /// `FileChange` events and prints them — the developer-facing
    /// proof that the M7.1 watcher pipeline is delivering events end
    /// to end. See ADR 0004 Q4.
    watcher: Option<FileWatcher>,
    /// Compile-queue chain: watcher (writing into an external sender)
    /// + queue (consuming the receiver, recompiling registered
    /// targets). Press F8 to arm. Disarms F7 if active. The example
    /// holds both the watcher and the queue so shutdown stops
    /// cleanly. See ADR 0004 Q4 + Q9 (M7.2).
    compile_chain: Option<CompileChain>,
    /// Full reload chain: watcher → queue → dispatcher → live
    /// AssetStore replace + GPU cache eviction. Press F9 to arm.
    /// Demonstrates the M7.3 end-to-end path described in ADR 0004
    /// Q9. Targets the floor and cube's authored textures so a
    /// `.bmp` saved at one of those paths flips the rendered
    /// surface within one frame.
    reload_chain: Option<ReloadChain>,
}

struct CompileChain {
    /// Kept alive so its worker thread keeps producing events; not
    /// directly read. Drop is the shutdown trigger.
    _watcher: FileWatcher,
    queue: CompileQueue,
}

/// Full hot-reload chain (M7.3): watcher → queue → dispatcher.
/// Whereas [`CompileChain`] just logs compile results, this struct
/// also applies them to the live AssetStore via the dispatcher and
/// evicts the renderer's GPU caches so changes appear next frame.
struct ReloadChain {
    /// Kept alive for thread shutdown. Events flow through
    /// `tx_changes → queue → tx_results → dispatcher`.
    _watcher: FileWatcher,
    /// Kept alive for thread shutdown.
    _queue: CompileQueue,
    dispatcher: ReloadDispatcher,
}

struct GpuScene {
    meshes: HashMap<Handle<MeshData>, GpuMesh>,
    programs: HashMap<Handle<Shader>, u32>,
    textures: HashMap<Handle<Texture>, GpuTexture>,
    normal_maps: HashMap<Handle<NormalMap>, GpuTexture>,
}

impl GpuScene {
    fn free(self, renderer: &mut OpenGLRenderer) {
        for (_, mesh) in self.meshes {
            renderer.free_mesh(mesh);
        }
        for (_, texture) in self.textures {
            renderer.free_texture(texture);
        }
        for (_, normal_map) in self.normal_maps {
            renderer.free_texture(normal_map);
        }
        for (_, program) in self.programs {
            renderer.delete_program(program);
        }
    }
}

impl TexturedScene {
    fn new(self_test_remaining: Option<f32>) -> Self {
        let mut assets = AssetStore::new();

        // Plain textured-lit shader for the floor — no normal map.
        let shader_textured = assets.insert(Shader {
            name: "textured-lit".into(),
            vertex_source: opengl_shaders::TEXTURED_VERT.into(),
            fragment_source: opengl_shaders::TEXTURED_FRAG.into(),
        });

        // Textured + normal-mapped shader for the cube — exercises the
        // location-3 tangent attribute and the typed `NormalMap` asset
        // path. See ADR 0002 Q4.
        let shader_normal_mapped = assets.insert(Shader {
            name: "textured-normal-lit".into(),
            vertex_source: opengl_shaders::TEXTURED_NORMAL_VERT.into(),
            fragment_source: opengl_shaders::TEXTURED_NORMAL_FRAG.into(),
        });

        let floor_texture = assets.insert(build_floor_texture());
        let crate_texture = assets.insert(build_crate_texture());
        let crate_normal_map = assets.insert(build_studded_normal_map());

        // Floor: dielectric, fairly rough — mid-tone painted concrete /
        // worn tile. Roughness 0.85 reads as matte under the moving
        // directional light without going chalky.
        let floor_material = assets.insert(
            Material::new(shader_textured, floor_texture)
                .with_metallic(0.0)
                .with_roughness(0.85),
        );
        // Cube: dielectric (painted wood crate), moderately rough at 0.55
        // so the studded bumps catch a clear specular highlight as the
        // camera orbits — the test we want is "do the normal map and the
        // PBR specular reinforce each other?", and a mid roughness gives
        // the cleanest answer.
        let cube_material = assets.insert(
            Material::new(shader_normal_mapped, crate_texture)
                .with_normal_map(crate_normal_map)
                .with_metallic(0.0)
                .with_roughness(0.55),
        );

        let mut floor_mesh = MeshData::plane(1, 1);
        for vertex in &mut floor_mesh.vertices {
            vertex.uv[0] *= 6.0;
            vertex.uv[1] *= 6.0;
        }

        let floor_mesh = assets.insert(floor_mesh);
        let cube_mesh = assets.insert(MeshData::cube());

        Self {
            assets,
            floor_mesh,
            cube_mesh,
            floor_material,
            cube_material,
            gpu: None,
            orbit_angle: 0.0,
            orbit_paused: false,
            self_test_remaining,
            capture_next_frame: false,
            watcher: None,
            compile_chain: None,
            reload_chain: None,
        }
    }

    fn ensure_gpu(&mut self, renderer: &mut OpenGLRenderer) {
        if self.gpu.is_some() {
            return;
        }

        let mut meshes = HashMap::new();
        let mut programs = HashMap::new();
        let mut textures = HashMap::new();
        let mut normal_maps = HashMap::new();

        for mesh_handle in [self.floor_mesh, self.cube_mesh] {
            let mesh = self
                .assets
                .get(mesh_handle)
                .expect("mesh handle should resolve");
            meshes.insert(mesh_handle, renderer.upload_mesh(mesh));
        }

        let material_handles = [self.floor_material, self.cube_material];
        for material_handle in material_handles {
            let material = self
                .assets
                .get(material_handle)
                .expect("material handle should resolve");

            programs.entry(material.shader).or_insert_with(|| {
                let shader = self
                    .assets
                    .get(material.shader)
                    .expect("shader handle should resolve");
                renderer.compile_shader_program(&shader.vertex_source, &shader.fragment_source)
            });

            textures.entry(material.albedo).or_insert_with(|| {
                let texture = self
                    .assets
                    .get(material.albedo)
                    .expect("texture handle should resolve");
                renderer.upload_texture(texture)
            });

            if let Some(nm_handle) = material.normal {
                normal_maps.entry(nm_handle).or_insert_with(|| {
                    let nm = self
                        .assets
                        .get(nm_handle)
                        .expect("normal map handle should resolve");
                    renderer.upload_normal_map(nm)
                });
            }
        }

        self.gpu = Some(GpuScene {
            meshes,
            programs,
            textures,
            normal_maps,
        });
    }

    fn shutdown(&mut self, renderer: &mut OpenGLRenderer) {
        if let Some(gpu) = self.gpu.take() {
            gpu.free(renderer);
        }
    }

    /// Resolve a (mesh, material) pair into a [`Pass`] suitable for
    /// dropping into the per-frame [`FrameGraph`].
    ///
    /// The pass is dispatched at the type-system boundary because
    /// `Material.normal` is `Option<NormalMapHandle>`, not
    /// `Option<TextureHandle>` — see ADR 0002 Q4. A normal-map-bearing
    /// material yields a [`DrawTexturedNormalMappedPass`]; a flat textured
    /// material yields a [`DrawTexturedPass`]. Both pass types own their
    /// `GpuMesh` / `GpuTexture` / matrix data by value (Copy), so the
    /// resulting `Box<dyn Pass>` carries no borrows from `self.assets`.
    fn build_material_pass(
        &self,
        name: &str,
        mesh_handle: Handle<MeshData>,
        material_handle: Handle<Material>,
        model: Mat4,
        view_proj: Mat4,
        view_pos: Vec3,
        light_dir: Vec3,
        ambient: f32,
        tint: Color,
    ) -> Box<dyn Pass> {
        let gpu = self.gpu.as_ref().expect("GPU scene should be initialized");
        let material = self
            .assets
            .get(material_handle)
            .expect("material handle should resolve");
        let mesh = *gpu
            .meshes
            .get(&mesh_handle)
            .expect("mesh should have been uploaded");
        let albedo = *gpu
            .textures
            .get(&material.albedo)
            .expect("texture should have been uploaded");
        let program = *gpu
            .programs
            .get(&material.shader)
            .expect("shader should have been compiled");
        let mvp = view_proj * model;

        let metallic = material.metallic;
        let roughness = material.roughness;

        if let Some(nm_handle) = material.normal {
            let normal_map = *gpu
                .normal_maps
                .get(&nm_handle)
                .expect("normal map should have been uploaded");
            Box::new(DrawTexturedNormalMappedPass::new(
                name,
                DrawTexturedNormalMappedPassParams {
                    mesh,
                    albedo,
                    normal_map,
                    program,
                    mvp,
                    model,
                    tint,
                    light_dir,
                    view_pos,
                    ambient,
                    metallic,
                    roughness,
                },
            ))
        } else {
            Box::new(DrawTexturedPass::new(
                name,
                DrawTexturedPassParams {
                    mesh,
                    albedo,
                    program,
                    mvp,
                    model,
                    tint,
                    light_dir,
                    view_pos,
                    ambient,
                    metallic,
                    roughness,
                },
            ))
        }
    }

    fn update(
        &mut self,
        world: &mut World,
        renderer: &mut OpenGLRenderer,
        input: &InputState,
        dt: f32,
    ) {
        self.ensure_gpu(renderer);

        if input.key_pressed(vk::ESCAPE) {
            self.shutdown(renderer);
            world.events.emit(AppExit);
            return;
        }

        if input.key_pressed(vk::F5) {
            self.orbit_paused = !self.orbit_paused;
        }

        if input.key_pressed(vk::F6) {
            // Editor frame inspector preview: capture once, dump
            // per-pass thumbnail summaries to stdout so the user can
            // see the data the inspector panel will visualise.
            self.capture_next_frame = true;
            println!("[textured] capturing per-pass thumbnails on next frame…");
        }

        if input.key_pressed(vk::F7) {
            // M7.1 demo: arm or disarm the file watcher pointed at
            // the current working directory. Every coalesced
            // FileChange event prints to stdout — the proof that the
            // hot-reload pipeline (ADR 0004) is delivering events
            // end to end.
            if self.watcher.is_some() {
                self.watcher = None;
                println!("[textured] file watcher disarmed");
            } else {
                let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
                let config = WatcherConfig {
                    root: cwd.clone(),
                    extensions: ExtensionFilter::default_engine_extensions(),
                    coalesce_window: Duration::from_millis(50),
                };
                match FileWatcher::start(config) {
                    Ok(watcher) => {
                        println!("[textured] file watcher armed on {}", cwd.display());
                        println!("[textured]   touch any .png/.tga/.obj/.glsl/.hsc/.material file to see events");
                        self.watcher = Some(watcher);
                    }
                    Err(e) => {
                        eprintln!("[textured] failed to arm file watcher: {}", e);
                    }
                }
            }
        }

        if let Some(watcher) = self.watcher.as_ref() {
            // Drain any coalesced events that landed since the last
            // frame. Non-blocking — the watcher's worker thread
            // produces independently.
            while let Ok(change) = watcher.try_recv() {
                println!("[textured] file change: {:?}", change);
            }
        }

        if input.key_pressed(vk::F8) {
            // M7.2 demo: arm or disarm the watcher → CompileQueue
            // chain. Disarms the F7 watcher first if it's active so
            // there's only one watcher running at a time. Registers
            // a synthetic target pointing at any .bmp file the user
            // touches in the cwd; compile failures (no real source
            // file at that path) surface as CompileResult::Err and
            // demonstrate the error-surfacing path.
            if self.compile_chain.is_some() {
                self.compile_chain = None;
                println!("[textured] compile chain disarmed");
            } else {
                if self.watcher.is_some() {
                    self.watcher = None;
                    println!("[textured]   (disarmed F7 watcher to prevent double-watch)");
                }
                let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
                match start_compile_chain(cwd.clone()) {
                    Ok(chain) => {
                        println!("[textured] compile chain armed on {}", cwd.display());
                        println!(
                            "[textured]   touch / save any .bmp in this dir to see compile results"
                        );
                        self.compile_chain = Some(chain);
                    }
                    Err(e) => {
                        eprintln!("[textured] failed to arm compile chain: {}", e);
                    }
                }
            }
        }

        if let Some(chain) = self.compile_chain.as_ref() {
            while let Ok(result) = chain.queue.try_recv() {
                match result {
                    CompileResult::Ok(ok) => {
                        println!(
                            "[textured] compile OK   {:>5} bytes  gen={:?}  {}",
                            ok.payload.len(),
                            ok.generation,
                            ok.logical_name
                        );
                    }
                    CompileResult::Err(err) => {
                        println!(
                            "[textured] compile ERR  gen={:?}  {} : {}",
                            err.generation, err.logical_name, err.error
                        );
                    }
                }
            }
        }

        if input.key_pressed(vk::F9) {
            // M7.3 demo: arm or disarm the full hot-reload chain
            // (watcher → queue → dispatcher → AssetStore replace →
            // GPU cache eviction). Disarms F7 / F8 first to avoid
            // double-watching. The dispatcher is registered with
            // the cube's authored albedo texture so saving a .bmp
            // at that path live-replaces the rendered surface.
            if self.reload_chain.is_some() {
                self.reload_chain = None;
                println!("[textured] reload chain disarmed");
            } else {
                if self.watcher.is_some() {
                    self.watcher = None;
                    println!("[textured]   (disarmed F7 watcher)");
                }
                if self.compile_chain.is_some() {
                    self.compile_chain = None;
                    println!("[textured]   (disarmed F8 compile chain)");
                }
                let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
                // Resolve the cube's albedo handle from the asset
                // store so the dispatcher binds to the actual
                // `TextureHandle` that the rendering pipeline reads.
                // Identity preservation through `AssetStore::replace`
                // means this single registration suffices for the
                // life of the chain.
                let cube_albedo = self
                    .assets
                    .get(self.cube_material)
                    .map(|m| m.albedo)
                    .expect("cube material should resolve");
                match start_reload_chain(cwd.clone(), cube_albedo) {
                    Ok((chain, source_path)) => {
                        println!("[textured] reload chain armed on {}", cwd.display());
                        println!(
                            "[textured]   target: {} (logical: textures/textured_scene_cube)",
                            source_path.display()
                        );
                        println!(
                            "[textured]   save a real .bmp to that path to live-update the cube"
                        );
                        self.reload_chain = Some(chain);
                    }
                    Err(e) => {
                        eprintln!("[textured] failed to arm reload chain: {}", e);
                    }
                }
            }
        }

        if let Some(chain) = self.reload_chain.as_mut() {
            // Drain compile results, apply replaces, evict GPU caches
            // for any successfully reloaded asset. The
            // ReloadDispatcher returns a summary so the caller (us)
            // routes events to the renderer + editor on its own
            // schedule.
            let summary = chain.dispatcher.dispatch(&mut self.assets);
            for event in &summary.applied {
                println!(
                    "[textured] reload APPLIED   kind={:?}  index={}  name={}",
                    event.kind, event.handle_index, event.logical_name
                );
                // Emit on the World event bus so game-layer code
                // can react (re-resolve entity refs, replay
                // animations, etc.). Pillar 2 (iteration speed) is
                // realised at the API surface here: any system
                // that reads `world.events.read::<AssetInvalidated>()`
                // sees reloads as ordinary events. ADR 0004 Q12.
                world.events.emit(event.clone());
                // Evict the example's local GPU caches keyed by
                // handle index. Since `AssetStore::replace`
                // preserves identity, the next frame's
                // `ensure_gpu` re-uploads from the new asset under
                // the same handle.
                if let Some(gpu) = self.gpu.as_mut() {
                    match event.kind {
                        AssetKind::Texture => {
                            gpu.textures.retain(|h, gpu_tex| {
                                let keep = h.index() != event.handle_index;
                                if !keep {
                                    renderer.free_texture(*gpu_tex);
                                }
                                keep
                            });
                            renderer.evict_texture_handle_by_index(event.handle_index);
                        }
                        AssetKind::NormalMap => {
                            gpu.normal_maps.retain(|h, gpu_tex| {
                                let keep = h.index() != event.handle_index;
                                if !keep {
                                    renderer.free_texture(*gpu_tex);
                                }
                                keep
                            });
                        }
                        AssetKind::Mesh => {
                            renderer.evict_mesh_handle_by_index(event.handle_index);
                        }
                        AssetKind::Shader => {
                            renderer.evict_shader_handle_by_index(event.handle_index);
                        }
                        _ => {}
                    }
                }
            }
            for err in &summary.errors {
                println!(
                    "[textured] reload ERROR     gen={:?}  name={}  : {}",
                    err.generation, err.logical_name, err.error
                );
            }
            for stale in &summary.stale {
                println!(
                    "[textured] reload stale     gen={:?}  name={}  (shadowed by newer)",
                    stale.generation, stale.logical_name
                );
            }
        }

        if !self.orbit_paused {
            self.orbit_angle = (self.orbit_angle + dt * 0.45).rem_euclid(TAU);
        }

        if let Some(remaining) = &mut self.self_test_remaining {
            *remaining -= dt;
            // Just before self-test exit, capture the final frame so
            // automated runs validate the readback path.
            if !self.capture_next_frame && *remaining <= dt {
                self.capture_next_frame = true;
                println!(
                    "[textured] self-test final frame: capturing per-pass thumbnails…"
                );
            }
            if *remaining <= 0.0 {
                self.shutdown(renderer);
                world.events.emit(AppExit);
                return;
            }
        }

        let aspect = renderer.width().max(1) as f32 / renderer.height().max(1) as f32;
        let orbit_radius = 5.8;
        let eye = Vec3::new(
            self.orbit_angle.cos() * orbit_radius,
            2.6 + (self.orbit_angle * 0.7).sin() * 0.25,
            self.orbit_angle.sin() * orbit_radius,
        );
        let view = Mat4::look_at(eye, Vec3::new(0.0, 0.65, 0.0), Vec3::Y);
        let proj = Mat4::perspective_gl(60.0_f32.to_radians(), aspect, 0.1, 100.0);
        let view_proj = proj * view;
        let light_dir = Vec3::new(-0.35, 0.85, 0.25).normalize();
        let ambient = 0.24;

        let floor_model =
            Mat4::translate(Vec3::new(0.0, -0.02, 0.0)) * Mat4::scale(Vec3::new(8.0, 1.0, 8.0));
        let cube_spin = self.orbit_angle * 1.6;
        let cube_model = Mat4::translate(Vec3::new(0.0, 0.75, 0.0))
            * Mat4::rotate_y(cube_spin)
            * Mat4::rotate_x(PI * 0.08)
            * Mat4::scale_uniform(1.4);

        // Build a fresh frame graph each frame. Topological compile is
        // microsecond-scale and avoids any lifetime entanglement between
        // the captured per-frame state (matrices, eye position) and the
        // long-lived asset store. The graph here is intentionally simple
        // — clear + two opaque draws — but the dependency-tracked shape
        // generalises to multi-target rendering, post-processing, and
        // editor-frame-inspector capture as M4 closes out (ADR 0002 Q8).
        let mut graph = FrameGraph::builder();
        graph.add_pass(Box::new(ClearPass::new("clear", CLEAR_COLOR)));
        graph.add_pass(self.build_material_pass(
            "floor",
            self.floor_mesh,
            self.floor_material,
            floor_model,
            view_proj,
            eye,
            light_dir,
            ambient,
            FLOOR_TINT,
        ));
        graph.add_pass(self.build_material_pass(
            "cube",
            self.cube_mesh,
            self.cube_material,
            cube_model,
            view_proj,
            eye,
            light_dir,
            ambient,
            CRATE_TINT,
        ));
        let graph = graph
            .compile()
            .expect("frame graph compile is infallible for this scene");

        // The textured_scene example predates the M9 scene-passes; its
        // passes carry all their draw data inline, so an empty
        // RenderFrame is enough to satisfy the API contract.
        let empty_frame = hold::RenderFrame::default();
        if self.capture_next_frame {
            self.capture_next_frame = false;
            let (tw, th) = DEFAULT_THUMBNAIL_SIZE;
            let thumbs = graph.execute_with_capture(renderer, &self.assets, &empty_frame, tw, th);
            println!(
                "[textured] captured {} per-pass thumbnails ({}×{}):",
                thumbs.len(),
                tw,
                th
            );
            for thumb in &thumbs {
                // Sample a centre pixel so the editor inspector — and
                // anyone reading this stdout — can sanity-check the
                // capture without a UI yet. RGBA centre pixel is enough
                // to confirm the readback round-tripped the right
                // colours.
                let cx = thumb.width / 2;
                let cy = thumb.height / 2;
                let i = ((cy * thumb.width + cx) * 4) as usize;
                let centre = if i + 3 < thumb.data.len() {
                    [
                        thumb.data[i],
                        thumb.data[i + 1],
                        thumb.data[i + 2],
                        thumb.data[i + 3],
                    ]
                } else {
                    [0, 0, 0, 0]
                };
                println!(
                    "[textured]   pass {} ({:>5}): {} bytes, centre RGBA = {:?}",
                    thumb.pass_index, thumb.pass_name, thumb.data.len(), centre
                );
            }
        } else {
            graph.execute(renderer, &self.assets, &empty_frame);
        }
    }
}

fn build_floor_texture() -> Texture {
    let size = 64u32;
    let cell = 8u32;
    let mut data = Vec::with_capacity((size * size * 4) as usize);

    for y in 0..size {
        for x in 0..size {
            let tile_x = x / cell;
            let tile_y = y / cell;
            let on_border = x % cell == 0 || y % cell == 0;
            let diagonal = (x + y) % (cell * 2) == 0;

            let color = if on_border {
                [22, 28, 32, 255]
            } else if diagonal {
                [48, 88, 92, 255]
            } else if (tile_x + tile_y) % 2 == 0 {
                [36, 52, 58, 255]
            } else {
                [28, 40, 46, 255]
            };
            data.extend_from_slice(&color);
        }
    }

    Texture {
        width: size,
        height: size,
        format: TextureFormat::Rgba8,
        // Authored colour content — sRGB. The OpenGL backend uploads
        // with GL_SRGB8_ALPHA8 so the GPU decodes to linear at sample
        // time. See ADR 0002 Q3.
        color_space: ColorSpace::Srgb,
        data,
    }
}

/// Procedural tangent-space normal map for the crate cube. Encodes a 4×4
/// grid of hemispherical "studs" — round bumps that catch the directional
/// light visibly even at arm's length. The flat regions between studs encode
/// the unperturbed surface normal `(0, 0, 1)` in tangent space, which
/// corresponds to `(128, 128, 255)` in unsigned-byte RGB after the standard
/// `n * 0.5 + 0.5` packing.
///
/// All values are produced in the engine's canonical OpenGL Y+ convention
/// (green channel up; see [`hold::NormalMapConvention`] and ADR 0002 Q4).
fn build_studded_normal_map() -> NormalMap {
    let size: u32 = 64;
    let cell: f32 = 16.0; // pixel spacing between stud centres
    let radius: f32 = 6.0; // stud cap radius in pixels
    let mut data = Vec::with_capacity((size * size * 3) as usize);

    for y in 0..size {
        for x in 0..size {
            // Distance from the centre of the nearest stud — find by
            // snapping to the cell grid centred on (cell/2, cell/2).
            let cx = (((x as f32) / cell).floor() + 0.5) * cell;
            let cy = (((y as f32) / cell).floor() + 0.5) * cell;
            let dx = x as f32 - cx;
            let dy = y as f32 - cy;
            let d = (dx * dx + dy * dy).sqrt();

            let (nx, ny, nz) = if d < radius {
                // Hemispherical bump: surface point lies on a hemisphere of
                // radius `radius` whose centre is at the stud's pixel
                // location. The outward unit normal at offset (dx, dy)
                // from the centre is `(dx, -dy, sqrt(r² - d²)) / radius`,
                // because the green channel in OpenGL Y+ convention points
                // up (toward smaller y in screen coordinates).
                let nz_unscaled = (radius * radius - d * d).sqrt();
                let inv_r = 1.0 / radius;
                (dx * inv_r, -dy * inv_r, nz_unscaled * inv_r)
            } else {
                // Flat surface — unperturbed tangent-space normal.
                (0.0, 0.0, 1.0)
            };

            // Pack [-1, 1] → [0, 255].
            let r = ((nx * 0.5 + 0.5).clamp(0.0, 1.0) * 255.0).round() as u8;
            let g = ((ny * 0.5 + 0.5).clamp(0.0, 1.0) * 255.0).round() as u8;
            let b = ((nz * 0.5 + 0.5).clamp(0.0, 1.0) * 255.0).round() as u8;
            data.extend_from_slice(&[r, g, b]);
        }
    }

    NormalMap::new_opengl(size, size, TextureFormat::Rgb8, data)
}

fn build_crate_texture() -> Texture {
    let size = 64u32;
    let panel = 16u32;
    let mut data = Vec::with_capacity((size * size * 4) as usize);

    for y in 0..size {
        for x in 0..size {
            let local_x = x % panel;
            let local_y = y % panel;
            let outer_border =
                local_x < 2 || local_y < 2 || local_x >= panel - 2 || local_y >= panel - 2;
            let inner_border =
                local_x == 4 || local_y == 4 || local_x == panel - 5 || local_y == panel - 5;
            let checker = ((x / panel) + (y / panel)) % 2 == 0;

            let color = if outer_border {
                [56, 34, 22, 255]
            } else if inner_border {
                [214, 176, 120, 255]
            } else if checker {
                [152, 98, 58, 255]
            } else {
                [120, 76, 44, 255]
            };
            data.extend_from_slice(&color);
        }
    }

    Texture {
        width: size,
        height: size,
        format: TextureFormat::Rgba8,
        // Authored colour content — sRGB. The OpenGL backend uploads
        // with GL_SRGB8_ALPHA8 so the GPU decodes to linear at sample
        // time. See ADR 0002 Q3.
        color_space: ColorSpace::Srgb,
        data,
    }
}

fn main() {
    let args = Args::parse();
    let engine = Engine::new(WINDOW_TITLE, 1280, 720);
    // SAFETY: `engine.window.hwnd` is the live window handle just created by
    // `Engine::new`.
    let renderer = unsafe {
        OpenGLRenderer::new(
            engine.window.hwnd,
            engine.window.width(),
            engine.window.height(),
        )
    };
    let mut scene = TexturedScene::new(args.self_test_seconds);

    println!("[textured] material + texture handle sample ready");
    println!("[textured] Esc=quit  F5=toggle orbit  F6=capture per-pass thumbnails");
    println!("[textured]            F7=toggle file watcher  F8=toggle compile chain  F9=toggle full hot-reload");
    if let Some(seconds) = args.self_test_seconds {
        println!(
            "[textured] self-test mode armed; automatic shutdown after {:.2}s",
            seconds
        );
    }

    engine.run(renderer, move |world, renderer, input, dt| {
        scene.update(world, renderer, input, dt);
    });
}

/// Build the full hot-reload chain (M7.3): watcher → queue →
/// dispatcher. The dispatcher is registered with the example's
/// cube albedo handle so saving a real .bmp at the canonical path
/// live-replaces the rendered cube texture.
///
/// Returns the chain plus the source path the dispatcher is
/// listening on, so the caller can print where to save the file.
fn start_reload_chain(
    root: PathBuf,
    cube_albedo: hold::TextureHandle,
) -> Result<(ReloadChain, PathBuf), Box<dyn std::error::Error + Send + Sync + 'static>> {
    use std::sync::mpsc::channel;

    // Two channels: watcher → queue → dispatcher.
    let (tx_changes, rx_changes) = channel();
    let (tx_results, rx_results) = channel();

    let watcher = FileWatcher::start_with_sender(
        WatcherConfig {
            root: root.clone(),
            extensions: ExtensionFilter::default_engine_extensions(),
            coalesce_window: Duration::from_millis(50),
        },
        tx_changes,
    )?;

    // Pick the canonical source path the chain listens on. The
    // example's procedural texture isn't on disk, so we use a
    // sentinel filename in cwd; a user who wants to drive the
    // demo writes a real BMP at this path.
    let source_path = root.join("textured_scene_cube.bmp");

    let queue = CompileQueue::start_with_sender(
        CompileQueueConfig {
            file_changes: rx_changes,
            targets: vec![CompileTarget::new(
                "textures/textured_scene_cube",
                CompiledAssetKind::Texture,
                source_path.clone(),
            )],
            compile_fn: default_compile_fn(),
        },
        tx_results,
    )?;

    let mut dispatcher = ReloadDispatcher::new(rx_results);
    // Bind the canonical logical name to the cube's live albedo
    // handle. AssetStore::replace preserves identity, so this
    // binding stays valid across reloads — the dispatcher writes
    // the new Texture under the same handle the rest of the
    // example continues to reference.
    dispatcher.register_texture("textures/textured_scene_cube", cube_albedo);

    Ok((
        ReloadChain {
            _watcher: watcher,
            _queue: queue,
            dispatcher,
        },
        source_path,
    ))
}

/// Build a watcher → compile-queue chain. The watcher writes
/// coalesced `FileChange`s into a channel the queue consumes;
/// successful compiles flow back as `CompileResult::Ok`, failures as
/// `CompileResult::Err`. Both end up printed in the run loop.
///
/// Demonstrates ADR 0004 Q4 + Q9 (M7.2): the compile-queue worker
/// running off the main thread, with last-wins generation counters
/// and explicit per-target registration.
fn start_compile_chain(
    root: PathBuf,
) -> Result<CompileChain, Box<dyn std::error::Error + Send + Sync + 'static>> {
    let (tx, rx) = std::sync::mpsc::channel();
    let watcher = FileWatcher::start_with_sender(
        WatcherConfig {
            root: root.clone(),
            extensions: ExtensionFilter::default_engine_extensions(),
            coalesce_window: Duration::from_millis(50),
        },
        tx,
    )?;

    // Register every .bmp directly under the watch root as a
    // synthetic "test_<filename>" texture target. The example does
    // not actually have a source asset on disk for these targets, so
    // most compiles will surface as CompileResult::Err — which is
    // itself the demo of the error path. Editing a real .bmp the
    // path can read produces a CompileResult::Ok.
    let mut targets = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().map(|e| e.eq_ignore_ascii_case("bmp")).unwrap_or(false) {
                let stem = path
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "asset".into());
                let logical = format!("textures/{}", stem);
                targets.push(CompileTarget::new(
                    logical,
                    CompiledAssetKind::Texture,
                    path,
                ));
            }
        }
    }
    if targets.is_empty() {
        // Add one synthetic target so the demo has something to
        // talk about even in an empty directory. Editing the
        // example file itself triggers the path lookup; the compile
        // will fail because the file isn't a BMP, demonstrating
        // the error-surfacing path.
        let synthetic = root.join("textured_scene_synthetic.bmp");
        targets.push(CompileTarget::new(
            "textures/synthetic_target",
            CompiledAssetKind::Texture,
            synthetic,
        ));
    }

    let queue = CompileQueue::start(CompileQueueConfig {
        file_changes: rx,
        targets,
        compile_fn: default_compile_fn(),
    })?;

    Ok(CompileChain {
        _watcher: watcher,
        queue,
    })
}
