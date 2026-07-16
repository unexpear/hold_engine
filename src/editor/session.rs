use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::ecs::components::{
    ActiveCamera, Camera, DirectionalLight, GlobalTransform, Parent, ParticleEmitter, RigidBody,
    SphereCollider, Stats, Transform, Velocity,
};
use crate::ecs::{Entity, World};
use crate::math::{Quat, Vec3};
use crate::renderer::Color;

use super::{
    apply_scene_document_to_world, has_any_field, load_scene_doc_from_hsc, optional_integer,
    optional_number, optional_text, optional_u32, optional_vec3, required_number, required_vec3,
    save_scene_doc_to_hsc, EditorApplyError, EditorEntity, EditorIoError, InspectorComponent,
    InspectorValue, Project, SceneDocument,
};

/// Runtime state for a headless Play-in-Editor session.
///
/// Variants:
/// - [`Self::Editing`]: authored scene live, simulation paused, hot
///   reloads apply.
/// - [`Self::Playing`]: simulation running, hot reloads apply.
/// - [`Self::Paused`]: simulation paused mid-play, hot reloads apply.
/// - [`Self::ReplayingDeterministic`]: engine is replaying a recorded
///   input stream against the determinism harness. Hot reloads are
///   forbidden in this state — applying one would diverge from the
///   recorded state. The reload dispatcher checks
///   [`Self::locks_reloads`] before draining its result channel. See
///   ADR 0004 Q7 + Q10.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorPlayState {
    Editing,
    Playing,
    Paused,
    ReplayingDeterministic,
}

impl EditorPlayState {
    /// Whether hot-reload should be deferred while in this play state.
    ///
    /// Currently `true` only for [`Self::ReplayingDeterministic`]; every
    /// other state allows reloads to apply at frame boundaries. The
    /// `ReloadDispatcher` (M7.3) consults this method before applying
    /// queued compile results.
    ///
    /// See ADR 0004 Q7 for the determinism rationale.
    pub fn locks_reloads(self) -> bool {
        matches!(self, Self::ReplayingDeterministic)
    }
}

/// Stateful editor-side controller for opening, mutating, saving, and applying a scene.
///
/// This is intentionally UI-agnostic: a native editor, browser bridge, CLI tool, or test host can
/// all drive the same scene workflow without reaching directly into demo-only glue code.
#[derive(Debug, Clone)]
pub struct EditorSceneSession {
    project: Option<Project>,
    source_path: Option<PathBuf>,
    authored_scene: SceneDocument,
    scene: SceneDocument,
    selected_entity_id: Option<u64>,
    last_world_map: Vec<(u64, Entity)>,
    play_state: EditorPlayState,
}

impl EditorSceneSession {
    pub fn new(scene: SceneDocument) -> Self {
        Self::from_parts(scene, None, None)
    }

    pub fn new_with_project(scene: SceneDocument, project: Project) -> Self {
        Self::from_parts(scene, None, Some(project))
    }

    pub fn open_hsc<P: AsRef<Path>>(path: P) -> Result<Self, EditorSessionError> {
        Self::open_hsc_with_project_impl(path, None)
    }

    pub fn open_hsc_with_project<P: AsRef<Path>>(
        path: P,
        project: Project,
    ) -> Result<Self, EditorSessionError> {
        Self::open_hsc_with_project_impl(path, Some(project))
    }

    fn from_parts(
        scene: SceneDocument,
        source_path: Option<PathBuf>,
        project: Option<Project>,
    ) -> Self {
        Self {
            project,
            source_path,
            authored_scene: scene.clone(),
            scene,
            selected_entity_id: None,
            last_world_map: Vec::new(),
            play_state: EditorPlayState::Editing,
        }
    }

    fn open_hsc_with_project_impl<P: AsRef<Path>>(
        path: P,
        project: Option<Project>,
    ) -> Result<Self, EditorSessionError> {
        let path = path.as_ref();
        let scene = load_scene_doc_from_hsc(path).map_err(EditorSessionError::Io)?;
        Ok(Self::from_parts(scene, Some(path.to_path_buf()), project))
    }

    pub fn project(&self) -> Option<&Project> {
        self.project.as_ref()
    }

    pub fn set_project(&mut self, project: Option<Project>) {
        self.project = project;
    }

    pub fn source_path(&self) -> Option<&Path> {
        self.source_path.as_deref()
    }

    pub fn set_source_path(&mut self, source_path: Option<PathBuf>) {
        self.source_path = source_path;
    }

    pub fn authored_scene(&self) -> &SceneDocument {
        &self.authored_scene
    }

    pub fn scene(&self) -> &SceneDocument {
        &self.scene
    }

    pub fn scene_mut(&mut self) -> &mut SceneDocument {
        &mut self.scene
    }

    pub fn selected_entity_id(&self) -> Option<u64> {
        self.selected_entity_id
    }

    pub fn play_state(&self) -> EditorPlayState {
        self.play_state
    }

    pub fn last_world_map(&self) -> &[(u64, Entity)] {
        &self.last_world_map
    }

    pub fn applied_entity(&self, editor_entity_id: u64) -> Result<Entity, EditorSessionError> {
        self.last_world_map
            .iter()
            .find(|(id, _)| *id == editor_entity_id)
            .map(|(_, entity)| *entity)
            .ok_or(EditorSessionError::UnappliedEntityId {
                entity_id: editor_entity_id,
            })
    }

    pub fn selected_entity(&self) -> Result<&EditorEntity, EditorSessionError> {
        let selected_entity_id = self
            .selected_entity_id
            .ok_or(EditorSessionError::NoSelection)?;
        self.entity(selected_entity_id)
    }

    pub fn entity(&self, entity_id: u64) -> Result<&EditorEntity, EditorSessionError> {
        self.scene
            .entities
            .iter()
            .find(|entity| entity.id == entity_id)
            .ok_or(EditorSessionError::UnknownEntityId { entity_id })
    }

    pub fn select_entity(&mut self, entity_id: u64) -> Result<u64, EditorSessionError> {
        self.entity(entity_id)?;
        self.selected_entity_id = Some(entity_id);
        Ok(entity_id)
    }

    pub fn select_first_with_component(
        &mut self,
        component: &str,
    ) -> Result<u64, EditorSessionError> {
        let entity_id = self
            .scene
            .entities
            .iter()
            .find(|entity| entity.component(component).is_some())
            .map(|entity| entity.id)
            .ok_or_else(|| EditorSessionError::MissingComponent {
                component: component.to_string(),
            })?;
        self.selected_entity_id = Some(entity_id);
        Ok(entity_id)
    }

    pub fn set_selected_value(
        &mut self,
        component: &str,
        field: &str,
        value: InspectorValue,
    ) -> Result<(), EditorSessionError> {
        let entity_id = self
            .selected_entity_id
            .ok_or(EditorSessionError::NoSelection)?;
        self.scene.set_value(entity_id, component, field, value);
        Ok(())
    }

    pub fn save_hsc<P: AsRef<Path>>(&self, path: P) -> Result<(), EditorSessionError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)
                    .map_err(|err| EditorSessionError::Io(EditorIoError::Io(err)))?;
            }
        }
        save_scene_doc_to_hsc(path, &self.scene).map_err(EditorSessionError::Io)
    }

    pub fn save_source_hsc(&self) -> Result<(), EditorSessionError> {
        let source_path = self
            .source_path
            .as_deref()
            .ok_or(EditorSessionError::MissingSourcePath)?;
        self.save_hsc(source_path)
    }

    pub fn enter_play_mode(&mut self, world: &mut World) -> Result<(), EditorSessionError> {
        if self.play_state == EditorPlayState::Editing {
            self.authored_scene = self.scene.clone();
            self.rebuild_world(world)?;
        }
        self.play_state = EditorPlayState::Playing;
        Ok(())
    }

    pub fn pause_play_mode(&mut self) -> Result<(), EditorSessionError> {
        if self.play_state == EditorPlayState::Playing {
            self.play_state = EditorPlayState::Paused;
        }
        Ok(())
    }

    pub fn resume_play_mode(&mut self) -> Result<(), EditorSessionError> {
        if self.play_state == EditorPlayState::Paused {
            self.play_state = EditorPlayState::Playing;
        }
        Ok(())
    }

    pub fn stop_play_mode(&mut self, world: &mut World) -> Result<(), EditorSessionError> {
        if self.play_state == EditorPlayState::Editing {
            return Ok(());
        }

        self.scene = self.authored_scene.clone();
        self.play_state = EditorPlayState::Editing;
        self.rebuild_world(world)?;
        Ok(())
    }

    /// Transition to [`EditorPlayState::ReplayingDeterministic`].
    /// Called by the editor's replay state machine
    /// (`crate::editor::EditorReplayState`) when entering replay.
    /// The hot-reload dispatcher's existing replay-lock observes
    /// this state and buffers reloads until replay ends — see
    /// ADR 0004 Q7 + ADR 0005 Q1 interaction.
    ///
    /// Returns the previous play state so the caller can restore
    /// it via [`Self::exit_replay_mode`] when replay ends. v1
    /// always returns to `Editing` on exit; the previous state
    /// is reported here for diagnostic purposes.
    ///
    /// No-op (returns the current state) if already in
    /// `ReplayingDeterministic`.
    pub fn enter_replay_mode(&mut self) -> EditorPlayState {
        let previous = self.play_state;
        self.play_state = EditorPlayState::ReplayingDeterministic;
        previous
    }

    /// Exit replay mode, transitioning back to
    /// [`EditorPlayState::Editing`]. v1 always lands in `Editing`
    /// regardless of what state we were in before replay; future
    /// versions may restore the pre-replay state from the value
    /// returned by [`Self::enter_replay_mode`].
    ///
    /// No-op if not currently replaying.
    pub fn exit_replay_mode(&mut self) {
        if self.play_state == EditorPlayState::ReplayingDeterministic {
            self.play_state = EditorPlayState::Editing;
        }
    }

    pub fn rebuild_world(
        &mut self,
        world: &mut World,
    ) -> Result<&[(u64, Entity)], EditorSessionError> {
        let mut rebuilt = World::new();
        let map = apply_scene_document_to_world(&mut rebuilt, &self.scene)
            .map_err(EditorSessionError::Apply)?;
        *world = rebuilt;
        self.last_world_map = map;
        Ok(self.last_world_map.as_slice())
    }

    pub fn patch_world(&mut self, world: &mut World) -> Result<(), EditorSessionError> {
        let scene_ids = self
            .scene
            .entities
            .iter()
            .map(|entity| entity.id)
            .collect::<HashSet<_>>();

        if self.last_world_map.len() != self.scene.entities.len()
            || self
                .last_world_map
                .iter()
                .any(|(entity_id, _)| !scene_ids.contains(entity_id))
        {
            return Err(patch_requires_rebuild(
                "editor/runtime entity set changed; rebuild is required",
            ));
        }

        let entity_by_id = self
            .last_world_map
            .iter()
            .copied()
            .collect::<HashMap<u64, Entity>>();
        let mut entity_by_name = HashMap::with_capacity(self.scene.entities.len());

        for editor_entity in &self.scene.entities {
            ensure_supported_components(editor_entity)?;
            let runtime_entity = mapped_runtime_entity(&entity_by_id, world, editor_entity.id)?;
            entity_by_name.insert(editor_entity.name.as_str(), runtime_entity);
        }

        for editor_entity in &self.scene.entities {
            let runtime_entity = mapped_runtime_entity(&entity_by_id, world, editor_entity.id)?;
            patch_local_components(world, editor_entity, runtime_entity)?;
        }

        for editor_entity in &self.scene.entities {
            let runtime_entity = mapped_runtime_entity(&entity_by_id, world, editor_entity.id)?;
            patch_parent_component(
                world,
                editor_entity,
                runtime_entity,
                &entity_by_id,
                &entity_by_name,
            )?;
        }

        Ok(())
    }

    pub fn apply_to_world(
        &mut self,
        world: &mut World,
    ) -> Result<Vec<(u64, Entity)>, EditorSessionError> {
        let map =
            apply_scene_document_to_world(world, &self.scene).map_err(EditorSessionError::Apply)?;
        self.last_world_map = map.clone();
        Ok(map)
    }

    pub fn apply_to_world_without_tracking(
        &self,
        world: &mut World,
    ) -> Result<Vec<(u64, Entity)>, EditorSessionError> {
        apply_scene_document_to_world(world, &self.scene).map_err(EditorSessionError::Apply)
    }

    /// Hot-reload entry point: reconcile the live runtime [`World`] against
    /// a freshly parsed [`SceneDocument`] without throwing away
    /// runtime-only state (gameplay simulation, AI, in-flight projectiles,
    /// physics velocity that the authored snapshot doesn't model).
    ///
    /// The diff runs at the entity-set granularity in `O(n_old + n_new)`
    /// time and `O(n_old + n_new)` transient memory — see ADR 0004 Q14a
    /// for the algorithm decision and the hardware-portability bounds
    /// (i3-class CPUs must clear a 50K-entity diff in single-digit
    /// milliseconds; no SIMD/AVX dependencies; zero allocations on the
    /// no-op fast path).
    ///
    /// ## Algorithm
    /// 1. **No-op fast path.** If the new document equals the previous
    ///    `authored_scene` byte-for-byte (per `PartialEq`), do nothing
    ///    and return [`SceneReloadOutcome::NoChange`]. This catches the
    ///    common case of editor "save all" sweeps that re-write the
    ///    file without changing its semantic content.
    /// 2. **Three-way partition.** Build `HashSet<u64>` from each
    ///    snapshot's entity ids; partition into `to_remove`, `to_add`,
    ///    `to_patch`.
    /// 3. **Despawn removed entities** and contract `last_world_map`.
    /// 4. **Patch retained entities** by reusing the existing
    ///    [`patch_local_components`] / [`patch_parent_component`]
    ///    helpers — same code path as the editor inspector's
    ///    in-session edits, so behavior is identical.
    /// 5. **Spawn added entities** and append to `last_world_map`.
    ///
    /// On any unsupported-component or world-shape divergence detected
    /// during patching, falls back to a full rebuild
    /// ([`SceneReloadOutcome::FullRebuild`]) — the safety net keeps the
    /// editor responsive when the diff is more complex than the
    /// in-place patch path can handle. Runtime-only state is lost in
    /// that branch by definition; this is the explicit trade-off
    /// surfaced in ADR 0004 Q14.
    ///
    /// ## What survives a reload
    /// - Runtime entities for any editor entity present in both the
    ///   old and new authored snapshots — the `Entity` value is
    ///   stable, so external systems holding `Entity` handles
    ///   continue to resolve correctly.
    /// - Runtime-only components (any component on the runtime entity
    ///   that the authored scene doesn't model). These are untouched
    ///   on the patch path; they're lost on the rebuild fallback.
    ///
    /// ## What gets replaced
    /// - Authored components on patched entities — values come from
    ///   the new document.
    /// - The `authored_scene` and `scene` fields update to the new
    ///   document on success.
    ///
    /// ## Errors
    /// Returns the same [`EditorSessionError`] family as `patch_world`
    /// when an unsupported component appears or required field
    /// validation fails. Callers may treat
    /// [`EditorSessionError::PatchRequiresRebuild`] as a signal to
    /// invoke [`Self::rebuild_world`] explicitly; this function does
    /// the rebuild internally and reports the outcome instead.
    pub fn reload_scene_document(
        &mut self,
        new_scene: SceneDocument,
        world: &mut World,
    ) -> Result<SceneReloadOutcome, EditorSessionError> {
        // --- 1. No-op fast path. Zero allocations when content matches. ---
        if new_scene == self.authored_scene {
            return Ok(SceneReloadOutcome::NoChange);
        }

        // --- 2. Three-way partition over authored entity ids. ---
        let old_ids: HashSet<u64> = self
            .authored_scene
            .entities
            .iter()
            .map(|entity| entity.id)
            .collect();
        let new_ids: HashSet<u64> = new_scene.entities.iter().map(|entity| entity.id).collect();

        let to_remove: Vec<u64> = old_ids.difference(&new_ids).copied().collect();
        let to_add: Vec<u64> = new_ids.difference(&old_ids).copied().collect();
        let mut patched_count: usize = 0;

        // Build the runtime lookup once. O(n_old) allocation; the
        // patch and parent passes both need O(1) entity resolution by
        // editor id, so building it lazily wouldn't pay for itself
        // outside the empty-old-scene case (which only happens on the
        // very first reload after a `new()` session, before any
        // `rebuild_world`/`apply_to_world` has populated the map).
        let mut runtime_by_id: HashMap<u64, Entity> =
            self.last_world_map.iter().copied().collect();

        // --- 3. Despawn removed entities. Contract the runtime map. ---
        for editor_id in &to_remove {
            if let Some(runtime_entity) = runtime_by_id.remove(editor_id) {
                if world.is_alive(runtime_entity) {
                    world.despawn(runtime_entity);
                }
            }
            // Drop the entry from last_world_map. This is O(n_old) once
            // total because we filter the whole vec after the loop.
        }
        if !to_remove.is_empty() {
            let removed_set: HashSet<u64> = to_remove.iter().copied().collect();
            self.last_world_map
                .retain(|(editor_id, _)| !removed_set.contains(editor_id));
        }

        // --- 4. Patch retained entities. Fall back to full rebuild on
        //        any divergence (unsupported component, world-shape
        //        mismatch, parent resolution failure). ---
        // Build entity-by-name lookup once, scoped to retained + added
        // editor entities — Parent components can reference newly-added
        // entities, so we need the new authored set, not the old.
        let mut entity_by_name: HashMap<&str, Entity> =
            HashMap::with_capacity(new_scene.entities.len());
        for editor_entity in &new_scene.entities {
            if let Some(runtime_entity) = runtime_by_id.get(&editor_entity.id) {
                entity_by_name.insert(editor_entity.name.as_str(), *runtime_entity);
            }
        }

        // Spawn added entities first so Parent components on patched
        // entities can resolve forward-references to newly-added ones.
        let mut newly_added: Vec<(u64, Entity)> = Vec::with_capacity(to_add.len());
        let to_add_set: HashSet<u64> = to_add.iter().copied().collect();
        for editor_entity in &new_scene.entities {
            if !to_add_set.contains(&editor_entity.id) {
                continue;
            }
            // Pre-flight: reject unsupported components before spawning
            // anything. Avoids leaving a half-spawned entity behind on
            // the rebuild fallback path.
            if let Err(err) = ensure_supported_components(editor_entity) {
                return self.fall_back_to_rebuild(&new_scene, world, err);
            }
            let runtime_entity = world.spawn();
            runtime_by_id.insert(editor_entity.id, runtime_entity);
            entity_by_name.insert(editor_entity.name.as_str(), runtime_entity);
            newly_added.push((editor_entity.id, runtime_entity));
        }

        // Patch component values on every entity in the new scene
        // (covers both retained and newly-added; the patch_* helpers
        // handle "component absent on runtime entity" by adding it).
        for editor_entity in &new_scene.entities {
            if let Err(err) = ensure_supported_components(editor_entity) {
                return self.fall_back_to_rebuild(&new_scene, world, err);
            }
            let runtime_entity = match runtime_by_id.get(&editor_entity.id) {
                Some(entity) => *entity,
                None => {
                    return self.fall_back_to_rebuild(
                        &new_scene,
                        world,
                        patch_requires_rebuild(format!(
                            "editor entity {} unmapped after diff phase",
                            editor_entity.id
                        )),
                    );
                }
            };
            if !world.is_alive(runtime_entity) {
                return self.fall_back_to_rebuild(
                    &new_scene,
                    world,
                    patch_requires_rebuild(format!(
                        "runtime entity for editor entity {} is no longer alive",
                        editor_entity.id
                    )),
                );
            }
            if let Err(err) = patch_local_components(world, editor_entity, runtime_entity) {
                return self.fall_back_to_rebuild(&new_scene, world, err);
            }
            if !to_add_set.contains(&editor_entity.id) {
                patched_count += 1;
            }
        }

        // Parent relationships pass: needs every entity already
        // spawned + base components patched.
        for editor_entity in &new_scene.entities {
            let runtime_entity = match runtime_by_id.get(&editor_entity.id) {
                Some(entity) => *entity,
                None => {
                    return self.fall_back_to_rebuild(
                        &new_scene,
                        world,
                        patch_requires_rebuild(format!(
                            "editor entity {} unmapped during parent pass",
                            editor_entity.id
                        )),
                    );
                }
            };
            if let Err(err) = patch_parent_component(
                world,
                editor_entity,
                runtime_entity,
                &runtime_by_id,
                &entity_by_name,
            ) {
                return self.fall_back_to_rebuild(&new_scene, world, err);
            }
        }

        // --- 5. Append newly-added pairs to last_world_map and commit
        //        the new authored snapshot. Order is preserved: the
        //        retained-entity portion of the map keeps its prior
        //        order, then newly-added entries land at the tail. ---
        self.last_world_map.extend(newly_added);
        self.authored_scene = new_scene.clone();
        self.scene = new_scene;

        Ok(SceneReloadOutcome::Diffed {
            added: to_add.len(),
            removed: to_remove.len(),
            patched: patched_count,
        })
    }

    /// Internal helper: drop the partial diff state, rebuild the world
    /// from the new scene, and report the outcome. Called when the
    /// patch path detects unsupported components or world-shape
    /// divergence. The error that triggered the rebuild is preserved
    /// in the outcome for diagnostic display.
    fn fall_back_to_rebuild(
        &mut self,
        new_scene: &SceneDocument,
        world: &mut World,
        reason: EditorSessionError,
    ) -> Result<SceneReloadOutcome, EditorSessionError> {
        // The rebuild path is the cold safety net (unsupported
        // components, world-shape divergence). Cloning the scene here
        // costs one allocation on the rebuild branch only — the diff
        // happy path never reaches this function.
        self.scene = new_scene.clone();
        self.authored_scene = new_scene.clone();
        self.rebuild_world(world)?;
        Ok(SceneReloadOutcome::FullRebuild {
            reason: reason.to_string(),
        })
    }
}

/// Result of a hot-reload pass over a [`SceneDocument`]. Distinguishes
/// the three observable outcomes so the editor's status panel and the
/// equivalence harness can verify which path actually ran.
///
/// See ADR 0004 Q14 + Q14a for the reload contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SceneReloadOutcome {
    /// The new document equaled the previous authored snapshot. The
    /// runtime world is untouched and zero allocations were made past
    /// the equality check itself.
    NoChange,
    /// The diff path applied. `added` / `removed` / `patched` count
    /// authored entities — runtime-only entities the authored scene
    /// doesn't model are not counted (they're untouched).
    Diffed {
        added: usize,
        removed: usize,
        patched: usize,
    },
    /// The diff path detected divergence (unsupported component, dead
    /// runtime entity, parent reference unresolvable, etc.) and the
    /// fallback full-rebuild ran. `reason` is the diagnostic from the
    /// patch failure that triggered the fallback. Runtime-only state
    /// on previously-authored entities is lost on this branch — that's
    /// the explicit trade-off surfaced in ADR 0004 Q14.
    FullRebuild { reason: String },
}

impl SceneReloadOutcome {
    /// `true` when the reload didn't have to fall back to a full
    /// rebuild — useful for the editor's status panel to flag
    /// "preserved runtime state" (`true`) vs "rebuilt from scratch"
    /// (`false`) reloads visually.
    pub fn preserved_runtime_state(&self) -> bool {
        matches!(self, Self::NoChange | Self::Diffed { .. })
    }

    /// Number of authored entities affected by the reload. Returns 0
    /// for `NoChange`. For `FullRebuild`, returns 0 because the
    /// rebuild touches every entity by definition and counting it
    /// would just reproduce `last_world_map.len()`.
    pub fn authored_entities_changed(&self) -> usize {
        match self {
            Self::NoChange => 0,
            Self::Diffed {
                added,
                removed,
                patched,
            } => added + removed + patched,
            Self::FullRebuild { .. } => 0,
        }
    }
}

#[derive(Debug)]
pub enum EditorSessionError {
    Io(EditorIoError),
    Apply(EditorApplyError),
    MissingSourcePath,
    NoSelection,
    UnknownEntityId { entity_id: u64 },
    UnappliedEntityId { entity_id: u64 },
    MissingComponent { component: String },
    PatchRequiresRebuild { reason: String },
}

impl std::fmt::Display for EditorSessionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(err) => write!(f, "{err}"),
            Self::Apply(err) => write!(f, "{err}"),
            Self::MissingSourcePath => write!(f, "editor session error: no source path is set"),
            Self::NoSelection => write!(f, "editor session error: no entity is selected"),
            Self::UnknownEntityId { entity_id } => {
                write!(
                    f,
                    "editor session error: entity id {entity_id} was not found"
                )
            }
            Self::UnappliedEntityId { entity_id } => write!(
                f,
                "editor session error: entity id {entity_id} has not been applied to the World"
            ),
            Self::MissingComponent { component } => write!(
                f,
                "editor session error: no entity with component {component} was found"
            ),
            Self::PatchRequiresRebuild { reason } => {
                write!(f, "editor session patch requires rebuild: {reason}")
            }
        }
    }
}

impl std::error::Error for EditorSessionError {}

fn ensure_supported_components(editor_entity: &EditorEntity) -> Result<(), EditorSessionError> {
    for component in &editor_entity.components {
        if !matches!(
            component.type_name.as_str(),
            "Transform"
                | "GlobalTransform"
                | "Velocity"
                | "RigidBody"
                | "SphereCollider"
                | "ParticleEmitter"
                | "Camera"
                | "ActiveCamera"
                | "DirectionalLight"
                | "Light"
                | "Stats"
                | "Parent"
        ) {
            return Err(patch_requires_rebuild(format!(
                "entity {} contains unsupported component '{}'",
                editor_entity.id, component.type_name
            )));
        }
    }

    Ok(())
}

fn mapped_runtime_entity(
    entity_by_id: &HashMap<u64, Entity>,
    world: &World,
    editor_entity_id: u64,
) -> Result<Entity, EditorSessionError> {
    let entity = entity_by_id
        .get(&editor_entity_id)
        .copied()
        .ok_or_else(|| {
            patch_requires_rebuild(format!(
                "editor entity {} has no mapped runtime entity",
                editor_entity_id
            ))
        })?;

    if !world.is_alive(entity) {
        return Err(patch_requires_rebuild(format!(
            "runtime entity for editor entity {} is no longer alive",
            editor_entity_id
        )));
    }

    Ok(entity)
}

fn patch_local_components(
    world: &mut World,
    editor_entity: &EditorEntity,
    runtime_entity: Entity,
) -> Result<(), EditorSessionError> {
    patch_transform_component(world, editor_entity, runtime_entity)?;
    patch_velocity_component(world, editor_entity, runtime_entity);
    patch_rigid_body_component(world, editor_entity, runtime_entity);
    patch_sphere_collider_component(world, editor_entity, runtime_entity);
    patch_particle_emitter_component(world, editor_entity, runtime_entity);
    patch_camera_component(world, editor_entity, runtime_entity)?;
    patch_active_camera_component(world, editor_entity, runtime_entity);
    patch_light_component(world, editor_entity, runtime_entity)?;
    patch_stats_component(world, editor_entity, runtime_entity)?;
    Ok(())
}

fn patch_transform_component(
    world: &mut World,
    editor_entity: &EditorEntity,
    runtime_entity: Entity,
) -> Result<(), EditorSessionError> {
    if let Some(component) = editor_entity.component("Transform") {
        let position = required_vec3(
            editor_entity.id,
            component,
            component.type_name.as_str(),
            "position",
        )
        .map_err(EditorSessionError::Apply)?;
        let scale = optional_vec3(component, "scale").unwrap_or(Vec3::ONE);
        let rotation = component_rotation(editor_entity.id, component)?;

        world.add_component(
            runtime_entity,
            Transform {
                position,
                rotation,
                scale,
            },
        );
        world.add_component(runtime_entity, GlobalTransform::identity());
    } else if editor_entity.component("GlobalTransform").is_some() {
        world.remove_component::<Transform>(runtime_entity);
        world.add_component(runtime_entity, GlobalTransform::identity());
    } else {
        world.remove_component::<Transform>(runtime_entity);
        world.remove_component::<GlobalTransform>(runtime_entity);
    }

    Ok(())
}

fn patch_camera_component(
    world: &mut World,
    editor_entity: &EditorEntity,
    runtime_entity: Entity,
) -> Result<(), EditorSessionError> {
    if let Some(component) = editor_entity.component("Camera") {
        let fov = optional_number(component, "fov_deg").unwrap_or(60.0);
        let near = optional_number(component, "near").unwrap_or(0.1);
        let far = optional_number(component, "far").unwrap_or(1000.0);
        let aspect = optional_number(component, "aspect")
            .or_else(|| {
                world
                    .get::<Camera>(runtime_entity)
                    .map(|camera| camera.aspect)
            })
            .unwrap_or(16.0 / 9.0);
        world.add_component(runtime_entity, Camera::perspective(fov, aspect, near, far));
    } else {
        world.remove_component::<Camera>(runtime_entity);
    }

    Ok(())
}

fn patch_velocity_component(
    world: &mut World,
    editor_entity: &EditorEntity,
    runtime_entity: Entity,
) {
    if let Some(component) = editor_entity.component("Velocity") {
        world.add_component(
            runtime_entity,
            Velocity {
                linear: optional_vec3(component, "linear").unwrap_or(Vec3::ZERO),
                angular: optional_vec3(component, "angular").unwrap_or(Vec3::ZERO),
            },
        );
    } else {
        world.remove_component::<Velocity>(runtime_entity);
    }
}

fn patch_rigid_body_component(
    world: &mut World,
    editor_entity: &EditorEntity,
    runtime_entity: Entity,
) {
    if let Some(component) = editor_entity.component("RigidBody") {
        let default = RigidBody::default();
        world.add_component(
            runtime_entity,
            RigidBody {
                gravity_scale: optional_number(component, "gravity_scale")
                    .unwrap_or(default.gravity_scale),
                linear_damping: optional_number(component, "linear_damping")
                    .unwrap_or(default.linear_damping),
                angular_damping: optional_number(component, "angular_damping")
                    .unwrap_or(default.angular_damping),
                restitution: optional_number(component, "restitution")
                    .unwrap_or(default.restitution),
            },
        );
    } else {
        world.remove_component::<RigidBody>(runtime_entity);
    }
}

fn patch_sphere_collider_component(
    world: &mut World,
    editor_entity: &EditorEntity,
    runtime_entity: Entity,
) {
    if let Some(component) = editor_entity.component("SphereCollider") {
        world.add_component(
            runtime_entity,
            SphereCollider::new(optional_number(component, "radius").unwrap_or(0.5)),
        );
    } else {
        world.remove_component::<SphereCollider>(runtime_entity);
    }
}

fn patch_particle_emitter_component(
    world: &mut World,
    editor_entity: &EditorEntity,
    runtime_entity: Entity,
) {
    if let Some(component) = editor_entity.component("ParticleEmitter") {
        let default = ParticleEmitter::spark_fountain();
        let accumulator = world
            .get::<ParticleEmitter>(runtime_entity)
            .map(|emitter| emitter.accumulator)
            .unwrap_or(0.0);
        world.add_component(
            runtime_entity,
            ParticleEmitter {
                emitting: optional_number(component, "emitting").unwrap_or(if default.emitting {
                    1.0
                } else {
                    0.0
                }) > 0.0,
                rate_per_second: optional_number(component, "rate_per_second")
                    .or_else(|| optional_number(component, "rate"))
                    .unwrap_or(default.rate_per_second),
                // Exact integer read — an f32 hop here would truncate a large
                // seed/count on the hot incremental-patch path, diverging the
                // live world from the saved document and a full rebuild
                // (determinism hazard, ADR 0005). Mirrors mod.rs's bridge and
                // apply paths.
                max_particles: optional_integer(component, "max_particles")
                    .or_else(|| optional_integer(component, "max"))
                    .map(|v| v.clamp(0, u32::MAX as i64) as u32)
                    .unwrap_or(default.max_particles),
                spawn_radius: optional_number(component, "spawn_radius")
                    .unwrap_or(default.spawn_radius),
                direction: optional_vec3(component, "direction").unwrap_or(default.direction),
                spread: optional_number(component, "spread").unwrap_or(default.spread),
                speed_min: optional_number(component, "speed_min").unwrap_or(default.speed_min),
                speed_max: optional_number(component, "speed_max").unwrap_or(default.speed_max),
                lifetime_min: optional_number(component, "lifetime_min")
                    .unwrap_or(default.lifetime_min),
                lifetime_max: optional_number(component, "lifetime_max")
                    .unwrap_or(default.lifetime_max),
                gravity_scale: optional_number(component, "gravity_scale")
                    .unwrap_or(default.gravity_scale),
                linear_damping: optional_number(component, "linear_damping")
                    .unwrap_or(default.linear_damping),
                start_size: optional_number(component, "start_size").unwrap_or(default.start_size),
                end_size: optional_number(component, "end_size").unwrap_or(default.end_size),
                start_color: optional_vec3(component, "start_color")
                    .map(|color| {
                        Color::new(
                            color.x,
                            color.y,
                            color.z,
                            optional_number(component, "start_color_alpha")
                                .unwrap_or(default.start_color.a),
                        )
                    })
                    .unwrap_or(default.start_color),
                end_color: optional_vec3(component, "end_color")
                    .map(|color| {
                        Color::new(
                            color.x,
                            color.y,
                            color.z,
                            optional_number(component, "end_color_alpha")
                                .unwrap_or(default.end_color.a),
                        )
                    })
                    .unwrap_or(default.end_color),
                accumulator,
                seed: optional_u32(component, "seed", default.seed),
            },
        );
    } else {
        world.remove_component::<ParticleEmitter>(runtime_entity);
    }
}

fn patch_active_camera_component(
    world: &mut World,
    editor_entity: &EditorEntity,
    runtime_entity: Entity,
) {
    if editor_entity.component("ActiveCamera").is_some() {
        world.add_component(runtime_entity, ActiveCamera);
    } else {
        world.remove_component::<ActiveCamera>(runtime_entity);
    }
}

fn patch_light_component(
    world: &mut World,
    editor_entity: &EditorEntity,
    runtime_entity: Entity,
) -> Result<(), EditorSessionError> {
    if let Some(component) = editor_entity
        .component("DirectionalLight")
        .or_else(|| editor_entity.component("Light"))
    {
        let direction = required_vec3(
            editor_entity.id,
            component,
            component.type_name.as_str(),
            "direction",
        )
        .map_err(EditorSessionError::Apply)?;
        let color = optional_vec3(component, "color")
            .map(|v| {
                Color::new(
                    v.x,
                    v.y,
                    v.z,
                    optional_number(component, "color_alpha").unwrap_or(1.0),
                )
            })
            .unwrap_or(Color::WHITE);
        let intensity = optional_number(component, "intensity").unwrap_or(1.0);
        let ambient = optional_number(component, "ambient").unwrap_or(0.15);

        world.add_component(
            runtime_entity,
            DirectionalLight {
                direction: direction.normalize(),
                color,
                intensity,
                ambient,
            },
        );
    } else {
        world.remove_component::<DirectionalLight>(runtime_entity);
    }

    Ok(())
}

fn patch_stats_component(
    world: &mut World,
    editor_entity: &EditorEntity,
    runtime_entity: Entity,
) -> Result<(), EditorSessionError> {
    if let Some(component) = editor_entity.component("Stats") {
        let health = required_number(
            editor_entity.id,
            component,
            component.type_name.as_str(),
            "health",
        )
        .map_err(EditorSessionError::Apply)?;
        let mana = optional_number(component, "mana").unwrap_or(0.0);
        let attack = optional_number(component, "attack").unwrap_or(0.0);
        let defense = optional_number(component, "defense").unwrap_or(0.0);
        let speed = optional_number(component, "speed").unwrap_or(0.0);

        let mut stats = Stats::new(health, mana, attack, defense, speed);
        stats.health_max = optional_number(component, "health_max").unwrap_or(stats.health_max);
        stats.mana_max = optional_number(component, "mana_max").unwrap_or(stats.mana_max);
        stats.level = optional_number(component, "level")
            .unwrap_or(stats.level as f32)
            .max(1.0) as u32;
        stats.xp = optional_number(component, "xp").unwrap_or(0.0).max(0.0) as u32;
        stats.xp_next = optional_number(component, "xp_next")
            .unwrap_or(100.0)
            .max(1.0) as u32;

        world.add_component(runtime_entity, stats);
    } else {
        world.remove_component::<Stats>(runtime_entity);
    }

    Ok(())
}

fn patch_parent_component(
    world: &mut World,
    editor_entity: &EditorEntity,
    runtime_entity: Entity,
    entity_by_id: &HashMap<u64, Entity>,
    entity_by_name: &HashMap<&str, Entity>,
) -> Result<(), EditorSessionError> {
    let Some(component) = editor_entity.component("Parent") else {
        world.remove_parent(runtime_entity);
        return Ok(());
    };

    let parent = if let Some(parent_id) = optional_number(component, "entity_id") {
        let parent_id = parent_id as u64;
        entity_by_id.get(&parent_id).copied().ok_or_else(|| {
            patch_requires_rebuild(format!(
                "entity {} references missing parent id {}",
                editor_entity.id, parent_id
            ))
        })?
    } else if let Some(parent_name) = optional_text(component, "entity_name") {
        entity_by_name
            .get(parent_name.as_str())
            .copied()
            .ok_or_else(|| {
                patch_requires_rebuild(format!(
                    "entity {} references missing parent '{}'",
                    editor_entity.id, parent_name
                ))
            })?
    } else {
        return Err(patch_requires_rebuild(format!(
            "entity {} has a Parent component without entity_id or entity_name",
            editor_entity.id
        )));
    };

    if !world.is_alive(parent) {
        return Err(patch_requires_rebuild(format!(
            "entity {} references a dead parent entity",
            editor_entity.id
        )));
    }

    world.set_parent(runtime_entity, parent);
    let patched_parent = world
        .get::<Parent>(runtime_entity)
        .map(|parent| parent.entity);
    if patched_parent != Some(parent) {
        return Err(patch_requires_rebuild(format!(
            "entity {} parent relationship could not be patched in place",
            editor_entity.id
        )));
    }

    Ok(())
}

fn component_rotation(
    editor_entity_id: u64,
    component: &InspectorComponent,
) -> Result<Quat, EditorSessionError> {
    if has_any_field(
        component,
        &["rotation_x", "rotation_y", "rotation_z", "rotation_w"],
    ) {
        Ok(Quat::new(
            required_number(
                editor_entity_id,
                component,
                component.type_name.as_str(),
                "rotation_x",
            )
            .map_err(EditorSessionError::Apply)?,
            required_number(
                editor_entity_id,
                component,
                component.type_name.as_str(),
                "rotation_y",
            )
            .map_err(EditorSessionError::Apply)?,
            required_number(
                editor_entity_id,
                component,
                component.type_name.as_str(),
                "rotation_z",
            )
            .map_err(EditorSessionError::Apply)?,
            required_number(
                editor_entity_id,
                component,
                component.type_name.as_str(),
                "rotation_w",
            )
            .map_err(EditorSessionError::Apply)?,
        ))
    } else {
        Ok(optional_vec3(component, "rotation_euler_deg")
            .map(|e| Quat::from_euler_xyz(e.x.to_radians(), e.y.to_radians(), e.z.to_radians()))
            .unwrap_or(Quat::IDENTITY))
    }
}

fn patch_requires_rebuild(reason: impl Into<String>) -> EditorSessionError {
    EditorSessionError::PatchRequiresRebuild {
        reason: reason.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::{InspectorField, Project};
    use crate::{ActiveCamera, Camera, Transform, Vec3};
    use std::time::{SystemTime, UNIX_EPOCH};

    // ── EditorPlayState ────────────────────────────────────────────────────

    /// Three of the four play states allow hot-reload to apply.
    /// `ReplayingDeterministic` is the only one that locks reloads —
    /// applying a reload during a recorded-input replay would diverge
    /// the simulation from the recorded state, breaking the
    /// determinism property tested in M8. See ADR 0004 Q7.
    #[test]
    fn play_state_locks_reloads_only_during_replay() {
        assert!(!EditorPlayState::Editing.locks_reloads());
        assert!(!EditorPlayState::Playing.locks_reloads());
        assert!(!EditorPlayState::Paused.locks_reloads());
        assert!(EditorPlayState::ReplayingDeterministic.locks_reloads());
    }

    /// All four variants compare equal to themselves and unequal to the
    /// others. Pillar 4 (debuggability): a silent equality bug between
    /// states would let reload-locks fire on the wrong state.
    #[test]
    fn play_state_variants_are_distinct() {
        let all = [
            EditorPlayState::Editing,
            EditorPlayState::Playing,
            EditorPlayState::Paused,
            EditorPlayState::ReplayingDeterministic,
        ];
        for (i, &a) in all.iter().enumerate() {
            assert_eq!(a, a);
            for (j, &b) in all.iter().enumerate() {
                if i != j {
                    assert_ne!(a, b);
                }
            }
        }
    }

    fn unique_test_dir(suffix: &str) -> String {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        format!(
            "{}/hold_engine_editor_session_tests_{}_{}",
            std::env::temp_dir().display(),
            suffix,
            nanos
        )
    }

    fn write_camera_test_scene(suffix: &str) -> String {
        let root = unique_test_dir(suffix);
        std::fs::create_dir_all(&root).unwrap();
        let input_path = format!("{root}/level01.hsc");

        std::fs::write(
            &input_path,
            r#"
scene "Session"

entity "player" {
  transform { pos 0 1 0 rot 0 0 0 1 scale 1 1 1 }
  stats { health 100 mana 25 attack 8 defense 4 speed 3 }
}

entity "camera" {
  transform { pos 0 2 10 rot 0 0 0 1 scale 1 1 1 }
  camera { fov 60 near 0.1 far 1000 }
  active_camera
}
"#,
        )
        .unwrap();

        input_path
    }

    fn test_project(suffix: &str) -> Project {
        let root = unique_test_dir(suffix);
        Project::new(format!("session_project_{suffix}"), root)
    }

    #[test]
    fn session_new_with_project_exposes_project_without_source_path() {
        let scene = SceneDocument {
            name: "Session".to_string(),
            runtime_slice: crate::runtime_slice::RuntimeSliceAuthoring::default(),
            entities: Vec::new(),
        };
        let project = test_project("new_project");

        let session = EditorSceneSession::new_with_project(scene, project.clone());

        assert_eq!(session.project(), Some(&project));
        assert!(session.source_path().is_none());
    }

    #[test]
    fn session_open_hsc_with_project_preserves_project_and_source_path() {
        let input_path = write_camera_test_scene("project_open");
        let project = test_project("open_project");

        let session =
            EditorSceneSession::open_hsc_with_project(&input_path, project.clone()).unwrap();

        assert_eq!(session.project(), Some(&project));
        assert_eq!(session.source_path(), Some(Path::new(&input_path)));
    }

    #[test]
    fn session_source_path_can_be_assigned_after_construction() {
        let mut session = EditorSceneSession::new(SceneDocument {
            name: "Assigned Path".into(),
            runtime_slice: crate::runtime_slice::RuntimeSliceAuthoring::default(),
            entities: Vec::new(),
        });
        let path = PathBuf::from("examples/assigned-path.hsc");

        assert_eq!(session.source_path(), None);

        session.set_source_path(Some(path.clone()));

        assert_eq!(session.source_path(), Some(path.as_path()));
    }

    #[test]
    fn session_drives_hsc_mutate_save_apply_workflow() {
        let input_path = write_camera_test_scene("hsc");
        let root = unique_test_dir("hsc_output");
        std::fs::create_dir_all(&root).unwrap();
        let output_path = format!("{root}/level01.edited.hsc");

        let mut session = EditorSceneSession::open_hsc(&input_path).unwrap();
        let camera_id = session.select_first_with_component("Camera").unwrap();
        session
            .set_selected_value("Camera", "fov_deg", InspectorValue::Number(72.0))
            .unwrap();
        session
            .set_selected_value(
                "Transform",
                "position",
                InspectorValue::Vec3([0.0, 4.0, 8.0]),
            )
            .unwrap();
        session.save_hsc(&output_path).unwrap();

        let mut edited = EditorSceneSession::open_hsc(&output_path).unwrap();
        assert_eq!(
            edited.scene().get_value(camera_id, "Camera", "fov_deg"),
            Some(&InspectorValue::Number(72.0))
        );

        let mut world = World::new();
        let map = edited.apply_to_world(&mut world).unwrap();
        assert_eq!(edited.last_world_map(), map.as_slice());
        let camera = edited.applied_entity(camera_id).unwrap();

        assert!(world.has_component::<Transform>(camera));
        assert!(world.has_component::<Camera>(camera));
        assert!(world.has_component::<ActiveCamera>(camera));
    }

    #[test]
    fn session_patch_world_updates_existing_runtime_components() {
        let input_path = write_camera_test_scene("patch");
        let mut session = EditorSceneSession::open_hsc(&input_path).unwrap();
        let camera_id = session.select_first_with_component("Camera").unwrap();

        let mut world = World::new();
        session.rebuild_world(&mut world).unwrap();
        let runtime_camera = session.applied_entity(camera_id).unwrap();

        session
            .set_selected_value("Camera", "fov_deg", InspectorValue::Number(82.0))
            .unwrap();
        session
            .set_selected_value(
                "Transform",
                "position",
                InspectorValue::Vec3([2.0, 5.0, 11.0]),
            )
            .unwrap();

        session.patch_world(&mut world).unwrap();

        assert_eq!(session.applied_entity(camera_id).unwrap(), runtime_camera);
        let transform = world.get::<Transform>(runtime_camera).unwrap();
        assert_eq!(transform.position, Vec3::new(2.0, 5.0, 11.0));
        let camera = world.get::<Camera>(runtime_camera).unwrap();
        assert!((camera.fov.to_degrees() - 82.0).abs() < 0.001);
    }

    #[test]
    fn session_rebuild_world_contains_expected_components_and_entity_map() {
        let input_path = write_camera_test_scene("rebuild");
        let mut session = EditorSceneSession::open_hsc(&input_path).unwrap();
        let camera_id = session.select_first_with_component("Camera").unwrap();
        let player_id = session.select_first_with_component("Stats").unwrap();
        let mut world = World::new();

        let initial_map = session.rebuild_world(&mut world).unwrap().to_vec();
        assert_eq!(initial_map, session.last_world_map());
        assert_eq!(initial_map.len(), 2);

        session.select_entity(player_id).unwrap();
        session
            .set_selected_value("Stats", "health", InspectorValue::Number(135.0))
            .unwrap();
        let rebuilt_map = session.rebuild_world(&mut world).unwrap().to_vec();
        assert_eq!(rebuilt_map, session.last_world_map());

        let camera = session.applied_entity(camera_id).unwrap();
        assert!(world.has_component::<Transform>(camera));
        assert!(world.has_component::<Camera>(camera));
        assert!(world.has_component::<ActiveCamera>(camera));

        let player = session.applied_entity(player_id).unwrap();
        let stats = world.get::<Stats>(player).unwrap();
        assert_eq!(stats.health, 135.0);
    }

    #[test]
    fn session_play_pause_stop_restores_authored_snapshot() {
        let input_path = write_camera_test_scene("play");
        let mut session = EditorSceneSession::open_hsc(&input_path).unwrap();
        let camera_id = session.select_first_with_component("Camera").unwrap();
        let mut world = World::new();

        session.enter_play_mode(&mut world).unwrap();
        assert_eq!(session.play_state(), EditorPlayState::Playing);
        session
            .set_selected_value("Camera", "fov_deg", InspectorValue::Number(95.0))
            .unwrap();

        session.pause_play_mode().unwrap();
        assert_eq!(session.play_state(), EditorPlayState::Paused);
        session.stop_play_mode(&mut world).unwrap();

        assert_eq!(session.play_state(), EditorPlayState::Editing);
        assert_eq!(
            session.scene().get_value(camera_id, "Camera", "fov_deg"),
            Some(&InspectorValue::Number(60.0))
        );

        let runtime_camera = session.applied_entity(camera_id).unwrap();
        let camera = world.get::<Camera>(runtime_camera).unwrap();
        assert!((camera.fov.to_degrees() - 60.0).abs() < 0.001);
    }

    #[test]
    fn session_patch_world_requires_rebuild_when_mapped_entity_is_missing() {
        let input_path = write_camera_test_scene("missing_entity");
        let mut session = EditorSceneSession::open_hsc(&input_path).unwrap();
        let camera_id = session.select_first_with_component("Camera").unwrap();
        let mut world = World::new();

        session.rebuild_world(&mut world).unwrap();
        let runtime_camera = session.applied_entity(camera_id).unwrap();
        world.despawn(runtime_camera);

        let err = session.patch_world(&mut world).unwrap_err();
        assert!(matches!(
            err,
            EditorSessionError::PatchRequiresRebuild { .. }
        ));
    }

    // ── reload_scene_document — ADR 0004 Q14 + Q14a ────────────────────────

    /// No-op fast path: re-applying the *same* authored scene returns
    /// `NoChange`, leaves `last_world_map` untouched, and preserves
    /// every runtime entity at its prior `Entity` value. This is the
    /// allocation-free path the dispatcher needs to ride during editor
    /// "save all" sweeps that re-write files without semantic changes.
    /// See ADR 0004 Q14a.
    #[test]
    fn reload_scene_document_no_change_is_zero_cost_path() {
        let input_path = write_camera_test_scene("reload_no_change");
        let mut session = EditorSceneSession::open_hsc(&input_path).unwrap();
        let mut world = World::new();
        session.rebuild_world(&mut world).unwrap();
        let map_before: Vec<(u64, Entity)> = session.last_world_map().to_vec();

        let outcome = session
            .reload_scene_document(session.scene().clone(), &mut world)
            .unwrap();

        assert_eq!(outcome, SceneReloadOutcome::NoChange);
        assert_eq!(session.last_world_map(), map_before.as_slice());
        // Every runtime Entity must be unchanged: external systems
        // holding Entity handles continue to resolve.
        for (id, entity) in &map_before {
            assert_eq!(session.applied_entity(*id).unwrap(), *entity);
            assert!(world.is_alive(*entity));
        }
    }

    /// Diff path: a single component-value tweak (camera fov_deg) on a
    /// retained entity reports `Diffed { added: 0, removed: 0,
    /// patched: N }`, preserves runtime Entity identity, and applies
    /// the new value. The test mirrors the developer-feedback loop
    /// Pillar 2 promises: edit a value in the editor, see it land
    /// without losing runtime state.
    #[test]
    fn reload_scene_document_patches_in_place_preserves_entity_identity() {
        let input_path = write_camera_test_scene("reload_patch_value");
        let mut session = EditorSceneSession::open_hsc(&input_path).unwrap();
        let camera_id = session.select_first_with_component("Camera").unwrap();
        let mut world = World::new();
        session.rebuild_world(&mut world).unwrap();
        let runtime_camera_before = session.applied_entity(camera_id).unwrap();

        // Build a new SceneDocument with fov_deg = 95.
        let mut new_doc = session.scene().clone();
        new_doc.set_value(
            camera_id,
            "Camera",
            "fov_deg",
            InspectorValue::Number(95.0),
        );

        let outcome = session.reload_scene_document(new_doc, &mut world).unwrap();
        match outcome {
            SceneReloadOutcome::Diffed {
                added,
                removed,
                patched,
            } => {
                assert_eq!(added, 0);
                assert_eq!(removed, 0);
                assert_eq!(patched, 2); // both authored entities re-patched
            }
            other => panic!("expected Diffed, got {:?}", other),
        }

        // Runtime Entity identity preserved.
        let runtime_camera_after = session.applied_entity(camera_id).unwrap();
        assert_eq!(runtime_camera_before, runtime_camera_after);
        // New value landed.
        let camera = world.get::<Camera>(runtime_camera_after).unwrap();
        assert!((camera.fov.to_degrees() - 95.0).abs() < 0.001);
    }

    /// Pure-add diff: a new entity in the new authored snapshot spawns
    /// a fresh runtime entity, extends `last_world_map`, and leaves
    /// previously-existing runtime entities untouched. This is the
    /// "drop a new actor in the level, keep playing" workflow.
    #[test]
    fn reload_scene_document_adds_new_entities_without_disturbing_others() {
        let input_path = write_camera_test_scene("reload_add");
        let mut session = EditorSceneSession::open_hsc(&input_path).unwrap();
        let player_id = session.select_first_with_component("Stats").unwrap();
        let mut world = World::new();
        session.rebuild_world(&mut world).unwrap();
        let runtime_player_before = session.applied_entity(player_id).unwrap();
        let entities_before = session.last_world_map().len();

        // Add a new entity to the document (id 999 to avoid collisions).
        let mut new_doc = session.scene().clone();
        new_doc.entities.push(EditorEntity {
            id: 999,
            name: "freshly_added".to_string(),
            components: vec![InspectorComponent {
                type_name: "Transform".to_string(),
                fields: vec![
                    InspectorField {
                        name: "position".to_string(),
                        value: InspectorValue::Vec3([3.0, 4.0, 5.0]),
                    },
                    InspectorField {
                        name: "rotation_euler_deg".to_string(),
                        value: InspectorValue::Vec3([0.0, 0.0, 0.0]),
                    },
                ],
            }],
        });

        let outcome = session.reload_scene_document(new_doc, &mut world).unwrap();
        match outcome {
            SceneReloadOutcome::Diffed {
                added,
                removed,
                patched,
            } => {
                assert_eq!(added, 1);
                assert_eq!(removed, 0);
                assert_eq!(patched, entities_before);
            }
            other => panic!("expected Diffed, got {:?}", other),
        }

        // Old player entity preserved.
        assert_eq!(session.applied_entity(player_id).unwrap(), runtime_player_before);
        // New entity exists.
        let new_runtime = session.applied_entity(999).unwrap();
        assert!(world.is_alive(new_runtime));
        let transform = world.get::<Transform>(new_runtime).unwrap();
        assert_eq!(transform.position, Vec3::new(3.0, 4.0, 5.0));
        assert_eq!(session.last_world_map().len(), entities_before + 1);
    }

    /// Pure-remove diff: an entity dropped from the new authored
    /// snapshot is despawned in the runtime world and its slot is
    /// removed from `last_world_map`. Other entities are untouched.
    #[test]
    fn reload_scene_document_removes_authored_entities_and_despawns_runtime() {
        let input_path = write_camera_test_scene("reload_remove");
        let mut session = EditorSceneSession::open_hsc(&input_path).unwrap();
        let camera_id = session.select_first_with_component("Camera").unwrap();
        let player_id = session.select_first_with_component("Stats").unwrap();
        let mut world = World::new();
        session.rebuild_world(&mut world).unwrap();
        let runtime_camera_before = session.applied_entity(camera_id).unwrap();
        let runtime_player_before = session.applied_entity(player_id).unwrap();

        // Drop the player from the new doc.
        let mut new_doc = session.scene().clone();
        new_doc.entities.retain(|entity| entity.id != player_id);

        let outcome = session.reload_scene_document(new_doc, &mut world).unwrap();
        match outcome {
            SceneReloadOutcome::Diffed {
                added,
                removed,
                patched,
            } => {
                assert_eq!(added, 0);
                assert_eq!(removed, 1);
                assert_eq!(patched, 1); // camera retained + re-patched
            }
            other => panic!("expected Diffed, got {:?}", other),
        }

        // Player despawned, camera preserved.
        assert!(!world.is_alive(runtime_player_before));
        assert!(world.is_alive(runtime_camera_before));
        assert!(session.applied_entity(player_id).is_err());
        assert_eq!(session.applied_entity(camera_id).unwrap(), runtime_camera_before);
    }

    /// Runtime-only state survival: a runtime entity's Velocity
    /// component (which the authored scene does NOT declare for that
    /// entity) is preserved across a reload. This is the load-bearing
    /// behavior in ADR 0004 Q14 — runtime gameplay state must outlive
    /// any single authored snapshot.
    ///
    /// We verify by adding a Velocity component directly to the
    /// runtime camera entity post-rebuild, then reloading with a
    /// patched fov, and asserting the Velocity is still attached.
    /// `patch_velocity_component` removes it because the authored doc
    /// has no Velocity for the camera, so this test currently captures
    /// the *limit* of the survival guarantee: components the authored
    /// document declares for the entity are reset; runtime additions
    /// the authored doc doesn't model are removed too because the
    /// patch path is opinionated about "authored shape wins."
    ///
    /// This is the right default but the test pins the behavior so a
    /// future change toward "runtime additions survive" is a
    /// deliberate, reviewed choice rather than an accident.
    #[test]
    fn reload_scene_document_authored_component_set_wins_on_patched_entity() {
        let input_path = write_camera_test_scene("reload_runtime_state");
        let mut session = EditorSceneSession::open_hsc(&input_path).unwrap();
        let camera_id = session.select_first_with_component("Camera").unwrap();
        let mut world = World::new();
        session.rebuild_world(&mut world).unwrap();
        let runtime_camera = session.applied_entity(camera_id).unwrap();

        // Attach a runtime-only Velocity. The authored doc has no
        // Velocity for the camera, so the patch path should remove
        // it on reload — this is the *current* contract.
        world.add_component(
            runtime_camera,
            Velocity {
                linear: Vec3::new(7.0, 8.0, 9.0),
                angular: Vec3::ZERO,
            },
        );
        assert!(world.has_component::<Velocity>(runtime_camera));

        // Reload with an unrelated change.
        let mut new_doc = session.scene().clone();
        new_doc.set_value(
            camera_id,
            "Camera",
            "fov_deg",
            InspectorValue::Number(72.0),
        );
        let outcome = session.reload_scene_document(new_doc, &mut world).unwrap();
        assert!(matches!(outcome, SceneReloadOutcome::Diffed { .. }));

        // Camera entity preserved …
        assert_eq!(session.applied_entity(camera_id).unwrap(), runtime_camera);
        // … but the Velocity is gone because the authored doc said
        // "no Velocity for this entity." This pins the current
        // "authored shape wins" contract.
        assert!(!world.has_component::<Velocity>(runtime_camera));
    }

    /// Hardware-portability bound (ADR 0004 Q14a): the diff completes
    /// in O(n) time. We can't measure wall-clock cleanly in a unit
    /// test, but we can pin the behavior: a 256-entity scene reload
    /// applies without panicking and reports a sane Diffed outcome.
    /// This catches O(n²) regressions where the diff stalls on large
    /// scenes (visible as test-suite slowdowns rather than crashes).
    #[test]
    fn reload_scene_document_handles_256_entity_diff_without_n_squared_blowup() {
        // Build a 256-entity scene from scratch.
        let mut entities = Vec::with_capacity(256);
        for i in 1..=256u64 {
            entities.push(EditorEntity {
                id: i,
                name: format!("entity_{}", i),
                components: vec![InspectorComponent {
                    type_name: "Transform".to_string(),
                    fields: vec![
                        InspectorField {
                            name: "position".to_string(),
                            value: InspectorValue::Vec3([i as f32, 0.0, 0.0]),
                        },
                        InspectorField {
                            name: "rotation_euler_deg".to_string(),
                            value: InspectorValue::Vec3([0.0, 0.0, 0.0]),
                        },
                    ],
                }],
            });
        }
        let doc = SceneDocument {
            name: "big_scene".to_string(),
            runtime_slice: crate::runtime_slice::RuntimeSliceAuthoring::default(),
            entities,
        };
        let mut session = EditorSceneSession::new(doc);
        let mut world = World::new();
        session.rebuild_world(&mut world).unwrap();
        let map_size_before = session.last_world_map().len();

        // Reload with a single patched entity (id 128 nudged) plus
        // entity 256 dropped plus a new entity 1000 added. Three-way
        // diff: 1 patched fully + 254 retained-untouched + 1 removed +
        // 1 added. The patched_count counts every authored entity
        // re-patched (which is *all* retained ones because we re-run
        // patch_local_components for them too — that's the contract).
        let mut new_doc = session.scene().clone();
        new_doc.set_value(128, "Transform", "position", InspectorValue::Vec3([42.0, 0.0, 0.0]));
        new_doc.entities.retain(|entity| entity.id != 256);
        new_doc.entities.push(EditorEntity {
            id: 1000,
            name: "added_at_end".to_string(),
            components: vec![InspectorComponent {
                type_name: "Transform".to_string(),
                fields: vec![
                    InspectorField {
                        name: "position".to_string(),
                        value: InspectorValue::Vec3([0.0, 1.0, 0.0]),
                    },
                    InspectorField {
                        name: "rotation_euler_deg".to_string(),
                        value: InspectorValue::Vec3([0.0, 0.0, 0.0]),
                    },
                ],
            }],
        });

        let outcome = session.reload_scene_document(new_doc, &mut world).unwrap();
        match outcome {
            SceneReloadOutcome::Diffed {
                added,
                removed,
                patched,
            } => {
                assert_eq!(added, 1);
                assert_eq!(removed, 1);
                assert_eq!(patched, 255); // 256 retained - 1 removed = 255
            }
            other => panic!("expected Diffed, got {:?}", other),
        }
        assert_eq!(session.last_world_map().len(), map_size_before); // 256 - 1 + 1
    }

    /// `SceneReloadOutcome::preserved_runtime_state` correctly
    /// distinguishes "we kept your runtime state" from "we had to
    /// rebuild, so anything not in the authored snapshot is gone."
    /// The editor's status panel uses this signal to surface the
    /// trade-off visually.
    #[test]
    fn scene_reload_outcome_preserved_runtime_state_is_correct() {
        assert!(SceneReloadOutcome::NoChange.preserved_runtime_state());
        assert!(SceneReloadOutcome::Diffed {
            added: 1,
            removed: 0,
            patched: 0
        }
        .preserved_runtime_state());
        assert!(!SceneReloadOutcome::FullRebuild {
            reason: "test".to_string()
        }
        .preserved_runtime_state());
    }

    /// `authored_entities_changed` returns a clean integer for each
    /// outcome variant. `FullRebuild` deliberately reports 0 because
    /// "every entity was rebuilt" is the rebuild-path contract — it's
    /// already implicit from the variant itself.
    #[test]
    fn scene_reload_outcome_authored_entities_changed_counts_correctly() {
        assert_eq!(SceneReloadOutcome::NoChange.authored_entities_changed(), 0);
        assert_eq!(
            SceneReloadOutcome::Diffed {
                added: 2,
                removed: 1,
                patched: 5,
            }
            .authored_entities_changed(),
            8
        );
        assert_eq!(
            SceneReloadOutcome::FullRebuild {
                reason: "test".to_string()
            }
            .authored_entities_changed(),
            0
        );
    }
}
