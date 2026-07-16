//! Editor-side data model for projects, scenes, and inspector editing.
//! This module is runtime-agnostic and keeps a dynamic editing layer on top
//! of typed ECS/runtime data.

pub mod bridge;
pub mod browser;
pub mod command;
pub mod drawlist;
pub mod frame_inspector;
mod host;
pub mod hot_reload_status;
pub mod native_ui;
pub mod protocol;
pub mod replay;
mod session;
pub mod viewport;

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use crate::platform::atomic_write::{atomic_write, AtomicWriteError};

// ── `.hscene` document version + truncation terminator (M11) ─────────────────
//
// The editor's full-fidelity scene document. Its parser already rejects a
// truncation *inside* an entity or component ("unterminated entity"), but a cut
// exactly on an `endentity` boundary left every record well-formed — so the
// document parsed as a valid, silently *smaller* scene and the trailing
// entities disappeared without error.
//
// V2 adds a required terminator. V1 documents (everything written before M11)
// are still accepted without one, so existing files are not orphaned.

/// Legacy header. Accepted on read; never written. Parsed leniently — a V1
/// document has no terminator to check.
pub const HSCENE_DOC_HEADER_V1: &str = "HOLD_SCENE_DOC_V1";

/// Current header. A document declaring it MUST close with [`HSCENE_DOC_END`].
pub const HSCENE_DOC_HEADER_V2: &str = "HOLD_SCENE_DOC_V2";

/// Terminator closing a V2 document. Its absence means the file was cut short.
pub const HSCENE_DOC_END: &str = "endscenedoc";

// ── Autosave + crash recovery (M11) ──────────────────────────────────────────

/// Directory (relative to the scene's own directory) holding recovery files.
/// Hidden so it does not litter the author's scene folder, yet co-located so
/// recovery is a single path lookup rather than a keyed directory scan.
pub const RECOVERY_DIR: &str = ".hold/recovery";

/// Extension appended to a scene's filename to form its recovery file.
pub const RECOVERY_EXT: &str = "autosave";

/// Seconds of edit inactivity before an autosave fires. Debouncing means the
/// durable write (an `FlushFileBuffers`) lands when the user has already
/// paused — the least noticeable moment — rather than mid-keystroke.
pub const AUTOSAVE_IDLE_DEBOUNCE_SECS: f32 = 3.0;

/// Upper bound between autosaves while continuously editing. Without this
/// floor, an author who never pauses would never checkpoint.
pub const AUTOSAVE_MAX_INTERVAL_SECS: f32 = 60.0;

/// The recovery file for a scene at `scene_path`:
/// `<scene_dir>/.hold/recovery/<filename>.autosave`.
///
/// Returns `None` for a path with no filename. Untitled scenes (no path at
/// all) have no stable identity to recover into and are not autosaved.
pub fn recovery_path_for_scene(scene_path: &Path) -> Option<PathBuf> {
    let file_name = scene_path.file_name()?;
    let dir = scene_path.parent().unwrap_or_else(|| Path::new("."));
    let mut name = file_name.to_os_string();
    name.push(".");
    name.push(RECOVERY_EXT);
    Some(dir.join(RECOVERY_DIR).join(name))
}

/// Retry back-off after a failed autosave, so a persistently-failing write
/// (read-only recovery dir, a locked file) does not re-attempt a full
/// flush-through write every single frame.
pub const AUTOSAVE_RETRY_BACKOFF_SECS: f32 = 5.0;

/// Decides *when* an autosave should run. Pure timing logic — no filesystem,
/// no clock of its own — so the cadence is unit-testable.
///
/// It is driven by a **checkpoint watermark**: the last edit revision that was
/// durably checkpointed. Work is pending whenever the current revision differs
/// from that watermark. This is what lets an unsaved edit made just before
/// entering play mode still autosave after returning to Editing — a bool
/// "edited since save" flag could not, because play temporarily masks the
/// dirty state and would clear it.
///
/// Two firing rules, both required:
/// - **Idle debounce:** fire once the author has stopped editing for
///   [`AUTOSAVE_IDLE_DEBOUNCE_SECS`].
/// - **Interval floor:** fire regardless after [`AUTOSAVE_MAX_INTERVAL_SECS`]
///   of continuous editing, so a long unbroken session still checkpoints.
///
/// After a fire, the caller MUST report the outcome via [`Self::note_saved`]
/// (advances the watermark) or [`Self::note_failed`] (arms a back-off and
/// retries). Until one is called the scheduler will not fire again.
#[derive(Debug, Clone)]
pub struct AutosaveScheduler {
    idle_debounce_secs: f32,
    max_interval_secs: f32,
    retry_backoff_secs: f32,
    since_last_edit: f32,
    since_last_checkpoint: f32,
    /// The most recent edit revision observed.
    last_seen_revision: u64,
    /// The revision durably checkpointed. Work is pending iff this differs
    /// from `last_seen_revision`.
    checkpointed_revision: u64,
    /// Set true when a fire is outstanding (awaiting note_saved/note_failed),
    /// so tick does not re-fire every frame in the meantime.
    fire_outstanding: bool,
    /// Remaining back-off after a failed attempt.
    backoff_remaining: f32,
}

impl Default for AutosaveScheduler {
    fn default() -> Self {
        Self::new(
            AUTOSAVE_IDLE_DEBOUNCE_SECS,
            AUTOSAVE_MAX_INTERVAL_SECS,
            AUTOSAVE_RETRY_BACKOFF_SECS,
        )
    }
}

impl AutosaveScheduler {
    pub fn new(idle_debounce_secs: f32, max_interval_secs: f32, retry_backoff_secs: f32) -> Self {
        Self {
            idle_debounce_secs,
            max_interval_secs,
            retry_backoff_secs,
            since_last_edit: 0.0,
            since_last_checkpoint: 0.0,
            last_seen_revision: 0,
            checkpointed_revision: 0,
            fire_outstanding: false,
            backoff_remaining: 0.0,
        }
    }

    /// Advance by `dt`.
    ///
    /// - `has_unsaved_work`: the host's raw persistent-dirty flag — true
    ///   whenever there is unsaved *authored* work, INCLUDING during play (it
    ///   is not masked by play state). When false the scene is saved, so the
    ///   watermark is advanced to the current revision.
    /// - `can_autosave`: whether the editor is in a state where checkpointing
    ///   the document is meaningful — i.e. `Editing`. During play/replay the
    ///   live document is transient simulation state, so we do not checkpoint,
    ///   but we also do not lose track of the pending pre-play edit.
    /// - `edit_revision`: the host's monotonic edit counter.
    ///
    /// Returns `true` exactly on the frames an autosave should run.
    pub fn tick(
        &mut self,
        dt: f32,
        has_unsaved_work: bool,
        can_autosave: bool,
        edit_revision: u64,
    ) -> bool {
        if !has_unsaved_work {
            // Saved (or never edited): current state is the checkpoint.
            self.last_seen_revision = edit_revision;
            self.checkpointed_revision = edit_revision;
            self.since_last_edit = 0.0;
            self.since_last_checkpoint = 0.0;
            self.backoff_remaining = 0.0;
            self.fire_outstanding = false;
            return false;
        }

        if edit_revision != self.last_seen_revision {
            self.last_seen_revision = edit_revision;
            self.since_last_edit = 0.0;
            // A fresh edit supersedes any outstanding/backed-off attempt.
            self.fire_outstanding = false;
            self.backoff_remaining = 0.0;
        }

        // Nothing pending relative to the watermark.
        if self.last_seen_revision == self.checkpointed_revision {
            return false;
        }

        self.since_last_edit += dt;
        self.since_last_checkpoint += dt;

        if self.fire_outstanding {
            return false; // waiting for the caller to report the outcome
        }
        if self.backoff_remaining > 0.0 {
            self.backoff_remaining -= dt;
            return false;
        }
        // Pending work exists, but we cannot checkpoint outside Editing. Keep
        // the timers running so we fire promptly once Editing resumes.
        if !can_autosave {
            return false;
        }

        let should_fire = self.since_last_edit >= self.idle_debounce_secs
            || self.since_last_checkpoint >= self.max_interval_secs;
        if should_fire {
            self.fire_outstanding = true;
        }
        should_fire
    }

    /// Report that a checkpoint (autosave or explicit save) durably landed.
    /// Advances the watermark to the revision that was current when the fire
    /// was requested.
    pub fn note_saved(&mut self) {
        self.checkpointed_revision = self.last_seen_revision;
        self.since_last_checkpoint = 0.0;
        self.backoff_remaining = 0.0;
        self.fire_outstanding = false;
    }

    /// Report that the checkpoint attempt failed. The watermark is NOT
    /// advanced (the work is still un-checkpointed), and a back-off is armed
    /// so the next attempt is not immediate — avoiding a per-frame retry of a
    /// persistently-failing write.
    pub fn note_failed(&mut self) {
        self.fire_outstanding = false;
        self.backoff_remaining = self.retry_backoff_secs;
    }
}

use crate::ecs::components::{
    ActiveCamera, Camera, DirectionalLight, GlobalTransform, ParticleEmitter, RigidBody,
    SphereCollider, Stats, Transform, Velocity,
};
use crate::ecs::{Entity, World};
use crate::math::{Quat, Vec3};
use crate::renderer::Color;
use crate::runtime_slice::{
    BreakablePropDef, PatrolRouteDef, RuntimeSliceAuthoring, SpawnPointDef, SpawnPointKind,
    StaticMeshActorDef, TerrainAuthoring, TerrainPaintLayer, VegetationPaintStroke,
    VegetationPrototypeDef, VehicleSpawnerDef, WaterVolumeDef,
};
use crate::scene::{
    load_scene, parse_scene, save_scene, serialize_scene, CameraDef, EntityDef, LightDef,
    ParticleEmitterDef, RigidBodyDef, SceneData, SceneError, SphereColliderDef, StatsDef,
    TransformDef, VelocityDef,
};

pub use bridge::EditorBridgeServer;
pub use browser::{
    EditorBrowserEntry, EditorBrowserEntryKind, EditorBrowserError, EditorBrowserModel,
    EditorBrowserRootKind,
};
pub use command::{
    EditorAxis, EditorCommand, EditorCommandError, EditorEvent, EditorInspectorCursor,
};
pub use drawlist::{generate_draw_list, EditorDrawList};
pub use frame_inspector::{FrameInspectorPass, FrameInspectorState};
pub use host::{DispatchApplyReport, EditorCamera, EditorRuntimeHost};
pub use hot_reload_status::{
    HotReloadEntry, HotReloadOutcome, HotReloadStatus, DEFAULT_HOT_RELOAD_CAPACITY,
};
pub use native_ui::{
    build_native_ui, EditorNativeUi, EditorNativeUiState, EditorUiActionLogEntry,
    EditorUiActionTone, EditorUiCommandPalette, EditorUiHitRegion, EditorUiPaletteEntry,
    EditorUiPaletteEntrySource, EditorUiPanel, EditorUiPanelRegion,
};
pub use protocol::{
    decode_command as decode_editor_command_protocol, decode_event as decode_editor_event_protocol,
    encode_command as encode_editor_command_protocol, encode_event as encode_editor_event_protocol,
    EditorProtocolError,
};
pub use replay::{EditorReplayState, ReplayMode, ReplayTickOutcome};
pub use session::{EditorPlayState, EditorSceneSession, EditorSessionError, SceneReloadOutcome};
pub use viewport::{
    build_viewport_overlay, EditorViewportGizmoHandle, EditorViewportGizmoHandleId,
    EditorViewportGizmoMode, EditorViewportOverlay,
};

/// Basic project metadata and folder layout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Project {
    pub name: String,
    pub root: String,
    pub assets_dir: String,
    pub scenes_dir: String,
}

impl Project {
    pub fn new(name: impl Into<String>, root: impl Into<String>) -> Self {
        let root = root.into();
        Self {
            name: name.into(),
            assets_dir: format!("{root}/assets"),
            scenes_dir: format!("{root}/scenes"),
            root,
        }
    }

    pub fn project_file(&self) -> String {
        format!("{}/{}.hproj", self.root, self.name)
    }

    /// Serialize project metadata into a minimal `.hproj` text format.
    pub fn to_hproj_string(&self) -> String {
        format!(
            "HOLD_PROJECT_V1\nname={}\nroot={}\nassets_dir={}\nscenes_dir={}\n",
            escape_text(&self.name),
            escape_text(&self.root),
            escape_text(&self.assets_dir),
            escape_text(&self.scenes_dir)
        )
    }

    /// Parse project metadata from a `.hproj` text payload.
    pub fn from_hproj_str(src: &str) -> Result<Self, &'static str> {
        let mut lines = src.lines();
        if lines.next() != Some("HOLD_PROJECT_V1") {
            return Err("invalid project header");
        }

        let mut name: Option<String> = None;
        let mut root: Option<String> = None;
        let mut assets_dir: Option<String> = None;
        let mut scenes_dir: Option<String> = None;

        for line in lines {
            if line.is_empty() {
                continue;
            }
            let (k, v) = line.split_once('=').ok_or("invalid project line")?;
            match k {
                "name" => name = Some(unescape_text(v)?),
                "root" => root = Some(unescape_text(v)?),
                "assets_dir" => assets_dir = Some(unescape_text(v)?),
                "scenes_dir" => scenes_dir = Some(unescape_text(v)?),
                _ => return Err("unknown project key"),
            }
        }

        let name = name.ok_or("missing project name")?;
        let root = root.ok_or("missing project root")?;
        let assets_dir = assets_dir.unwrap_or_else(|| format!("{root}/assets"));
        let scenes_dir = scenes_dir.unwrap_or_else(|| format!("{root}/scenes"));
        Ok(Self {
            name,
            root,
            assets_dir,
            scenes_dir,
        })
    }
}

/// A scalar/vector value editable through an inspector.
#[derive(Debug, Clone, PartialEq)]
pub enum InspectorValue {
    Bool(bool),
    /// A floating-point field (positions, colours, fov, …).
    Number(f32),
    /// An integer field (particle `seed`, `max_particles`, …). Distinct from
    /// [`Self::Number`] so a `u32` field keeps FULL precision: an f32 only
    /// represents integers exactly up to 2^24, so a large seed stored as a
    /// number would not round-trip — a determinism hazard (ADR 0005). `i64`
    /// covers every `u8`/`u16`/`u32`/`i32` field and their negatives.
    Integer(i64),
    Text(String),
    Vec3([f32; 3]),
}

/// A single editable field under a component.
#[derive(Debug, Clone, PartialEq)]
pub struct InspectorField {
    pub name: String,
    pub value: InspectorValue,
}

/// Dynamic component snapshot used by the editor.
#[derive(Debug, Clone, PartialEq)]
pub struct InspectorComponent {
    pub type_name: String,
    pub fields: Vec<InspectorField>,
}

impl InspectorComponent {
    pub fn get_field(&self, field: &str) -> Option<&InspectorValue> {
        self.fields
            .iter()
            .find(|f| f.name == field)
            .map(|f| &f.value)
    }

    pub fn set_field(&mut self, field: &str, value: InspectorValue) -> bool {
        if let Some(existing) = self.fields.iter_mut().find(|f| f.name == field) {
            existing.value = value;
            true
        } else {
            false
        }
    }
}

/// Editable entity representation in a scene document.
#[derive(Debug, Clone, PartialEq)]
pub struct EditorEntity {
    pub id: u64,
    pub name: String,
    pub components: Vec<InspectorComponent>,
}

impl EditorEntity {
    pub fn component(&self, type_name: &str) -> Option<&InspectorComponent> {
        self.components.iter().find(|c| c.type_name == type_name)
    }

    pub fn component_mut(&mut self, type_name: &str) -> Option<&mut InspectorComponent> {
        self.components
            .iter_mut()
            .find(|c| c.type_name == type_name)
    }
}

/// Scene editing document owned by the editor.
#[derive(Debug, Clone, PartialEq)]
pub struct SceneDocument {
    pub name: String,
    pub runtime_slice: RuntimeSliceAuthoring,
    pub entities: Vec<EditorEntity>,
}

impl SceneDocument {
    pub fn from_scene_data(scene: &SceneData) -> Self {
        let mut name_to_id = HashMap::with_capacity(scene.entities.len());
        for (index, entity) in scene.entities.iter().enumerate() {
            name_to_id.insert(entity.name.clone(), index as u64 + 1);
        }

        let entities = scene
            .entities
            .iter()
            .enumerate()
            .map(|(index, entity)| EditorEntity {
                id: index as u64 + 1,
                name: entity.name.clone(),
                components: components_from_scene_entity(entity, &name_to_id),
            })
            .collect();

        Self {
            name: scene.name.clone(),
            runtime_slice: scene.runtime_slice.clone(),
            entities,
        }
    }

    pub fn to_scene_data(&self) -> Result<SceneData, SceneBridgeError> {
        let unique_names = unique_scene_names(&self.entities);

        let entities = self
            .entities
            .iter()
            .map(|entity| {
                Ok(EntityDef {
                    name: unique_names
                        .get(&entity.id)
                        .cloned()
                        .unwrap_or_else(|| format!("Entity{}", entity.id)),
                    transform: entity
                        .component("Transform")
                        .map(|component| bridge_transform(entity.id, component))
                        .transpose()?,
                    velocity: entity
                        .component("Velocity")
                        .map(|component| bridge_velocity(entity.id, component))
                        .transpose()?,
                    rigid_body: entity
                        .component("RigidBody")
                        .map(|component| bridge_rigid_body(entity.id, component))
                        .transpose()?,
                    sphere_collider: entity
                        .component("SphereCollider")
                        .map(|component| bridge_sphere_collider(entity.id, component))
                        .transpose()?,
                    particle_emitter: entity
                        .component("ParticleEmitter")
                        .map(|component| bridge_particle_emitter(entity.id, component))
                        .transpose()?,
                    camera: entity
                        .component("Camera")
                        .map(|component| bridge_camera(entity.id, component))
                        .transpose()?,
                    active_camera: entity.component("ActiveCamera").is_some(),
                    light: entity
                        .component("DirectionalLight")
                        .or_else(|| entity.component("Light"))
                        .map(|component| bridge_light(entity.id, component))
                        .transpose()?,
                    stats: entity
                        .component("Stats")
                        .map(|component| bridge_stats(entity.id, component))
                        .transpose()?,
                    parent_name: entity
                        .component("Parent")
                        .map(|component| bridge_parent_name(entity.id, component, &unique_names))
                        .transpose()?
                        .flatten(),
                })
            })
            .collect::<Result<Vec<_>, SceneBridgeError>>()?;

        Ok(SceneData {
            name: self.name.clone(),
            runtime_slice: self.runtime_slice.clone(),
            entities,
        })
    }

    /// Parse a runtime `.hsc` payload into the editor-facing scene document.
    pub fn from_hsc_str(src: &str) -> Result<Self, SceneError> {
        let scene = parse_scene(src)?;
        Ok(Self::from_scene_data(&scene))
    }

    /// Export the editor-facing scene document into runtime `.hsc` text.
    pub fn to_hsc_string(&self) -> Result<String, SceneBridgeError> {
        let scene = self.to_scene_data()?;
        Ok(serialize_scene(&scene))
    }

    /// Dynamic path-style getter:
    /// entity -> component -> field -> value
    pub fn get_value(
        &self,
        entity_id: u64,
        component: &str,
        field: &str,
    ) -> Option<&InspectorValue> {
        self.entities
            .iter()
            .find(|e| e.id == entity_id)?
            .component(component)?
            .get_field(field)
    }

    /// Dynamic path-style setter:
    /// entity -> component -> field -> value
    ///
    /// If the target entity/component/field does not exist yet, it is created.
    pub fn set_value(
        &mut self,
        entity_id: u64,
        component: &str,
        field: &str,
        value: InspectorValue,
    ) -> bool {
        let entity = if let Some(existing) = self.entities.iter_mut().find(|e| e.id == entity_id) {
            existing
        } else {
            self.entities.push(EditorEntity {
                id: entity_id,
                name: format!("Entity{entity_id}"),
                components: Vec::new(),
            });
            let idx = self.entities.len() - 1;
            &mut self.entities[idx]
        };

        let component = if let Some(existing) = entity.component_mut(component) {
            existing
        } else {
            entity.components.push(InspectorComponent {
                type_name: component.to_string(),
                fields: Vec::new(),
            });
            let idx = entity.components.len() - 1;
            &mut entity.components[idx]
        };

        if let Some(existing_field) = component.fields.iter_mut().find(|f| f.name == field) {
            existing_field.value = value;
        } else {
            component.fields.push(InspectorField {
                name: field.to_string(),
                value,
            });
        }

        true
    }

    pub fn set_terrain_settings(
        &mut self,
        size_meters: Vec3,
        chunk_size_meters: u32,
        height_scale_meters: f32,
    ) {
        self.runtime_slice.terrain = TerrainAuthoring {
            size_meters: Vec3::new(
                size_meters.x.max(1.0),
                size_meters.y.max(0.0),
                size_meters.z.max(1.0),
            ),
            chunk_size_meters: chunk_size_meters.max(1),
            height_scale_meters: height_scale_meters.max(0.0),
            paint_layers: self.runtime_slice.terrain.paint_layers.clone(),
        };
    }

    pub fn upsert_terrain_paint_layer(&mut self, mut layer: TerrainPaintLayer) {
        normalize_named_runtime_slice_item(&mut layer.name);
        layer.material = normalize_optional_runtime_slice_ref(layer.material);
        layer.tiling = layer.tiling.max(0.01);
        if let Some(existing) = self
            .runtime_slice
            .terrain
            .paint_layers
            .iter_mut()
            .find(|existing| existing.name == layer.name)
        {
            *existing = layer;
        } else {
            self.runtime_slice.terrain.paint_layers.push(layer);
        }
    }

    pub fn remove_terrain_paint_layer(&mut self, name: &str) -> bool {
        remove_named_item(
            &mut self.runtime_slice.terrain.paint_layers,
            name,
            |layer| &layer.name,
        )
    }

    pub fn paint_vegetation(&mut self, mut stroke: VegetationPaintStroke) {
        normalize_named_runtime_slice_item(&mut stroke.prototype);
        stroke.radius = stroke.radius.max(0.0);
        stroke.density = stroke.density.max(0.0);
        self.runtime_slice.vegetation_paints.push(stroke);
    }

    pub fn erase_vegetation(
        &mut self,
        prototype: Option<&str>,
        center: Vec3,
        radius: f32,
    ) -> usize {
        let radius_sq = radius.max(0.0) * radius.max(0.0);
        let prototype = prototype
            .map(|value| value.trim())
            .filter(|value| !value.is_empty());
        let original_len = self.runtime_slice.vegetation_paints.len();
        self.runtime_slice.vegetation_paints.retain(|stroke| {
            let prototype_matches = prototype
                .map(|expected| stroke.prototype == expected)
                .unwrap_or(true);
            !prototype_matches || stroke.center.distance_sq(center) > radius_sq
        });
        original_len - self.runtime_slice.vegetation_paints.len()
    }

    pub fn upsert_patrol_route(&mut self, mut route: PatrolRouteDef) {
        normalize_named_runtime_slice_item(&mut route.name);
        if let Some(existing) = self
            .runtime_slice
            .patrol_routes
            .iter_mut()
            .find(|existing| existing.name == route.name)
        {
            *existing = route;
        } else {
            self.runtime_slice.patrol_routes.push(route);
        }
    }

    pub fn remove_patrol_route(&mut self, name: &str) -> bool {
        remove_named_item(&mut self.runtime_slice.patrol_routes, name, |route| {
            &route.name
        })
    }

    pub fn upsert_vehicle_spawner(&mut self, mut spawner: VehicleSpawnerDef) {
        normalize_named_runtime_slice_item(&mut spawner.name);
        spawner.mesh = normalize_optional_runtime_slice_ref(spawner.mesh);
        spawner.material = normalize_optional_runtime_slice_ref(spawner.material);
        if let Some(existing) = self
            .runtime_slice
            .vehicle_spawners
            .iter_mut()
            .find(|existing| existing.name == spawner.name)
        {
            *existing = spawner;
        } else {
            self.runtime_slice.vehicle_spawners.push(spawner);
        }
    }

    pub fn remove_vehicle_spawner(&mut self, name: &str) -> bool {
        remove_named_item(&mut self.runtime_slice.vehicle_spawners, name, |spawner| {
            &spawner.name
        })
    }

    /// Serialize the editor scene document to a stable line-based text format.
    ///
    /// Writes the V2 header, which obliges a reader to require the
    /// [`HSCENE_DOC_END`] terminator (M11). See that constant for why.
    pub fn to_hscene_string(&self) -> String {
        let mut out = String::new();
        out.push_str(HSCENE_DOC_HEADER_V2);
        out.push('\n');
        out.push_str(&format!("name|{}\n", escape_text(&self.name)));
        encode_runtime_slice_lines(&mut out, &self.runtime_slice);
        for entity in &self.entities {
            out.push_str(&format!(
                "entity|{}|{}\n",
                entity.id,
                escape_text(&entity.name)
            ));
            for component in &entity.components {
                out.push_str(&format!(
                    "component|{}\n",
                    escape_text(&component.type_name)
                ));
                for field in &component.fields {
                    out.push_str(&encode_field(field));
                    out.push('\n');
                }
                out.push_str("endcomponent\n");
            }
            out.push_str("endentity\n");
        }
        out.push_str(HSCENE_DOC_END);
        out.push('\n');
        out
    }

    /// Parse the editor scene document from the line-based text format.
    ///
    /// A V2 document must close with [`HSCENE_DOC_END`]; a V1 document
    /// (written before M11) is accepted without it, so existing files are not
    /// orphaned.
    pub fn from_hscene_str(src: &str) -> Result<Self, &'static str> {
        let mut lines = src.lines();
        let strict = match lines.next() {
            Some(HSCENE_DOC_HEADER_V2) => true,
            Some(HSCENE_DOC_HEADER_V1) => false,
            _ => return Err("invalid scene document header"),
        };

        let name_line = lines.next().ok_or("missing scene name line")?;
        let (_, name) = name_line.split_once('|').ok_or("invalid scene name line")?;
        if !name_line.starts_with("name|") {
            return Err("invalid scene name prefix");
        }

        let mut doc = Self {
            name: unescape_text(name)?,
            runtime_slice: RuntimeSliceAuthoring::default(),
            entities: Vec::new(),
        };

        let mut current_entity: Option<EditorEntity> = None;
        let mut current_component: Option<InspectorComponent> = None;
        let mut saw_end = false;

        for line in lines {
            if line.is_empty() {
                continue;
            }

            if saw_end {
                return Err("content after scene document terminator");
            }

            if line == HSCENE_DOC_END {
                saw_end = true;
                continue;
            }

            if line == "endcomponent" {
                let component = current_component
                    .take()
                    .ok_or("endcomponent without component")?;
                let entity = current_entity
                    .as_mut()
                    .ok_or("component outside entity in scene doc")?;
                entity.components.push(component);
                continue;
            }

            if line == "endentity" {
                if current_component.is_some() {
                    return Err("entity ended before component closed");
                }
                let entity = current_entity.take().ok_or("endentity without entity")?;
                doc.entities.push(entity);
                continue;
            }

            if let Some(rest) = line.strip_prefix("entity|") {
                if current_entity.is_some() {
                    return Err("nested entity not allowed");
                }
                let mut parts = rest.splitn(2, '|');
                let id = parts
                    .next()
                    .ok_or("missing entity id")?
                    .parse::<u64>()
                    .map_err(|_| "invalid entity id")?;
                let name = unescape_text(parts.next().ok_or("missing entity name")?)?;
                current_entity = Some(EditorEntity {
                    id,
                    name,
                    components: Vec::new(),
                });
                continue;
            }

            if let Some(type_name) = line.strip_prefix("component|") {
                if current_entity.is_none() {
                    return Err("component outside entity");
                }
                if current_component.is_some() {
                    return Err("nested component not allowed");
                }
                current_component = Some(InspectorComponent {
                    type_name: unescape_text(type_name)?,
                    fields: Vec::new(),
                });
                continue;
            }

            if let Some(rest) = line.strip_prefix("field|") {
                let component = current_component
                    .as_mut()
                    .ok_or("field outside component in scene doc")?;
                component.fields.push(decode_field(rest)?);
                continue;
            }

            if let Some(rest) = line.strip_prefix("slice|") {
                if current_entity.is_some() || current_component.is_some() {
                    return Err("runtime slice data must be top-level");
                }
                decode_runtime_slice_line(&mut doc.runtime_slice, rest)?;
                continue;
            }

            return Err("unknown scene document line");
        }

        if current_component.is_some() {
            return Err("unterminated component");
        }
        if current_entity.is_some() {
            return Err("unterminated entity");
        }
        // A V2 document that stops before its terminator was cut short. Without
        // this, a truncation on an `endentity` boundary parses as a valid but
        // silently smaller document — every entity after the cut vanishes.
        if strict && !saw_end {
            return Err("truncated scene document: missing terminator");
        }

        Ok(doc)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SceneBridgeError {
    MissingField {
        entity_id: u64,
        component: String,
        field: &'static str,
    },
    InvalidFieldType {
        entity_id: u64,
        component: String,
        field: &'static str,
    },
    UnknownParentEntityId {
        entity_id: u64,
        parent_id: u64,
    },
}

impl std::fmt::Display for SceneBridgeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingField {
                entity_id,
                component,
                field,
            } => write!(
                f,
                "scene bridge error: entity {} missing {}.{}",
                entity_id, component, field
            ),
            Self::InvalidFieldType {
                entity_id,
                component,
                field,
            } => write!(
                f,
                "scene bridge error: entity {} invalid type for {}.{}",
                entity_id, component, field
            ),
            Self::UnknownParentEntityId {
                entity_id,
                parent_id,
            } => write!(
                f,
                "scene bridge error: entity {} references unknown parent id {}",
                entity_id, parent_id
            ),
        }
    }
}

impl std::error::Error for SceneBridgeError {}

fn normalize_named_runtime_slice_item(name: &mut String) {
    *name = name.trim().to_string();
}

fn normalize_optional_runtime_slice_ref(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn remove_named_item<T>(items: &mut Vec<T>, name: &str, get_name: impl Fn(&T) -> &str) -> bool {
    let original_len = items.len();
    items.retain(|item| get_name(item) != name);
    original_len != items.len()
}

fn components_from_scene_entity(
    entity: &EntityDef,
    name_to_id: &HashMap<String, u64>,
) -> Vec<InspectorComponent> {
    let mut components = Vec::new();

    if let Some(transform) = &entity.transform {
        let euler = transform.rotation.to_euler_xyz();
        components.push(InspectorComponent {
            type_name: "Transform".into(),
            fields: vec![
                vec3_field(
                    "position",
                    [
                        transform.position.x,
                        transform.position.y,
                        transform.position.z,
                    ],
                ),
                vec3_field(
                    "scale",
                    [transform.scale.x, transform.scale.y, transform.scale.z],
                ),
                vec3_field(
                    "rotation_euler_deg",
                    [
                        euler.x.to_degrees(),
                        euler.y.to_degrees(),
                        euler.z.to_degrees(),
                    ],
                ),
            ],
        });
    }

    if let Some(velocity) = &entity.velocity {
        components.push(InspectorComponent {
            type_name: "Velocity".into(),
            fields: vec![
                vec3_field(
                    "linear",
                    [velocity.linear.x, velocity.linear.y, velocity.linear.z],
                ),
                vec3_field(
                    "angular",
                    [velocity.angular.x, velocity.angular.y, velocity.angular.z],
                ),
            ],
        });
    }

    if let Some(rigid_body) = &entity.rigid_body {
        components.push(InspectorComponent {
            type_name: "RigidBody".into(),
            fields: vec![
                number_field("gravity_scale", rigid_body.gravity_scale),
                number_field("linear_damping", rigid_body.linear_damping),
                number_field("angular_damping", rigid_body.angular_damping),
                number_field("restitution", rigid_body.restitution),
            ],
        });
    }

    if let Some(collider) = &entity.sphere_collider {
        components.push(InspectorComponent {
            type_name: "SphereCollider".into(),
            fields: vec![number_field("radius", collider.radius)],
        });
    }

    if let Some(emitter) = &entity.particle_emitter {
        components.push(InspectorComponent {
            type_name: "ParticleEmitter".into(),
            fields: vec![
                number_field("emitting", if emitter.emitting { 1.0 } else { 0.0 }),
                number_field("rate_per_second", emitter.rate_per_second),
                integer_field("max_particles", emitter.max_particles as i64),
                number_field("spawn_radius", emitter.spawn_radius),
                vec3_field(
                    "direction",
                    [
                        emitter.direction.x,
                        emitter.direction.y,
                        emitter.direction.z,
                    ],
                ),
                number_field("spread", emitter.spread),
                number_field("speed_min", emitter.speed_min),
                number_field("speed_max", emitter.speed_max),
                number_field("lifetime_min", emitter.lifetime_min),
                number_field("lifetime_max", emitter.lifetime_max),
                number_field("gravity_scale", emitter.gravity_scale),
                number_field("linear_damping", emitter.linear_damping),
                number_field("start_size", emitter.start_size),
                number_field("end_size", emitter.end_size),
                vec3_field(
                    "start_color",
                    [
                        emitter.start_color.r,
                        emitter.start_color.g,
                        emitter.start_color.b,
                    ],
                ),
                number_field("start_color_alpha", emitter.start_color.a),
                vec3_field(
                    "end_color",
                    [
                        emitter.end_color.r,
                        emitter.end_color.g,
                        emitter.end_color.b,
                    ],
                ),
                number_field("end_color_alpha", emitter.end_color.a),
                integer_field("seed", emitter.seed as i64),
            ],
        });
    }

    if let Some(camera) = &entity.camera {
        components.push(InspectorComponent {
            type_name: "Camera".into(),
            fields: vec![
                number_field("fov_deg", camera.fov),
                number_field("near", camera.near),
                number_field("far", camera.far),
            ],
        });
    }

    if entity.active_camera {
        components.push(InspectorComponent {
            type_name: "ActiveCamera".into(),
            fields: Vec::new(),
        });
    }

    if let Some(light) = &entity.light {
        components.push(InspectorComponent {
            type_name: "DirectionalLight".into(),
            fields: vec![
                vec3_field(
                    "direction",
                    [light.direction.x, light.direction.y, light.direction.z],
                ),
                vec3_field("color", [light.color.r, light.color.g, light.color.b]),
                number_field("color_alpha", light.color.a),
                number_field("intensity", light.intensity),
                number_field("ambient", light.ambient),
            ],
        });
    }

    if let Some(stats) = &entity.stats {
        components.push(InspectorComponent {
            type_name: "Stats".into(),
            fields: vec![
                number_field("health", stats.health),
                number_field("mana", stats.mana),
                number_field("attack", stats.attack),
                number_field("defense", stats.defense),
                number_field("speed", stats.speed),
            ],
        });
    }

    if let Some(parent_name) = &entity.parent_name {
        let mut fields = vec![text_field("entity_name", parent_name)];
        if let Some(parent_id) = name_to_id.get(parent_name) {
            fields.push(number_field("entity_id", *parent_id as f32));
        }
        components.push(InspectorComponent {
            type_name: "Parent".into(),
            fields,
        });
    }

    components
}

fn unique_scene_names(entities: &[EditorEntity]) -> HashMap<u64, String> {
    let mut used = HashSet::new();
    let mut names = HashMap::with_capacity(entities.len());

    for entity in entities {
        let base_name = if entity.name.trim().is_empty() {
            format!("Entity{}", entity.id)
        } else {
            entity.name.trim().to_string()
        };
        let mut name = base_name.clone();
        let mut suffix = 2;
        while used.contains(&name) {
            name = format!("{} {}", base_name, suffix);
            suffix += 1;
        }
        used.insert(name.clone());
        names.insert(entity.id, name);
    }

    names
}

fn number_field(name: &str, value: f32) -> InspectorField {
    InspectorField {
        name: name.into(),
        value: InspectorValue::Number(value),
    }
}

fn integer_field(name: &str, value: i64) -> InspectorField {
    InspectorField {
        name: name.into(),
        value: InspectorValue::Integer(value),
    }
}

fn text_field(name: &str, value: &str) -> InspectorField {
    InspectorField {
        name: name.into(),
        value: InspectorValue::Text(value.into()),
    }
}

fn vec3_field(name: &str, value: [f32; 3]) -> InspectorField {
    InspectorField {
        name: name.into(),
        value: InspectorValue::Vec3(value),
    }
}

fn bridge_transform(
    entity_id: u64,
    component: &InspectorComponent,
) -> Result<TransformDef, SceneBridgeError> {
    let position = required_vec3_bridge(entity_id, component, "position")?;
    let scale = optional_vec3(component, "scale").unwrap_or(Vec3::ONE);
    let rotation = bridge_rotation(entity_id, component)?;

    Ok(TransformDef {
        position,
        rotation,
        scale,
    })
}

fn bridge_rotation(
    entity_id: u64,
    component: &InspectorComponent,
) -> Result<Quat, SceneBridgeError> {
    let quat_fields = [
        optional_number(component, "rotation_x"),
        optional_number(component, "rotation_y"),
        optional_number(component, "rotation_z"),
        optional_number(component, "rotation_w"),
    ];
    if quat_fields.iter().any(|value| value.is_some()) {
        return Ok(Quat::new(
            required_number_bridge(entity_id, component, "rotation_x")?,
            required_number_bridge(entity_id, component, "rotation_y")?,
            required_number_bridge(entity_id, component, "rotation_z")?,
            required_number_bridge(entity_id, component, "rotation_w")?,
        ));
    }

    Ok(optional_vec3(component, "rotation_euler_deg")
        .map(|e| Quat::from_euler_xyz(e.x.to_radians(), e.y.to_radians(), e.z.to_radians()))
        .unwrap_or(Quat::IDENTITY))
}

fn bridge_camera(
    entity_id: u64,
    component: &InspectorComponent,
) -> Result<CameraDef, SceneBridgeError> {
    Ok(CameraDef {
        fov: required_number_bridge(entity_id, component, "fov_deg")?,
        near: optional_number(component, "near").unwrap_or(0.1),
        far: optional_number(component, "far").unwrap_or(1000.0),
    })
}

fn bridge_velocity(
    _entity_id: u64,
    component: &InspectorComponent,
) -> Result<VelocityDef, SceneBridgeError> {
    Ok(VelocityDef {
        linear: optional_vec3(component, "linear").unwrap_or(Vec3::ZERO),
        angular: optional_vec3(component, "angular").unwrap_or(Vec3::ZERO),
    })
}

fn bridge_rigid_body(
    _entity_id: u64,
    component: &InspectorComponent,
) -> Result<RigidBodyDef, SceneBridgeError> {
    let default = RigidBody::default();
    Ok(RigidBodyDef {
        gravity_scale: optional_number(component, "gravity_scale").unwrap_or(default.gravity_scale),
        linear_damping: optional_number(component, "linear_damping")
            .unwrap_or(default.linear_damping),
        angular_damping: optional_number(component, "angular_damping")
            .unwrap_or(default.angular_damping),
        restitution: optional_number(component, "restitution").unwrap_or(default.restitution),
    })
}

fn bridge_sphere_collider(
    _entity_id: u64,
    component: &InspectorComponent,
) -> Result<SphereColliderDef, SceneBridgeError> {
    Ok(SphereColliderDef {
        radius: optional_number(component, "radius").unwrap_or(0.5),
    })
}

fn bridge_particle_emitter(
    _entity_id: u64,
    component: &InspectorComponent,
) -> Result<ParticleEmitterDef, SceneBridgeError> {
    let default = ParticleEmitter::spark_fountain();
    Ok(ParticleEmitterDef {
        emitting: optional_number(component, "emitting").unwrap_or(if default.emitting {
            1.0
        } else {
            0.0
        }) > 0.0,
        rate_per_second: optional_number(component, "rate_per_second")
            .or_else(|| optional_number(component, "rate"))
            .unwrap_or(default.rate_per_second),
        max_particles: optional_integer(component, "max_particles")
            .or_else(|| optional_integer(component, "max"))
            .map(|v| v.clamp(0, u32::MAX as i64) as u32)
            .unwrap_or(default.max_particles),
        spawn_radius: optional_number(component, "spawn_radius").unwrap_or(default.spawn_radius),
        direction: optional_vec3(component, "direction").unwrap_or(default.direction),
        spread: optional_number(component, "spread").unwrap_or(default.spread),
        speed_min: optional_number(component, "speed_min").unwrap_or(default.speed_min),
        speed_max: optional_number(component, "speed_max").unwrap_or(default.speed_max),
        lifetime_min: optional_number(component, "lifetime_min").unwrap_or(default.lifetime_min),
        lifetime_max: optional_number(component, "lifetime_max").unwrap_or(default.lifetime_max),
        gravity_scale: optional_number(component, "gravity_scale").unwrap_or(default.gravity_scale),
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
                    optional_number(component, "end_color_alpha").unwrap_or(default.end_color.a),
                )
            })
            .unwrap_or(default.end_color),
        seed: optional_u32(component, "seed", default.seed),
    })
}

fn bridge_light(
    entity_id: u64,
    component: &InspectorComponent,
) -> Result<LightDef, SceneBridgeError> {
    let direction = required_vec3_bridge(entity_id, component, "direction")?;
    let color = required_vec3_bridge(entity_id, component, "color")?;

    Ok(LightDef {
        direction,
        color: Color::new(
            color.x,
            color.y,
            color.z,
            optional_number(component, "color_alpha").unwrap_or(1.0),
        ),
        intensity: optional_number(component, "intensity").unwrap_or(1.0),
        ambient: optional_number(component, "ambient").unwrap_or(0.15),
    })
}

fn bridge_stats(
    entity_id: u64,
    component: &InspectorComponent,
) -> Result<StatsDef, SceneBridgeError> {
    Ok(StatsDef {
        health: required_number_bridge(entity_id, component, "health")?,
        mana: optional_number(component, "mana").unwrap_or(0.0),
        attack: optional_number(component, "attack").unwrap_or(0.0),
        defense: optional_number(component, "defense").unwrap_or(0.0),
        speed: optional_number(component, "speed").unwrap_or(0.0),
    })
}

fn bridge_parent_name(
    entity_id: u64,
    component: &InspectorComponent,
    unique_names: &HashMap<u64, String>,
) -> Result<Option<String>, SceneBridgeError> {
    if let Some(parent_id) = optional_number(component, "entity_id") {
        let parent_id = parent_id as u64;
        return unique_names.get(&parent_id).cloned().map(Some).ok_or(
            SceneBridgeError::UnknownParentEntityId {
                entity_id,
                parent_id,
            },
        );
    }

    Ok(optional_text(component, "entity_name"))
}

fn required_number_bridge(
    entity_id: u64,
    component: &InspectorComponent,
    field: &'static str,
) -> Result<f32, SceneBridgeError> {
    let value = component
        .get_field(field)
        .ok_or(SceneBridgeError::MissingField {
            entity_id,
            component: component.type_name.clone(),
            field,
        })?;
    match value {
        InspectorValue::Number(v) => Ok(*v),
        _ => Err(SceneBridgeError::InvalidFieldType {
            entity_id,
            component: component.type_name.clone(),
            field,
        }),
    }
}

fn required_vec3_bridge(
    entity_id: u64,
    component: &InspectorComponent,
    field: &'static str,
) -> Result<Vec3, SceneBridgeError> {
    let value = component
        .get_field(field)
        .ok_or(SceneBridgeError::MissingField {
            entity_id,
            component: component.type_name.clone(),
            field,
        })?;
    match value {
        InspectorValue::Vec3(v) => Ok(Vec3::new(v[0], v[1], v[2])),
        _ => Err(SceneBridgeError::InvalidFieldType {
            entity_id,
            component: component.type_name.clone(),
            field,
        }),
    }
}

fn encode_field(field: &InspectorField) -> String {
    match &field.value {
        InspectorValue::Bool(v) => {
            format!(
                "field|{}|bool|{}",
                escape_text(&field.name),
                if *v { 1 } else { 0 }
            )
        }
        InspectorValue::Number(v) => format!("field|{}|number|{}", escape_text(&field.name), v),
        InspectorValue::Integer(v) => format!("field|{}|integer|{}", escape_text(&field.name), v),
        InspectorValue::Text(v) => {
            format!("field|{}|text|{}", escape_text(&field.name), escape_text(v))
        }
        InspectorValue::Vec3(v) => {
            format!(
                "field|{}|vec3|{}|{}|{}",
                escape_text(&field.name),
                v[0],
                v[1],
                v[2]
            )
        }
    }
}

fn decode_field(rest: &str) -> Result<InspectorField, &'static str> {
    let mut parts = rest.split('|');
    let name = unescape_text(parts.next().ok_or("missing field name")?)?;
    let kind = parts.next().ok_or("missing field type")?;

    let value = match kind {
        "bool" => {
            let raw = parts.next().ok_or("missing bool value")?;
            InspectorValue::Bool(raw == "1")
        }
        "number" => {
            let raw = parts.next().ok_or("missing number value")?;
            InspectorValue::Number(raw.parse::<f32>().map_err(|_| "invalid number value")?)
        }
        "integer" => {
            let raw = parts.next().ok_or("missing integer value")?;
            InspectorValue::Integer(raw.parse::<i64>().map_err(|_| "invalid integer value")?)
        }
        "text" => InspectorValue::Text(unescape_text(parts.next().ok_or("missing text value")?)?),
        "vec3" => {
            let x = parts
                .next()
                .ok_or("missing vec3 x")?
                .parse::<f32>()
                .map_err(|_| "invalid vec3 x")?;
            let y = parts
                .next()
                .ok_or("missing vec3 y")?
                .parse::<f32>()
                .map_err(|_| "invalid vec3 y")?;
            let z = parts
                .next()
                .ok_or("missing vec3 z")?
                .parse::<f32>()
                .map_err(|_| "invalid vec3 z")?;
            InspectorValue::Vec3([x, y, z])
        }
        _ => return Err("unknown field type"),
    };

    Ok(InspectorField { name, value })
}

fn encode_runtime_slice_lines(out: &mut String, slice: &RuntimeSliceAuthoring) {
    if slice.terrain != crate::runtime_slice::TerrainAuthoring::default() {
        out.push_str(&format!(
            "slice|terrain|{}|{}|{}|{}|{}\n",
            slice.terrain.size_meters.x,
            slice.terrain.size_meters.y,
            slice.terrain.size_meters.z,
            slice.terrain.chunk_size_meters,
            slice.terrain.height_scale_meters,
        ));
    }

    for layer in &slice.terrain.paint_layers {
        out.push_str(&format!(
            "slice|paint_layer|{}|{}|{}|{}\n",
            escape_text(&layer.name),
            encode_optional_text(layer.material.as_deref()),
            layer.tiling,
            layer.weightmap_channel,
        ));
    }

    for prototype in &slice.vegetation_prototypes {
        out.push_str(&format!(
            "slice|vegetation_prototype|{}|{}|{}|{}|{}|{}\n",
            escape_text(&prototype.name),
            encode_optional_text(prototype.mesh.as_deref()),
            encode_optional_text(prototype.material.as_deref()),
            prototype.density_per_100m2,
            if prototype.random_yaw { 1 } else { 0 },
            if prototype.align_to_normal { 1 } else { 0 },
        ));
    }

    for stroke in &slice.vegetation_paints {
        out.push_str(&format!(
            "slice|vegetation_paint|{}|{}|{}|{}|{}|{}\n",
            escape_text(&stroke.prototype),
            stroke.center.x,
            stroke.center.y,
            stroke.center.z,
            stroke.radius,
            stroke.density,
        ));
    }

    for actor in &slice.static_mesh_actors {
        out.push_str(&format!(
            "slice|static_mesh_actor|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}\n",
            escape_text(&actor.name),
            encode_optional_text(actor.mesh.as_deref()),
            encode_optional_text(actor.material.as_deref()),
            actor.position.x,
            actor.position.y,
            actor.position.z,
            actor.rotation_euler_deg.x,
            actor.rotation_euler_deg.y,
            actor.rotation_euler_deg.z,
            actor.scale.x,
            actor.scale.y,
            actor.scale.z,
        ));
    }

    for volume in &slice.water_volumes {
        out.push_str(&format!(
            "slice|water_volume|{}|{}|{}|{}|{}|{}|{}|{}|{}\n",
            escape_text(&volume.name),
            encode_optional_text(volume.material.as_deref()),
            volume.center.x,
            volume.center.y,
            volume.center.z,
            volume.extents.x,
            volume.extents.y,
            volume.extents.z,
            volume.wave_strength,
        ));
    }

    for spawn in &slice.spawn_points {
        let kind = match spawn.kind {
            SpawnPointKind::Generic => "generic",
            SpawnPointKind::Player => "player",
            SpawnPointKind::Enemy => "enemy",
            SpawnPointKind::Vehicle => "vehicle",
        };
        out.push_str(&format!(
            "slice|spawn_point|{}|{}|{}|{}|{}|{}\n",
            escape_text(&spawn.name),
            kind,
            spawn.position.x,
            spawn.position.y,
            spawn.position.z,
            spawn.facing_yaw_deg,
        ));
    }

    for route in &slice.patrol_routes {
        out.push_str(&format!(
            "slice|patrol_route|{}|{}\n",
            escape_text(&route.name),
            if route.looped { 1 } else { 0 },
        ));
        for point in &route.points {
            out.push_str(&format!(
                "slice|patrol_point|{}|{}|{}|{}\n",
                escape_text(&route.name),
                point.x,
                point.y,
                point.z,
            ));
        }
    }

    for spawner in &slice.vehicle_spawners {
        out.push_str(&format!(
            "slice|vehicle_spawner|{}|{}|{}|{}|{}|{}|{}\n",
            escape_text(&spawner.name),
            encode_optional_text(spawner.mesh.as_deref()),
            encode_optional_text(spawner.material.as_deref()),
            spawner.position.x,
            spawner.position.y,
            spawner.position.z,
            spawner.facing_yaw_deg,
        ));
    }

    for prop in &slice.breakable_props {
        out.push_str(&format!(
            "slice|breakable_prop|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}\n",
            escape_text(&prop.name),
            encode_optional_text(prop.intact_mesh.as_deref()),
            encode_optional_text(prop.broken_mesh.as_deref()),
            encode_optional_text(prop.material.as_deref()),
            prop.position.x,
            prop.position.y,
            prop.position.z,
            prop.rotation_euler_deg.x,
            prop.rotation_euler_deg.y,
            prop.rotation_euler_deg.z,
            prop.scale.x,
            prop.scale.y,
            prop.scale.z,
            prop.hit_points,
        ));
        out.push_str(&format!(
            "slice|breakable_prop_impulse|{}|{}\n",
            escape_text(&prop.name),
            prop.break_impulse,
        ));
    }
}

fn decode_runtime_slice_line(
    slice: &mut RuntimeSliceAuthoring,
    rest: &str,
) -> Result<(), &'static str> {
    let mut parts = rest.split('|');
    let kind = parts.next().ok_or("missing runtime slice line kind")?;

    match kind {
        "terrain" => {
            slice.terrain.size_meters = Vec3::new(
                parse_doc_f32(parts.next(), "missing terrain size x")?,
                parse_doc_f32(parts.next(), "missing terrain size y")?,
                parse_doc_f32(parts.next(), "missing terrain size z")?,
            );
            slice.terrain.chunk_size_meters =
                parse_doc_u32(parts.next(), "missing terrain chunk size")?;
            slice.terrain.height_scale_meters =
                parse_doc_f32(parts.next(), "missing terrain height scale")?;
        }
        "paint_layer" => {
            slice.terrain.paint_layers.push(TerrainPaintLayer {
                name: parse_doc_text(parts.next(), "missing paint layer name")?,
                material: parse_doc_optional_text(parts.next(), "missing paint layer material")?,
                tiling: parse_doc_f32(parts.next(), "missing paint layer tiling")?,
                weightmap_channel: parse_doc_u8(parts.next(), "missing paint layer channel")?,
            });
        }
        "vegetation_prototype" => {
            slice.vegetation_prototypes.push(VegetationPrototypeDef {
                name: parse_doc_text(parts.next(), "missing vegetation prototype name")?,
                mesh: parse_doc_optional_text(parts.next(), "missing vegetation prototype mesh")?,
                material: parse_doc_optional_text(
                    parts.next(),
                    "missing vegetation prototype material",
                )?,
                density_per_100m2: parse_doc_f32(
                    parts.next(),
                    "missing vegetation prototype density",
                )?,
                random_yaw: parse_doc_bool01(
                    parts.next(),
                    "missing vegetation prototype random_yaw",
                )?,
                align_to_normal: parse_doc_bool01(
                    parts.next(),
                    "missing vegetation prototype align_to_normal",
                )?,
            });
        }
        "vegetation_paint" => {
            slice.vegetation_paints.push(VegetationPaintStroke {
                prototype: parse_doc_text(parts.next(), "missing vegetation paint prototype")?,
                center: Vec3::new(
                    parse_doc_f32(parts.next(), "missing vegetation paint center x")?,
                    parse_doc_f32(parts.next(), "missing vegetation paint center y")?,
                    parse_doc_f32(parts.next(), "missing vegetation paint center z")?,
                ),
                radius: parse_doc_f32(parts.next(), "missing vegetation paint radius")?,
                density: parse_doc_f32(parts.next(), "missing vegetation paint density")?,
            });
        }
        "static_mesh_actor" => {
            slice.static_mesh_actors.push(StaticMeshActorDef {
                name: parse_doc_text(parts.next(), "missing static mesh actor name")?,
                mesh: parse_doc_optional_text(parts.next(), "missing static mesh actor mesh")?,
                material: parse_doc_optional_text(
                    parts.next(),
                    "missing static mesh actor material",
                )?,
                position: Vec3::new(
                    parse_doc_f32(parts.next(), "missing static mesh actor position x")?,
                    parse_doc_f32(parts.next(), "missing static mesh actor position y")?,
                    parse_doc_f32(parts.next(), "missing static mesh actor position z")?,
                ),
                rotation_euler_deg: Vec3::new(
                    parse_doc_f32(parts.next(), "missing static mesh actor rotation x")?,
                    parse_doc_f32(parts.next(), "missing static mesh actor rotation y")?,
                    parse_doc_f32(parts.next(), "missing static mesh actor rotation z")?,
                ),
                scale: Vec3::new(
                    parse_doc_f32(parts.next(), "missing static mesh actor scale x")?,
                    parse_doc_f32(parts.next(), "missing static mesh actor scale y")?,
                    parse_doc_f32(parts.next(), "missing static mesh actor scale z")?,
                ),
            });
        }
        "water_volume" => {
            slice.water_volumes.push(WaterVolumeDef {
                name: parse_doc_text(parts.next(), "missing water volume name")?,
                material: parse_doc_optional_text(parts.next(), "missing water volume material")?,
                center: Vec3::new(
                    parse_doc_f32(parts.next(), "missing water volume center x")?,
                    parse_doc_f32(parts.next(), "missing water volume center y")?,
                    parse_doc_f32(parts.next(), "missing water volume center z")?,
                ),
                extents: Vec3::new(
                    parse_doc_f32(parts.next(), "missing water volume extents x")?,
                    parse_doc_f32(parts.next(), "missing water volume extents y")?,
                    parse_doc_f32(parts.next(), "missing water volume extents z")?,
                ),
                wave_strength: parse_doc_f32(parts.next(), "missing water volume wave strength")?,
            });
        }
        "spawn_point" => {
            let name = parse_doc_text(parts.next(), "missing spawn point name")?;
            let kind = match parts.next().ok_or("missing spawn point kind")? {
                "generic" => SpawnPointKind::Generic,
                "player" => SpawnPointKind::Player,
                "enemy" => SpawnPointKind::Enemy,
                "vehicle" => SpawnPointKind::Vehicle,
                _ => return Err("unknown spawn point kind"),
            };
            slice.spawn_points.push(SpawnPointDef {
                name,
                kind,
                position: Vec3::new(
                    parse_doc_f32(parts.next(), "missing spawn point position x")?,
                    parse_doc_f32(parts.next(), "missing spawn point position y")?,
                    parse_doc_f32(parts.next(), "missing spawn point position z")?,
                ),
                facing_yaw_deg: parse_doc_f32(parts.next(), "missing spawn point yaw")?,
            });
        }
        "patrol_route" => {
            slice.patrol_routes.push(PatrolRouteDef {
                name: parse_doc_text(parts.next(), "missing patrol route name")?,
                points: Vec::new(),
                looped: parse_doc_bool01(parts.next(), "missing patrol route looped")?,
            });
        }
        "patrol_point" => {
            let route_name = parse_doc_text(parts.next(), "missing patrol route name")?;
            let route = slice
                .patrol_routes
                .iter_mut()
                .find(|route| route.name == route_name)
                .ok_or("patrol point references unknown route")?;
            route.points.push(Vec3::new(
                parse_doc_f32(parts.next(), "missing patrol point x")?,
                parse_doc_f32(parts.next(), "missing patrol point y")?,
                parse_doc_f32(parts.next(), "missing patrol point z")?,
            ));
        }
        "vehicle_spawner" => {
            slice.vehicle_spawners.push(VehicleSpawnerDef {
                name: parse_doc_text(parts.next(), "missing vehicle spawner name")?,
                mesh: parse_doc_optional_text(parts.next(), "missing vehicle spawner mesh")?,
                material: parse_doc_optional_text(
                    parts.next(),
                    "missing vehicle spawner material",
                )?,
                position: Vec3::new(
                    parse_doc_f32(parts.next(), "missing vehicle spawner position x")?,
                    parse_doc_f32(parts.next(), "missing vehicle spawner position y")?,
                    parse_doc_f32(parts.next(), "missing vehicle spawner position z")?,
                ),
                facing_yaw_deg: parse_doc_f32(parts.next(), "missing vehicle spawner yaw")?,
            });
        }
        "breakable_prop" => {
            slice.breakable_props.push(BreakablePropDef {
                name: parse_doc_text(parts.next(), "missing breakable prop name")?,
                intact_mesh: parse_doc_optional_text(
                    parts.next(),
                    "missing breakable intact mesh",
                )?,
                broken_mesh: parse_doc_optional_text(
                    parts.next(),
                    "missing breakable broken mesh",
                )?,
                material: parse_doc_optional_text(parts.next(), "missing breakable material")?,
                position: Vec3::new(
                    parse_doc_f32(parts.next(), "missing breakable position x")?,
                    parse_doc_f32(parts.next(), "missing breakable position y")?,
                    parse_doc_f32(parts.next(), "missing breakable position z")?,
                ),
                rotation_euler_deg: Vec3::new(
                    parse_doc_f32(parts.next(), "missing breakable rotation x")?,
                    parse_doc_f32(parts.next(), "missing breakable rotation y")?,
                    parse_doc_f32(parts.next(), "missing breakable rotation z")?,
                ),
                scale: Vec3::new(
                    parse_doc_f32(parts.next(), "missing breakable scale x")?,
                    parse_doc_f32(parts.next(), "missing breakable scale y")?,
                    parse_doc_f32(parts.next(), "missing breakable scale z")?,
                ),
                hit_points: parse_doc_f32(parts.next(), "missing breakable hit points")?,
                break_impulse: 0.0,
            });
        }
        "breakable_prop_impulse" => {
            let name = parse_doc_text(parts.next(), "missing breakable impulse name")?;
            let impulse = parse_doc_f32(parts.next(), "missing breakable impulse value")?;
            let prop = slice
                .breakable_props
                .iter_mut()
                .find(|prop| prop.name == name)
                .ok_or("breakable impulse references unknown prop")?;
            prop.break_impulse = impulse;
        }
        _ => return Err("unknown runtime slice line"),
    }

    Ok(())
}

fn encode_optional_text(value: Option<&str>) -> String {
    value.map(escape_text).unwrap_or_default()
}

fn parse_doc_text(value: Option<&str>, missing: &'static str) -> Result<String, &'static str> {
    unescape_text(value.ok_or(missing)?)
}

fn parse_doc_optional_text(
    value: Option<&str>,
    missing: &'static str,
) -> Result<Option<String>, &'static str> {
    let raw = value.ok_or(missing)?;
    if raw.is_empty() {
        Ok(None)
    } else {
        Ok(Some(unescape_text(raw)?))
    }
}

fn parse_doc_f32(value: Option<&str>, missing: &'static str) -> Result<f32, &'static str> {
    value
        .ok_or(missing)?
        .parse::<f32>()
        .map_err(|_| "invalid numeric value")
}

fn parse_doc_u32(value: Option<&str>, missing: &'static str) -> Result<u32, &'static str> {
    value
        .ok_or(missing)?
        .parse::<u32>()
        .map_err(|_| "invalid u32 value")
}

fn parse_doc_u8(value: Option<&str>, missing: &'static str) -> Result<u8, &'static str> {
    value
        .ok_or(missing)?
        .parse::<u8>()
        .map_err(|_| "invalid u8 value")
}

fn parse_doc_bool01(value: Option<&str>, missing: &'static str) -> Result<bool, &'static str> {
    match value.ok_or(missing)? {
        "0" => Ok(false),
        "1" => Ok(true),
        _ => Err("invalid boolean value"),
    }
}

fn escape_text(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '|' => out.push_str("\\p"),
            _ => out.push(ch),
        }
    }
    out
}

fn unescape_text(input: &str) -> Result<String, &'static str> {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }

        let esc = chars.next().ok_or("dangling escape")?;
        match esc {
            '\\' => out.push('\\'),
            'n' => out.push('\n'),
            'r' => out.push('\r'),
            'p' => out.push('|'),
            _ => return Err("invalid escape sequence"),
        }
    }
    Ok(out)
}

#[derive(Debug)]
pub enum EditorIoError {
    Io(std::io::Error),
    Parse(&'static str),
    Scene(SceneError),
    Bridge(SceneBridgeError),
    /// An atomic save failed. The destination file is unchanged and still
    /// holds its previous complete contents, so the caller MUST NOT clear a
    /// dirty flag — the user's work is still only in memory. The variant
    /// names the failing phase and carries the Win32 error code (M11).
    AtomicWrite(AtomicWriteError),
}

impl std::fmt::Display for EditorIoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "editor io error: {e}"),
            Self::Parse(msg) => write!(f, "editor parse error: {msg}"),
            Self::Scene(err) => write!(f, "editor scene error: {err}"),
            Self::Bridge(err) => write!(f, "editor scene bridge error: {err}"),
            Self::AtomicWrite(err) => write!(f, "editor save failed: {err}"),
        }
    }
}

impl std::error::Error for EditorIoError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            Self::Scene(e) => Some(e),
            Self::AtomicWrite(e) => Some(e),
            _ => None,
        }
    }
}

/// Save a project document to its `.hproj` location, atomically and durably.
/// A crash mid-save leaves the previous `.hproj` intact rather than an
/// unopenable truncated one (ADR 0003 Pillar 3).
pub fn save_project(project: &Project) -> Result<(), EditorIoError> {
    atomic_write(
        Path::new(&project.project_file()),
        project.to_hproj_string().as_bytes(),
    )
    .map_err(EditorIoError::AtomicWrite)
}

/// Load a project document from a `.hproj` file path.
pub fn load_project<P: AsRef<Path>>(path: P) -> Result<Project, EditorIoError> {
    let src = fs::read_to_string(path).map_err(EditorIoError::Io)?;
    Project::from_hproj_str(&src).map_err(EditorIoError::Parse)
}

/// Save an editor scene document to a file path, atomically and durably.
///
/// `.hscene` is the only format that round-trips the full-fidelity inspector
/// representation (the `.hsc` bridge drops editor-only components), so a torn
/// write here loses strictly more than a torn `.hsc`.
pub fn save_scene_doc<P: AsRef<Path>>(path: P, scene: &SceneDocument) -> Result<(), EditorIoError> {
    atomic_write(path.as_ref(), scene.to_hscene_string().as_bytes())
        .map_err(EditorIoError::AtomicWrite)
}

/// Load an editor scene document from a file path.
pub fn load_scene_doc<P: AsRef<Path>>(path: P) -> Result<SceneDocument, EditorIoError> {
    let src = fs::read_to_string(path).map_err(EditorIoError::Io)?;
    SceneDocument::from_hscene_str(&src).map_err(EditorIoError::Parse)
}

/// Load a runtime `.hsc` file into the editor-facing scene document.
pub fn load_scene_doc_from_hsc<P: AsRef<Path>>(path: P) -> Result<SceneDocument, EditorIoError> {
    let scene = load_scene(path).map_err(EditorIoError::Scene)?;
    Ok(SceneDocument::from_scene_data(&scene))
}

/// Export an editor scene document back into a runtime `.hsc` file.
pub fn save_scene_doc_to_hsc<P: AsRef<Path>>(
    path: P,
    scene: &SceneDocument,
) -> Result<(), EditorIoError> {
    let runtime_scene = scene.to_scene_data().map_err(EditorIoError::Bridge)?;
    save_scene(path, &runtime_scene).map_err(EditorIoError::AtomicWrite)
}

#[derive(Debug)]
pub enum EditorApplyError {
    MissingField {
        entity_id: u64,
        component: String,
        field: &'static str,
    },
    InvalidFieldType {
        entity_id: u64,
        component: String,
        field: &'static str,
    },
    UnknownParentReference {
        entity_id: u64,
        reference: String,
    },
}

impl std::fmt::Display for EditorApplyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingField {
                entity_id,
                component,
                field,
            } => write!(
                f,
                "editor apply error: entity {} missing {}.{}",
                entity_id, component, field
            ),
            Self::InvalidFieldType {
                entity_id,
                component,
                field,
            } => write!(
                f,
                "editor apply error: entity {} invalid type for {}.{}",
                entity_id, component, field
            ),
            Self::UnknownParentReference {
                entity_id,
                reference,
            } => write!(
                f,
                "editor apply error: entity {} references unknown parent {}",
                entity_id, reference
            ),
        }
    }
}

impl std::error::Error for EditorApplyError {}

#[derive(Debug)]
pub enum EditorPipelineError {
    Io(EditorIoError),
    Apply(EditorApplyError),
}

impl std::fmt::Display for EditorPipelineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "{e}"),
            Self::Apply(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for EditorPipelineError {}

/// Load `.hproj` and `.hscene`, then apply scene entities/components into a world.
pub fn load_and_apply_scene_from_project<P: AsRef<Path>>(
    project_path: P,
    scene_file_name: &str,
    world: &mut World,
) -> Result<Vec<(u64, Entity)>, EditorPipelineError> {
    let project = load_project(project_path).map_err(EditorPipelineError::Io)?;
    let scene_path = format!("{}/{}", project.scenes_dir, scene_file_name);
    let scene = load_scene_doc(scene_path).map_err(EditorPipelineError::Io)?;
    apply_scene_document_to_world(world, &scene).map_err(EditorPipelineError::Apply)
}

/// Load a runtime `.hsc` file through the editor bridge, then apply it to a world.
pub fn load_and_apply_scene_from_hsc<P: AsRef<Path>>(
    scene_path: P,
    world: &mut World,
) -> Result<Vec<(u64, Entity)>, EditorPipelineError> {
    let scene = load_scene_doc_from_hsc(scene_path).map_err(EditorPipelineError::Io)?;
    apply_scene_document_to_world(world, &scene).map_err(EditorPipelineError::Apply)
}

/// Apply a dynamic editor scene document to typed ECS components in `world`.
///
/// Returns a vector that maps editor entity ids to spawned ECS entities.
pub fn apply_scene_document_to_world(
    world: &mut World,
    scene: &SceneDocument,
) -> Result<Vec<(u64, Entity)>, EditorApplyError> {
    let mut map = Vec::with_capacity(scene.entities.len());
    let mut entity_by_id = HashMap::with_capacity(scene.entities.len());
    let mut entity_by_name = HashMap::with_capacity(scene.entities.len());
    let mut pending_parents = Vec::new();

    for editor_entity in &scene.entities {
        let entity = world.spawn();
        map.push((editor_entity.id, entity));
        entity_by_id.insert(editor_entity.id, entity);
        entity_by_name.insert(editor_entity.name.clone(), entity);

        for component in &editor_entity.components {
            match component.type_name.as_str() {
                "Transform" => {
                    let position = required_vec3(
                        editor_entity.id,
                        component,
                        component.type_name.as_str(),
                        "position",
                    )?;
                    let scale = optional_vec3(component, "scale").unwrap_or(Vec3::ONE);
                    let rotation = if has_any_field(
                        component,
                        &["rotation_x", "rotation_y", "rotation_z", "rotation_w"],
                    ) {
                        Quat::new(
                            required_number(
                                editor_entity.id,
                                component,
                                component.type_name.as_str(),
                                "rotation_x",
                            )?,
                            required_number(
                                editor_entity.id,
                                component,
                                component.type_name.as_str(),
                                "rotation_y",
                            )?,
                            required_number(
                                editor_entity.id,
                                component,
                                component.type_name.as_str(),
                                "rotation_z",
                            )?,
                            required_number(
                                editor_entity.id,
                                component,
                                component.type_name.as_str(),
                                "rotation_w",
                            )?,
                        )
                    } else {
                        optional_vec3(component, "rotation_euler_deg")
                            .map(|e| {
                                Quat::from_euler_xyz(
                                    e.x.to_radians(),
                                    e.y.to_radians(),
                                    e.z.to_radians(),
                                )
                            })
                            .unwrap_or(Quat::IDENTITY)
                    };
                    world.add_component(
                        entity,
                        Transform {
                            position,
                            rotation,
                            scale,
                        },
                    );
                    world.add_component(entity, GlobalTransform::identity());
                }
                "Velocity" => {
                    world.add_component(
                        entity,
                        Velocity {
                            linear: optional_vec3(component, "linear").unwrap_or(Vec3::ZERO),
                            angular: optional_vec3(component, "angular").unwrap_or(Vec3::ZERO),
                        },
                    );
                }
                "RigidBody" => {
                    let default = RigidBody::default();
                    world.add_component(
                        entity,
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
                }
                "SphereCollider" => {
                    world.add_component(
                        entity,
                        SphereCollider::new(optional_number(component, "radius").unwrap_or(0.5)),
                    );
                }
                "ParticleEmitter" => {
                    let default = ParticleEmitter::spark_fountain();
                    world.add_component(
                        entity,
                        ParticleEmitter {
                            emitting: optional_number(component, "emitting")
                                .unwrap_or(if default.emitting { 1.0 } else { 0.0 })
                                > 0.0,
                            rate_per_second: optional_number(component, "rate_per_second")
                                .or_else(|| optional_number(component, "rate"))
                                .unwrap_or(default.rate_per_second),
                            max_particles: optional_integer(component, "max_particles")
                                .or_else(|| optional_integer(component, "max"))
                                .map(|v| v.clamp(0, u32::MAX as i64) as u32)
                                .unwrap_or(default.max_particles),
                            spawn_radius: optional_number(component, "spawn_radius")
                                .unwrap_or(default.spawn_radius),
                            direction: optional_vec3(component, "direction")
                                .unwrap_or(default.direction),
                            spread: optional_number(component, "spread").unwrap_or(default.spread),
                            speed_min: optional_number(component, "speed_min")
                                .unwrap_or(default.speed_min),
                            speed_max: optional_number(component, "speed_max")
                                .unwrap_or(default.speed_max),
                            lifetime_min: optional_number(component, "lifetime_min")
                                .unwrap_or(default.lifetime_min),
                            lifetime_max: optional_number(component, "lifetime_max")
                                .unwrap_or(default.lifetime_max),
                            gravity_scale: optional_number(component, "gravity_scale")
                                .unwrap_or(default.gravity_scale),
                            linear_damping: optional_number(component, "linear_damping")
                                .unwrap_or(default.linear_damping),
                            start_size: optional_number(component, "start_size")
                                .unwrap_or(default.start_size),
                            end_size: optional_number(component, "end_size")
                                .unwrap_or(default.end_size),
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
                            accumulator: 0.0,
                            seed: optional_u32(component, "seed", default.seed),
                        },
                    );
                }
                "Stats" => {
                    let health = required_number(
                        editor_entity.id,
                        component,
                        component.type_name.as_str(),
                        "health",
                    )?;
                    let mana = optional_number(component, "mana").unwrap_or(0.0);
                    let attack = optional_number(component, "attack").unwrap_or(0.0);
                    let defense = optional_number(component, "defense").unwrap_or(0.0);
                    let speed = optional_number(component, "speed").unwrap_or(0.0);

                    let mut stats = Stats::new(health, mana, attack, defense, speed);
                    stats.health_max =
                        optional_number(component, "health_max").unwrap_or(stats.health_max);
                    stats.mana_max =
                        optional_number(component, "mana_max").unwrap_or(stats.mana_max);
                    stats.level = optional_number(component, "level")
                        .unwrap_or(stats.level as f32)
                        .max(1.0) as u32;
                    stats.xp = optional_number(component, "xp").unwrap_or(0.0).max(0.0) as u32;
                    stats.xp_next = optional_number(component, "xp_next")
                        .unwrap_or(100.0)
                        .max(1.0) as u32;
                    world.add_component(entity, stats);
                }
                "DirectionalLight" | "Light" => {
                    let direction = required_vec3(
                        editor_entity.id,
                        component,
                        component.type_name.as_str(),
                        "direction",
                    )?;
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
                        entity,
                        DirectionalLight {
                            direction: direction.normalize(),
                            color,
                            intensity,
                            ambient,
                        },
                    );
                }
                "Camera" => {
                    let fov = optional_number(component, "fov_deg").unwrap_or(60.0);
                    let near = optional_number(component, "near").unwrap_or(0.1);
                    let far = optional_number(component, "far").unwrap_or(1000.0);
                    let aspect = optional_number(component, "aspect").unwrap_or(16.0 / 9.0);
                    world.add_component(entity, Camera::perspective(fov, aspect, near, far));
                }
                "ActiveCamera" => {
                    world.add_component(entity, ActiveCamera);
                }
                "Parent" => {
                    pending_parents.push((
                        editor_entity.id,
                        entity,
                        optional_number(component, "entity_id").map(|value| value as u64),
                        optional_text(component, "entity_name"),
                    ));
                }
                _ => {}
            }
        }
    }

    for (child_id, child, parent_id, parent_name) in pending_parents {
        let parent = if let Some(parent_id) = parent_id {
            entity_by_id.get(&parent_id).copied().ok_or(
                EditorApplyError::UnknownParentReference {
                    entity_id: child_id,
                    reference: format!("id {}", parent_id),
                },
            )?
        } else if let Some(parent_name) = parent_name {
            entity_by_name.get(parent_name.as_str()).copied().ok_or(
                EditorApplyError::UnknownParentReference {
                    entity_id: child_id,
                    reference: format!("name '{}'", parent_name),
                },
            )?
        } else {
            return Err(EditorApplyError::MissingField {
                entity_id: child_id,
                component: "Parent".into(),
                field: "entity_id",
            });
        };

        world.set_parent(child, parent);
    }

    Ok(map)
}

fn required_number(
    entity_id: u64,
    component: &InspectorComponent,
    component_name: &str,
    field: &'static str,
) -> Result<f32, EditorApplyError> {
    let value = component
        .get_field(field)
        .ok_or(EditorApplyError::MissingField {
            entity_id,
            component: component_name.to_string(),
            field,
        })?;
    match value {
        InspectorValue::Number(v) => Ok(*v),
        _ => Err(EditorApplyError::InvalidFieldType {
            entity_id,
            component: component_name.to_string(),
            field,
        }),
    }
}

fn required_vec3(
    entity_id: u64,
    component: &InspectorComponent,
    component_name: &str,
    field: &'static str,
) -> Result<Vec3, EditorApplyError> {
    let value = component
        .get_field(field)
        .ok_or(EditorApplyError::MissingField {
            entity_id,
            component: component_name.to_string(),
            field,
        })?;
    match value {
        InspectorValue::Vec3(v) => Ok(Vec3::new(v[0], v[1], v[2])),
        _ => Err(EditorApplyError::InvalidFieldType {
            entity_id,
            component: component_name.to_string(),
            field,
        }),
    }
}

fn optional_number(component: &InspectorComponent, field: &str) -> Option<f32> {
    match component.get_field(field)? {
        InspectorValue::Number(v) => Some(*v),
        InspectorValue::Integer(v) => Some(*v as f32),
        _ => None,
    }
}

/// Read an integer field, accepting either an [`InspectorValue::Integer`]
/// (exact) or a legacy [`InspectorValue::Number`] (a pre-integer-variant
/// `.hscene` on disk stored these as f32; truncate toward zero, matching the
/// old behaviour). Returns `None` if the field is absent or a non-numeric
/// kind.
fn optional_integer(component: &InspectorComponent, field: &str) -> Option<i64> {
    match component.get_field(field)? {
        InspectorValue::Integer(v) => Some(*v),
        InspectorValue::Number(v) => Some(*v as i64),
        _ => None,
    }
}

/// Read an integer field and clamp it into `u32`, falling back to `default`.
/// Used for `u32` component fields (`seed`, `max_particles`).
fn optional_u32(component: &InspectorComponent, field: &str, default: u32) -> u32 {
    match optional_integer(component, field) {
        Some(v) => v.clamp(0, u32::MAX as i64) as u32,
        None => default,
    }
}

fn optional_vec3(component: &InspectorComponent, field: &str) -> Option<Vec3> {
    match component.get_field(field)? {
        InspectorValue::Vec3(v) => Some(Vec3::new(v[0], v[1], v[2])),
        _ => None,
    }
}

fn optional_text(component: &InspectorComponent, field: &str) -> Option<String> {
    match component.get_field(field)? {
        InspectorValue::Text(v) => Some(v.clone()),
        _ => None,
    }
}

fn has_any_field(component: &InspectorComponent, fields: &[&str]) -> bool {
    fields
        .iter()
        .any(|field| component.get_field(field).is_some())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::parse_scene;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_test_dir(suffix: &str) -> String {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        format!(
            "{}/hold_engine_editor_tests_{}_{}",
            std::env::temp_dir().display(),
            suffix,
            nanos
        )
    }

    fn sample_runtime_slice() -> RuntimeSliceAuthoring {
        RuntimeSliceAuthoring {
            terrain: crate::runtime_slice::TerrainAuthoring {
                size_meters: Vec3::new(1024.0, 320.0, 1024.0),
                chunk_size_meters: 64,
                height_scale_meters: 72.0,
                paint_layers: vec![TerrainPaintLayer {
                    name: "sand".into(),
                    material: Some("materials/sand".into()),
                    tiling: 6.0,
                    weightmap_channel: 0,
                }],
            },
            vegetation_prototypes: vec![VegetationPrototypeDef {
                name: "palm".into(),
                mesh: Some("meshes/palm".into()),
                material: Some("materials/palm".into()),
                density_per_100m2: 2.5,
                random_yaw: true,
                align_to_normal: true,
            }],
            vegetation_paints: vec![VegetationPaintStroke {
                prototype: "palm".into(),
                center: Vec3::new(16.0, 0.0, 24.0),
                radius: 8.0,
                density: 1.5,
            }],
            static_mesh_actors: vec![StaticMeshActorDef {
                name: "rock".into(),
                mesh: Some("meshes/rock".into()),
                material: Some("materials/rock".into()),
                position: Vec3::new(2.0, 0.0, 5.0),
                rotation_euler_deg: Vec3::new(0.0, 30.0, 0.0),
                scale: Vec3::new(1.5, 1.5, 1.5),
            }],
            water_volumes: vec![WaterVolumeDef {
                name: "ocean".into(),
                material: Some("materials/ocean".into()),
                center: Vec3::new(512.0, 0.0, 512.0),
                extents: Vec3::new(1024.0, 8.0, 1024.0),
                wave_strength: 0.35,
            }],
            spawn_points: vec![SpawnPointDef {
                name: "player_start".into(),
                kind: SpawnPointKind::Player,
                position: Vec3::new(32.0, 4.0, 32.0),
                facing_yaw_deg: 180.0,
            }],
            patrol_routes: vec![PatrolRouteDef {
                name: "route_a".into(),
                points: vec![Vec3::ZERO, Vec3::new(10.0, 0.0, 10.0)],
                looped: true,
            }],
            vehicle_spawners: vec![VehicleSpawnerDef {
                name: "jeep".into(),
                mesh: Some("meshes/jeep".into()),
                material: Some("materials/jeep".into()),
                position: Vec3::new(40.0, 0.0, 40.0),
                facing_yaw_deg: 90.0,
            }],
            breakable_props: vec![BreakablePropDef {
                name: "crate".into(),
                intact_mesh: Some("meshes/crate".into()),
                broken_mesh: Some("meshes/crate_broken".into()),
                material: Some("materials/crate".into()),
                position: Vec3::new(14.0, 0.0, 20.0),
                rotation_euler_deg: Vec3::ZERO,
                scale: Vec3::ONE,
                hit_points: 40.0,
                break_impulse: 22.0,
            }],
        }
    }

    #[test]
    fn project_layout_paths() {
        let p = Project::new("demo", "C:/games/demo");
        assert_eq!(p.assets_dir, "C:/games/demo/assets");
        assert_eq!(p.scenes_dir, "C:/games/demo/scenes");
        assert_eq!(p.project_file(), "C:/games/demo/demo.hproj");
    }

    #[test]
    fn project_hproj_roundtrip() {
        let p = Project::new("demo", "C:/games/demo");
        let text = p.to_hproj_string();
        let parsed = Project::from_hproj_str(&text).unwrap();
        assert_eq!(parsed, p);
    }

    #[test]
    fn scene_document_set_value_creates_missing_path() {
        let mut doc = SceneDocument {
            name: "Level02".into(),
            runtime_slice: RuntimeSliceAuthoring::default(),
            entities: Vec::new(),
        };

        assert!(doc.set_value(42, "Stats", "health", InspectorValue::Number(100.0)));

        assert_eq!(doc.entities.len(), 1);
        assert_eq!(doc.entities[0].name, "Entity42");
        assert_eq!(
            doc.get_value(42, "Stats", "health"),
            Some(&InspectorValue::Number(100.0))
        );
    }
    #[test]
    fn scene_document_dynamic_access() {
        let mut doc = SceneDocument {
            name: "Level01".into(),
            runtime_slice: RuntimeSliceAuthoring::default(),
            entities: vec![EditorEntity {
                id: 1,
                name: "Player".into(),
                components: vec![InspectorComponent {
                    type_name: "Transform".into(),
                    fields: vec![InspectorField {
                        name: "position".into(),
                        value: InspectorValue::Vec3([0.0, 1.0, 2.0]),
                    }],
                }],
            }],
        };

        assert_eq!(
            doc.get_value(1, "Transform", "position"),
            Some(&InspectorValue::Vec3([0.0, 1.0, 2.0]))
        );

        assert!(doc.set_value(
            1,
            "Transform",
            "position",
            InspectorValue::Vec3([3.0, 4.0, 5.0])
        ));

        assert_eq!(
            doc.get_value(1, "Transform", "position"),
            Some(&InspectorValue::Vec3([3.0, 4.0, 5.0]))
        );
    }

    #[test]
    fn scene_document_roundtrip() {
        let doc = SceneDocument {
            name: "Roundtrip".into(),
            runtime_slice: sample_runtime_slice(),
            entities: vec![EditorEntity {
                id: 7,
                name: "Hero".into(),
                components: vec![
                    InspectorComponent {
                        type_name: "Transform".into(),
                        fields: vec![InspectorField {
                            name: "position".into(),
                            value: InspectorValue::Vec3([1.0, 2.0, 3.0]),
                        }],
                    },
                    InspectorComponent {
                        type_name: "Stats".into(),
                        fields: vec![
                            InspectorField {
                                name: "health".into(),
                                value: InspectorValue::Number(100.0),
                            },
                            InspectorField {
                                name: "alive".into(),
                                value: InspectorValue::Bool(true),
                            },
                            InspectorField {
                                name: "title".into(),
                                value: InspectorValue::Text("Knight".into()),
                            },
                        ],
                    },
                ],
            }],
        };

        let text = doc.to_hscene_string();
        let parsed = SceneDocument::from_hscene_str(&text).unwrap();
        assert_eq!(parsed, doc);
    }

    #[test]
    fn scene_document_hsc_roundtrip_supports_editor_mutation() {
        let source = r#"
scene "Workflow"

entity "player" {
  transform { pos 0 1 0 rot 0 0 0 1 scale 1 1 1 }
  stats { health 100 mana 25 attack 8 defense 4 speed 3 }
}

entity "camera" {
  transform { pos 0 3 8 rot 0 0 0 1 scale 1 1 1 }
  camera { fov 60 near 0.1 far 1000 }
  active_camera
}
"#;

        let mut doc = SceneDocument::from_hsc_str(source).unwrap();
        assert_eq!(doc.name, "Workflow");
        assert_eq!(
            doc.get_value(1, "Stats", "health"),
            Some(&InspectorValue::Number(100.0))
        );

        doc.set_value(1, "Stats", "health", InspectorValue::Number(135.0));
        doc.set_value(
            2,
            "Transform",
            "position",
            InspectorValue::Vec3([4.0, 6.0, 12.0]),
        );

        let exported = doc.to_hsc_string().unwrap();
        let scene = parse_scene(&exported).unwrap();

        assert_eq!(scene.name, "Workflow");
        assert_eq!(scene.entities.len(), 2);
        assert_eq!(scene.entities[0].stats.as_ref().unwrap().health, 135.0);
        assert_eq!(
            scene.entities[1].transform.as_ref().unwrap().position,
            Vec3::new(4.0, 6.0, 12.0)
        );
    }

    #[test]
    fn scene_bridge_roundtrip_preserves_runtime_scene_data() {
        let scene = SceneData {
            name: "Bridge".into(),
            runtime_slice: sample_runtime_slice(),
            entities: vec![
                EntityDef {
                    name: "player".into(),
                    transform: Some(TransformDef {
                        position: Vec3::new(1.0, 2.0, 3.0),
                        rotation: Quat::new(0.0, 0.70710677, 0.0, 0.70710677),
                        scale: Vec3::new(1.0, 1.5, 1.0),
                    }),
                    velocity: Some(VelocityDef {
                        linear: Vec3::new(0.0, 0.5, 0.0),
                        angular: Vec3::new(0.0, 1.25, 0.0),
                    }),
                    rigid_body: Some(RigidBodyDef {
                        gravity_scale: 1.0,
                        linear_damping: 0.2,
                        angular_damping: 0.1,
                        restitution: 0.6,
                    }),
                    sphere_collider: Some(SphereColliderDef { radius: 0.75 }),
                    particle_emitter: None,
                    camera: Some(CameraDef {
                        fov: 75.0,
                        near: 0.2,
                        far: 500.0,
                    }),
                    active_camera: true,
                    light: None,
                    stats: Some(StatsDef {
                        health: 100.0,
                        mana: 50.0,
                        attack: 12.0,
                        defense: 6.0,
                        speed: 4.0,
                    }),
                    parent_name: None,
                },
                EntityDef {
                    name: "sun".into(),
                    transform: None,
                    velocity: None,
                    rigid_body: None,
                    sphere_collider: None,
                    particle_emitter: None,
                    camera: None,
                    active_camera: false,
                    light: Some(LightDef {
                        direction: Vec3::new(0.4, -1.0, 0.2),
                        color: Color::new(1.0, 0.9, 0.8, 0.75),
                        intensity: 1.5,
                        ambient: 0.2,
                    }),
                    stats: None,
                    parent_name: None,
                },
                EntityDef {
                    name: "crate".into(),
                    transform: Some(TransformDef {
                        position: Vec3::new(5.0, 0.0, -3.0),
                        rotation: Quat::IDENTITY,
                        scale: Vec3::ONE,
                    }),
                    velocity: None,
                    rigid_body: None,
                    sphere_collider: None,
                    particle_emitter: Some(ParticleEmitterDef {
                        emitting: true,
                        rate_per_second: 20.0,
                        max_particles: 64,
                        spawn_radius: 0.2,
                        direction: Vec3::Y,
                        spread: 0.3,
                        speed_min: 1.5,
                        speed_max: 2.5,
                        lifetime_min: 0.4,
                        lifetime_max: 0.9,
                        gravity_scale: 0.7,
                        linear_damping: 1.1,
                        start_size: 0.15,
                        end_size: 0.03,
                        start_color: Color::new(1.0, 0.8, 0.2, 1.0),
                        end_color: Color::new(1.0, 0.1, 0.05, 0.0),
                        seed: 7,
                    }),
                    camera: None,
                    active_camera: false,
                    light: None,
                    stats: None,
                    parent_name: Some("player".into()),
                },
            ],
        };

        let doc = SceneDocument::from_scene_data(&scene);
        let roundtrip = doc.to_scene_data().unwrap();

        assert_eq!(roundtrip.name, scene.name);
        assert_eq!(roundtrip.runtime_slice, scene.runtime_slice);
        assert_eq!(roundtrip.entities.len(), 3);

        let player = &roundtrip.entities[0];
        assert_eq!(player.name, "player");
        assert!(player.active_camera);
        let player_transform = player.transform.as_ref().unwrap();
        assert_eq!(player_transform.position, Vec3::new(1.0, 2.0, 3.0));
        assert!((player_transform.rotation.y - 0.70710677).abs() < 1e-5);
        assert_eq!(player.velocity.as_ref().unwrap().angular.y, 1.25);
        assert_eq!(player.sphere_collider.as_ref().unwrap().radius, 0.75);
        assert_eq!(player.stats.as_ref().unwrap().attack, 12.0);

        let sun = &roundtrip.entities[1];
        let light = sun.light.as_ref().unwrap();
        assert!((light.color.a - 0.75).abs() < 1e-5);
        assert!((light.intensity - 1.5).abs() < 1e-5);

        let crate_entity = &roundtrip.entities[2];
        assert_eq!(crate_entity.parent_name.as_deref(), Some("player"));
        assert_eq!(
            crate_entity
                .particle_emitter
                .as_ref()
                .unwrap()
                .max_particles,
            64
        );
    }

    #[test]
    fn scene_document_to_scene_data_deduplicates_names_and_resolves_parent_ids() {
        let doc = SceneDocument {
            name: "Deduped".into(),
            runtime_slice: RuntimeSliceAuthoring::default(),
            entities: vec![
                EditorEntity {
                    id: 1,
                    name: "Entity".into(),
                    components: vec![InspectorComponent {
                        type_name: "Transform".into(),
                        fields: vec![InspectorField {
                            name: "position".into(),
                            value: InspectorValue::Vec3([0.0, 0.0, 0.0]),
                        }],
                    }],
                },
                EditorEntity {
                    id: 2,
                    name: "Entity".into(),
                    components: vec![InspectorComponent {
                        type_name: "Parent".into(),
                        fields: vec![InspectorField {
                            name: "entity_id".into(),
                            value: InspectorValue::Number(1.0),
                        }],
                    }],
                },
            ],
        };

        let scene = doc.to_scene_data().unwrap();

        assert_eq!(scene.entities[0].name, "Entity");
        assert_eq!(scene.entities[1].name, "Entity 2");
        assert_eq!(scene.entities[1].parent_name.as_deref(), Some("Entity"));
    }

    #[test]
    fn project_file_io_roundtrip() {
        let root = unique_test_dir("project");
        fs::create_dir_all(&root).unwrap();

        let project = Project::new("demo", root.clone());
        save_project(&project).unwrap();

        let loaded = load_project(project.project_file()).unwrap();
        assert_eq!(loaded, project);
    }

    #[test]
    fn scene_document_file_io_roundtrip() {
        let root = unique_test_dir("scene");
        fs::create_dir_all(&root).unwrap();
        let scene_path = format!("{root}/level01.hscene");

        let doc = SceneDocument {
            name: "Level01".into(),
            runtime_slice: RuntimeSliceAuthoring::default(),
            entities: vec![EditorEntity {
                id: 1,
                name: "Player".into(),
                components: vec![InspectorComponent {
                    type_name: "Stats".into(),
                    fields: vec![InspectorField {
                        name: "health".into(),
                        value: InspectorValue::Number(100.0),
                    }],
                }],
            }],
        };

        save_scene_doc(&scene_path, &doc).unwrap();
        let loaded = load_scene_doc(&scene_path).unwrap();
        assert_eq!(loaded, doc);
    }

    // ── M11: the sibling authored writers route through atomic_write (review)
    //
    // save_scene (.hsc) has round-trip + typed-error tests in scene.rs.
    // save_project (.hproj) and save_scene_doc (.hscene) go through the same
    // atomic_write primitive. These pin that a smaller document round-trips
    // over a larger one without corruption, and that a failure surfaces a
    // typed EditorIoError::AtomicWrite, not a stringified io::Error.
    //
    // These do NOT by themselves prove atomicity: a plain truncating fs::write
    // would also produce the correct success-path result (File::create
    // truncates, so it leaves no trailing bytes). The atomicity — old-or-new
    // under a crash — is proven in platform::atomic_write's fault-injection
    // suite, which these writers inherit by construction.

    #[test]
    fn save_project_surfaces_typed_atomic_write_error() {
        // A project rooted at a path whose parent directory does not exist:
        // the temp create fails, and the error must be the typed AtomicWrite.
        let root = unique_test_dir("proj_err");
        let missing = format!("{root}/does/not/exist");
        let project = Project::new("demo", missing);
        match save_project(&project) {
            Err(EditorIoError::AtomicWrite(_)) => {}
            other => panic!("expected EditorIoError::AtomicWrite, got {other:?}"),
        }
    }

    #[test]
    fn save_scene_doc_over_larger_file_round_trips_smaller_content() {
        let root = unique_test_dir("doc_replace");
        fs::create_dir_all(&root).unwrap();
        let path = format!("{root}/level.hscene");

        let big = SceneDocument {
            name: "Big".into(),
            runtime_slice: RuntimeSliceAuthoring::default(),
            entities: (0..20)
                .map(|i| EditorEntity {
                    id: i,
                    name: format!("entity_{i}_with_a_long_name"),
                    components: Vec::new(),
                })
                .collect(),
        };
        save_scene_doc(&path, &big).unwrap();

        let small = SceneDocument {
            name: "Small".into(),
            runtime_slice: RuntimeSliceAuthoring::default(),
            entities: Vec::new(),
        };
        save_scene_doc(&path, &small).unwrap();

        // The file now holds exactly `small` — the larger `big` content is
        // fully gone and the smaller document reloads correctly.
        let loaded = load_scene_doc(&path).unwrap();
        assert_eq!(loaded, small);
        assert_eq!(fs::read_to_string(&path).unwrap(), small.to_hscene_string());
    }

    #[test]
    fn save_scene_doc_surfaces_typed_atomic_write_error() {
        let root = unique_test_dir("doc_err");
        fs::create_dir_all(&root).unwrap();
        let path = format!("{root}/missing/level.hscene");
        let doc = SceneDocument {
            name: "X".into(),
            runtime_slice: RuntimeSliceAuthoring::default(),
            entities: Vec::new(),
        };
        match save_scene_doc(&path, &doc) {
            Err(EditorIoError::AtomicWrite(_)) => {}
            other => panic!("expected EditorIoError::AtomicWrite, got {other:?}"),
        }
    }

    #[test]
    fn scene_document_hsc_file_io_roundtrip() {
        let root = unique_test_dir("scene_hsc");
        fs::create_dir_all(&root).unwrap();
        let scene_path = format!("{root}/level01.hsc");

        let doc = SceneDocument {
            name: "Level01".into(),
            runtime_slice: RuntimeSliceAuthoring::default(),
            entities: vec![EditorEntity {
                id: 1,
                name: "Sun".into(),
                components: vec![InspectorComponent {
                    type_name: "DirectionalLight".into(),
                    fields: vec![
                        InspectorField {
                            name: "direction".into(),
                            value: InspectorValue::Vec3([0.3, -1.0, 0.2]),
                        },
                        InspectorField {
                            name: "color".into(),
                            value: InspectorValue::Vec3([1.0, 0.95, 0.8]),
                        },
                        InspectorField {
                            name: "color_alpha".into(),
                            value: InspectorValue::Number(1.0),
                        },
                        InspectorField {
                            name: "intensity".into(),
                            value: InspectorValue::Number(1.0),
                        },
                        InspectorField {
                            name: "ambient".into(),
                            value: InspectorValue::Number(0.15),
                        },
                    ],
                }],
            }],
        };

        save_scene_doc_to_hsc(&scene_path, &doc).unwrap();
        let loaded = load_scene_doc_from_hsc(&scene_path).unwrap();
        assert_eq!(loaded, doc);
    }

    #[test]
    fn serialization_escapes_reserved_characters() {
        let project = Project {
            name: "demo|name".into(),
            root: "C:/games/demo\\test".into(),
            assets_dir: "C:/games/demo/assets".into(),
            scenes_dir: "C:/games/demo/scenes".into(),
        };
        let project_text = project.to_hproj_string();
        let project_loaded = Project::from_hproj_str(&project_text).unwrap();
        assert_eq!(project_loaded, project);

        let doc = SceneDocument {
            name: "Level|One".into(),
            runtime_slice: RuntimeSliceAuthoring::default(),
            entities: vec![EditorEntity {
                id: 9,
                name: "Boss|Unit".into(),
                components: vec![InspectorComponent {
                    type_name: "Dialogue|State".into(),
                    fields: vec![InspectorField {
                        name: "line|1".into(),
                        value: InspectorValue::Text("slash\\\\pipe|newline\nok".into()),
                    }],
                }],
            }],
        };

        let text = doc.to_hscene_string();
        let loaded = SceneDocument::from_hscene_str(&text).unwrap();
        assert_eq!(loaded, doc);
    }

    #[test]
    fn apply_scene_document_to_world_maps_core_components() {
        let scene = SceneDocument {
            name: "Apply".into(),
            runtime_slice: RuntimeSliceAuthoring::default(),
            entities: vec![EditorEntity {
                id: 100,
                name: "Player".into(),
                components: vec![
                    InspectorComponent {
                        type_name: "Transform".into(),
                        fields: vec![
                            InspectorField {
                                name: "position".into(),
                                value: InspectorValue::Vec3([1.0, 2.0, 3.0]),
                            },
                            InspectorField {
                                name: "scale".into(),
                                value: InspectorValue::Vec3([2.0, 2.0, 2.0]),
                            },
                        ],
                    },
                    InspectorComponent {
                        type_name: "Stats".into(),
                        fields: vec![InspectorField {
                            name: "health".into(),
                            value: InspectorValue::Number(120.0),
                        }],
                    },
                ],
            }],
        };

        let mut world = World::new();
        let map = apply_scene_document_to_world(&mut world, &scene).unwrap();
        assert_eq!(map.len(), 1);
        assert_eq!(map[0].0, 100);

        let entity = map[0].1;
        let t = world.get::<Transform>(entity).unwrap();
        assert_eq!(t.position, Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(t.scale, Vec3::new(2.0, 2.0, 2.0));
        assert!(world.has_component::<GlobalTransform>(entity));

        let stats = world.get::<Stats>(entity).unwrap();
        assert_eq!(stats.health, 120.0);
    }

    #[test]
    fn apply_scene_document_to_world_supports_scene_bridge_components() {
        let scene = SceneData {
            name: "ApplyBridge".into(),
            runtime_slice: RuntimeSliceAuthoring::default(),
            entities: vec![
                EntityDef {
                    name: "camera_root".into(),
                    transform: Some(TransformDef {
                        position: Vec3::new(0.0, 3.0, 8.0),
                        rotation: Quat::new(0.0, 0.0, 0.0, 1.0),
                        scale: Vec3::ONE,
                    }),
                    velocity: None,
                    rigid_body: None,
                    sphere_collider: None,
                    particle_emitter: None,
                    camera: Some(CameraDef {
                        fov: 70.0,
                        near: 0.1,
                        far: 250.0,
                    }),
                    active_camera: true,
                    light: None,
                    stats: None,
                    parent_name: None,
                },
                EntityDef {
                    name: "child".into(),
                    transform: Some(TransformDef {
                        position: Vec3::new(1.0, 0.0, 0.0),
                        rotation: Quat::new(0.0, 0.38268343, 0.0, 0.9238795),
                        scale: Vec3::new(2.0, 2.0, 2.0),
                    }),
                    velocity: Some(VelocityDef {
                        linear: Vec3::new(0.0, 0.0, 1.0),
                        angular: Vec3::new(0.0, 1.0, 0.0),
                    }),
                    rigid_body: Some(RigidBodyDef {
                        gravity_scale: 1.0,
                        linear_damping: 0.15,
                        angular_damping: 0.05,
                        restitution: 0.4,
                    }),
                    sphere_collider: Some(SphereColliderDef { radius: 0.5 }),
                    particle_emitter: Some(ParticleEmitterDef {
                        emitting: true,
                        rate_per_second: 12.0,
                        max_particles: 48,
                        spawn_radius: 0.1,
                        direction: Vec3::new(0.0, 1.0, 0.0),
                        spread: 0.2,
                        speed_min: 1.0,
                        speed_max: 2.0,
                        lifetime_min: 0.3,
                        lifetime_max: 0.8,
                        gravity_scale: 0.6,
                        linear_damping: 1.0,
                        start_size: 0.1,
                        end_size: 0.02,
                        start_color: Color::new(0.8, 0.9, 1.0, 1.0),
                        end_color: Color::new(0.3, 0.4, 1.0, 0.0),
                        seed: 11,
                    }),
                    camera: None,
                    active_camera: false,
                    light: Some(LightDef {
                        direction: Vec3::new(0.0, -1.0, 0.0),
                        color: Color::new(0.5, 0.6, 0.7, 0.8),
                        intensity: 2.0,
                        ambient: 0.25,
                    }),
                    stats: Some(StatsDef {
                        health: 80.0,
                        mana: 10.0,
                        attack: 5.0,
                        defense: 2.0,
                        speed: 1.0,
                    }),
                    parent_name: Some("camera_root".into()),
                },
            ],
        };

        let doc = SceneDocument::from_scene_data(&scene);
        let mut world = World::new();
        let map = apply_scene_document_to_world(&mut world, &doc).unwrap();
        let id_to_entity: HashMap<u64, Entity> = map.into_iter().collect();

        let root = id_to_entity[&1];
        assert!(world.has_component::<GlobalTransform>(root));
        assert!(world.has_component::<Camera>(root));
        assert!(world.has_component::<ActiveCamera>(root));

        let child = id_to_entity[&2];
        let transform = world.get::<Transform>(child).unwrap();
        assert!((transform.rotation.y - 0.38268343).abs() < 1e-5);
        assert!(world.has_component::<GlobalTransform>(child));
        assert!(world.has_component::<Velocity>(child));
        assert!(world.has_component::<RigidBody>(child));
        assert!(world.has_component::<SphereCollider>(child));
        assert!(world.has_component::<ParticleEmitter>(child));
        assert!(world.has_component::<DirectionalLight>(child));
        assert!(world.has_component::<Stats>(child));

        let parent = world.get::<crate::Parent>(child).unwrap();
        assert_eq!(parent.entity, root);
    }

    #[test]
    fn load_and_apply_scene_from_project_pipeline() {
        let root = unique_test_dir("pipeline");
        let scenes_dir = format!("{root}/scenes");
        fs::create_dir_all(&scenes_dir).unwrap();

        let project = Project::new("demo", root.clone());
        save_project(&project).unwrap();

        let scene = SceneDocument {
            name: "Level01".into(),
            runtime_slice: RuntimeSliceAuthoring::default(),
            entities: vec![EditorEntity {
                id: 1,
                name: "Cam".into(),
                components: vec![
                    InspectorComponent {
                        type_name: "Transform".into(),
                        fields: vec![InspectorField {
                            name: "position".into(),
                            value: InspectorValue::Vec3([0.0, 1.0, 5.0]),
                        }],
                    },
                    InspectorComponent {
                        type_name: "Camera".into(),
                        fields: vec![InspectorField {
                            name: "fov_deg".into(),
                            value: InspectorValue::Number(75.0),
                        }],
                    },
                ],
            }],
        };

        let scene_file = "level01.hscene";
        let scene_path = format!("{}/{}", project.scenes_dir, scene_file);
        save_scene_doc(scene_path, &scene).unwrap();

        let mut world = World::new();
        let map = load_and_apply_scene_from_project(project.project_file(), scene_file, &mut world)
            .unwrap();
        assert_eq!(map.len(), 1);
        let entity = map[0].1;
        assert!(world.has_component::<Transform>(entity));
        assert!(world.has_component::<Camera>(entity));
    }

    #[test]
    fn load_and_apply_scene_from_hsc_pipeline() {
        let root = unique_test_dir("pipeline_hsc");
        fs::create_dir_all(&root).unwrap();
        let scene_path = format!("{root}/level01.hsc");

        let source = r#"
scene "Pipeline"

entity "camera" {
  transform { pos 0 2 10 rot 0 0 0 1 scale 1 1 1 }
  camera { fov 75 near 0.5 far 300 }
  active_camera
}

entity "sun" {
  light { dir 0.4 -1 0.3 color 1 1 1 1 intensity 1.2 ambient 0.2 }
}
"#;

        fs::write(&scene_path, source).unwrap();

        let mut world = World::new();
        let map = load_and_apply_scene_from_hsc(&scene_path, &mut world).unwrap();
        assert_eq!(map.len(), 2);

        let id_to_entity: HashMap<u64, Entity> = map.into_iter().collect();
        let camera = id_to_entity[&1];
        assert!(world.has_component::<Transform>(camera));
        assert!(world.has_component::<Camera>(camera));
        assert!(world.has_component::<ActiveCamera>(camera));

        let sun = id_to_entity[&2];
        assert!(world.has_component::<DirectionalLight>(sun));
    }

    // ── M11: .hscene truncation detection ────────────────────────────────

    fn sample_doc() -> SceneDocument {
        SceneDocument {
            name: "Doc".into(),
            runtime_slice: RuntimeSliceAuthoring::default(),
            entities: vec![
                EditorEntity {
                    id: 1,
                    name: "a".into(),
                    components: vec![InspectorComponent {
                        type_name: "Transform".into(),
                        fields: vec![InspectorField {
                            name: "position".into(),
                            value: InspectorValue::Vec3([1.0, 2.0, 3.0]),
                        }],
                    }],
                },
                EditorEntity {
                    id: 2,
                    name: "b".into(),
                    components: Vec::new(),
                },
            ],
        }
    }

    #[test]
    fn hscene_doc_carries_v2_header_and_terminator_and_round_trips() {
        let doc = sample_doc();
        let text = doc.to_hscene_string();
        assert!(text.starts_with(HSCENE_DOC_HEADER_V2));
        assert!(text.trim_end().ends_with(HSCENE_DOC_END));
        assert_eq!(SceneDocument::from_hscene_str(&text).unwrap(), doc);
    }

    /// A V2 document cut on an `endentity` boundary — every record well-formed,
    /// so the old parser accepted it as a smaller scene — is now an error.
    #[test]
    fn truncated_hscene_doc_is_an_error_not_a_smaller_doc() {
        let doc = sample_doc();
        let text = doc.to_hscene_string();

        // Cut right after the FIRST entity's `endentity`, dropping entity "b".
        let cut = text.find("endentity\n").unwrap() + "endentity\n".len();
        let truncated = &text[..cut];

        let err = SceneDocument::from_hscene_str(truncated).unwrap_err();
        assert!(err.contains("truncated"), "got: {err}");

        // The hazard, demonstrated: the same bytes under the V1 header parse
        // as a valid but silently smaller document.
        let legacy = truncated.replacen(HSCENE_DOC_HEADER_V2, HSCENE_DOC_HEADER_V1, 1);
        let smaller = SceneDocument::from_hscene_str(&legacy).unwrap();
        assert_eq!(smaller.entities.len(), 1, "entity 'b' silently vanished");
    }

    /// Pre-M11 V1 documents still load, terminator or not.
    #[test]
    fn legacy_v1_hscene_doc_still_parses() {
        let doc = sample_doc();
        let v1 = doc
            .to_hscene_string()
            .replacen(HSCENE_DOC_HEADER_V2, HSCENE_DOC_HEADER_V1, 1)
            .replace(&format!("{HSCENE_DOC_END}\n"), "");
        let parsed = SceneDocument::from_hscene_str(&v1).unwrap();
        assert_eq!(parsed.entities.len(), 2);
    }

    #[test]
    fn content_after_hscene_terminator_is_rejected() {
        let mut text = sample_doc().to_hscene_string();
        text.push_str("entity|9|sneaky\nendentity\n");
        let err = SceneDocument::from_hscene_str(&text).unwrap_err();
        assert!(err.contains("content after"), "got: {err}");
    }

    // ── InspectorValue::Integer — exact integer fields ───────────────────
    //
    // `seed` / `max_particles` are u32 component fields. The pre-integer
    // editor stored them as `InspectorValue::Number(f32)`, which silently
    // corrupts any value past f32's 2^24 exact-integer ceiling — a
    // determinism hazard, since a scene's RNG seed must round-trip
    // byte-for-byte (ADR 0005). These tests pin the exact-integer guarantee,
    // the fractional-rejection guarantee, and the legacy-`number` back-compat
    // read path.

    /// A u32 seed that is a genuine f32 hazard: it does NOT survive a trip
    /// through f32. Each test below asserts this precondition up front — if it
    /// ever stopped holding, the tests would quietly stop discriminating.
    const HAZARD_SEED: u32 = 0xDEAD_BEEF; // 3_735_928_559, > 2^24

    #[test]
    fn integer_field_codec_round_trips_f32_hazard_exactly() {
        assert_ne!(
            (HAZARD_SEED as f32) as u32,
            HAZARD_SEED,
            "seed must be an f32 hazard or this test proves nothing"
        );

        let field = integer_field("seed", HAZARD_SEED as i64);
        let line = encode_field(&field);
        let rest = line.strip_prefix("field|").expect("encoded field| prefix");
        let decoded = decode_field(rest).unwrap();

        assert_eq!(decoded, field);
        assert_eq!(decoded.value, InspectorValue::Integer(HAZARD_SEED as i64));
    }

    /// The codec REJECTS a fractional literal in an integer field rather than
    /// truncating it. The silent-truncation bug (`2.5 as u32 == 2`) that this
    /// whole variant exists to kill is structurally impossible on this path.
    #[test]
    fn integer_field_codec_rejects_fractional_and_garbage_literals() {
        assert!(decode_field("seed|integer|2.5").is_err());
        assert!(decode_field("seed|integer|not_a_number").is_err());
        // A well-formed integer still parses, as a control.
        assert!(decode_field("seed|integer|42").is_ok());
    }

    /// A large seed survives the full component → `ParticleEmitterDef`
    /// conversion exactly when carried as an Integer. The same value carried
    /// as a legacy `Number` is corrupted — the precise reason the Integer
    /// variant exists.
    #[test]
    fn particle_emitter_seed_survives_conversion_when_integer() {
        assert_ne!((HAZARD_SEED as f32) as u32, HAZARD_SEED);

        let exact = InspectorComponent {
            type_name: "ParticleEmitter".into(),
            fields: vec![
                integer_field("seed", HAZARD_SEED as i64),
                integer_field("max_particles", 20_000_001),
            ],
        };
        let def = bridge_particle_emitter(1, &exact).unwrap();
        assert_eq!(def.seed, HAZARD_SEED, "integer seed must be exact");
        assert_eq!(def.max_particles, 20_000_001);

        // Legacy Number path: still parses (back-compat) but is lossy —
        // demonstrating the hazard the Integer path closes.
        let legacy = InspectorComponent {
            type_name: "ParticleEmitter".into(),
            fields: vec![number_field("seed", HAZARD_SEED as f32)],
        };
        let legacy_def = bridge_particle_emitter(1, &legacy).unwrap();
        assert_ne!(
            legacy_def.seed, HAZARD_SEED,
            "an f32-backed seed is expected to be corrupted"
        );
    }

    /// A pre-integer `.hscene` stored `seed` as `field|seed|number|<f32>`.
    /// Such a file must still load: the reader accepts the legacy `Number`
    /// kind. Values within f32's exact range round-trip exactly; only >2^24
    /// seeds were ever at risk, and those are what the `integer` kind fixes
    /// going forward.
    #[test]
    fn legacy_number_seed_field_still_reads() {
        let legacy = InspectorComponent {
            type_name: "ParticleEmitter".into(),
            fields: vec![number_field("seed", 12_345.0)],
        };
        let def = bridge_particle_emitter(1, &legacy).unwrap();
        assert_eq!(def.seed, 12_345);
    }

    /// The clamp guards in the integer read helpers saturate rather than wrap:
    /// a negative value floors at 0 and an above-`u32::MAX` value ceilings at
    /// `u32::MAX`. These bounds are unreachable through the authored formats
    /// (the `.hsc` parser rejects them outright), but the helpers are the
    /// safety net for a hand-edited `.hscene` or a wire-supplied value.
    #[test]
    fn integer_read_helpers_saturate_out_of_u32_range() {
        let comp = InspectorComponent {
            type_name: "ParticleEmitter".into(),
            fields: vec![
                integer_field("seed", -5),
                integer_field("max_particles", u32::MAX as i64 + 1000),
            ],
        };
        let def = bridge_particle_emitter(1, &comp).unwrap();
        assert_eq!(def.seed, 0, "a negative seed floors at 0");
        assert_eq!(
            def.max_particles,
            u32::MAX,
            "an oversized count ceilings at u32::MAX rather than wrapping"
        );
    }

    /// End-to-end: a `.hscene` document carrying an integer `seed` field
    /// round-trips through the on-disk text codec with the exact value intact.
    #[test]
    fn hscene_doc_round_trips_integer_seed_field_exactly() {
        assert_ne!((HAZARD_SEED as f32) as u32, HAZARD_SEED);

        let doc = SceneDocument {
            name: "Fx".into(),
            runtime_slice: RuntimeSliceAuthoring::default(),
            entities: vec![EditorEntity {
                id: 1,
                name: "sparks".into(),
                components: vec![InspectorComponent {
                    type_name: "ParticleEmitter".into(),
                    fields: vec![integer_field("seed", HAZARD_SEED as i64)],
                }],
            }],
        };

        let text = doc.to_hscene_string();
        let parsed = SceneDocument::from_hscene_str(&text).unwrap();
        assert_eq!(parsed, doc);
        assert_eq!(
            parsed.entities[0].components[0].get_field("seed"),
            Some(&InspectorValue::Integer(HAZARD_SEED as i64)),
        );
    }

    // ── M11: autosave scheduling (pure timing, no filesystem) ────────────

    #[test]
    fn recovery_path_is_a_hidden_sidecar_next_to_the_scene() {
        let p = recovery_path_for_scene(Path::new("C:\\proj\\levels\\forest.hsc")).unwrap();
        assert_eq!(
            p,
            Path::new("C:\\proj\\levels")
                .join(RECOVERY_DIR)
                .join("forest.hsc.autosave")
        );
    }

    fn sched(idle: f32, floor: f32) -> AutosaveScheduler {
        AutosaveScheduler::new(idle, floor, 5.0)
    }

    /// A saved scene never autosaves, however long it sits.
    #[test]
    fn scheduler_never_fires_when_no_unsaved_work() {
        let mut s = AutosaveScheduler::default();
        for _ in 0..1000 {
            assert!(!s.tick(1.0, false, true, 0));
        }
    }

    /// Debounce: an edit does not fire immediately; it fires once the author
    /// has been idle for the debounce window.
    #[test]
    fn scheduler_fires_after_idle_debounce() {
        let mut s = sched(3.0, 60.0);
        assert!(!s.tick(0.1, true, true, 1), "must not fire immediately on edit");
        let mut t = 0.1;
        while t < 2.9 {
            assert!(!s.tick(0.1, true, true, 1), "fired too early at t={t}");
            t += 0.1;
        }
        assert!(s.tick(0.5, true, true, 1), "must fire after the idle debounce");
    }

    /// Continuous editing keeps restarting the debounce, so the idle rule
    /// alone would never fire — the interval floor guarantees a checkpoint.
    #[test]
    fn scheduler_interval_floor_fires_during_continuous_editing() {
        let mut s = sched(3.0, 10.0);
        let mut rev = 0u64;
        let mut fired_at = None;
        for i in 1..=200 {
            rev += 1; // an edit every frame: the idle timer never matures
            if s.tick(0.1, true, true, rev) {
                fired_at = Some(i as f32 * 0.1);
                break;
            }
        }
        let t = fired_at.expect("interval floor must fire during continuous editing");
        assert!(
            (9.9..=10.2).contains(&t),
            "floor should fire near the 10s bound, fired at {t}"
        );
    }

    /// After a checkpoint (note_saved advances the watermark), an idle scene
    /// with unsaved work is not rewritten again — only a new edit re-arms it.
    #[test]
    fn scheduler_does_not_refire_without_a_new_edit() {
        let mut s = sched(3.0, 60.0);
        assert!(!s.tick(0.1, true, true, 1));
        assert!(s.tick(5.0, true, true, 1), "first fire");
        s.note_saved();

        // Still unsaved relative to the SOURCE, but the autosave watermark is
        // at rev 1, so no new checkpoint until a new edit.
        for _ in 0..1000 {
            assert!(!s.tick(1.0, true, true, 1), "re-fired without a new edit");
        }
        assert!(!s.tick(0.1, true, true, 2));
        assert!(s.tick(5.0, true, true, 2), "a new edit must re-arm the scheduler");
    }

    /// Going saved advances the watermark, so reopening a dirty state starts a
    /// fresh debounce rather than firing instantly.
    #[test]
    fn scheduler_advances_watermark_when_scene_becomes_saved() {
        let mut s = sched(3.0, 60.0);
        assert!(!s.tick(0.1, true, true, 1));
        s.tick(2.0, true, true, 1); // partway through the debounce
        assert!(!s.tick(0.1, false, true, 1), "saved scene must not fire");
        assert!(!s.tick(0.1, true, true, 2));
        assert!(!s.tick(2.0, true, true, 2), "debounce must have restarted");
        assert!(s.tick(1.5, true, true, 2));
    }

    /// Regression (review #2): an unsaved edit made just before entering play
    /// mode must still autosave after returning to Editing. During play,
    /// can_autosave is false and the revision does not change, but the
    /// unsaved-work watermark keeps the edit pending — so it fires on return.
    #[test]
    fn scheduler_checkpoints_pre_play_edit_after_a_play_cycle() {
        let mut s = sched(3.0, 60.0);
        // Edit in Editing, but enter play before the debounce matures.
        assert!(!s.tick(0.1, true, true, 1));
        // Play: has_unsaved_work stays TRUE (raw dirty), can_autosave FALSE.
        for _ in 0..600 {
            assert!(
                !s.tick(0.1, true, false, 1),
                "must never checkpoint during play"
            );
        }
        // Back to Editing: the pending pre-play edit checkpoints promptly.
        assert!(
            s.tick(0.1, true, true, 1),
            "the pre-play edit must autosave once Editing resumes"
        );
    }

    /// Regression (review #1): a failing autosave does not re-fire every frame.
    /// After note_failed the scheduler backs off, then retries.
    #[test]
    fn scheduler_backs_off_after_a_failed_autosave() {
        let mut s = AutosaveScheduler::new(3.0, 60.0, 5.0);
        assert!(!s.tick(0.1, true, true, 1));
        assert!(s.tick(5.0, true, true, 1), "first fire");
        s.note_failed();

        // Within the back-off window: no re-fire, even though work is pending.
        for _ in 0..8 {
            assert!(!s.tick(0.5, true, true, 1), "must back off, not re-fire");
        }
        // The tick that drains the last of the back-off still returns false...
        assert!(!s.tick(2.0, true, true, 1), "the draining tick does not itself fire");
        // ...and the next one retries.
        assert!(s.tick(0.1, true, true, 1), "must retry after the back-off");
    }

    /// After a fire, the scheduler will not fire again until the caller reports
    /// the outcome (note_saved / note_failed) — no double-fire in one attempt.
    #[test]
    fn scheduler_holds_until_outcome_reported() {
        let mut s = sched(3.0, 60.0);
        assert!(!s.tick(0.1, true, true, 1));
        assert!(s.tick(5.0, true, true, 1), "fires");
        // No outcome reported yet: stays quiet.
        for _ in 0..100 {
            assert!(!s.tick(1.0, true, true, 1), "must not re-fire before outcome");
        }
        s.note_saved();
        assert!(!s.tick(1.0, true, true, 1), "quiet after checkpoint");
    }
}
