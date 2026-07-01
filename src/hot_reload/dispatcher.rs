//! Reload dispatcher: applies [`super::CompileResult`]s to the live
//! [`crate::assets::AssetStore`] at frame boundaries, preserves
//! handle identity, and surfaces a per-frame summary of what
//! changed (for cache eviction + editor inspector refresh).
//!
//! ## Pipeline position
//!
//! ```text
//!  Receiver<CompileResult>      (from CompileQueue, M7.2)
//!         │
//!         ▼
//!  ReloadDispatcher::dispatch
//!         │
//!         ├── decode payload via assets::decode_compiled_*_payload
//!         ├── AssetStore::replace<T> (handle identity preserved, M7.0)
//!         └── emit AssetInvalidated events into DispatchSummary
//!         │
//!         ▼
//!   caller routes events to:
//!     - InvalidationBus subscribers (editor inspector, future caches)
//!     - direct GPU cache eviction (renderer caches keyed by handle)
//! ```
//!
//! ## Last-wins discipline
//!
//! Channel order is the truth. When the dispatcher drains, it groups
//! incoming `CompileResult`s by logical name and only the final
//! result per name in that drain wins; earlier ones are dropped as
//! stale. This sidesteps explicit generation tracking on the
//! dispatcher side — the per-asset generation counter on the
//! [`super::CompileQueue`] worker is preserved in the result
//! payload and surfaced in the summary for telemetry, but the
//! dispatcher's filter is the channel-order property, which is
//! simpler and equivalent.
//!
//! See ADR 0004 Q4 + Q9 (M7.3).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::Receiver;

use crate::assets::{
    decode_compiled_material_payload, decode_compiled_mesh_payload,
    decode_compiled_normal_map_payload, decode_compiled_scene_payload,
    decode_compiled_shader_payload, decode_compiled_texture_payload, AssetInvalidated, AssetKind,
    AssetStore, CompiledAssetKind, Handle, Material, MaterialHandle, MeshData, MeshHandle,
    NormalMap, NormalMapHandle, Shader, ShaderHandle, Texture, TextureHandle,
};
use crate::editor::{EditorPlayState, SceneDocument};

use super::compile_queue::{CompileErr, CompileOk, CompileResult, Generation};

// ── Public types ──────────────────────────────────────────────────────────

/// Summary of a single dispatch invocation. Returned by
/// [`ReloadDispatcher::dispatch`] so the caller can route
/// invalidation events to the renderer / editor / bus on its own
/// schedule.
#[derive(Debug, Default)]
pub struct DispatchSummary {
    /// Asset reloads that successfully landed in the AssetStore.
    /// In dispatch order (one event per logical name even if
    /// multiple results arrived for it — the last-wins filter
    /// already collapsed duplicates).
    pub applied: Vec<AssetInvalidated>,
    /// Scene reloads decoded this drain. Scenes don't sit in
    /// `AssetStore` — they live as `SceneDocument`s on the editor's
    /// session — so they ride a separate field instead of sharing
    /// `applied`. The caller (typically the editor's frame loop)
    /// drains this and routes each result to
    /// [`crate::editor::EditorSceneSession::reload_scene_document`]
    /// against its current `World`.
    ///
    /// Why a separate field rather than overloading `applied`?
    /// `AssetInvalidated::handle_index` is meaningful for assets
    /// keyed by `Handle<T>` in `AssetStore` (textures, meshes,
    /// shaders, materials). Scenes have no such handle — the
    /// authored entity ids are the identity. Surfacing scenes
    /// through their own typed field keeps the contract clean and
    /// avoids forcing GPU cache subscribers to filter out
    /// scene-shaped events that don't apply to them.
    ///
    /// See ADR 0004 Q14 + Q14a (M7.4.1 — scene hot-reload).
    pub scenes_reloaded: Vec<SceneReloadResult>,
    /// Compile errors surfaced this drain. Caller should display
    /// these in the editor pane and otherwise leave the previous
    /// asset in place.
    pub errors: Vec<CompileErr>,
    /// Stale results that were filtered out by the last-wins
    /// rule. Carries the dropped generations for telemetry only;
    /// no action required.
    pub stale: Vec<DroppedResult>,
}

/// Outcome of a successful scene-reload decode. The dispatcher does
/// NOT touch the runtime `World` — it only parses the new
/// `SceneDocument` and hands it back to the caller, who has the
/// `EditorSceneSession` + `World` and can run
/// [`crate::editor::EditorSceneSession::reload_scene_document`]
/// against them.
///
/// This separation keeps the dispatcher single-responsibility (asset
/// pipeline) and lets the ECS-mutation path stay where the
/// session-state already lives. The cost is one extra field on
/// `DispatchSummary` that the caller drains; the benefit is no ECS
/// dependency in the hot-reload module.
#[derive(Debug, Clone)]
pub struct SceneReloadResult {
    /// Logical name the scene was registered under (e.g.
    /// `levels/level01`). Stable across reloads.
    pub logical_name: String,
    /// Source path the scene's `.hsc` was loaded from. Surfaced for
    /// editor diagnostics and the equivalence harness; the diff
    /// path itself doesn't need it.
    pub source_path: PathBuf,
    /// Freshly parsed `SceneDocument`. Caller passes this to
    /// `EditorSceneSession::reload_scene_document` to run the
    /// entity-preserving diff against the runtime world. See ADR
    /// 0004 Q14a for the diff contract.
    pub document: SceneDocument,
}

#[derive(Debug)]
pub struct DroppedResult {
    pub logical_name: String,
    pub generation: Generation,
    pub kind: CompiledAssetKind,
}

/// Errors emitted when a `CompileResult` lands but can't be
/// applied — typically because the dispatcher has no registered
/// binding for the logical name, or the payload can't be decoded.
#[derive(Debug)]
pub enum DispatchError {
    /// No registered binding for the result's logical name. The
    /// reload can't land because there's no `Handle<T>` to
    /// `AssetStore::replace`.
    UnknownLogicalName { logical_name: String },
    /// Payload decode failed (corrupt blob, version mismatch).
    DecodeFailed { logical_name: String, error: String },
    /// Compile produced a kind the dispatcher doesn't yet support
    /// for hot-reload. Mesh / Material / Shader / Scene land in
    /// M7.4; v1 of the dispatcher handles Texture and NormalMap
    /// only.
    UnsupportedKind {
        logical_name: String,
        kind: CompiledAssetKind,
    },
    /// A Material reload referenced a dependency (its albedo
    /// texture, normal map, or specular-gloss texture) that the
    /// dispatcher has no binding for. The previous Material stays
    /// live; the editor pane shows this so the developer can
    /// register the missing dependency. See ADR 0004 Q4 for why
    /// the dispatcher's bindings map serves dual-purpose as the
    /// dependency lookup table.
    MissingMaterialDependency {
        material_logical_name: String,
        dependency_logical_name: String,
        slot: &'static str,
    },
}

impl std::fmt::Display for DispatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownLogicalName { logical_name } => write!(
                f,
                "ReloadDispatcher: no registered handle for {:?}",
                logical_name
            ),
            Self::DecodeFailed { logical_name, error } => write!(
                f,
                "ReloadDispatcher: failed to decode {:?}: {}",
                logical_name, error
            ),
            Self::UnsupportedKind { logical_name, kind } => write!(
                f,
                "ReloadDispatcher: hot-reload of {:?} not yet supported (logical name {:?})",
                kind, logical_name
            ),
            Self::MissingMaterialDependency {
                material_logical_name,
                dependency_logical_name,
                slot,
            } => write!(
                f,
                "ReloadDispatcher: material {:?} {} dependency {:?} not registered \
                 with the dispatcher; call register_texture / register_normal_map first",
                material_logical_name, slot, dependency_logical_name
            ),
        }
    }
}

impl std::error::Error for DispatchError {}

// ── Internal binding state ────────────────────────────────────────────────

#[derive(Debug, Clone, Copy)]
enum AssetBinding {
    Texture {
        handle_index: u32,
        handle_generation: u32,
    },
    NormalMap {
        handle_index: u32,
        handle_generation: u32,
    },
    Mesh {
        handle_index: u32,
        handle_generation: u32,
    },
    /// Material binding carries the shader handle alongside the
    /// material's own handle because a Material always wraps a
    /// shader and the dispatcher reconstructs the full Material
    /// struct on reload. Texture / normal-map dependencies are
    /// resolved at apply time via this dispatcher's existing
    /// bindings map (the same map serving as the dependency
    /// lookup table — ADR 0004 Q4).
    Material {
        handle_index: u32,
        handle_generation: u32,
        shader_handle_index: u32,
        shader_handle_generation: u32,
    },
    /// Shader binding. When the source `.hshd` file changes,
    /// `compile_shader_asset` produces a new vertex/fragment pair
    /// and the dispatcher decodes + replaces the [`Shader`] under
    /// this handle. Materials referencing the same handle keep
    /// resolving — identity preservation does the work.
    Shader {
        handle_index: u32,
        handle_generation: u32,
    },
    /// Scene binding. Scenes don't have an `AssetStore` handle (the
    /// authored entity-id ↔ runtime-`Entity` mapping lives on
    /// `EditorSceneSession::last_world_map`), so the binding only
    /// records the source path for diagnostics and the dispatcher
    /// hands the parsed `SceneDocument` back to the caller via
    /// [`SceneReloadResult`].
    Scene {
        /// Index into the dispatcher's parallel `scene_paths` vec.
        /// Direct `PathBuf` storage on the enum would force the
        /// `AssetBinding: Copy` constraint (used by the existing
        /// bindings map's `.copied()` calls) to drop, breaking the
        /// other paths. Indirection through a separate vec keeps
        /// `AssetBinding: Copy` intact.
        path_index: u32,
    },
}


// ── ReloadDispatcher ──────────────────────────────────────────────────────

/// Drains [`CompileResult`]s, applies the corresponding
/// `AssetStore::replace`, and produces a [`DispatchSummary`] the
/// caller routes to GPU caches and the editor.
///
/// **Threading**: lives on the main thread. The
/// [`Engine::on_reload_dispatch`](crate::engine::Engine::on_reload_dispatch)
/// hook is the canonical integration point; callers wire a closure
/// that calls [`Self::dispatch`] there. See ADR 0004 Q10.
pub struct ReloadDispatcher {
    compile_results: Receiver<CompileResult>,
    /// `logical_name -> AssetBinding`. Populated by `register_*`
    /// methods; read by `dispatch` to convert names to handles.
    bindings: HashMap<String, AssetBinding>,
    /// Source paths for scene bindings. `AssetBinding::Scene`
    /// indexes into this vec. Stored separately so the binding
    /// enum stays `Copy`-compatible — see comment on
    /// [`AssetBinding::Scene`].
    scene_paths: Vec<PathBuf>,
    /// Pending compile results buffered while play state locks
    /// reloads. Drained on the next `dispatch` after the lock
    /// clears. See ADR 0004 Q7 for the determinism rationale.
    locked_buffer: Vec<CompileResult>,
}

impl ReloadDispatcher {
    /// Construct a dispatcher consuming `compile_results`. Typical
    /// caller wiring:
    ///
    /// ```ignore
    /// let (tx_changes, rx_changes) = channel();
    /// let watcher = FileWatcher::start_with_sender(wcfg, tx_changes)?;
    /// let (tx_results, rx_results) = channel();
    /// let queue = CompileQueue::start_with_sender(qcfg, tx_results)?;
    /// let mut dispatcher = ReloadDispatcher::new(rx_results);
    /// dispatcher.register_texture("textures/crate", crate_handle);
    /// // ... per frame:
    /// let summary = dispatcher.dispatch(&mut assets);
    /// ```
    pub fn new(compile_results: Receiver<CompileResult>) -> Self {
        Self {
            compile_results,
            bindings: HashMap::new(),
            scene_paths: Vec::new(),
            locked_buffer: Vec::new(),
        }
    }

    /// Bind a logical name to a [`TextureHandle`] so reloads of
    /// that name land on this asset. Replaces any previous
    /// registration.
    pub fn register_texture(&mut self, logical_name: impl Into<String>, handle: TextureHandle) {
        self.bindings.insert(
            logical_name.into(),
            AssetBinding::Texture {
                handle_index: handle.index,
                handle_generation: handle.generation,
            },
        );
    }

    /// Bind a logical name to a [`NormalMapHandle`].
    pub fn register_normal_map(
        &mut self,
        logical_name: impl Into<String>,
        handle: NormalMapHandle,
    ) {
        self.bindings.insert(
            logical_name.into(),
            AssetBinding::NormalMap {
                handle_index: handle.index,
                handle_generation: handle.generation,
            },
        );
    }

    /// Bind a logical name to a [`MeshHandle`]. Compile results
    /// flowing in for this name decode via
    /// [`decode_compiled_mesh_payload`] and replace the
    /// `MeshData` under this handle.
    pub fn register_mesh(&mut self, logical_name: impl Into<String>, handle: MeshHandle) {
        self.bindings.insert(
            logical_name.into(),
            AssetBinding::Mesh {
                handle_index: handle.index,
                handle_generation: handle.generation,
            },
        );
    }

    /// Bind a logical name to a [`MaterialHandle`] and the
    /// [`ShaderHandle`] this material wraps. Compile results for
    /// this name decode via [`decode_compiled_material_payload`],
    /// look up the material's albedo / normal / specular-gloss
    /// dependencies in this dispatcher's existing texture +
    /// normal-map bindings, reconstruct the full [`Material`]
    /// struct, and replace under the material handle.
    ///
    /// The shader handle is supplied by the caller because shader
    /// hot-reload is not yet wired (M7.4.1 follow-up — needs a
    /// `compile_shader_asset` path that doesn't exist today).
    /// Until then, a Material's shader handle is fixed at
    /// registration time; only its texture dependencies and PBR
    /// factors update through the reload pipeline.
    ///
    /// Pre-conditions for the reload to land:
    /// - Every dependent texture / normal-map logical name is
    ///   already registered with the dispatcher via
    ///   [`Self::register_texture`] / [`Self::register_normal_map`].
    /// - The shader handle remains valid for the lifetime of the
    ///   material binding.
    ///
    /// If a dependency isn't registered, the reload fails with
    /// [`DispatchError::MissingMaterialDependency`] and the
    /// previous Material stays live.
    pub fn register_material(
        &mut self,
        logical_name: impl Into<String>,
        handle: MaterialHandle,
        shader: ShaderHandle,
    ) {
        self.bindings.insert(
            logical_name.into(),
            AssetBinding::Material {
                handle_index: handle.index,
                handle_generation: handle.generation,
                shader_handle_index: shader.index,
                shader_handle_generation: shader.generation,
            },
        );
    }

    /// Bind a logical name to a [`ShaderHandle`]. Compile results
    /// flowing in for this name decode via
    /// [`decode_compiled_shader_payload`] and replace the
    /// [`Shader`] under this handle. Materials that reference the
    /// same shader handle continue to resolve — the dispatcher's
    /// `replace` preserves identity, so referenced materials see
    /// the new GLSL source the next time they sample the shader
    /// asset (which is per-frame, in practice).
    ///
    /// Source format: `.hshd` files with `#shader vertex` /
    /// `#shader fragment` section markers. See
    /// [`crate::assets::compile_shader_asset`].
    pub fn register_shader(&mut self, logical_name: impl Into<String>, handle: ShaderHandle) {
        self.bindings.insert(
            logical_name.into(),
            AssetBinding::Shader {
                handle_index: handle.index,
                handle_generation: handle.generation,
            },
        );
    }

    /// Bind a logical name to a scene source path. Compile results
    /// flowing in for this name decode via
    /// [`decode_compiled_scene_payload`] + `SceneDocument::from_hscene_str`
    /// and surface as a [`SceneReloadResult`] in
    /// [`DispatchSummary::scenes_reloaded`].
    ///
    /// Unlike the other `register_*` methods, this one does NOT take
    /// a `Handle<T>` — scenes don't live in `AssetStore`. The `source_path`
    /// is recorded only for editor diagnostics and the equivalence
    /// harness's "where did this reload come from?" question.
    ///
    /// Re-registering the same logical name updates the recorded
    /// path and reuses the existing path slot (no leak in
    /// `scene_paths` from repeated registration of the same scene).
    ///
    /// See ADR 0004 Q14 + Q14a (M7.4.1).
    pub fn register_scene(
        &mut self,
        logical_name: impl Into<String>,
        source_path: impl Into<PathBuf>,
    ) {
        let logical_name = logical_name.into();
        let source_path = source_path.into();
        match self.bindings.get(&logical_name) {
            Some(AssetBinding::Scene { path_index }) => {
                // Reuse the existing slot — common for editor
                // workflow where a scene is opened, edited, opened
                // again from a different path during a project
                // re-link.
                let idx = *path_index as usize;
                if idx < self.scene_paths.len() {
                    self.scene_paths[idx] = source_path;
                    return;
                }
                // Fall through to fresh-slot allocation if the
                // index was somehow out of range — defensive only;
                // shouldn't happen in correct callers.
            }
            _ => {}
        }
        let path_index = self.scene_paths.len() as u32;
        self.scene_paths.push(source_path);
        self.bindings.insert(
            logical_name,
            AssetBinding::Scene { path_index },
        );
    }

    /// Look up a registered texture binding by logical name.
    /// Returns `None` if no binding exists or the binding is for
    /// a different asset kind. Used internally by the Material
    /// apply path to resolve dependencies.
    fn lookup_texture_handle(&self, logical_name: &str) -> Option<TextureHandle> {
        match self.bindings.get(logical_name)? {
            AssetBinding::Texture {
                handle_index,
                handle_generation,
            } => Some(Handle::<Texture>::new(*handle_index, *handle_generation)),
            _ => None,
        }
    }

    /// Look up a registered normal-map binding by logical name.
    /// See [`Self::lookup_texture_handle`].
    fn lookup_normal_map_handle(&self, logical_name: &str) -> Option<NormalMapHandle> {
        match self.bindings.get(logical_name)? {
            AssetBinding::NormalMap {
                handle_index,
                handle_generation,
            } => Some(Handle::<NormalMap>::new(*handle_index, *handle_generation)),
            _ => None,
        }
    }

    /// Number of logical names currently bound.
    pub fn binding_count(&self) -> usize {
        self.bindings.len()
    }

    /// Drain pending compile results and apply them to `assets`.
    /// Equivalent to [`Self::dispatch_with_play_state`] called with
    /// `EditorPlayState::Editing` (no replay lock).
    ///
    /// Last-wins via channel order: if multiple results for the
    /// same logical name are queued, only the most recently
    /// received one applies. Earlier ones are returned as
    /// `DroppedResult` for telemetry.
    pub fn dispatch(&mut self, assets: &mut AssetStore) -> DispatchSummary {
        self.dispatch_with_play_state(assets, EditorPlayState::Editing)
    }

    /// Drain compile results, but respect the determinism
    /// replay-lock per [`EditorPlayState::locks_reloads`]. When the
    /// supplied state locks reloads, results are buffered
    /// internally and not applied; they remain queued until a
    /// subsequent `dispatch` is called with an unlocked play state.
    ///
    /// This is the load-bearing M7.3-time gate that ADR 0004 Q7
    /// describes: hot-reload during a deterministic replay would
    /// diverge the simulation from the recorded state, so the
    /// dispatcher refuses until replay ends.
    pub fn dispatch_with_play_state(
        &mut self,
        assets: &mut AssetStore,
        play_state: EditorPlayState,
    ) -> DispatchSummary {
        let mut summary = DispatchSummary::default();

        // Always drain the channel so the queue's worker doesn't
        // block on a full mpsc buffer while replay runs. When
        // locked, drained results land in the buffer for later
        // application instead of being applied now.
        let mut newly_drained: Vec<CompileResult> = Vec::new();
        while let Ok(result) = self.compile_results.try_recv() {
            newly_drained.push(result);
        }

        if play_state.locks_reloads() {
            // Replay lock active: stash everything for later. The
            // summary surfaces nothing but counts the deferral via
            // `stale` (with the lock-suspended generation tag) so
            // telemetry shows reloads were observed but not
            // applied.
            for result in newly_drained {
                summary.stale.push(DroppedResult {
                    logical_name: result.logical_name().to_string(),
                    generation: result.generation(),
                    kind: match &result {
                        CompileResult::Ok(ok) => ok.kind,
                        CompileResult::Err(err) => err.kind,
                    },
                });
                self.locked_buffer.push(result);
            }
            return summary;
        }

        // Lock not active: combine any previously buffered results
        // with the newly drained ones. The buffered ones are older
        // (from previous locked frames); they go first into the
        // last-wins map so newer ones can shadow them per the
        // channel-order rule.
        let mut to_apply: Vec<CompileResult> =
            std::mem::take(&mut self.locked_buffer);
        to_apply.extend(newly_drained);

        // 1. Group into a per-name latest map. Channel order is
        //    the truth — later inserts shadow earlier ones for the
        //    same logical name.
        let mut latest: HashMap<String, CompileResult> = HashMap::new();
        let mut shadowed: Vec<CompileResult> = Vec::new();
        for result in to_apply {
            let key = result.logical_name().to_string();
            if let Some(prior) = latest.insert(key, result) {
                shadowed.push(prior);
            }
        }

        // Record the shadowed results as stale telemetry.
        for prior in shadowed {
            summary.stale.push(DroppedResult {
                logical_name: prior.logical_name().to_string(),
                generation: prior.generation(),
                kind: match &prior {
                    CompileResult::Ok(ok) => ok.kind,
                    CompileResult::Err(err) => err.kind,
                },
            });
        }

        // 2. Apply the surviving results.
        for (_, result) in latest {
            match result {
                CompileResult::Err(err) => summary.errors.push(err),
                CompileResult::Ok(ok) => match self.apply_ok(assets, &ok) {
                    Ok(ApplyOutcome::Asset(event)) => summary.applied.push(event),
                    Ok(ApplyOutcome::Scene(scene)) => summary.scenes_reloaded.push(scene),
                    Err(DispatchError::UnknownLogicalName { logical_name }) => {
                        summary.errors.push(CompileErr {
                            logical_name,
                            kind: ok.kind,
                            error: "ReloadDispatcher: no registered handle".into(),
                            generation: ok.generation,
                        });
                    }
                    Err(DispatchError::DecodeFailed { logical_name, error }) => {
                        summary.errors.push(CompileErr {
                            logical_name,
                            kind: ok.kind,
                            error,
                            generation: ok.generation,
                        });
                    }
                    Err(DispatchError::UnsupportedKind { logical_name, kind }) => {
                        summary.errors.push(CompileErr {
                            logical_name,
                            kind,
                            error: "hot-reload not yet supported for this kind (M7.4)".into(),
                            generation: ok.generation,
                        });
                    }
                    Err(DispatchError::MissingMaterialDependency {
                        material_logical_name,
                        dependency_logical_name,
                        slot,
                    }) => {
                        summary.errors.push(CompileErr {
                            logical_name: material_logical_name,
                            kind: ok.kind,
                            error: format!(
                                "missing {} dependency {:?}; \
                                 register the dependency before the material reloads",
                                slot, dependency_logical_name
                            ),
                            generation: ok.generation,
                        });
                    }
                },
            }
        }

        summary
    }

    fn apply_ok(
        &self,
        assets: &mut AssetStore,
        ok: &CompileOk,
    ) -> Result<ApplyOutcome, DispatchError> {
        let binding = self.bindings.get(&ok.logical_name).copied().ok_or_else(|| {
            DispatchError::UnknownLogicalName {
                logical_name: ok.logical_name.clone(),
            }
        })?;

        match (ok.kind, binding) {
            (CompiledAssetKind::Texture, AssetBinding::Texture { handle_index, handle_generation }) => {
                let texture = decode_compiled_texture_payload(&ok.payload).map_err(|e| {
                    DispatchError::DecodeFailed {
                        logical_name: ok.logical_name.clone(),
                        error: format!("{}", e),
                    }
                })?;
                let handle: TextureHandle =
                    Handle::<Texture>::new(handle_index, handle_generation);
                let _previous = assets.replace(handle, texture);
                Ok(ApplyOutcome::Asset(AssetInvalidated {
                    kind: AssetKind::Texture,
                    handle_index,
                    logical_name: ok.logical_name.clone(),
                }))
            }
            (CompiledAssetKind::Texture, AssetBinding::NormalMap { handle_index, handle_generation }) => {
                // The compile target was registered as a normal map.
                // Resolve the payload through the SAME decoder the
                // runtime loader's `ensure_normal_map` uses, so the
                // payload's authored NormalMapConvention (M10.4) is
                // honoured: a DirectX payload is green-flipped to the
                // engine-canonical OpenGL Y+ on load, and the source
                // convention round-trips. Decoding via
                // `decode_compiled_texture_payload` here (which drops
                // the convention tag) would leave a DirectX normal map
                // unflipped and mislabelled, diverging from the runtime
                // loader for the identical on-disk blob.
                let normal_map = decode_compiled_normal_map_payload(&ok.payload).map_err(|e| {
                    DispatchError::DecodeFailed {
                        logical_name: ok.logical_name.clone(),
                        error: format!("{}", e),
                    }
                })?;
                let handle: NormalMapHandle =
                    Handle::<NormalMap>::new(handle_index, handle_generation);
                let _previous = assets.replace(handle, normal_map);
                Ok(ApplyOutcome::Asset(AssetInvalidated {
                    kind: AssetKind::NormalMap,
                    handle_index,
                    logical_name: ok.logical_name.clone(),
                }))
            }
            (CompiledAssetKind::Mesh, AssetBinding::Mesh { handle_index, handle_generation }) => {
                let mesh_data = decode_compiled_mesh_payload(&ok.payload).map_err(|e| {
                    DispatchError::DecodeFailed {
                        logical_name: ok.logical_name.clone(),
                        error: format!("{}", e),
                    }
                })?;
                let handle: MeshHandle =
                    Handle::<MeshData>::new(handle_index, handle_generation);
                let _previous = assets.replace(handle, mesh_data);
                Ok(ApplyOutcome::Asset(AssetInvalidated {
                    kind: AssetKind::Mesh,
                    handle_index,
                    logical_name: ok.logical_name.clone(),
                }))
            }
            (
                CompiledAssetKind::Material,
                AssetBinding::Material {
                    handle_index,
                    handle_generation,
                    shader_handle_index,
                    shader_handle_generation,
                },
            ) => {
                // Decode the recompiled material payload (logical
                // names of dependent textures, the double-sided +
                // alpha-cutout flags) and reconstruct the Material
                // by resolving each dependency through *this*
                // dispatcher's binding map. Per ADR 0004 Q4 the
                // bindings map serves dual-purpose: target lookup
                // for the dispatcher's own reloads, and dependency
                // lookup for material reconstruction.
                let payload = decode_compiled_material_payload(&ok.payload).map_err(|e| {
                    DispatchError::DecodeFailed {
                        logical_name: ok.logical_name.clone(),
                        error: format!("{}", e),
                    }
                })?;

                let albedo = self
                    .lookup_texture_handle(&payload.albedo_logical_name)
                    .ok_or_else(|| DispatchError::MissingMaterialDependency {
                        material_logical_name: ok.logical_name.clone(),
                        dependency_logical_name: payload.albedo_logical_name.clone(),
                        slot: "albedo",
                    })?;

                let normal = match payload.normal_logical_name.as_deref() {
                    Some(name) => Some(self.lookup_normal_map_handle(name).ok_or_else(|| {
                        DispatchError::MissingMaterialDependency {
                            material_logical_name: ok.logical_name.clone(),
                            dependency_logical_name: name.to_string(),
                            slot: "normal",
                        }
                    })?),
                    None => None,
                };

                let specular_gloss = match payload.specular_gloss_logical_name.as_deref() {
                    Some(name) => Some(self.lookup_texture_handle(name).ok_or_else(|| {
                        DispatchError::MissingMaterialDependency {
                            material_logical_name: ok.logical_name.clone(),
                            dependency_logical_name: name.to_string(),
                            slot: "specular_gloss",
                        }
                    })?),
                    None => None,
                };

                let shader: ShaderHandle =
                    Handle::<Shader>::new(shader_handle_index, shader_handle_generation);
                let mut material = Material::new(shader, albedo)
                    .with_double_sided(payload.double_sided)
                    .with_alpha_cutout(payload.alpha_cutout);
                if let Some(n) = normal {
                    material = material.with_normal_map(n);
                }
                if let Some(sg) = specular_gloss {
                    material = material.with_specular_gloss(sg);
                }

                let handle: MaterialHandle =
                    Handle::<Material>::new(handle_index, handle_generation);
                let _previous = assets.replace(handle, material);
                Ok(ApplyOutcome::Asset(AssetInvalidated {
                    kind: AssetKind::Material,
                    handle_index,
                    logical_name: ok.logical_name.clone(),
                }))
            }
            (
                CompiledAssetKind::Shader,
                AssetBinding::Shader {
                    handle_index,
                    handle_generation,
                },
            ) => {
                let payload = decode_compiled_shader_payload(&ok.payload).map_err(|e| {
                    DispatchError::DecodeFailed {
                        logical_name: ok.logical_name.clone(),
                        error: format!("{}", e),
                    }
                })?;
                let shader = Shader {
                    name: payload.name,
                    vertex_source: payload.vertex_source,
                    fragment_source: payload.fragment_source,
                };
                let handle: ShaderHandle =
                    Handle::<Shader>::new(handle_index, handle_generation);
                let _previous = assets.replace(handle, shader);
                Ok(ApplyOutcome::Asset(AssetInvalidated {
                    kind: AssetKind::Shader,
                    handle_index,
                    logical_name: ok.logical_name.clone(),
                }))
            }
            (CompiledAssetKind::Scene, AssetBinding::Scene { path_index }) => {
                let payload = decode_compiled_scene_payload(&ok.payload).map_err(|e| {
                    DispatchError::DecodeFailed {
                        logical_name: ok.logical_name.clone(),
                        error: format!("{}", e),
                    }
                })?;
                // The compile step already validated the .hsc → .hscene
                // round-trip. Decoding back into a SceneDocument should
                // not fail in practice, but if it does we surface the
                // raw error string for the editor pane rather than
                // panicking — failure surfaces, not silences (ADR
                // 0004 guardrail).
                let document = SceneDocument::from_hscene_str(&payload.hscene_text)
                    .map_err(|err| DispatchError::DecodeFailed {
                        logical_name: ok.logical_name.clone(),
                        error: format!("scene document parse failed: {}", err),
                    })?;
                let source_path = self
                    .scene_paths
                    .get(path_index as usize)
                    .cloned()
                    .unwrap_or_default();
                Ok(ApplyOutcome::Scene(SceneReloadResult {
                    logical_name: ok.logical_name.clone(),
                    source_path,
                    document,
                }))
            }
            (kind, _) => Err(DispatchError::UnsupportedKind {
                logical_name: ok.logical_name.clone(),
                kind,
            }),
        }
    }
}

/// Internal: distinguishes the two output channels of `apply_ok` —
/// asset reloads land in `DispatchSummary::applied` (consumed by GPU
/// caches keyed by handle); scene reloads land in
/// `DispatchSummary::scenes_reloaded` (consumed by the editor's
/// reload path against its `World`).
///
/// Kept private to the module — callers see the typed
/// `DispatchSummary` fields rather than this enum.
enum ApplyOutcome {
    Asset(AssetInvalidated),
    Scene(SceneReloadResult),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::{AssetStore, ColorSpace, Texture, TextureFormat};
    use crate::hot_reload::compile_queue::{CompileOk, Generation};
    use std::sync::mpsc::channel;

    fn synthetic_texture_payload(red: u8) -> Vec<u8> {
        // Build a small synthetic compiled-texture payload by
        // encoding a real Texture through the existing encoder.
        // Identity round-trip: dispatcher's decode + replace must
        // reconstruct exactly this Texture.
        let texture = Texture {
            width: 2,
            height: 2,
            format: TextureFormat::Rgba8,
            color_space: ColorSpace::Srgb,
            data: vec![
                red, 0, 0, 255, red, 0, 0, 255, red, 0, 0, 255, red, 0, 0, 255,
            ],
        };
        crate::assets::encode_compiled_texture_payload(&texture).unwrap()
    }

    /// Happy path: a registered texture's logical name resolves to
    /// its handle; a `CompileResult::Ok` with the matching name
    /// applies via `AssetStore::replace`; identity is preserved.
    #[test]
    fn dispatch_applies_texture_replace_and_preserves_identity() {
        let mut assets = AssetStore::new();
        let initial = Texture {
            width: 1,
            height: 1,
            format: TextureFormat::Rgba8,
            color_space: ColorSpace::Srgb,
            data: vec![255, 255, 255, 255],
        };
        let handle = assets.insert(initial);

        let (tx, rx) = channel::<CompileResult>();
        let mut dispatcher = ReloadDispatcher::new(rx);
        dispatcher.register_texture("textures/test", handle);

        // Send a result with new payload.
        let payload = synthetic_texture_payload(99);
        tx.send(CompileResult::Ok(CompileOk {
            logical_name: "textures/test".into(),
            kind: CompiledAssetKind::Texture,
            payload,
            generation: Generation(1),
        }))
        .unwrap();

        let summary = dispatcher.dispatch(&mut assets);
        assert_eq!(summary.applied.len(), 1, "expected one applied event");
        assert!(summary.errors.is_empty());
        assert!(summary.stale.is_empty());

        // Identity preserved: same handle still resolves, now to
        // the new value.
        let after = assets.get(handle).unwrap();
        assert_eq!(after.data[0], 99, "red channel should be 99 after reload");

        // Event carries the right metadata.
        assert_eq!(summary.applied[0].kind, AssetKind::Texture);
        assert_eq!(summary.applied[0].handle_index, handle.index);
        assert_eq!(summary.applied[0].logical_name, "textures/test");
    }

    /// Regression (M10.4 review): a normal-map reload must honour the
    /// payload's authored convention. A DirectX-convention texture
    /// payload applied to a NormalMap binding is green-flipped on load
    /// exactly as the runtime loader's `ensure_normal_map` does, so the
    /// identical on-disk blob yields identical in-memory pixels via
    /// both paths. Guards against the dispatcher decoding via the
    /// convention-dropping `decode_compiled_texture_payload`.
    #[test]
    fn dispatch_normal_map_reload_honours_directx_convention() {
        use crate::assets::{
            decode_compiled_normal_map_payload, encode_compiled_texture_payload_with_convention,
            NormalMap, NormalMapConvention,
        };
        let mut assets = AssetStore::new();
        let seed = NormalMap::new_opengl(1, 1, TextureFormat::Rgba8, vec![0, 0, 0, 255]);
        let handle = assets.insert(seed);

        let (tx, rx) = channel::<CompileResult>();
        let mut dispatcher = ReloadDispatcher::new(rx);
        dispatcher.register_normal_map("normals/test", handle);

        // DirectX-convention payload: one RGBA pixel, green = 200.
        let texture = Texture {
            width: 1,
            height: 1,
            format: TextureFormat::Rgba8,
            color_space: ColorSpace::Linear,
            data: vec![10, 200, 30, 255],
        };
        let payload =
            encode_compiled_texture_payload_with_convention(&texture, NormalMapConvention::DirectX)
                .unwrap();
        tx.send(CompileResult::Ok(CompileOk {
            logical_name: "normals/test".into(),
            kind: CompiledAssetKind::Texture,
            payload: payload.clone(),
            generation: Generation(1),
        }))
        .unwrap();

        let summary = dispatcher.dispatch(&mut assets);
        assert_eq!(summary.applied.len(), 1);
        let reloaded = assets.get(handle).unwrap();
        // Green flipped (255 - 200 = 55); convention recorded.
        assert_eq!(
            reloaded.data[1], 55,
            "dispatcher must flip DirectX green on reload"
        );
        assert_eq!(reloaded.source_convention, NormalMapConvention::DirectX);

        // Byte-identical to the runtime-loader decoder for the same blob.
        let via_loader = decode_compiled_normal_map_payload(&payload).unwrap();
        assert_eq!(reloaded.data, via_loader.data);
        assert_eq!(reloaded.source_convention, via_loader.source_convention);
    }

    /// Last-wins via channel order: when two `Ok` results for the
    /// same logical name arrive in one drain, only the second
    /// applies. The first is recorded as stale telemetry.
    #[test]
    fn dispatch_applies_only_latest_when_multiple_for_same_name() {
        let mut assets = AssetStore::new();
        let initial = Texture {
            width: 1,
            height: 1,
            format: TextureFormat::Rgba8,
            color_space: ColorSpace::Srgb,
            data: vec![0, 0, 0, 255],
        };
        let handle = assets.insert(initial);

        let (tx, rx) = channel::<CompileResult>();
        let mut dispatcher = ReloadDispatcher::new(rx);
        dispatcher.register_texture("textures/test", handle);

        // Two results queued; only the second wins.
        tx.send(CompileResult::Ok(CompileOk {
            logical_name: "textures/test".into(),
            kind: CompiledAssetKind::Texture,
            payload: synthetic_texture_payload(50),
            generation: Generation(1),
        }))
        .unwrap();
        tx.send(CompileResult::Ok(CompileOk {
            logical_name: "textures/test".into(),
            kind: CompiledAssetKind::Texture,
            payload: synthetic_texture_payload(200),
            generation: Generation(2),
        }))
        .unwrap();

        let summary = dispatcher.dispatch(&mut assets);
        assert_eq!(summary.applied.len(), 1);
        assert_eq!(summary.stale.len(), 1);
        assert_eq!(summary.stale[0].logical_name, "textures/test");
        assert_eq!(summary.stale[0].generation, Generation(1));

        // Live texture is the LATEST one (red=200), not the first.
        let after = assets.get(handle).unwrap();
        assert_eq!(after.data[0], 200);
    }

    /// Compile errors propagate through to `summary.errors`
    /// without touching the live asset. The previous asset stays
    /// in place.
    #[test]
    fn dispatch_surfaces_compile_errors_without_replacing() {
        let mut assets = AssetStore::new();
        let initial = Texture {
            width: 1,
            height: 1,
            format: TextureFormat::Rgba8,
            color_space: ColorSpace::Srgb,
            data: vec![10, 20, 30, 255],
        };
        let handle = assets.insert(initial);

        let (tx, rx) = channel::<CompileResult>();
        let mut dispatcher = ReloadDispatcher::new(rx);
        dispatcher.register_texture("textures/test", handle);

        tx.send(CompileResult::Err(CompileErr {
            logical_name: "textures/test".into(),
            kind: CompiledAssetKind::Texture,
            error: "synthetic compile failure".into(),
            generation: Generation(1),
        }))
        .unwrap();

        let summary = dispatcher.dispatch(&mut assets);
        assert!(summary.applied.is_empty(), "no replace should happen on error");
        assert_eq!(summary.errors.len(), 1);
        assert!(summary.errors[0].error.contains("synthetic compile failure"));

        // Asset unchanged.
        let after = assets.get(handle).unwrap();
        assert_eq!(after.data, vec![10, 20, 30, 255]);
    }

    /// Unknown logical name (no registered binding) surfaces as
    /// an error rather than panicking. Common case: the compile
    /// queue knows about a target the dispatcher hasn't been told
    /// about.
    #[test]
    fn dispatch_emits_error_for_unknown_logical_name() {
        let mut assets = AssetStore::new();
        let (tx, rx) = channel::<CompileResult>();
        let mut dispatcher = ReloadDispatcher::new(rx);
        // Note: no register_texture call.

        tx.send(CompileResult::Ok(CompileOk {
            logical_name: "textures/unknown".into(),
            kind: CompiledAssetKind::Texture,
            payload: synthetic_texture_payload(0),
            generation: Generation(1),
        }))
        .unwrap();

        let summary = dispatcher.dispatch(&mut assets);
        assert!(summary.applied.is_empty());
        assert_eq!(summary.errors.len(), 1);
        assert!(summary.errors[0].error.contains("no registered handle"));
    }

    /// Unsupported asset kinds (Mesh, Material, ...) surface as
    /// errors rather than silently dropping. M7.4 will extend the
    /// dispatcher to handle them.
    #[test]
    fn dispatch_rejects_unsupported_kinds_loudly() {
        let mut assets = AssetStore::new();
        // Register a texture binding so the lookup hits, but send
        // a Mesh result — the (kind, binding) match arm fails.
        let dummy_texture = Texture {
            width: 1,
            height: 1,
            format: TextureFormat::Rgba8,
            color_space: ColorSpace::Srgb,
            data: vec![0; 4],
        };
        let handle = assets.insert(dummy_texture);

        let (tx, rx) = channel::<CompileResult>();
        let mut dispatcher = ReloadDispatcher::new(rx);
        dispatcher.register_texture("collision", handle);

        tx.send(CompileResult::Ok(CompileOk {
            logical_name: "collision".into(),
            kind: CompiledAssetKind::Mesh,
            payload: vec![0, 1, 2, 3],
            generation: Generation(1),
        }))
        .unwrap();

        let summary = dispatcher.dispatch(&mut assets);
        assert!(summary.applied.is_empty());
        assert_eq!(summary.errors.len(), 1);
        assert!(summary.errors[0].error.contains("not yet supported"));
    }

    /// Empty drain returns an empty summary. Caller can safely call
    /// dispatch every frame without checking for queued results.
    #[test]
    fn dispatch_with_empty_channel_returns_empty_summary() {
        let mut assets = AssetStore::new();
        let (_tx, rx) = channel::<CompileResult>();
        let mut dispatcher = ReloadDispatcher::new(rx);
        let summary = dispatcher.dispatch(&mut assets);
        assert!(summary.applied.is_empty());
        assert!(summary.errors.is_empty());
        assert!(summary.stale.is_empty());
    }

    /// Re-registering the same logical name replaces the previous
    /// binding. Useful when the user reloads the project — every
    /// asset's handle index changes, but the logical name is
    /// stable.
    #[test]
    fn register_texture_replaces_previous_binding() {
        let mut assets = AssetStore::new();
        let h1 = assets.insert(Texture {
            width: 1, height: 1, format: TextureFormat::Rgba8,
            color_space: ColorSpace::Srgb, data: vec![0; 4],
        });
        let h2 = assets.insert(Texture {
            width: 1, height: 1, format: TextureFormat::Rgba8,
            color_space: ColorSpace::Srgb, data: vec![0; 4],
        });
        let (tx, rx) = channel::<CompileResult>();
        let mut dispatcher = ReloadDispatcher::new(rx);
        dispatcher.register_texture("name", h1);
        dispatcher.register_texture("name", h2);
        assert_eq!(dispatcher.binding_count(), 1);

        tx.send(CompileResult::Ok(CompileOk {
            logical_name: "name".into(),
            kind: CompiledAssetKind::Texture,
            payload: synthetic_texture_payload(77),
            generation: Generation(1),
        }))
        .unwrap();

        let summary = dispatcher.dispatch(&mut assets);
        assert_eq!(summary.applied.len(), 1);
        // h1 unchanged; h2 reloaded.
        assert_eq!(assets.get(h1).unwrap().data[0], 0);
        assert_eq!(assets.get(h2).unwrap().data[0], 77);
    }

    /// Dispatching after the queue's sender is dropped doesn't
    /// hang or panic — the channel returns `Disconnected` from
    /// `try_recv` which the loop handles cleanly.
    #[test]
    fn dispatch_after_sender_drop_is_safe() {
        let mut assets = AssetStore::new();
        let (tx, rx) = channel::<CompileResult>();
        drop(tx);
        let mut dispatcher = ReloadDispatcher::new(rx);
        let summary = dispatcher.dispatch(&mut assets);
        assert!(summary.applied.is_empty());
        assert!(summary.errors.is_empty());
    }

    // ── Mesh reload tests ─────────────────────────────────────────────────

    fn synthetic_mesh_payload() -> Vec<u8> {
        // Round-trip a real cube mesh through the encoder so the
        // dispatcher's decode produces a valid MeshData.
        let mesh = crate::assets::MeshData::cube();
        crate::assets::encode_compiled_mesh_payload(&mesh).unwrap()
    }

    /// Mesh reload mirrors the texture path: register a
    /// MeshHandle, send a `Mesh` CompileResult, the dispatcher
    /// decodes the payload, calls `replace`, and emits an
    /// `AssetKind::Mesh` invalidation.
    #[test]
    fn dispatch_applies_mesh_replace_and_emits_event() {
        let mut assets = AssetStore::new();
        let initial = crate::assets::MeshData::plane(1, 1);
        let initial_count = initial.vertex_count();
        let handle = assets.insert(initial);

        let (tx, rx) = channel::<CompileResult>();
        let mut dispatcher = ReloadDispatcher::new(rx);
        dispatcher.register_mesh("meshes/test", handle);

        tx.send(CompileResult::Ok(CompileOk {
            logical_name: "meshes/test".into(),
            kind: CompiledAssetKind::Mesh,
            payload: synthetic_mesh_payload(),
            generation: Generation(1),
        }))
        .unwrap();

        let summary = dispatcher.dispatch(&mut assets);
        assert_eq!(summary.applied.len(), 1);
        assert!(summary.errors.is_empty());
        assert_eq!(summary.applied[0].kind, AssetKind::Mesh);

        // The new mesh is the cube (24 vertices); the old plane
        // had 4. Identity preserved: same handle resolves to the
        // new content.
        let after = assets.get(handle).unwrap();
        assert_ne!(after.vertex_count(), initial_count);
        assert_eq!(after.vertex_count(), 24);
    }

    // ── Determinism replay-lock tests ─────────────────────────────────────

    /// While `EditorPlayState::ReplayingDeterministic` is active,
    /// dispatch buffers compile results without applying them.
    /// Pillar 4 (determinism): hot-reload during replay would
    /// diverge from the recorded state.
    #[test]
    fn dispatch_buffers_when_replay_lock_active() {
        let mut assets = AssetStore::new();
        let initial = Texture {
            width: 1,
            height: 1,
            format: TextureFormat::Rgba8,
            color_space: ColorSpace::Srgb,
            data: vec![10, 10, 10, 255],
        };
        let handle = assets.insert(initial);

        let (tx, rx) = channel::<CompileResult>();
        let mut dispatcher = ReloadDispatcher::new(rx);
        dispatcher.register_texture("textures/test", handle);

        tx.send(CompileResult::Ok(CompileOk {
            logical_name: "textures/test".into(),
            kind: CompiledAssetKind::Texture,
            payload: synthetic_texture_payload(99),
            generation: Generation(1),
        }))
        .unwrap();

        // Replay lock active: result is buffered, not applied.
        let summary = dispatcher.dispatch_with_play_state(
            &mut assets,
            EditorPlayState::ReplayingDeterministic,
        );
        assert!(summary.applied.is_empty(), "no apply during replay");
        assert!(summary.errors.is_empty());
        assert_eq!(summary.stale.len(), 1, "buffered result counted as stale");

        // Asset unchanged.
        assert_eq!(assets.get(handle).unwrap().data[0], 10);

        // Lock cleared on next dispatch: buffered result applies.
        let summary = dispatcher.dispatch_with_play_state(
            &mut assets,
            EditorPlayState::Editing,
        );
        assert_eq!(summary.applied.len(), 1, "buffered result drains");
        assert_eq!(assets.get(handle).unwrap().data[0], 99);
    }

    /// Even while locked, the dispatcher drains the channel — the
    /// queue worker mustn't back up if replay runs for a while.
    /// Buffered results merge with new arrivals on unlock and the
    /// last-wins rule still picks the freshest.
    #[test]
    fn replay_lock_drains_channel_and_preserves_last_wins() {
        let mut assets = AssetStore::new();
        let initial = Texture {
            width: 1,
            height: 1,
            format: TextureFormat::Rgba8,
            color_space: ColorSpace::Srgb,
            data: vec![0, 0, 0, 255],
        };
        let handle = assets.insert(initial);

        let (tx, rx) = channel::<CompileResult>();
        let mut dispatcher = ReloadDispatcher::new(rx);
        dispatcher.register_texture("textures/test", handle);

        // Two results during lock — both buffered.
        tx.send(CompileResult::Ok(CompileOk {
            logical_name: "textures/test".into(),
            kind: CompiledAssetKind::Texture,
            payload: synthetic_texture_payload(50),
            generation: Generation(1),
        }))
        .unwrap();
        tx.send(CompileResult::Ok(CompileOk {
            logical_name: "textures/test".into(),
            kind: CompiledAssetKind::Texture,
            payload: synthetic_texture_payload(100),
            generation: Generation(2),
        }))
        .unwrap();

        let _ = dispatcher.dispatch_with_play_state(
            &mut assets,
            EditorPlayState::ReplayingDeterministic,
        );

        // After unlock + a third (newest) result, the third wins.
        tx.send(CompileResult::Ok(CompileOk {
            logical_name: "textures/test".into(),
            kind: CompiledAssetKind::Texture,
            payload: synthetic_texture_payload(200),
            generation: Generation(3),
        }))
        .unwrap();
        let summary = dispatcher.dispatch_with_play_state(
            &mut assets,
            EditorPlayState::Editing,
        );

        // Exactly one applied — the newest. The two older
        // buffered results were shadowed.
        assert_eq!(summary.applied.len(), 1);
        assert_eq!(assets.get(handle).unwrap().data[0], 200);
        // Two shadowed entries (the lock-buffered ones) report
        // as stale.
        assert!(summary.stale.len() >= 2);
    }

    /// Playing and Paused states do not lock reloads — only
    /// ReplayingDeterministic does. Pin the behaviour.
    #[test]
    fn dispatch_applies_during_playing_and_paused() {
        for state in [EditorPlayState::Editing, EditorPlayState::Playing, EditorPlayState::Paused] {
            let mut assets = AssetStore::new();
            let handle = assets.insert(Texture {
                width: 1,
                height: 1,
                format: TextureFormat::Rgba8,
                color_space: ColorSpace::Srgb,
                data: vec![0, 0, 0, 255],
            });

            let (tx, rx) = channel::<CompileResult>();
            let mut dispatcher = ReloadDispatcher::new(rx);
            dispatcher.register_texture("textures/test", handle);

            tx.send(CompileResult::Ok(CompileOk {
                logical_name: "textures/test".into(),
                kind: CompiledAssetKind::Texture,
                payload: synthetic_texture_payload(123),
                generation: Generation(1),
            }))
            .unwrap();

            let summary = dispatcher.dispatch_with_play_state(&mut assets, state);
            assert_eq!(
                summary.applied.len(),
                1,
                "expected apply for state {:?}",
                state
            );
            assert_eq!(assets.get(handle).unwrap().data[0], 123);
        }
    }

    // ── Material reload tests ─────────────────────────────────────────────

    /// Encode a synthetic compiled-material payload referencing
    /// the given dependency logical names. Wraps the existing
    /// `encode_compiled_material_payload` so tests can build
    /// realistic compile results without touching disk.
    fn synthetic_material_payload(
        albedo_logical: &str,
        normal_logical: Option<&str>,
        specular_gloss_logical: Option<&str>,
        double_sided: bool,
        alpha_cutout: bool,
    ) -> Vec<u8> {
        let payload = crate::assets::CompiledMaterialPayload {
            albedo_logical_name: albedo_logical.to_string(),
            normal_logical_name: normal_logical.map(String::from),
            specular_gloss_logical_name: specular_gloss_logical.map(String::from),
            double_sided,
            alpha_cutout,
        };
        crate::assets::encode_compiled_material_payload(&payload).unwrap()
    }

    /// Insert a Shader into the AssetStore so the dispatcher has
    /// a valid ShaderHandle to bind. Tests don't exercise shader
    /// reload, just material reload, so the shader's content
    /// doesn't matter.
    fn insert_test_shader(assets: &mut AssetStore) -> crate::assets::ShaderHandle {
        assets.insert(crate::assets::Shader {
            name: "test-shader".into(),
            vertex_source: "// test".into(),
            fragment_source: "// test".into(),
        })
    }

    /// Material reload happy path: register albedo + normal-map
    /// dependencies first, then register the material itself,
    /// send a Material CompileResult, and verify the dispatcher
    /// reconstructed the Material with the right dependency
    /// handles and emitted an `AssetKind::Material` event. Pin
    /// the load-bearing reconstruction property: the rebuilt
    /// `Material.albedo` resolves to the texture handle the
    /// dispatcher was registered with.
    #[test]
    fn dispatch_applies_material_with_resolved_dependencies() {
        let mut assets = AssetStore::new();
        // Insert dependencies first so they have stable handles.
        let albedo_handle = assets.insert(Texture {
            width: 1,
            height: 1,
            format: TextureFormat::Rgba8,
            color_space: ColorSpace::Srgb,
            data: vec![10, 20, 30, 255],
        });
        let normal_handle = assets.insert(NormalMap::new_opengl(
            1,
            1,
            TextureFormat::Rgb8,
            vec![128, 128, 255],
        ));
        let shader_handle = insert_test_shader(&mut assets);
        // Insert a placeholder material to occupy the slot.
        let material_handle = assets.insert(Material::new(shader_handle, albedo_handle));

        let (tx, rx) = channel::<CompileResult>();
        let mut dispatcher = ReloadDispatcher::new(rx);
        // Dependencies must be registered with the dispatcher
        // before the material is registered, or the dependency
        // lookup at apply time fails. ADR 0004 Q4.
        dispatcher.register_texture("textures/albedo", albedo_handle);
        dispatcher.register_normal_map("textures/normal", normal_handle);
        dispatcher.register_material(
            "materials/test",
            material_handle,
            shader_handle,
        );

        tx.send(CompileResult::Ok(CompileOk {
            logical_name: "materials/test".into(),
            kind: CompiledAssetKind::Material,
            payload: synthetic_material_payload(
                "textures/albedo",
                Some("textures/normal"),
                None,
                true,  // double_sided
                false, // alpha_cutout
            ),
            generation: Generation(1),
        }))
        .unwrap();

        let summary = dispatcher.dispatch(&mut assets);
        assert_eq!(summary.applied.len(), 1);
        assert!(summary.errors.is_empty());
        assert_eq!(summary.applied[0].kind, AssetKind::Material);

        // Identity preserved: same material_handle resolves.
        // Reconstruction correct: albedo + normal handles match
        // the dependency registrations; flags reflect payload.
        let reloaded = assets.get(material_handle).expect("material present");
        assert_eq!(reloaded.albedo, albedo_handle);
        assert_eq!(reloaded.normal, Some(normal_handle));
        assert_eq!(reloaded.specular_gloss, None);
        assert!(reloaded.double_sided);
        assert!(!reloaded.alpha_cutout);
    }

    /// Material reload with a missing albedo dependency surfaces
    /// `MissingMaterialDependency` rather than panicking or
    /// silently resolving to a null handle. The previous Material
    /// stays live in the AssetStore.
    #[test]
    fn dispatch_surfaces_missing_albedo_dependency() {
        let mut assets = AssetStore::new();
        let placeholder_albedo = assets.insert(Texture {
            width: 1,
            height: 1,
            format: TextureFormat::Rgba8,
            color_space: ColorSpace::Srgb,
            data: vec![0, 0, 0, 255],
        });
        let shader = insert_test_shader(&mut assets);
        let material_handle = assets.insert(Material::new(shader, placeholder_albedo));
        // Pre-reload material content for the assertion below.
        let pre_albedo = assets.get(material_handle).unwrap().albedo;

        let (tx, rx) = channel::<CompileResult>();
        let mut dispatcher = ReloadDispatcher::new(rx);
        // Deliberately skip register_texture for the dependency
        // the payload will reference. The reload should fail.
        dispatcher.register_material(
            "materials/test",
            material_handle,
            shader,
        );

        tx.send(CompileResult::Ok(CompileOk {
            logical_name: "materials/test".into(),
            kind: CompiledAssetKind::Material,
            payload: synthetic_material_payload(
                "textures/unregistered_albedo",
                None,
                None,
                false,
                false,
            ),
            generation: Generation(1),
        }))
        .unwrap();

        let summary = dispatcher.dispatch(&mut assets);
        assert!(summary.applied.is_empty(), "no apply when dep missing");
        assert_eq!(summary.errors.len(), 1, "exactly one error surfaced");
        let err = &summary.errors[0];
        assert_eq!(err.logical_name, "materials/test");
        assert!(
            err.error.contains("albedo"),
            "error names the missing slot: {}",
            err.error
        );
        assert!(
            err.error.contains("unregistered_albedo"),
            "error names the missing dependency: {}",
            err.error
        );

        // Previous Material still live, untouched.
        let still_there = assets.get(material_handle).expect("material preserved");
        assert_eq!(still_there.albedo, pre_albedo);
    }

    /// Material reload with a missing normal-map dependency
    /// surfaces the same typed error with `slot = "normal"`.
    /// Pinning per-slot error specificity so the editor pane can
    /// tell the developer exactly which dependency is missing.
    #[test]
    fn dispatch_surfaces_missing_normal_map_dependency() {
        let mut assets = AssetStore::new();
        let albedo_handle = assets.insert(Texture {
            width: 1,
            height: 1,
            format: TextureFormat::Rgba8,
            color_space: ColorSpace::Srgb,
            data: vec![0, 0, 0, 255],
        });
        let shader = insert_test_shader(&mut assets);
        let material_handle = assets.insert(Material::new(shader, albedo_handle));

        let (tx, rx) = channel::<CompileResult>();
        let mut dispatcher = ReloadDispatcher::new(rx);
        dispatcher.register_texture("textures/albedo", albedo_handle);
        // Skip normal-map registration.
        dispatcher.register_material(
            "materials/test",
            material_handle,
            shader,
        );

        tx.send(CompileResult::Ok(CompileOk {
            logical_name: "materials/test".into(),
            kind: CompiledAssetKind::Material,
            payload: synthetic_material_payload(
                "textures/albedo",
                Some("textures/missing_normal"),
                None,
                false,
                false,
            ),
            generation: Generation(1),
        }))
        .unwrap();

        let summary = dispatcher.dispatch(&mut assets);
        assert_eq!(summary.errors.len(), 1);
        let err = &summary.errors[0];
        assert!(
            err.error.contains("normal"),
            "error names the normal slot: {}",
            err.error
        );
        assert!(err.error.contains("missing_normal"));
    }

    /// Material reload preserves identity: the original
    /// MaterialHandle continues to resolve after replace, with
    /// the same generation. Pinning the property at the
    /// dispatcher level (the AssetStore::replace test pins it at
    /// the storage level; this is the integration version).
    #[test]
    fn material_reload_preserves_handle_identity() {
        let mut assets = AssetStore::new();
        let albedo = assets.insert(Texture {
            width: 1,
            height: 1,
            format: TextureFormat::Rgba8,
            color_space: ColorSpace::Srgb,
            data: vec![5, 5, 5, 255],
        });
        let shader = insert_test_shader(&mut assets);
        let material_handle = assets.insert(Material::new(shader, albedo));
        let original_index = material_handle.index;
        let original_generation = material_handle.generation;

        let (tx, rx) = channel::<CompileResult>();
        let mut dispatcher = ReloadDispatcher::new(rx);
        dispatcher.register_texture("textures/albedo", albedo);
        dispatcher.register_material("materials/test", material_handle, shader);

        // Send three reloads back-to-back; identity must hold
        // through every one.
        for _ in 0..3 {
            tx.send(CompileResult::Ok(CompileOk {
                logical_name: "materials/test".into(),
                kind: CompiledAssetKind::Material,
                payload: synthetic_material_payload(
                    "textures/albedo",
                    None,
                    None,
                    false,
                    false,
                ),
                generation: Generation(1),
            }))
            .unwrap();
            let _ = dispatcher.dispatch(&mut assets);
        }

        // Original handle still resolves; index + generation
        // unchanged.
        assert_eq!(material_handle.index, original_index);
        assert_eq!(material_handle.generation, original_generation);
        assert!(assets.get(material_handle).is_some());
    }

    // ── Shader reload tests ───────────────────────────────────────────────

    /// Build a synthetic compiled-shader payload for tests so the
    /// dispatcher's decode arm has something realistic to chew on.
    fn synthetic_shader_payload(
        name: &str,
        vertex: &str,
        fragment: &str,
    ) -> Vec<u8> {
        let payload = crate::assets::CompiledShaderPayload {
            name: name.to_string(),
            vertex_source: vertex.to_string(),
            fragment_source: fragment.to_string(),
        };
        crate::assets::encode_compiled_shader_payload(&payload).unwrap()
    }

    /// Shader reload happy path: register a Shader handle, send a
    /// Shader CompileResult, verify the dispatcher decoded the
    /// payload and replaced under the handle. The reloaded shader
    /// has the expected vertex/fragment sources; identity holds.
    #[test]
    fn dispatch_applies_shader_replace_and_emits_event() {
        let mut assets = AssetStore::new();
        let initial = crate::assets::Shader {
            name: "shaders/test".into(),
            vertex_source: "// initial vertex".into(),
            fragment_source: "// initial fragment".into(),
        };
        let handle = assets.insert(initial);

        let (tx, rx) = channel::<CompileResult>();
        let mut dispatcher = ReloadDispatcher::new(rx);
        dispatcher.register_shader("shaders/test", handle);

        tx.send(CompileResult::Ok(CompileOk {
            logical_name: "shaders/test".into(),
            kind: CompiledAssetKind::Shader,
            payload: synthetic_shader_payload(
                "shaders/test",
                "// reloaded vertex source\nvoid main() {}\n",
                "// reloaded fragment source\nvoid main() {}\n",
            ),
            generation: Generation(1),
        }))
        .unwrap();

        let summary = dispatcher.dispatch(&mut assets);
        assert_eq!(summary.applied.len(), 1);
        assert!(summary.errors.is_empty());
        assert_eq!(summary.applied[0].kind, AssetKind::Shader);

        // Identity preserved + content updated. The "reloaded
        // vertex source" string didn't appear in the initial
        // shader.
        let after = assets.get(handle).unwrap();
        assert!(after.vertex_source.contains("reloaded vertex source"));
        assert!(after.fragment_source.contains("reloaded fragment source"));
        assert_eq!(after.name, "shaders/test");
    }

    /// Materials referencing a reloaded shader continue to
    /// resolve through the same `ShaderHandle` — identity
    /// preservation does the work, no Material reload required.
    /// This is the load-bearing property of M7.4.1's shader
    /// reload: a shader edit doesn't ripple into every dependent
    /// material.
    #[test]
    fn materials_referencing_reloaded_shader_keep_resolving() {
        let mut assets = AssetStore::new();
        let initial = crate::assets::Shader {
            name: "shaders/lit".into(),
            vertex_source: "// v1".into(),
            fragment_source: "// v1".into(),
        };
        let shader_handle = assets.insert(initial);
        let albedo = assets.insert(Texture {
            width: 1,
            height: 1,
            format: TextureFormat::Rgba8,
            color_space: ColorSpace::Srgb,
            data: vec![0, 0, 0, 255],
        });
        // A material that captures the shader handle.
        let material_handle = assets.insert(Material::new(shader_handle, albedo));

        let (tx, rx) = channel::<CompileResult>();
        let mut dispatcher = ReloadDispatcher::new(rx);
        dispatcher.register_shader("shaders/lit", shader_handle);

        // Hot-reload the shader.
        tx.send(CompileResult::Ok(CompileOk {
            logical_name: "shaders/lit".into(),
            kind: CompiledAssetKind::Shader,
            payload: synthetic_shader_payload(
                "shaders/lit",
                "// v2 vertex\n",
                "// v2 fragment\n",
            ),
            generation: Generation(1),
        }))
        .unwrap();
        let _ = dispatcher.dispatch(&mut assets);

        // Material still resolves; its shader handle still
        // resolves; the resolved shader is now the v2 source.
        let mat = assets.get(material_handle).expect("material live");
        let shader = assets.get(mat.shader).expect("shader live");
        assert!(shader.vertex_source.contains("v2 vertex"));
        assert!(shader.fragment_source.contains("v2 fragment"));
    }

    /// Decode failure (a corrupt shader payload) surfaces as a
    /// `CompileErr` with context rather than panicking. The
    /// previous Shader stays live in the AssetStore.
    #[test]
    fn shader_reload_with_corrupt_payload_surfaces_decode_error() {
        let mut assets = AssetStore::new();
        let initial = crate::assets::Shader {
            name: "shaders/test".into(),
            vertex_source: "// keep me".into(),
            fragment_source: "// keep me".into(),
        };
        let handle = assets.insert(initial);

        let (tx, rx) = channel::<CompileResult>();
        let mut dispatcher = ReloadDispatcher::new(rx);
        dispatcher.register_shader("shaders/test", handle);

        // Garbage bytes — version field decodes to a value that
        // doesn't match COMPILED_PAYLOAD_VERSION.
        let mut corrupt = vec![0u8; 32];
        corrupt[0] = 0xFF;
        corrupt[1] = 0xFF;
        corrupt[2] = 0xFF;
        corrupt[3] = 0xFF;
        tx.send(CompileResult::Ok(CompileOk {
            logical_name: "shaders/test".into(),
            kind: CompiledAssetKind::Shader,
            payload: corrupt,
            generation: Generation(1),
        }))
        .unwrap();

        let summary = dispatcher.dispatch(&mut assets);
        assert!(summary.applied.is_empty());
        assert_eq!(summary.errors.len(), 1);

        // Previous Shader still live and unchanged.
        let still_there = assets.get(handle).expect("shader preserved");
        assert!(still_there.vertex_source.contains("keep me"));
    }

    // ── Scene reload tests ────────────────────────────────────────────────

    /// Build a synthetic compiled-scene payload by encoding a
    /// hand-built `SceneDocument` through the same pipeline the
    /// compile step uses. Mirrors `synthetic_shader_payload` so
    /// scene-reload tests share the spirit of the rest of the
    /// dispatcher's test surface.
    fn synthetic_scene_payload(name: &str, hscene_text: &str) -> Vec<u8> {
        let payload = crate::assets::CompiledScenePayload {
            name: name.to_string(),
            hscene_text: hscene_text.to_string(),
        };
        crate::assets::encode_compiled_scene_payload(&payload).unwrap()
    }

    /// Build a tiny `.hscene` document body by going SceneDocument →
    /// `to_hscene_string`. Returns the canonical text the dispatcher's
    /// decode path will round-trip through `from_hscene_str`. Using
    /// the real serializer (not a hand-typed string literal) keeps
    /// these tests aligned with the production canonical format.
    fn synthetic_hscene_document_text() -> String {
        let doc = crate::editor::SceneDocument {
            name: "test_scene".to_string(),
            runtime_slice: crate::runtime_slice::RuntimeSliceAuthoring::default(),
            entities: vec![crate::editor::EditorEntity {
                id: 1,
                name: "anchor".to_string(),
                components: vec![crate::editor::InspectorComponent {
                    type_name: "Transform".to_string(),
                    fields: vec![
                        crate::editor::InspectorField {
                            name: "position".to_string(),
                            value: crate::editor::InspectorValue::Vec3([1.0, 2.0, 3.0]),
                        },
                        crate::editor::InspectorField {
                            name: "rotation_euler_deg".to_string(),
                            value: crate::editor::InspectorValue::Vec3([0.0, 0.0, 0.0]),
                        },
                    ],
                }],
            }],
        };
        doc.to_hscene_string()
    }

    /// Scene reload happy path: register a scene under a logical name
    /// + source path, send a Scene CompileResult, verify the
    /// dispatcher decoded the payload and surfaced a SceneReloadResult
    /// in `summary.scenes_reloaded` (NOT in `summary.applied`, which
    /// is reserved for asset-pool reloads).
    #[test]
    fn dispatch_routes_scene_reloads_to_scenes_reloaded_field() {
        let mut assets = AssetStore::new();
        let (tx, rx) = channel::<CompileResult>();
        let mut dispatcher = ReloadDispatcher::new(rx);

        dispatcher.register_scene("levels/level01", "C:/projects/foo/levels/level01.hsc");
        let body = synthetic_hscene_document_text();

        tx.send(CompileResult::Ok(CompileOk {
            logical_name: "levels/level01".into(),
            kind: CompiledAssetKind::Scene,
            payload: synthetic_scene_payload("levels/level01", &body),
            generation: Generation(1),
        }))
        .unwrap();

        let summary = dispatcher.dispatch(&mut assets);
        assert!(summary.errors.is_empty(), "errors: {:?}", summary.errors);
        // Asset reloads MUST NOT include scene events — scenes ride
        // their own typed field.
        assert!(summary.applied.is_empty());
        assert_eq!(summary.scenes_reloaded.len(), 1);

        let result = &summary.scenes_reloaded[0];
        assert_eq!(result.logical_name, "levels/level01");
        assert_eq!(
            result.source_path.to_string_lossy(),
            "C:/projects/foo/levels/level01.hsc"
        );
        // The decoded SceneDocument carries the entity we authored.
        assert_eq!(result.document.entities.len(), 1);
        assert_eq!(result.document.entities[0].name, "anchor");
    }

    /// Scene reload with a corrupt payload: the previous scene is
    /// untouched (no scene state lives on the dispatcher itself —
    /// the editor's session keeps the live document — so this test
    /// just verifies the failure surfaces as a CompileErr rather
    /// than a panic).
    #[test]
    fn dispatch_surfaces_scene_decode_failure_as_compile_error() {
        let mut assets = AssetStore::new();
        let (tx, rx) = channel::<CompileResult>();
        let mut dispatcher = ReloadDispatcher::new(rx);
        dispatcher.register_scene("levels/broken", "C:/scenes/broken.hsc");

        let mut corrupt = synthetic_scene_payload("levels/broken", "this is not a scene");
        corrupt.truncate(corrupt.len() - 4); // chop the tail to break the cursor

        tx.send(CompileResult::Ok(CompileOk {
            logical_name: "levels/broken".into(),
            kind: CompiledAssetKind::Scene,
            payload: corrupt,
            generation: Generation(1),
        }))
        .unwrap();

        let summary = dispatcher.dispatch(&mut assets);
        assert!(summary.scenes_reloaded.is_empty());
        assert_eq!(summary.errors.len(), 1);
        assert_eq!(summary.errors[0].kind, CompiledAssetKind::Scene);
    }

    /// Scene reload without a registered binding produces an error
    /// rather than silently dropping the result. Mirrors the
    /// "unknown logical name" behavior for asset reloads — never
    /// silently ignore work the queue handed us.
    #[test]
    fn dispatch_surfaces_unregistered_scene_logical_name() {
        let mut assets = AssetStore::new();
        let (tx, rx) = channel::<CompileResult>();
        let mut dispatcher = ReloadDispatcher::new(rx);
        // Note: NO register_scene call.

        tx.send(CompileResult::Ok(CompileOk {
            logical_name: "levels/unregistered".into(),
            kind: CompiledAssetKind::Scene,
            payload: synthetic_scene_payload(
                "levels/unregistered",
                &synthetic_hscene_document_text(),
            ),
            generation: Generation(1),
        }))
        .unwrap();

        let summary = dispatcher.dispatch(&mut assets);
        assert!(summary.scenes_reloaded.is_empty());
        assert_eq!(summary.errors.len(), 1);
        assert!(summary.errors[0]
            .error
            .contains("no registered handle"));
    }

    /// Re-registering a scene under the same logical name reuses
    /// the existing path slot rather than leaking entries in the
    /// internal `scene_paths` vec. Editor workflow: a project
    /// re-link or "open scene…" with the same logical name swaps
    /// the source path without growing memory unboundedly.
    #[test]
    fn register_scene_reuses_existing_slot_on_reregistration() {
        let (_tx, rx) = channel::<CompileResult>();
        let mut dispatcher = ReloadDispatcher::new(rx);
        dispatcher.register_scene("levels/main", "C:/v1/main.hsc");
        dispatcher.register_scene("levels/main", "C:/v2/main.hsc");
        dispatcher.register_scene("levels/main", "C:/v3/main.hsc");
        // Single binding, single path slot.
        assert_eq!(dispatcher.binding_count(), 1);
        assert_eq!(dispatcher.scene_paths.len(), 1);
        assert_eq!(
            dispatcher.scene_paths[0].to_string_lossy(),
            "C:/v3/main.hsc"
        );
    }
}
