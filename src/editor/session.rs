use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::ecs::components::{
    ActiveCamera, Camera, DirectionalLight, GlobalTransform, Parent, Stats, Transform,
};
use crate::ecs::{Entity, World};
use crate::math::{Quat, Vec3};
use crate::renderer::Color;

use super::{
    apply_scene_document_to_world, has_any_field, load_scene_doc_from_hsc, optional_number,
    optional_text, optional_vec3, required_number, required_vec3, save_scene_doc_to_hsc,
    EditorApplyError, EditorEntity, EditorIoError, InspectorComponent, InspectorValue,
    SceneDocument,
};

/// Runtime state for a headless Play-in-Editor session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorPlayState {
    Editing,
    Playing,
    Paused,
}

/// Stateful editor-side controller for opening, mutating, saving, and applying a scene.
///
/// This is intentionally UI-agnostic: a native editor, browser bridge, CLI tool, or test host can
/// all drive the same scene workflow without reaching directly into demo-only glue code.
#[derive(Debug, Clone)]
pub struct EditorSceneSession {
    source_path: Option<PathBuf>,
    authored_scene: SceneDocument,
    scene: SceneDocument,
    selected_entity_id: Option<u64>,
    last_world_map: Vec<(u64, Entity)>,
    play_state: EditorPlayState,
}

impl EditorSceneSession {
    pub fn new(scene: SceneDocument) -> Self {
        Self {
            source_path: None,
            authored_scene: scene.clone(),
            scene,
            selected_entity_id: None,
            last_world_map: Vec::new(),
            play_state: EditorPlayState::Editing,
        }
    }

    pub fn open_hsc<P: AsRef<Path>>(path: P) -> Result<Self, EditorSessionError> {
        let path = path.as_ref();
        let scene = load_scene_doc_from_hsc(path).map_err(EditorSessionError::Io)?;
        Ok(Self {
            source_path: Some(path.to_path_buf()),
            authored_scene: scene.clone(),
            scene,
            selected_entity_id: None,
            last_world_map: Vec::new(),
            play_state: EditorPlayState::Editing,
        })
    }

    pub fn source_path(&self) -> Option<&Path> {
        self.source_path.as_deref()
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
    use crate::{ActiveCamera, Camera, Transform, Vec3};
    use std::time::{SystemTime, UNIX_EPOCH};

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
}
