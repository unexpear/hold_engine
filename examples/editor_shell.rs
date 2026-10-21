//! Native windowed editor shell for the M2 milestone.
//!
//! Usage:
//!   cargo run --example editor_shell
//!   cargo run --example editor_shell -- --scene path/to/scene.hsc
//!   cargo run --example editor_shell -- --backend opengl

use std::path::PathBuf;

use hold::{
    decode_editor_event_protocol, encode_editor_command_protocol,
    engine::App,
    input::{vk, InputState},
    math::Mat4,
    renderer::{BackendKind, OpenGLRenderer, Renderer, SoftwareRenderer},
    EditorAxis, EditorBridgeServer, EditorCommand, EditorEvent, EditorPlayState,
    EditorProtocolError, Engine,
};

const DEFAULT_SCENE: &str = "examples/demo.hsc";
const EDITOR_FOV_DEG: f32 = 60.0;
const DELTA_SMALL: f32 = 0.1;
const DELTA_LARGE: f32 = 1.0;

struct Args {
    scene: PathBuf,
    backend: BackendKind,
}

impl Args {
    fn parse() -> Self {
        let mut args = std::env::args().skip(1).peekable();
        let mut scene = PathBuf::from(DEFAULT_SCENE);
        let mut backend = BackendKind::Software;

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
                            _ => BackendKind::Software,
                        };
                    }
                }
                _ => {}
            }
        }

        Self { scene, backend }
    }
}

struct EditorShellApp {
    server: EditorBridgeServer,
    backend: BackendKind,
    status: String,
}

impl EditorShellApp {
    fn new(server: EditorBridgeServer, backend: BackendKind) -> Self {
        let mut app = Self {
            server,
            backend,
            status: String::new(),
        };
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
            "{}{} | {} | {} | {}{}",
            scene_name, dirty, backend, play_state, sync_state, cursor_str
        );
    }

    fn process_input(&mut self, input: &InputState) {
        let ctrl = input.key(vk::CTRL);
        let shift = input.key(vk::SHIFT);

        if input.key_pressed(vk::ESCAPE) {
            std::process::exit(0);
        }

        if ctrl && input.key_pressed(vk::S) {
            self.dispatch(EditorCommand::Save);
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

        if input.key_pressed(vk::F8) {
            match self.server.host().play_state() {
                EditorPlayState::Editing => self.dispatch(EditorCommand::EnterPlayMode),
                EditorPlayState::Playing | EditorPlayState::Paused => {
                    self.dispatch(EditorCommand::StopPlayMode)
                }
            }
            return;
        }

        if input.key_pressed(vk::F9) {
            match self.server.host().play_state() {
                EditorPlayState::Playing => self.dispatch(EditorCommand::PausePlayMode),
                EditorPlayState::Paused => self.dispatch(EditorCommand::ResumePlayMode),
                EditorPlayState::Editing => {}
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

    fn dispatch(&mut self, command: EditorCommand) {
        match self.dispatch_protocol(command) {
            Ok(events) => self.handle_events(&events),
            Err(err) => self.print_error(&err),
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

    fn handle_events(&mut self, events: &[EditorEvent]) {
        for event in events {
            match event {
                EditorEvent::SelectionChanged(cursor) => {
                    let axis = cursor
                        .axis
                        .map(|axis| format!(" axis={}", axis))
                        .unwrap_or_default();
                    println!(
                        "[editor] selected entity {} · {} · {}{}",
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
                EditorEvent::Reloaded => {
                    println!("[editor] scene reloaded from source");
                }
            }
        }
        self.refresh_status();
        println!("[editor] {}", self.status);
    }

    fn print_error(&self, err: &EditorProtocolError) {
        eprintln!("[editor] {err}");
    }
}

impl App for EditorShellApp {
    fn update(
        &mut self,
        _world: &mut hold::World,
        renderer: &mut dyn Renderer,
        input: &InputState,
        dt: f32,
    ) {
        self.server.host_mut().update_editor_camera(input, dt);
        self.process_input(input);

        let height = renderer.height().max(1) as f32;
        let aspect = renderer.width() as f32 / height;
        let vp: Mat4 = self.server.host().active_view_proj(EDITOR_FOV_DEG, aspect);

        renderer.clear(hold::Color::new(0.10, 0.10, 0.12, 1.0));

        let draw_list = hold::generate_draw_list(
            self.server.host().world(),
            self.server.host().session().last_world_map(),
            self.server.host().session().scene(),
            self.server.host().cursor().as_ref(),
        );
        renderer.render_debug_lines(&draw_list.lines, &vp);
    }

    fn window_title(&self) -> Option<String> {
        Some(format!("Hold Editor - {}", self.status))
    }
}

fn main() {
    let args = Args::parse();

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

    let title = format!(
        "Hold Editor - {} | {:?}",
        args.scene.display(),
        args.backend
    );
    let engine = Engine::new(&title, 1280, 720);
    let app = EditorShellApp::new(server, args.backend);

    println!("[editor] {}", app.status);
    println!(
        "[editor] Tab/S+Tab=entity  [/]=component  ;/'=field  1/2/3=axis  -/==delta  F7=reload  F8=play  F9=pause  Ctrl+S/P/R=save/patch/rebuild  Esc=quit"
    );

    match args.backend {
        BackendKind::Software => {
            let (hwnd, w, h) = (
                engine.window.hwnd,
                engine.window.width(),
                engine.window.height(),
            );
            let renderer = SoftwareRenderer::new(hwnd, w, h);
            engine.run_app(renderer, app);
        }
        BackendKind::OpenGL => {
            let (hwnd, w, h) = (
                engine.window.hwnd,
                engine.window.width(),
                engine.window.height(),
            );
            let renderer = OpenGLRenderer::new(hwnd, w, h);
            engine.run_app(renderer, app);
        }
        BackendKind::Vulkan => {
            eprintln!(
                "error: Vulkan backend is not supported in M2; use --backend software or --backend opengl"
            );
            std::process::exit(1);
        }
    }
}
