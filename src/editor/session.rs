use std::path::{Path, PathBuf};

use crate::ecs::{Entity, World};

use super::{
    apply_scene_document_to_world, load_scene_doc_from_hsc, save_scene_doc_to_hsc,
    EditorApplyError, EditorEntity, EditorIoError, InspectorValue, SceneDocument,
};

/// Stateful editor-side controller for opening, mutating, saving, and applying a scene.
///
/// This is intentionally UI-agnostic: a native editor, browser bridge, CLI tool, or test host can
/// all drive the same scene workflow without reaching directly into demo-only glue code.
#[derive(Debug, Clone)]
pub struct EditorSceneSession {
    source_path: Option<PathBuf>,
    scene: SceneDocument,
    selected_entity_id: Option<u64>,
    last_world_map: Vec<(u64, Entity)>,
}

impl EditorSceneSession {
    pub fn new(scene: SceneDocument) -> Self {
        Self {
            source_path: None,
            scene,
            selected_entity_id: None,
            last_world_map: Vec::new(),
        }
    }

    pub fn open_hsc<P: AsRef<Path>>(path: P) -> Result<Self, EditorSessionError> {
        let path = path.as_ref();
        let scene = load_scene_doc_from_hsc(path).map_err(EditorSessionError::Io)?;
        Ok(Self {
            source_path: Some(path.to_path_buf()),
            scene,
            selected_entity_id: None,
            last_world_map: Vec::new(),
        })
    }

    pub fn source_path(&self) -> Option<&Path> {
        self.source_path.as_deref()
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
        }
    }
}

impl std::error::Error for EditorSessionError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ActiveCamera, Camera, Transform};
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

    #[test]
    fn session_drives_hsc_mutate_save_apply_workflow() {
        let root = unique_test_dir("hsc");
        std::fs::create_dir_all(&root).unwrap();
        let input_path = format!("{root}/level01.hsc");
        let output_path = format!("{root}/level01.edited.hsc");

        std::fs::write(
            &input_path,
            r#"
scene "Session"

entity "camera" {
  transform { pos 0 2 10 rot 0 0 0 1 scale 1 1 1 }
  camera { fov 60 near 0.1 far 1000 }
  active_camera
}
"#,
        )
        .unwrap();

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
}
