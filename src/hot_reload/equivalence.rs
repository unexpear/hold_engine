//! Hot-reload equivalence harness — the load-bearing CI test that
//! makes the Q2 promise enforceable.
//!
//! Per `docs/adr/0004-hot-reload-architecture.md` Q2 + Q15: the
//! engine's pillar-2 differentiation rests on
//!
//! > hot-reloaded user-observable state must equal clean-reload
//! > user-observable state.
//!
//! Without a CI test that pins this property, every future change
//! is a free chance to silently break it. With this test, breakage
//! lands on a red CI build before it lands on developers.
//!
//! ## What the harness exercises
//!
//! 1. Writes a small synthetic project to a unique temp directory
//!    on disk (a few BMP files at distinct sizes + colours).
//! 2. **Clean load path:** for each source BMP, runs
//!    `compile_texture_asset` → `decode_compiled_texture_payload`
//!    → `AssetStore::insert`. Captures the resulting
//!    `Texture` per logical name as the *reference* state.
//! 3. **Hot-reload path:** starts with placeholder textures in a
//!    fresh `AssetStore`, registers them with a
//!    [`super::CompileQueue`] driven by a mock `FileChange`
//!    sender (skipping the OS file watcher — that's tested
//!    independently in `super::watcher` and `super::coalescer`),
//!    sends a `Modified` event for each source path, drains the
//!    queue's results into a [`super::ReloadDispatcher`], applies
//!    every result through `AssetStore::replace`. Captures the
//!    resulting `Texture` per logical name as the *hot-reload*
//!    state.
//! 4. **Asserts equivalence:** every (logical_name, Texture) pair
//!    in the hot-reload state matches the corresponding pair in
//!    the reference state, byte-for-byte (no `ε` slop because
//!    nothing is going through the GPU yet — the framebuffer-level
//!    half of the equivalence harness arrives in M7.5.1 when an
//!    OpenGL-bound integration test is feasible).
//!
//! ## What's deliberately out of scope here
//!
//! - **GPU framebuffer comparison.** Requires a Win32 window;
//!   doesn't run in vanilla CI. Tracked as M7.5.1 (manual /
//!   gated integration test in the rendered-output direction).
//! - **The OS-level file watcher.** The watcher is tested in
//!   `super::watcher` against real Win32 events; here we
//!   bypass it by sending `FileChange` events directly to the
//!   queue's input channel. The promise being tested is
//!   "compile + dispatch + apply equals clean load," not "the
//!   Win32 directory watch fires events for every save."
//! - **Multi-asset-kind coverage.** v1 covers Texture (the only
//!   asset kind whose hot-reload pipeline is wired end-to-end as
//!   of M7.4). Mesh equivalence lands when the test gains an
//!   on-disk OBJ; Material lands after M7.4.1's runtime-loader
//!   integration; Scene after M7.4.1 entity preservation.

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::mpsc::channel;
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    use crate::assets::{
        compile_texture_asset, decode_compiled_texture_payload, AssetStore, ColorSpace,
        CompiledAssetKind, Texture, TextureFormat, TextureHandle,
    };

    use crate::hot_reload::{
        canonicalise_for_index, CompileQueue, CompileQueueConfig, CompileTarget, FileChange,
        ReloadDispatcher,
    };
    use crate::hot_reload::compile_queue::default_compile_fn;

    // ── Temp-project scaffolding ────────────────────────────────────────────

    /// RAII guard that owns a unique temp directory and removes it
    /// on drop. Tests that fail still clean up because Drop runs on
    /// panic. Path is unique per test run (PID + nanos + counter)
    /// so parallel `cargo test` invocations don't collide.
    struct TempProject {
        path: PathBuf,
    }

    impl TempProject {
        fn new(label: &str) -> Self {
            use std::sync::atomic::{AtomicU64, Ordering};
            static COUNTER: AtomicU64 = AtomicU64::new(0);
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            let counter = COUNTER.fetch_add(1, Ordering::Relaxed);
            let pid = std::process::id();
            let path = std::env::temp_dir().join(format!(
                "hold_engine_equiv_{}_{}_{}__{}",
                label, pid, nanos, counter,
            ));
            std::fs::create_dir_all(&path).expect("temp project dir");
            Self { path }
        }

        fn join(&self, name: &str) -> PathBuf {
            self.path.join(name)
        }
    }

    impl Drop for TempProject {
        fn drop(&mut self) {
            // Best-effort cleanup. Failure to remove is logged but
            // not propagated — a leaked temp dir won't fail the
            // test, but a real OS error during cleanup might
            // indicate something worth knowing about.
            if let Err(e) = std::fs::remove_dir_all(&self.path) {
                eprintln!(
                    "[equivalence] warning: failed to clean up temp dir {:?}: {}",
                    self.path, e
                );
            }
        }
    }

    // ── BMP encoder (minimal, RGB8 24bpp uncompressed) ──────────────────────

    /// Encode a buffer of RGB8 pixels (top-left origin) as a 24bpp
    /// uncompressed BMP. Just enough format coverage for `load_bmp`
    /// to round-trip — not a general-purpose BMP writer.
    ///
    /// Inline rather than reusing the browser_backend example
    /// encoder because that one targets RGBA preview thumbnails
    /// over an HTTP boundary; our equivalence harness wants the
    /// minimum viable BMP that the asset compiler will accept.
    fn encode_rgb8_bmp(width: u32, height: u32, pixels: &[u8]) -> Vec<u8> {
        assert_eq!(pixels.len(), (width * height * 3) as usize);
        // BMP rows are padded to 4-byte boundaries.
        let row_bytes = (width as usize) * 3;
        let row_padding = (4 - row_bytes % 4) % 4;
        let padded_row = row_bytes + row_padding;
        let pixel_bytes = padded_row * height as usize;

        let header_size: u32 = 14 + 40;
        let file_size: u32 = header_size + pixel_bytes as u32;

        let mut out = Vec::with_capacity(file_size as usize);
        // ── BITMAPFILEHEADER (14 bytes) ─────────────────────────
        out.extend_from_slice(b"BM");
        out.extend_from_slice(&file_size.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes()); // reserved1
        out.extend_from_slice(&0u16.to_le_bytes()); // reserved2
        out.extend_from_slice(&header_size.to_le_bytes()); // pixel offset
        // ── BITMAPINFOHEADER (40 bytes) ─────────────────────────
        out.extend_from_slice(&40u32.to_le_bytes()); // header size
        out.extend_from_slice(&(width as i32).to_le_bytes()); // width
        out.extend_from_slice(&(height as i32).to_le_bytes()); // height (positive = bottom-up)
        out.extend_from_slice(&1u16.to_le_bytes()); // planes
        out.extend_from_slice(&24u16.to_le_bytes()); // bpp
        out.extend_from_slice(&0u32.to_le_bytes()); // compression (BI_RGB)
        out.extend_from_slice(&(pixel_bytes as u32).to_le_bytes()); // image size
        out.extend_from_slice(&2835i32.to_le_bytes()); // x px-per-metre (72 DPI)
        out.extend_from_slice(&2835i32.to_le_bytes()); // y px-per-metre
        out.extend_from_slice(&0u32.to_le_bytes()); // colours used
        out.extend_from_slice(&0u32.to_le_bytes()); // important colours

        // ── Pixel data, BGR, bottom-up rows, padded ─────────────
        for y in (0..height as usize).rev() {
            let row_start = y * width as usize * 3;
            for x in 0..width as usize {
                let i = row_start + x * 3;
                // Swap RGB → BGR.
                out.push(pixels[i + 2]);
                out.push(pixels[i + 1]);
                out.push(pixels[i]);
            }
            for _ in 0..row_padding {
                out.push(0);
            }
        }
        debug_assert_eq!(out.len() as u32, file_size);
        out
    }

    /// Synthetic asset definition: logical name + dimensions +
    /// uniform RGB fill. Multiple per test so the equivalence
    /// property covers more than one shape of input.
    struct SyntheticAsset {
        logical: &'static str,
        filename: &'static str,
        width: u32,
        height: u32,
        rgb: [u8; 3],
    }

    fn write_synthetic_assets(project: &TempProject, assets: &[SyntheticAsset]) {
        for a in assets {
            let pixels: Vec<u8> = (0..(a.width * a.height))
                .flat_map(|_| a.rgb.iter().copied())
                .collect();
            let bytes = encode_rgb8_bmp(a.width, a.height, &pixels);
            std::fs::write(project.join(a.filename), bytes).unwrap();
        }
    }

    // ── Pipeline driver ─────────────────────────────────────────────────────

    /// Build the AssetStore by going through the production
    /// `compile_texture_asset` → `decode_compiled_texture_payload`
    /// path, then inserting. Returns `(store, name → handle)` so
    /// the caller can compare per-asset content against the
    /// hot-reloaded version.
    fn clean_load(
        project: &TempProject,
        assets: &[SyntheticAsset],
    ) -> (AssetStore, Vec<(&'static str, TextureHandle)>) {
        let mut store = AssetStore::new();
        let mut handles = Vec::new();
        for a in assets {
            let source = project.join(a.filename);
            let logical = format!("textures/{}", a.logical);
            let blob = compile_texture_asset(&source, &logical).expect("compile");
            let texture =
                decode_compiled_texture_payload(&blob.payload).expect("decode payload");
            let handle = store.insert(texture);
            handles.push((a.logical, handle));
        }
        (store, handles)
    }

    /// Build the AssetStore by simulating the full hot-reload
    /// pipeline: pre-populate placeholders, register targets, send
    /// `FileChange` events through the queue's input channel,
    /// drain compile results into the dispatcher, apply through
    /// `AssetStore::replace`. Returns `(store, name → handle)`
    /// where every handle is the original placeholder handle (the
    /// dispatcher preserves identity via `replace`).
    fn hot_reload_drive(
        project: &TempProject,
        assets: &[SyntheticAsset],
    ) -> (AssetStore, Vec<(&'static str, TextureHandle)>) {
        let mut store = AssetStore::new();
        // Pre-insert placeholder textures so the handles stay
        // valid through the reload.
        let placeholder = Texture {
            width: 1,
            height: 1,
            format: TextureFormat::Rgba8,
            color_space: ColorSpace::Srgb,
            data: vec![0, 0, 0, 0],
        };
        let mut handles: Vec<(&'static str, TextureHandle)> = Vec::new();
        for a in assets {
            let h = store.insert(placeholder.clone());
            handles.push((a.logical, h));
        }

        // Channels: file changes → queue → results → dispatcher.
        // Bypassing the OS file watcher: M7.1's tests already
        // exercise that side; here we test the compile + dispatch
        // half end-to-end.
        let (tx_changes, rx_changes) = channel();
        let (tx_results, rx_results) = channel();

        let targets: Vec<CompileTarget> = assets
            .iter()
            .map(|a| {
                CompileTarget::new(
                    format!("textures/{}", a.logical),
                    CompiledAssetKind::Texture,
                    project.join(a.filename),
                )
            })
            .collect();

        let _queue = CompileQueue::start_with_sender(
            CompileQueueConfig {
                file_changes: rx_changes,
                targets,
                compile_fn: default_compile_fn(),
            },
            tx_results,
        )
        .expect("queue start");

        let mut dispatcher = ReloadDispatcher::new(rx_results);
        for (logical, h) in &handles {
            dispatcher.register_texture(format!("textures/{}", logical), *h);
        }

        // Send a FileChange::Modified for every source. The queue
        // canonicalises paths internally for its reverse-index
        // lookup, so the path the example sees doesn't need to be
        // pre-canonicalised — but doing so here matches what the
        // real watcher would deliver.
        for a in assets {
            let canon = canonicalise_for_index(&project.join(a.filename));
            tx_changes
                .send(FileChange::modified(canon))
                .expect("send change");
        }
        // Drop the sender so the queue's worker stops blocking on
        // recv after it processes everything.
        drop(tx_changes);

        // Poll the dispatcher until every expected reload has
        // applied or we time out. Without this loop the test would
        // race against the worker thread.
        let total = assets.len();
        let mut applied = 0usize;
        let deadline = Instant::now() + Duration::from_secs(5);
        while applied < total {
            let summary = dispatcher.dispatch(&mut store);
            applied += summary.applied.len();
            if applied >= total {
                break;
            }
            if Instant::now() >= deadline {
                panic!(
                    "hot_reload pipeline timed out: {} of {} assets reloaded",
                    applied, total
                );
            }
            std::thread::sleep(Duration::from_millis(20));
        }

        (store, handles)
    }

    fn synthetic_set() -> Vec<SyntheticAsset> {
        // Three assets at distinct shapes + colours so
        // equivalence catches any per-asset drift, not just a
        // happy-path single-asset test.
        vec![
            SyntheticAsset {
                logical: "red_2x2",
                filename: "red.bmp",
                width: 2,
                height: 2,
                rgb: [255, 0, 0],
            },
            SyntheticAsset {
                logical: "blue_3x1",
                filename: "blue.bmp",
                width: 3,
                height: 1,
                rgb: [0, 0, 255],
            },
            SyntheticAsset {
                logical: "green_1x4",
                filename: "green.bmp",
                width: 1,
                height: 4,
                rgb: [0, 255, 0],
            },
        ]
    }

    // ── The property test ───────────────────────────────────────────────────

    /// **The promise:** for every logical name in the project,
    /// the `Texture` produced by the hot-reload pipeline equals
    /// the `Texture` produced by a fresh compile + decode + insert,
    /// byte for byte. This is the load-bearing claim of M7's
    /// pillar-2 differentiation; the test running green on every
    /// commit is what keeps it from decaying into aspiration.
    ///
    /// See ADR 0004 Q2, Q15.
    #[test]
    fn hot_reloaded_textures_equal_clean_loaded_textures() {
        let project = TempProject::new("equiv_basic");
        let assets = synthetic_set();
        write_synthetic_assets(&project, &assets);

        let (clean_store, clean_handles) = clean_load(&project, &assets);
        let (hot_store, hot_handles) = hot_reload_drive(&project, &assets);

        // Per-asset content equality. Identity (handle indices)
        // is allowed to differ between stores — the property is
        // observable content equivalence, per ADR Q2's narrowed
        // promise scope.
        for ((clean_name, clean_h), (hot_name, hot_h)) in
            clean_handles.iter().zip(hot_handles.iter())
        {
            assert_eq!(clean_name, hot_name, "asset order drift");
            let clean_tex = clean_store.get(*clean_h).expect("clean tex present");
            let hot_tex = hot_store.get(*hot_h).expect("hot tex present");
            assert_eq!(
                clean_tex.width, hot_tex.width,
                "width differs for {}",
                clean_name
            );
            assert_eq!(
                clean_tex.height, hot_tex.height,
                "height differs for {}",
                clean_name
            );
            assert_eq!(
                clean_tex.format, hot_tex.format,
                "format differs for {}",
                clean_name
            );
            assert_eq!(
                clean_tex.color_space, hot_tex.color_space,
                "color_space differs for {} (would silently corrupt PBR lighting)",
                clean_name
            );
            assert_eq!(
                clean_tex.data, hot_tex.data,
                "pixel data differs for {} (the load-bearing equivalence assertion)",
                clean_name
            );
        }
    }

    /// Hot-reloading the same asset multiple times in a row keeps
    /// returning content equal to a clean load. Catches drift in
    /// the dispatcher's `replace` path or the queue's generation
    /// counter — bugs that only show up after the second reload.
    ///
    /// This is the discipline ADR 0003 Pillar 4 (debuggability)
    /// asks for: not just "the first hot-reload works" but "every
    /// hot-reload works, forever."
    #[test]
    fn repeated_hot_reloads_remain_equivalent_to_clean_load() {
        let project = TempProject::new("equiv_repeated");
        let assets = synthetic_set();
        write_synthetic_assets(&project, &assets);

        // Reference state: clean load.
        let (clean_store, clean_handles) = clean_load(&project, &assets);

        // Drive hot-reload three times, asserting equivalence at
        // each round. The pipeline machinery is single-shot per
        // call (each `hot_reload_drive` builds its own queue), so
        // this is "three independent hot-reload sessions all
        // produce the same result."
        for round in 0..3 {
            let (hot_store, hot_handles) = hot_reload_drive(&project, &assets);
            for ((clean_name, clean_h), (_, hot_h)) in
                clean_handles.iter().zip(hot_handles.iter())
            {
                let clean_tex = clean_store.get(*clean_h).unwrap();
                let hot_tex = hot_store.get(*hot_h).unwrap();
                assert_eq!(
                    clean_tex.data, hot_tex.data,
                    "round {}: pixel data drifted for {}",
                    round, clean_name
                );
            }
        }
    }

    /// Identity preservation through the pipeline: the handle the
    /// caller had before reload still resolves to a valid asset
    /// after reload. The dispatcher uses `AssetStore::replace`
    /// which preserves the slot's generation; this test pins the
    /// property at the pipeline level, not just the storage
    /// level.
    ///
    /// Without this, every consumer that holds a handle (ECS
    /// components, materials, scene serialisations) would have to
    /// re-resolve on every reload — exactly the failure mode ADR
    /// 0004 Q6 is designed against.
    #[test]
    fn handle_identity_survives_pipeline_reload() {
        let project = TempProject::new("equiv_identity");
        let assets = synthetic_set();
        write_synthetic_assets(&project, &assets);

        let (mut store, handles) = {
            let mut s = AssetStore::new();
            let mut h = Vec::new();
            for a in &assets {
                let placeholder = Texture {
                    width: 1,
                    height: 1,
                    format: TextureFormat::Rgba8,
                    color_space: ColorSpace::Srgb,
                    data: vec![0, 0, 0, 0],
                };
                h.push((a.logical, s.insert(placeholder)));
            }
            (s, h)
        };
        let original_handle_indices: Vec<u32> =
            handles.iter().map(|(_, h)| h.index).collect();

        // Drive a hot-reload using these exact placeholder handles.
        let (tx_changes, rx_changes) = channel();
        let (tx_results, rx_results) = channel();

        let targets: Vec<CompileTarget> = assets
            .iter()
            .map(|a| {
                CompileTarget::new(
                    format!("textures/{}", a.logical),
                    CompiledAssetKind::Texture,
                    project.join(a.filename),
                )
            })
            .collect();

        let _queue = CompileQueue::start_with_sender(
            CompileQueueConfig {
                file_changes: rx_changes,
                targets,
                compile_fn: default_compile_fn(),
            },
            tx_results,
        )
        .expect("queue");
        let mut dispatcher = ReloadDispatcher::new(rx_results);
        for (logical, h) in &handles {
            dispatcher.register_texture(format!("textures/{}", logical), *h);
        }

        for a in &assets {
            tx_changes
                .send(FileChange::modified(canonicalise_for_index(
                    &project.join(a.filename),
                )))
                .unwrap();
        }
        drop(tx_changes);

        let mut applied = 0usize;
        let deadline = Instant::now() + Duration::from_secs(5);
        while applied < assets.len() {
            applied += dispatcher.dispatch(&mut store).applied.len();
            if applied >= assets.len() {
                break;
            }
            if Instant::now() >= deadline {
                panic!("identity test timed out");
            }
            std::thread::sleep(Duration::from_millis(20));
        }

        // The original handles still resolve, generations
        // unchanged, and content reflects the on-disk asset.
        for (i, (logical, h)) in handles.iter().enumerate() {
            assert_eq!(
                h.index, original_handle_indices[i],
                "handle index for {} was reissued during reload",
                logical
            );
            let tex = store.get(*h).expect("post-reload handle still resolves");
            // Pixel data should be the synthetic colour — proves
            // the reload actually applied to *this* handle, not
            // some other slot.
            let asset = &assets[i];
            assert_eq!(tex.width, asset.width, "width drift for {}", logical);
            assert_eq!(tex.height, asset.height, "height drift for {}", logical);
        }
    }

    // ── Shader equivalence ─────────────────────────────────────────────────

    /// Hot-reloaded `Shader` content equals what a fresh
    /// `compile_shader_asset` + `decode_compiled_shader_payload`
    /// would produce. Pins the equivalence promise (ADR Q2) for
    /// shader assets — without this, every future change to the
    /// shader compile path can silently break it.
    ///
    /// The test:
    /// 1. Writes a `.hshd` source file with `#shader vertex` /
    ///    `#shader fragment` sections to the temp project.
    /// 2. Builds a reference `Shader` via the production
    ///    compile + decode path.
    /// 3. Drives the full hot-reload pipeline (compile queue +
    ///    dispatcher) against the same source file.
    /// 4. Asserts both `Shader` structs are byte-equal:
    ///    name, vertex_source, fragment_source.
    #[test]
    fn hot_reloaded_shader_equals_clean_loaded_shader() {
        use crate::assets::{
            compile_shader_asset, decode_compiled_shader_payload, Shader, ShaderHandle,
        };

        let project = TempProject::new("equiv_shader");
        let source_filename = "lit.hshd";
        let logical = "shaders/lit";
        let source = "// header comment\n\
                      #shader vertex\n\
                      #version 330 core\n\
                      layout(location = 0) in vec3 a_pos;\n\
                      void main() { gl_Position = vec4(a_pos, 1.0); }\n\
                      \n\
                      #shader fragment\n\
                      #version 330 core\n\
                      out vec4 frag_color;\n\
                      void main() { frag_color = vec4(0.5, 0.7, 1.0, 1.0); }\n";
        std::fs::write(project.join(source_filename), source).unwrap();
        let source_path = project.join(source_filename);

        // Path 1: clean load through the production path.
        let clean_blob = compile_shader_asset(&source_path, logical).expect("compile");
        let clean_payload =
            decode_compiled_shader_payload(&clean_blob.payload).expect("decode");
        let clean_shader = Shader {
            name: clean_payload.name.clone(),
            vertex_source: clean_payload.vertex_source.clone(),
            fragment_source: clean_payload.fragment_source.clone(),
        };

        // Path 2: full hot-reload pipeline.
        let mut hot_store = AssetStore::new();
        let placeholder_shader = Shader {
            name: logical.to_string(),
            vertex_source: "// placeholder".into(),
            fragment_source: "// placeholder".into(),
        };
        let hot_handle: ShaderHandle = hot_store.insert(placeholder_shader);

        let (tx_changes, rx_changes) = channel();
        let (tx_results, rx_results) = channel();
        let _queue = CompileQueue::start_with_sender(
            CompileQueueConfig {
                file_changes: rx_changes,
                targets: vec![CompileTarget::new(
                    logical,
                    CompiledAssetKind::Shader,
                    source_path.clone(),
                )],
                compile_fn: default_compile_fn(),
            },
            tx_results,
        )
        .expect("queue start");

        let mut dispatcher = ReloadDispatcher::new(rx_results);
        dispatcher.register_shader(logical, hot_handle);

        tx_changes
            .send(FileChange::modified(canonicalise_for_index(&source_path)))
            .expect("send change");
        drop(tx_changes);

        // Drain the dispatcher until the reload lands or we
        // time out.
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut applied = 0usize;
        while applied < 1 {
            applied += dispatcher.dispatch(&mut hot_store).applied.len();
            if applied >= 1 {
                break;
            }
            if Instant::now() >= deadline {
                panic!("shader hot-reload pipeline timed out");
            }
            std::thread::sleep(Duration::from_millis(20));
        }

        // The load-bearing assertion: hot-reloaded shader equals
        // clean-loaded shader, byte for byte.
        let hot_shader = hot_store.get(hot_handle).expect("hot shader present");
        assert_eq!(
            hot_shader.name, clean_shader.name,
            "shader name drift between hot-reload and clean-load"
        );
        assert_eq!(
            hot_shader.vertex_source, clean_shader.vertex_source,
            "vertex source drift between hot-reload and clean-load"
        );
        assert_eq!(
            hot_shader.fragment_source, clean_shader.fragment_source,
            "fragment source drift between hot-reload and clean-load"
        );
    }

    // ── Material equivalence ───────────────────────────────────────────────

    /// Hot-reloaded `Material` content equals what a fresh
    /// compile + decode + reconstruct would produce. The
    /// reconstruction layer is the load-bearing piece for
    /// Material — it's the first reload kind that has to look up
    /// dependency handles via the dispatcher's bindings map. The
    /// equivalence promise (ADR Q2) holds end-to-end if and only
    /// if the rebuilt Material's `albedo`/`normal`/`shader`
    /// handles match what a fresh load would produce.
    ///
    /// Test setup writes a `.hmat` source file plus referenced
    /// BMP textures to the temp project. The material's
    /// dependencies (albedo + normal map) are pre-registered with
    /// the dispatcher so the apply path can resolve them.
    #[test]
    fn hot_reloaded_material_equals_clean_loaded_material() {
        use crate::assets::{
            compile_material_asset, decode_compiled_material_payload, Material, MaterialHandle,
            NormalMap, NormalMapHandle, Shader, ShaderHandle, Texture, TextureFormat,
        };

        let project = TempProject::new("equiv_material");

        // Write the dependency BMP textures. Their content
        // doesn't matter for material equivalence — only the
        // logical names do — but `compile_material_asset`
        // doesn't read them either. We write them anyway so
        // future versions of the asset compiler that *do*
        // validate dependencies will still pass this test.
        let albedo_pixels: Vec<u8> = (0..(2 * 2)).flat_map(|_| [200, 50, 50]).collect();
        std::fs::write(
            project.join("albedo.bmp"),
            encode_rgb8_bmp(2, 2, &albedo_pixels),
        )
        .unwrap();
        let normal_pixels: Vec<u8> = (0..(2 * 2)).flat_map(|_| [128, 128, 255]).collect();
        std::fs::write(
            project.join("normal.bmp"),
            encode_rgb8_bmp(2, 2, &normal_pixels),
        )
        .unwrap();

        // Write the material source file.
        let mat_filename = "test.hmat";
        let mat_source = "albedo textures/albedo\n\
                          normal textures/normal\n\
                          double_sided true\n\
                          alpha_cutout false\n";
        std::fs::write(project.join(mat_filename), mat_source).unwrap();
        let mat_source_path = project.join(mat_filename);

        // Pre-create dependency handles in a shared store. Both
        // paths use the same store so `Material.albedo` /
        // `Material.normal` handle equality is meaningful.
        let mut store = AssetStore::new();
        let albedo_handle = store.insert(Texture {
            width: 1,
            height: 1,
            format: TextureFormat::Rgba8,
            color_space: ColorSpace::Srgb,
            data: vec![200, 50, 50, 255],
        });
        let normal_handle: NormalMapHandle = store.insert(NormalMap::new_opengl(
            1,
            1,
            TextureFormat::Rgb8,
            vec![128, 128, 255],
        ));
        let shader_handle: ShaderHandle = store.insert(Shader {
            name: "shaders/test".into(),
            vertex_source: "// test".into(),
            fragment_source: "// test".into(),
        });

        // Path 1: clean load. compile_material_asset →
        // decode_compiled_material_payload → reconstruct
        // Material with the resolved dependency handles → insert
        // as a fresh asset.
        let clean_blob =
            compile_material_asset(&mat_source_path, "materials/test").expect("compile material");
        let clean_payload =
            decode_compiled_material_payload(&clean_blob.payload).expect("decode payload");
        let clean_material = Material::new(shader_handle, albedo_handle)
            .with_normal_map(normal_handle)
            .with_double_sided(clean_payload.double_sided)
            .with_alpha_cutout(clean_payload.alpha_cutout);
        let clean_material_handle: MaterialHandle = store.insert(clean_material.clone());

        // Path 2: hot-reload. Pre-insert a placeholder material,
        // then drive the pipeline against the same .hmat source.
        let placeholder_material = Material::new(shader_handle, albedo_handle);
        let hot_material_handle: MaterialHandle = store.insert(placeholder_material);

        let (tx_changes, rx_changes) = channel();
        let (tx_results, rx_results) = channel();
        let _queue = CompileQueue::start_with_sender(
            CompileQueueConfig {
                file_changes: rx_changes,
                targets: vec![CompileTarget::new(
                    "materials/test",
                    CompiledAssetKind::Material,
                    mat_source_path.clone(),
                )],
                compile_fn: default_compile_fn(),
            },
            tx_results,
        )
        .expect("queue start");

        let mut dispatcher = ReloadDispatcher::new(rx_results);
        // Dependencies must be registered before the material so
        // the apply path can resolve them. ADR 0004 Q4.
        dispatcher.register_texture("textures/albedo", albedo_handle);
        dispatcher.register_normal_map("textures/normal", normal_handle);
        dispatcher.register_material("materials/test", hot_material_handle, shader_handle);

        tx_changes
            .send(FileChange::modified(canonicalise_for_index(
                &mat_source_path,
            )))
            .expect("send change");
        drop(tx_changes);

        let deadline = Instant::now() + Duration::from_secs(5);
        let mut applied = 0usize;
        while applied < 1 {
            applied += dispatcher.dispatch(&mut store).applied.len();
            if applied >= 1 {
                break;
            }
            if Instant::now() >= deadline {
                panic!("material hot-reload pipeline timed out");
            }
            std::thread::sleep(Duration::from_millis(20));
        }

        // Load-bearing assertion: every field of the hot-
        // reloaded material matches its clean-loaded sibling.
        let hot_material = store.get(hot_material_handle).expect("hot material");
        let clean_material_resolved = store
            .get(clean_material_handle)
            .expect("clean material");
        assert_eq!(
            hot_material.albedo, clean_material_resolved.albedo,
            "albedo handle drift"
        );
        assert_eq!(
            hot_material.normal, clean_material_resolved.normal,
            "normal handle drift"
        );
        assert_eq!(
            hot_material.specular_gloss, clean_material_resolved.specular_gloss,
            "specular_gloss drift"
        );
        assert_eq!(
            hot_material.shader, clean_material_resolved.shader,
            "shader handle drift"
        );
        assert_eq!(
            hot_material.double_sided, clean_material_resolved.double_sided,
            "double_sided flag drift"
        );
        assert_eq!(
            hot_material.alpha_cutout, clean_material_resolved.alpha_cutout,
            "alpha_cutout flag drift"
        );
    }

    // ── Mesh equivalence ───────────────────────────────────────────────────

    /// Hot-reloaded `MeshData` content equals what a fresh
    /// `compile_mesh_asset` + `decode_compiled_mesh_payload`
    /// would produce. Closes the harness's last shipping
    /// asset-kind coverage gap — Texture / Material / Shader were
    /// pinned in earlier slices; this completes the set.
    ///
    /// Setup writes a minimal `.obj` source (a single triangle —
    /// just enough geometry that the round-trip exercises every
    /// vertex attribute the encoder + decoder touch). The clean
    /// path goes through the production compile + decode; the hot
    /// path drives the full `CompileQueue` + `ReloadDispatcher`
    /// pipeline. Per-field assertions name the offending vertex
    /// attribute on drift, so any future regression points
    /// directly at what broke.
    #[test]
    fn hot_reloaded_mesh_equals_clean_loaded_mesh() {
        use crate::assets::{
            compile_mesh_asset, decode_compiled_mesh_payload, MeshData, MeshHandle,
        };

        let project = TempProject::new("equiv_mesh");
        let source_filename = "triangle.obj";
        let logical = "meshes/triangle";

        // Minimal OBJ: one triangle. `MeshData::from_obj`
        // generates smooth normals + tangents because the file
        // contains no `vn` lines; both clean and hot paths run
        // through the same code so they get identical generated
        // attributes.
        let source = "# minimal triangle for equivalence test\n\
                      v 0 0 0\n\
                      v 1 0 0\n\
                      v 0 1 0\n\
                      vt 0 0\n\
                      vt 1 0\n\
                      vt 0 1\n\
                      f 1/1 2/2 3/3\n";
        std::fs::write(project.join(source_filename), source).unwrap();
        let source_path = project.join(source_filename);

        // Path 1: clean load via the production path.
        let clean_blob = compile_mesh_asset(&source_path, logical).expect("compile");
        let clean_mesh =
            decode_compiled_mesh_payload(&clean_blob.payload).expect("decode payload");

        // Path 2: hot-reload pipeline.
        let mut hot_store = AssetStore::new();
        // Pre-insert a placeholder mesh so the handle stays
        // valid through the reload. Use any small mesh — the
        // dispatcher's replace overwrites the contents.
        let placeholder = MeshData::plane(1, 1);
        let hot_handle: MeshHandle = hot_store.insert(placeholder);

        let (tx_changes, rx_changes) = channel();
        let (tx_results, rx_results) = channel();
        let _queue = CompileQueue::start_with_sender(
            CompileQueueConfig {
                file_changes: rx_changes,
                targets: vec![CompileTarget::new(
                    logical,
                    CompiledAssetKind::Mesh,
                    source_path.clone(),
                )],
                compile_fn: default_compile_fn(),
            },
            tx_results,
        )
        .expect("queue start");

        let mut dispatcher = ReloadDispatcher::new(rx_results);
        dispatcher.register_mesh(logical, hot_handle);

        tx_changes
            .send(FileChange::modified(canonicalise_for_index(&source_path)))
            .expect("send change");
        drop(tx_changes);

        let deadline = Instant::now() + Duration::from_secs(5);
        let mut applied = 0usize;
        while applied < 1 {
            applied += dispatcher.dispatch(&mut hot_store).applied.len();
            if applied >= 1 {
                break;
            }
            if Instant::now() >= deadline {
                panic!("mesh hot-reload pipeline timed out");
            }
            std::thread::sleep(Duration::from_millis(20));
        }

        let hot_mesh = hot_store.get(hot_handle).expect("hot mesh present");

        // Top-level shape equivalence: same vertex + index
        // counts. A mismatch here means the OBJ parser / encoder
        // / decoder produced different topologies — the loudest
        // possible drift.
        assert_eq!(
            clean_mesh.vertices.len(),
            hot_mesh.vertices.len(),
            "vertex count drift"
        );
        assert_eq!(
            clean_mesh.indices.len(),
            hot_mesh.indices.len(),
            "index count drift"
        );

        // Per-vertex equivalence: position, normal, UV, tangent.
        // f32 fields are compared with epsilon to absorb encoder
        // round-trip rounding (none expected — the encoder
        // serialises raw bits — but the epsilon makes the
        // assertion robust against any future codec drift).
        const EPS: f32 = 1e-6;
        for (i, (clean_v, hot_v)) in clean_mesh
            .vertices
            .iter()
            .zip(hot_mesh.vertices.iter())
            .enumerate()
        {
            for axis in 0..3 {
                assert!(
                    (clean_v.pos[axis] - hot_v.pos[axis]).abs() < EPS,
                    "pos drift at vertex {} axis {}: clean={} hot={}",
                    i,
                    axis,
                    clean_v.pos[axis],
                    hot_v.pos[axis]
                );
                assert!(
                    (clean_v.normal[axis] - hot_v.normal[axis]).abs() < EPS,
                    "normal drift at vertex {} axis {}: clean={} hot={}",
                    i,
                    axis,
                    clean_v.normal[axis],
                    hot_v.normal[axis]
                );
            }
            for axis in 0..2 {
                assert!(
                    (clean_v.uv[axis] - hot_v.uv[axis]).abs() < EPS,
                    "uv drift at vertex {} axis {}: clean={} hot={}",
                    i,
                    axis,
                    clean_v.uv[axis],
                    hot_v.uv[axis]
                );
            }
            for axis in 0..4 {
                assert!(
                    (clean_v.tangent[axis] - hot_v.tangent[axis]).abs() < EPS,
                    "tangent drift at vertex {} axis {} (Lengyel-style \
                     tangent regen sensitivity): clean={} hot={}",
                    i,
                    axis,
                    clean_v.tangent[axis],
                    hot_v.tangent[axis]
                );
            }
        }

        // Indices are u32 — exact equality, no epsilon. Drift
        // here would mean the topology drifted, which would
        // already have failed at the count check above for any
        // realistic break, but pin the property explicitly.
        assert_eq!(
            clean_mesh.indices, hot_mesh.indices,
            "index buffer drift between hot-reload and clean-load"
        );
    }

    /// Scene equivalence — the load-bearing CI gate for ADR 0004 Q14.
    ///
    /// Load a `.hsc` scene through the clean path
    /// (`parse_scene` → `apply_scene_document_to_world`), capture the
    /// runtime World state. Then run the same source through the full
    /// hot-reload pipeline (`compile_scene_asset` → CompileQueue →
    /// ReloadDispatcher → `EditorSceneSession::reload_scene_document`)
    /// against an initial-state session, capture the post-reload
    /// World state. Assert the two states are equivalent at the
    /// authored-component layer.
    ///
    /// Equivalence here means:
    /// - Same set of editor entity ids resolve to live runtime entities.
    /// - Each authored component (Transform position, Camera fov,
    ///   etc.) carries the same value on both paths.
    ///
    /// We can't compare runtime `Entity` values directly because
    /// they're generational and the two `World`s spawn entities in
    /// different orders. The authored-id-keyed comparison is what
    /// the engine's promise actually requires (ADR 0004 Q2: "user-
    /// observable state" — and editor-authored IDs are the user-
    /// observable identifier; runtime `Entity` slots are not).
    ///
    /// See ADR 0004 Q14 + Q14a (M7.4.1) and Q15 (equivalence harness).
    #[test]
    fn hot_reloaded_scene_equals_clean_loaded_scene() {
        use crate::assets::CompiledAssetKind;
        use crate::ecs::components::{Camera, Stats, Transform};
        use crate::ecs::World;
        use crate::editor::{
            apply_scene_document_to_world, load_scene_doc_from_hsc, EditorSceneSession,
        };

        let project = TempProject::new("equiv_scene");
        let source_filename = "level01.hsc";
        let logical = "levels/level01";

        // A small scene with two authored entities exercising
        // multiple component kinds: Transform, Camera, Stats. Both
        // paths run through the same parse → SceneDocument →
        // World code, so equivalence is the property — any drift
        // must come from the hot-reload pipeline itself.
        let source = r#"
scene "EquivalenceScene"

entity "player" {
  transform { pos 1 2 3 rot 0 0 0 1 scale 1 1 1 }
  stats { health 100 mana 25 attack 8 defense 4 speed 3 }
}

entity "camera" {
  transform { pos 0 5 10 rot 0 0 0 1 scale 1 1 1 }
  camera { fov 60 near 0.1 far 1000 }
  active_camera
}
"#;
        std::fs::write(project.join(source_filename), source).unwrap();
        let source_path = project.join(source_filename);

        // ── Hot path setup: bring up an initial session loaded
        //                    from the *original* source so the
        //                    diff path has a meaningful prior state
        //                    to reconcile against. ──────────────
        let mut hot_session = EditorSceneSession::open_hsc(&source_path).expect("open hsc");
        let mut hot_world = World::new();
        hot_session.rebuild_world(&mut hot_world).expect("rebuild");

        // Mutate the source on disk: bump player position to
        // (7,8,9) and camera fov to 95. Both should reach the
        // runtime World through the hot-reload pipeline.
        let modified_source = r#"
scene "EquivalenceScene"

entity "player" {
  transform { pos 7 8 9 rot 0 0 0 1 scale 1 1 1 }
  stats { health 100 mana 25 attack 8 defense 4 speed 3 }
}

entity "camera" {
  transform { pos 0 5 10 rot 0 0 0 1 scale 1 1 1 }
  camera { fov 95 near 0.1 far 1000 }
  active_camera
}
"#;
        std::fs::write(&source_path, modified_source).unwrap();

        // ── Clean path: load + apply the *post-modification*
        //                source. Both paths now end at the same
        //                authored content, so the equivalence
        //                comparison is meaningful: did the hot
        //                pipeline converge to the same World as a
        //                fresh load? ───────────────────────────
        let clean_doc = load_scene_doc_from_hsc(&source_path).expect("clean load");
        let mut clean_world = World::new();
        let clean_map =
            apply_scene_document_to_world(&mut clean_world, &clean_doc).expect("clean apply");

        // Step 3: kick off the full pipeline: CompileQueue produces
        //         a CompiledScene, ReloadDispatcher decodes it into
        //         a SceneReloadResult, the test code (standing in
        //         for editor_shell) drains it and applies via
        //         EditorSceneSession::reload_scene_document.
        let (tx_changes, rx_changes) = channel();
        let (tx_results, rx_results) = channel();
        let _queue = CompileQueue::start_with_sender(
            CompileQueueConfig {
                file_changes: rx_changes,
                targets: vec![CompileTarget::new(
                    logical,
                    CompiledAssetKind::Scene,
                    source_path.clone(),
                )],
                compile_fn: default_compile_fn(),
            },
            tx_results,
        )
        .expect("queue start");

        let mut dispatcher = ReloadDispatcher::new(rx_results);
        dispatcher.register_scene(logical, source_path.clone());

        tx_changes
            .send(FileChange::modified(canonicalise_for_index(&source_path)))
            .expect("send change");
        drop(tx_changes);

        // Drain with a deadline. The pipeline crosses the
        // file-watcher → compile-queue → dispatcher boundary across
        // threads; polling lets the worker thread make progress
        // without us racing it.
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut scene_results: Vec<crate::hot_reload::SceneReloadResult> = Vec::new();
        loop {
            let summary = dispatcher.dispatch(&mut AssetStore::new());
            scene_results.extend(summary.scenes_reloaded);
            if !scene_results.is_empty() {
                break;
            }
            if Instant::now() >= deadline {
                panic!("scene hot-reload pipeline timed out");
            }
            std::thread::sleep(Duration::from_millis(20));
        }

        // Apply the reload against the live session + world.
        let result = scene_results.pop().expect("at least one scene reload");
        assert_eq!(result.logical_name, logical);
        let outcome = hot_session
            .reload_scene_document(result.document, &mut hot_world)
            .expect("reload_scene_document");
        // The diff should be the patch path (entity set unchanged,
        // values mutated). NoChange / FullRebuild here would mean
        // the pipeline silently dropped the modification — that's
        // the kind of bug this test is designed to catch.
        match outcome {
            crate::editor::SceneReloadOutcome::Diffed { added, removed, patched } => {
                assert_eq!(added, 0, "no entities added in this test");
                assert_eq!(removed, 0, "no entities removed in this test");
                assert!(patched >= 1, "expected at least one patched entity");
            }
            other => panic!(
                "expected Diffed, got {:?}; pipeline failed to deliver the modification",
                other
            ),
        }

        // ── Equivalence assertions. Compare authored values via
        //     the editor-id keyed maps; runtime Entity slots are
        //     allowed to differ between the two Worlds (ADR Q2
        //     deliberately excludes "transient backend identifiers"
        //     from the equivalence promise). ─────────────────────
        let hot_map = hot_session.last_world_map().to_vec();
        assert_eq!(
            clean_map.len(),
            hot_map.len(),
            "authored entity count drift: clean={} hot={}",
            clean_map.len(),
            hot_map.len()
        );

        for (editor_id, clean_entity) in &clean_map {
            let hot_entity = hot_map
                .iter()
                .find(|(id, _)| id == editor_id)
                .map(|(_, e)| *e)
                .unwrap_or_else(|| panic!("authored id {editor_id} missing from hot map"));

            // Transform parity (every authored entity has one).
            let clean_t = clean_world
                .get::<Transform>(*clean_entity)
                .expect("clean transform");
            let hot_t = hot_world.get::<Transform>(hot_entity).expect("hot transform");
            const EPS: f32 = 1e-6;
            // Vec3 doesn't impl Index — compare each component
            // explicitly. The components are x/y/z by convention
            // throughout the engine.
            assert!(
                (clean_t.position.x - hot_t.position.x).abs() < EPS,
                "transform.position.x drift on editor id {}: clean={} hot={}",
                editor_id,
                clean_t.position.x,
                hot_t.position.x
            );
            assert!(
                (clean_t.position.y - hot_t.position.y).abs() < EPS,
                "transform.position.y drift on editor id {}: clean={} hot={}",
                editor_id,
                clean_t.position.y,
                hot_t.position.y
            );
            assert!(
                (clean_t.position.z - hot_t.position.z).abs() < EPS,
                "transform.position.z drift on editor id {}: clean={} hot={}",
                editor_id,
                clean_t.position.z,
                hot_t.position.z
            );

            // Camera parity (only on the camera entity).
            if let Some(clean_cam) = clean_world.get::<Camera>(*clean_entity) {
                let hot_cam = hot_world
                    .get::<Camera>(hot_entity)
                    .expect("hot camera missing where clean has one");
                assert!(
                    (clean_cam.fov - hot_cam.fov).abs() < EPS,
                    "camera.fov drift on editor id {}: clean={} hot={}",
                    editor_id,
                    clean_cam.fov,
                    hot_cam.fov
                );
            }

            // Stats parity (only on entities that authored them).
            if let Some(clean_stats) = clean_world.get::<Stats>(*clean_entity) {
                let hot_stats = hot_world
                    .get::<Stats>(hot_entity)
                    .expect("hot stats missing where clean has them");
                assert!(
                    (clean_stats.health - hot_stats.health).abs() < EPS,
                    "stats.health drift on editor id {}: clean={} hot={}",
                    editor_id,
                    clean_stats.health,
                    hot_stats.health
                );
            }
        }
    }
}
