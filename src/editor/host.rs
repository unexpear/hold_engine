use std::path::Path;

use crate::ecs::{Entity, World};

use super::{
    EditorPlayState, EditorSceneSession, EditorSessionError, InspectorValue, SceneDocument,
};

/// Headless editor/runtime host for the authoritative editor scene workflow.
///
/// A native UI shell can sit on top of this without re-implementing scene loading,
/// inspector mutation, play-mode transitions, or world synchronization.
pub struct EditorRuntimeHost {
    session: EditorSceneSession,
    world: World,
}

impl EditorRuntimeHost {
    pub fn new(session: EditorSceneSession) -> Result<Self, EditorSessionError> {
        let mut host = Self {
            session,
            world: World::new(),
        };
        host.rebuild_world()?;
        Ok(host)
    }

    pub fn from_scene(scene: SceneDocument) -> Result<Self, EditorSessionError> {
        Self::new(EditorSceneSession::new(scene))
    }

    pub fn open_hsc<P: AsRef<Path>>(path: P) -> Result<Self, EditorSessionError> {
        Self::new(EditorSceneSession::open_hsc(path)?)
    }

    pub fn session(&self) -> &EditorSceneSession {
        &self.session
    }

    pub fn session_mut(&mut self) -> &mut EditorSceneSession {
        &mut self.session
    }

    pub fn world(&self) -> &World {
        &self.world
    }

    pub fn world_mut(&mut self) -> &mut World {
        &mut self.world
    }

    pub fn play_state(&self) -> EditorPlayState {
        self.session.play_state()
    }

    pub fn selected_entity_id(&self) -> Option<u64> {
        self.session.selected_entity_id()
    }

    pub fn select_entity(&mut self, entity_id: u64) -> Result<u64, EditorSessionError> {
        self.session.select_entity(entity_id)
    }

    pub fn select_first_with_component(
        &mut self,
        component: &str,
    ) -> Result<u64, EditorSessionError> {
        self.session.select_first_with_component(component)
    }

    pub fn set_selected_value(
        &mut self,
        component: &str,
        field: &str,
        value: InspectorValue,
    ) -> Result<(), EditorSessionError> {
        self.session.set_selected_value(component, field, value)
    }

    pub fn selected_runtime_entity(&self) -> Result<Entity, EditorSessionError> {
        let editor_entity_id = self
            .selected_entity_id()
            .ok_or(EditorSessionError::NoSelection)?;
        self.session.applied_entity(editor_entity_id)
    }

    pub fn applied_entity(&self, editor_entity_id: u64) -> Result<Entity, EditorSessionError> {
        self.session.applied_entity(editor_entity_id)
    }

    pub fn enter_play_mode(&mut self) -> Result<(), EditorSessionError> {
        self.session.enter_play_mode(&mut self.world)
    }

    pub fn pause_play_mode(&mut self) -> Result<(), EditorSessionError> {
        self.session.pause_play_mode()
    }

    pub fn stop_play_mode(&mut self) -> Result<(), EditorSessionError> {
        self.session.stop_play_mode(&mut self.world)
    }

    pub fn rebuild_world(&mut self) -> Result<&[(u64, Entity)], EditorSessionError> {
        self.session.rebuild_world(&mut self.world)
    }

    pub fn patch_world(&mut self) -> Result<(), EditorSessionError> {
        self.session.patch_world(&mut self.world)
    }

    pub fn save_hsc<P: AsRef<Path>>(&self, path: P) -> Result<(), EditorSessionError> {
        self.session.save_hsc(path)
    }

    pub fn save_source_hsc(&self) -> Result<(), EditorSessionError> {
        self.session.save_source_hsc()
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
            "{}/hold_engine_editor_host_tests_{}_{}",
            std::env::temp_dir().display(),
            suffix,
            nanos
        )
    }

    fn write_host_test_scene(suffix: &str) -> String {
        let root = unique_test_dir(suffix);
        std::fs::create_dir_all(&root).unwrap();
        let input_path = format!("{root}/host_level.hsc");

        std::fs::write(
            &input_path,
            r#"
scene "Host"

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
    fn host_opens_scene_and_rebuilds_runtime_world() {
        let input_path = write_host_test_scene("open");
        let mut host = EditorRuntimeHost::open_hsc(&input_path).unwrap();
        let camera_id = host.select_first_with_component("Camera").unwrap();
        let runtime_camera = host.applied_entity(camera_id).unwrap();

        assert!(host.world().has_component::<Transform>(runtime_camera));
        assert!(host.world().has_component::<Camera>(runtime_camera));
        assert!(host.world().has_component::<ActiveCamera>(runtime_camera));
    }

    #[test]
    fn host_mutates_and_patches_live_world() {
        let input_path = write_host_test_scene("patch");
        let mut host = EditorRuntimeHost::open_hsc(&input_path).unwrap();
        let camera_id = host.select_first_with_component("Camera").unwrap();
        let runtime_camera = host.applied_entity(camera_id).unwrap();

        host.set_selected_value("Camera", "fov_deg", InspectorValue::Number(75.0))
            .unwrap();
        host.set_selected_value(
            "Transform",
            "position",
            InspectorValue::Vec3([3.0, 5.0, 9.0]),
        )
        .unwrap();
        host.patch_world().unwrap();

        assert_eq!(host.applied_entity(camera_id).unwrap(), runtime_camera);
        let transform = host.world().get::<Transform>(runtime_camera).unwrap();
        assert_eq!(transform.position, Vec3::new(3.0, 5.0, 9.0));
        let camera = host.world().get::<Camera>(runtime_camera).unwrap();
        assert!((camera.fov.to_degrees() - 75.0).abs() < 0.001);
    }

    #[test]
    fn host_play_pause_stop_restores_authored_scene() {
        let input_path = write_host_test_scene("play");
        let mut host = EditorRuntimeHost::open_hsc(&input_path).unwrap();
        let camera_id = host.select_first_with_component("Camera").unwrap();

        host.enter_play_mode().unwrap();
        assert_eq!(host.play_state(), EditorPlayState::Playing);
        host.set_selected_value("Camera", "fov_deg", InspectorValue::Number(95.0))
            .unwrap();
        host.pause_play_mode().unwrap();
        assert_eq!(host.play_state(), EditorPlayState::Paused);

        host.stop_play_mode().unwrap();
        assert_eq!(host.play_state(), EditorPlayState::Editing);
        assert_eq!(
            host.session()
                .scene()
                .get_value(camera_id, "Camera", "fov_deg"),
            Some(&InspectorValue::Number(60.0))
        );
    }
}
