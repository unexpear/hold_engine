//! Native bridge server for the shared editor command/event surface.
//!
//! This keeps transport concerns out of the UI shell while still letting the
//! shell, tests, and future external clients talk to one authoritative editor
//! core.

use std::path::Path;

use super::{
    encode_editor_event_protocol, EditorCommand, EditorCommandError, EditorEvent,
    EditorProtocolError, EditorRuntimeHost, EditorSessionError,
};

/// Same-process bridge facade over [`EditorRuntimeHost`].
///
/// The native editor shell can use this directly, while external clients can
/// drive the same core via [`execute_message`](Self::execute_message).
pub struct EditorBridgeServer {
    host: EditorRuntimeHost,
}

impl EditorBridgeServer {
    pub fn new(host: EditorRuntimeHost) -> Self {
        Self { host }
    }

    pub fn open_hsc<P: AsRef<Path>>(path: P) -> Result<Self, EditorSessionError> {
        Ok(Self::new(EditorRuntimeHost::open_hsc(path)?))
    }

    pub fn host(&self) -> &EditorRuntimeHost {
        &self.host
    }

    pub fn host_mut(&mut self) -> &mut EditorRuntimeHost {
        &mut self.host
    }

    /// Initial event snapshot for a newly opened scene.
    ///
    /// Consumers get a deterministic starting state without needing a special
    /// out-of-band API for selection/current-world status.
    pub fn startup_events(&self) -> Vec<EditorEvent> {
        let mut events = vec![EditorEvent::WorldRebuilt];
        if let Some(cursor) = self.host.cursor() {
            events.push(EditorEvent::SelectionChanged(cursor));
        }
        events
    }

    pub fn startup_messages(&self) -> Vec<String> {
        self.startup_events()
            .iter()
            .map(encode_editor_event_protocol)
            .collect()
    }

    pub fn execute(
        &mut self,
        command: EditorCommand,
    ) -> Result<Vec<EditorEvent>, EditorCommandError> {
        self.host.execute(command)
    }

    pub fn execute_message(&mut self, message: &str) -> Result<Vec<String>, EditorProtocolError> {
        self.host.execute_protocol_message(message)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::editor::decode_editor_event_protocol;

    fn write_scene_file(label: &str) -> PathBuf {
        let unique = format!(
            "hold_bridge_{}_{}_{}.hsc",
            label,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let path = std::env::temp_dir().join(unique);
        std::fs::write(
            &path,
            r#"scene "Bridge Test"
entity "camera" {
  transform {
    pos 0 5 0
    rot 0 0 0 1
    scale 1 1 1
  }
  camera { fov 60 near 0.1 far 200 }
  active_camera
}
entity "sun" {
  light {
    dir -0.3 -1.0 -0.2
    color 1.0 0.95 0.85 1.0
    intensity 4
  }
}
"#,
        )
        .unwrap();
        path
    }

    #[test]
    fn startup_snapshot_includes_world_and_selection() {
        let path = write_scene_file("startup");
        let server = EditorBridgeServer::open_hsc(&path).unwrap();
        let events = server.startup_events();

        assert!(matches!(events.first(), Some(EditorEvent::WorldRebuilt)));
        assert!(matches!(
            events.get(1),
            Some(EditorEvent::SelectionChanged(_))
        ));
    }

    #[test]
    fn encoded_message_execution_roundtrips_events() {
        let path = write_scene_file("message");
        let mut server = EditorBridgeServer::open_hsc(&path).unwrap();

        let messages = server.execute_message("cmd|SelectNextEntity").unwrap();
        assert_eq!(messages.len(), 1);

        let event = decode_editor_event_protocol(&messages[0]).unwrap();
        match event {
            EditorEvent::SelectionChanged(cursor) => {
                assert_eq!(cursor.entity_id, server.host().cursor().unwrap().entity_id);
            }
            other => panic!("expected SelectionChanged, got {other:?}"),
        }
    }
}
