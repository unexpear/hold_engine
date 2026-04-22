//! Headless editor/runtime host with the authoritative command/event surface.
//!
//! `EditorRuntimeHost` owns:
//!  - an [`EditorSceneSession`] (scene document, play-state, IO),
//!  - a live [`World`] (runtime ECS state),
//!  - an [`EditorCamera`] (free-fly editor camera, separate from the scene),
//!  - inspector cursor state (entity / component / field indices + axis),
//!  - persistent dirty tracking (scene modified since last save).
//!
//! The public command surface is [`EditorRuntimeHost::execute`].  All other
//! methods remain available for headless tests and direct API consumers.

use std::path::Path;

use crate::ecs::components::{ActiveCamera, Camera};
use crate::ecs::{Entity, World};
use crate::input::{vk, InputState, MouseButton};
use crate::math::{Mat4, Vec3};

use super::{
    command::{EditorAxis, EditorCommand, EditorCommandError, EditorEvent, EditorInspectorCursor},
    protocol::{decode_command, encode_event, EditorProtocolError},
    EditorPlayState, EditorSceneSession, EditorSessionError, InspectorValue, SceneDocument,
};

// ── EditorCamera ──────────────────────────────────────────────────────────────

/// A host-owned free-fly perspective camera for the `Editing` and `Paused`
/// play states.
///
/// Not stored in the [`SceneDocument`] — it is purely a shell-side viewport
/// aid.  During `Playing`, the shell switches to the runtime `ActiveCamera`
/// instead.
#[derive(Debug, Clone)]
pub struct EditorCamera {
    pub position: Vec3,
    /// Horizontal rotation in radians (0 = looking along −Z).
    pub yaw: f32,
    /// Vertical rotation in radians, clamped to ±89°.
    pub pitch: f32,
}

impl EditorCamera {
    pub fn new() -> Self {
        Self {
            position: Vec3::new(0.0, 5.0, 12.0),
            yaw: 0.0,
            pitch: -0.3,
        }
    }

    /// World-space forward direction (unit length).
    pub fn forward(&self) -> Vec3 {
        Vec3::new(
            self.yaw.sin() * self.pitch.cos(),
            self.pitch.sin(),
            -(self.yaw.cos() * self.pitch.cos()),
        )
    }

    /// View matrix (world → camera space).
    pub fn view_matrix(&self) -> Mat4 {
        let fwd = self.forward();
        let right = fwd.cross(Vec3::Y).normalize();
        let up = right.cross(fwd).normalize();
        Mat4::look_at(self.position, self.position + fwd, up)
    }

    /// Combined view-projection matrix for the editor camera.
    pub fn view_proj(&self, fov_deg: f32, aspect: f32, near: f32, far: f32) -> Mat4 {
        let proj = Mat4::perspective_gl(fov_deg.to_radians(), aspect, near, far);
        proj * self.view_matrix()
    }

    /// Update the camera from input.
    ///
    /// Free-fly controls are only active while the **right mouse button** is
    /// held.  When it is released the camera stays still (no accidental drift).
    ///
    /// Key bindings: `WASD` — horizontal strafe/forward, `Q`/`E` — down/up.
    /// `Shift` multiplies speed by 10.
    pub fn update(&mut self, input: &InputState, dt: f32) {
        if !input.mouse_button(MouseButton::Right) {
            return;
        }

        // Mouse look.
        let (dx, dy) = input.mouse_delta();
        self.yaw += dx as f32 * 0.004;
        self.pitch -= dy as f32 * 0.004;
        self.pitch = self.pitch.clamp(-1.55, 1.55); // ±~89°

        // Movement.
        let speed = if input.key(vk::SHIFT) { 40.0 } else { 6.0 };
        let fwd = self.forward();
        let right = fwd.cross(Vec3::Y).normalize();
        let up = Vec3::Y;

        let mut dir = Vec3::ZERO;
        if input.key(vk::W) {
            dir += fwd;
        }
        if input.key(vk::S) {
            dir -= fwd;
        }
        if input.key(vk::D) {
            dir += right;
        }
        if input.key(vk::A) {
            dir -= right;
        }
        if input.key(vk::E) {
            dir += up;
        }
        if input.key(vk::Q) {
            dir -= up;
        }

        if dir.length_sq() > 1e-8 {
            self.position += dir.normalize() * (speed * dt);
        }
    }
}

impl Default for EditorCamera {
    fn default() -> Self {
        Self::new()
    }
}

// ── EditorRuntimeHost ─────────────────────────────────────────────────────────

/// Headless editor/runtime host for the authoritative scene workflow.
///
/// A native UI shell (or test harness) sits on top of this without
/// re-implementing scene loading, inspector mutation, play-mode transitions,
/// or world synchronization.
pub struct EditorRuntimeHost {
    session: EditorSceneSession,
    world: World,

    // ── Inspector cursor ──────────────────────────────────────────────────
    /// Index into `session.scene().entities` of the currently selected entity.
    /// `None` when the scene is empty.
    cursor_entity_idx: Option<usize>,
    /// Index into the selected entity's `components` slice.
    cursor_component_idx: usize,
    /// Index into the selected component's `fields` slice.
    cursor_field_idx: usize,
    /// Active axis for Vec3 field edits.
    cursor_axis: Option<EditorAxis>,

    // ── Dirty tracking ────────────────────────────────────────────────────
    /// `true` when the scene document has unsaved changes (Editing only).
    persistent_dirty: bool,
    /// Snapshot of `persistent_dirty` taken on `EnterPlayMode`; restored by
    /// `StopPlayMode` so a play cycle does not clear an unsaved modification.
    pre_play_persistent_dirty: bool,
    /// `true` when the live world is stale and requires a rebuild to match the
    /// current scene document.
    world_requires_rebuild: bool,

    // ── Editor camera ─────────────────────────────────────────────────────
    pub editor_camera: EditorCamera,
}

impl EditorRuntimeHost {
    // ── Construction ─────────────────────────────────────────────────────

    pub fn new(session: EditorSceneSession) -> Result<Self, EditorSessionError> {
        let mut host = Self {
            session,
            world: World::new(),
            cursor_entity_idx: None,
            cursor_component_idx: 0,
            cursor_field_idx: 0,
            cursor_axis: None,
            persistent_dirty: false,
            pre_play_persistent_dirty: false,
            world_requires_rebuild: false,
            editor_camera: EditorCamera::new(),
        };
        host.rebuild_world()?;
        // Auto-select the first entity after loading.
        host.init_cursor_to_first();
        Ok(host)
    }

    pub fn from_scene(scene: SceneDocument) -> Result<Self, EditorSessionError> {
        Self::new(EditorSceneSession::new(scene))
    }

    pub fn open_hsc<P: AsRef<Path>>(path: P) -> Result<Self, EditorSessionError> {
        Self::new(EditorSceneSession::open_hsc(path)?)
    }

    // ── Accessors ─────────────────────────────────────────────────────────

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

    /// Whether the scene document has unsaved changes.
    ///
    /// Always `false` during play mode (transient play edits are not
    /// persistent).
    pub fn is_dirty(&self) -> bool {
        self.persistent_dirty && self.play_state() == EditorPlayState::Editing
    }

    /// Whether the live world is stale and needs a rebuild to match the scene.
    pub fn world_requires_rebuild(&self) -> bool {
        self.world_requires_rebuild
    }

    /// The current inspector cursor, or `None` if the scene is empty.
    pub fn cursor(&self) -> Option<EditorInspectorCursor> {
        let idx = self.cursor_entity_idx?;
        let scene = self.session.scene();
        let entity = scene.entities.get(idx)?;
        let component = entity.components.get(self.cursor_component_idx)?;
        let field = component.fields.get(self.cursor_field_idx)?;
        Some(EditorInspectorCursor {
            entity_id: entity.id,
            component_name: component.type_name.clone(),
            field_name: field.name.clone(),
            axis: self.cursor_axis,
        })
    }

    // ── Selection (direct API — bypasses execute) ─────────────────────────

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

    // ── Play mode (direct API) ─────────────────────────────────────────────

    pub fn enter_play_mode(&mut self) -> Result<(), EditorSessionError> {
        self.session.enter_play_mode(&mut self.world)?;
        self.world_requires_rebuild = false;
        Ok(())
    }

    pub fn pause_play_mode(&mut self) -> Result<(), EditorSessionError> {
        self.session.pause_play_mode()
    }

    pub fn resume_play_mode(&mut self) -> Result<(), EditorSessionError> {
        self.session.resume_play_mode()
    }

    pub fn stop_play_mode(&mut self) -> Result<(), EditorSessionError> {
        self.session.stop_play_mode(&mut self.world)?;
        self.world_requires_rebuild = false;
        Ok(())
    }

    // ── World sync (direct API) ────────────────────────────────────────────

    pub fn rebuild_world(&mut self) -> Result<&[(u64, Entity)], EditorSessionError> {
        let map = self.session.rebuild_world(&mut self.world)?;
        self.world_requires_rebuild = false;
        Ok(map)
    }

    pub fn patch_world(&mut self) -> Result<(), EditorSessionError> {
        match self.session.patch_world(&mut self.world) {
            Ok(()) => {
                self.world_requires_rebuild = false;
                Ok(())
            }
            Err(EditorSessionError::PatchRequiresRebuild { reason }) => {
                self.world_requires_rebuild = true;
                Err(EditorSessionError::PatchRequiresRebuild { reason })
            }
            Err(err) => Err(err),
        }
    }

    // ── Save (direct API) ─────────────────────────────────────────────────

    pub fn save_hsc<P: AsRef<Path>>(&self, path: P) -> Result<(), EditorSessionError> {
        self.session.save_hsc(path)
    }

    pub fn save_source_hsc(&self) -> Result<(), EditorSessionError> {
        self.session.save_source_hsc()
    }

    // ── Editor camera ──────────────────────────────────────────────────────

    /// Update the editor camera from frame input.
    ///
    /// This is a no-op during `Playing` — the viewport uses the runtime
    /// `ActiveCamera` instead.
    pub fn update_editor_camera(&mut self, input: &InputState, dt: f32) {
        if self.play_state() == EditorPlayState::Playing {
            return;
        }
        self.editor_camera.update(input, dt);
    }

    /// Return the active view-projection matrix for the current frame.
    ///
    /// - `Editing` / `Paused` — uses the host-owned [`EditorCamera`].
    /// - `Playing` — uses the first [`ActiveCamera`] found in the live world.
    ///
    /// Falls back to `Mat4::IDENTITY` if no camera is available.
    pub fn active_view_proj(&self, fov_deg: f32, aspect: f32) -> Mat4 {
        match self.play_state() {
            EditorPlayState::Editing | EditorPlayState::Paused => {
                self.editor_camera.view_proj(fov_deg, aspect, 0.1, 1000.0)
            }
            EditorPlayState::Playing => {
                // Find the runtime ActiveCamera and use its view/proj matrices.
                self.world
                    .query::<ActiveCamera>()
                    .filter_map(|(e, _)| self.world.get::<Camera>(e))
                    .next()
                    .map(|cam| cam.proj * cam.view)
                    .unwrap_or(Mat4::IDENTITY)
            }
        }
    }

    // ── Command surface ────────────────────────────────────────────────────

    /// The authoritative shell/bridge entrypoint.
    ///
    /// Processes `command`, updates internal state, applies world mutations,
    /// and returns a list of events describing what changed.  Events are
    /// ordered causally (e.g. [`EditorEvent::FieldEdited`] precedes the patch
    /// result event).
    ///
    /// A return value of `Err` means the command was rejected before any state
    /// changed.  Events on the `Ok` path always reflect real mutations.
    pub fn execute(
        &mut self,
        command: EditorCommand,
    ) -> Result<Vec<EditorEvent>, EditorCommandError> {
        match command {
            EditorCommand::SelectNextEntity => self.cmd_cycle_entity(1),
            EditorCommand::SelectPrevEntity => self.cmd_cycle_entity(-1),
            EditorCommand::SelectNextComponent => self.cmd_cycle_component(1),
            EditorCommand::SelectPrevComponent => self.cmd_cycle_component(-1),
            EditorCommand::SelectNextField => self.cmd_cycle_field(1),
            EditorCommand::SelectPrevField => self.cmd_cycle_field(-1),
            EditorCommand::SetAxis(axis) => self.cmd_set_axis(axis),
            EditorCommand::ApplyDelta(delta) => self.cmd_apply_delta(delta),
            EditorCommand::EnterPlayMode => self.cmd_enter_play_mode(),
            EditorCommand::PausePlayMode => self.cmd_pause_play_mode(),
            EditorCommand::ResumePlayMode => self.cmd_resume_play_mode(),
            EditorCommand::StopPlayMode => self.cmd_stop_play_mode(),
            EditorCommand::Save => self.cmd_save(),
            EditorCommand::ReloadSource => self.cmd_reload_source(),
            EditorCommand::ForcePatch => self.cmd_force_patch(),
            EditorCommand::ForceRebuild => self.cmd_force_rebuild(),
        }
    }

    /// Decode a bridge-protocol command message, execute it, and return the
    /// resulting events encoded back into bridge-protocol messages.
    pub fn execute_protocol_message(
        &mut self,
        message: &str,
    ) -> Result<Vec<String>, EditorProtocolError> {
        let command = decode_command(message)?;
        let events = self
            .execute(command)
            .map_err(EditorProtocolError::Command)?;
        Ok(events.iter().map(encode_event).collect())
    }

    // ── Private: cursor helpers ────────────────────────────────────────────

    /// Set cursor to the first entity / component / field in the scene.
    fn init_cursor_to_first(&mut self) {
        let count = self.session.scene().entities.len();
        if count == 0 {
            self.cursor_entity_idx = None;
            return;
        }
        self.cursor_entity_idx = Some(0);
        self.cursor_component_idx = 0;
        self.cursor_field_idx = 0;
        self.cursor_axis = None;

        // Sync session selection.
        let id = self.session.scene().entities[0].id;
        let _ = self.session.select_entity(id);
    }

    /// Apply a selection (entity index already validated) and sync session.
    fn apply_cursor_entity(&mut self, idx: usize) {
        self.cursor_entity_idx = Some(idx);
        self.cursor_component_idx = 0;
        self.cursor_field_idx = 0;
        self.cursor_axis = None;

        let id = self.session.scene().entities[idx].id;
        let _ = self.session.select_entity(id);
    }

    /// Current field value, resolved from cursor indices.
    fn current_field_value(&self) -> Option<&InspectorValue> {
        let idx = self.cursor_entity_idx?;
        let scene = self.session.scene();
        let entity = scene.entities.get(idx)?;
        let comp = entity.components.get(self.cursor_component_idx)?;
        comp.fields.get(self.cursor_field_idx).map(|f| &f.value)
    }

    /// Current (component_name, field_name) pair.
    fn current_component_field_names(&self) -> Option<(String, String)> {
        let idx = self.cursor_entity_idx?;
        let scene = self.session.scene();
        let entity = scene.entities.get(idx)?;
        let comp = entity.components.get(self.cursor_component_idx)?;
        let field = comp.fields.get(self.cursor_field_idx)?;
        Some((comp.type_name.clone(), field.name.clone()))
    }

    /// Restore the cursor to `(entity_id, component_name, field_name)` after a
    /// rebuild, clamping each level to 0 when the target is not found.
    fn restore_cursor(
        &mut self,
        target_entity_id: Option<u64>,
        target_component: &str,
        target_field: &str,
    ) {
        let entities = &self.session.scene().entities;
        if entities.is_empty() {
            self.cursor_entity_idx = None;
            return;
        }

        // Find entity index by id; clamp to 0 if not found.
        let entity_idx = target_entity_id
            .and_then(|id| entities.iter().position(|e| e.id == id))
            .unwrap_or(0);
        self.cursor_entity_idx = Some(entity_idx);

        // Find component by name; clamp to 0 if not found.
        let entity = &entities[entity_idx];
        let comp_idx = entity
            .components
            .iter()
            .position(|c| c.type_name == target_component)
            .unwrap_or(0);
        self.cursor_component_idx = comp_idx;

        // Find field by name; clamp to 0 if not found.
        let field_idx = entity
            .components
            .get(comp_idx)
            .and_then(|c| c.fields.iter().position(|f| f.name == target_field))
            .unwrap_or(0);
        self.cursor_field_idx = field_idx;
        self.cursor_axis = None;

        // Sync session.
        let id = entities[entity_idx].id;
        let _ = self.session.select_entity(id);
    }

    /// Auto-attempt `patch_world`; translate the result into events.
    fn auto_patch_events(&mut self) -> Result<Vec<EditorEvent>, EditorCommandError> {
        match self.session.patch_world(&mut self.world) {
            Ok(()) => {
                self.world_requires_rebuild = false;
                Ok(vec![EditorEvent::PatchApplied])
            }
            Err(EditorSessionError::PatchRequiresRebuild { reason }) => {
                self.world_requires_rebuild = true;
                Ok(vec![EditorEvent::PatchRequiresRebuild { reason }])
            }
            Err(err) => Err(EditorCommandError::Session(err)),
        }
    }

    // ── Private: command implementations ──────────────────────────────────

    fn cmd_cycle_entity(&mut self, dir: i32) -> Result<Vec<EditorEvent>, EditorCommandError> {
        let count = self.session.scene().entities.len();
        if count == 0 {
            return Ok(vec![]);
        }

        let current = self.cursor_entity_idx.unwrap_or(0);
        let next = ((current as i32 + dir).rem_euclid(count as i32)) as usize;
        self.apply_cursor_entity(next);

        Ok(vec![EditorEvent::SelectionChanged(self.cursor().unwrap())])
    }

    fn cmd_cycle_component(&mut self, dir: i32) -> Result<Vec<EditorEvent>, EditorCommandError> {
        let idx = self
            .cursor_entity_idx
            .ok_or(EditorCommandError::NoSelection)?;
        let entity = &self.session.scene().entities[idx];
        let count = entity.components.len();
        if count == 0 {
            return Ok(vec![]);
        }

        let next = ((self.cursor_component_idx as i32 + dir).rem_euclid(count as i32)) as usize;
        self.cursor_component_idx = next;
        self.cursor_field_idx = 0;
        self.cursor_axis = None;

        Ok(vec![EditorEvent::SelectionChanged(self.cursor().unwrap())])
    }

    fn cmd_cycle_field(&mut self, dir: i32) -> Result<Vec<EditorEvent>, EditorCommandError> {
        let idx = self
            .cursor_entity_idx
            .ok_or(EditorCommandError::NoSelection)?;
        let entity = &self.session.scene().entities[idx];
        let comp = entity
            .components
            .get(self.cursor_component_idx)
            .ok_or(EditorCommandError::NoField)?;
        let count = comp.fields.len();
        if count == 0 {
            return Ok(vec![]);
        }

        let next = ((self.cursor_field_idx as i32 + dir).rem_euclid(count as i32)) as usize;
        self.cursor_field_idx = next;
        self.cursor_axis = None;

        Ok(vec![EditorEvent::SelectionChanged(self.cursor().unwrap())])
    }

    fn cmd_set_axis(&mut self, axis: EditorAxis) -> Result<Vec<EditorEvent>, EditorCommandError> {
        // Only meaningful for Vec3 fields; silently accept for others (no error).
        self.cursor_axis = Some(axis);
        if let Some(cursor) = self.cursor() {
            Ok(vec![EditorEvent::SelectionChanged(cursor)])
        } else {
            Ok(vec![])
        }
    }

    fn cmd_apply_delta(&mut self, delta: f32) -> Result<Vec<EditorEvent>, EditorCommandError> {
        // Playing blocks field edits.
        if self.play_state() == EditorPlayState::Playing {
            return Err(EditorCommandError::UnsupportedCommandState {
                command: "ApplyDelta",
                state: EditorPlayState::Playing,
            });
        }

        // Resolve current field.
        let cursor = self.cursor().ok_or(EditorCommandError::NoSelection)?;
        let current_value = self
            .current_field_value()
            .ok_or(EditorCommandError::NoField)?
            .clone();

        let (comp_name, field_name) = self
            .current_component_field_names()
            .ok_or(EditorCommandError::NoField)?;

        let new_value = match &current_value {
            InspectorValue::Number(n) => InspectorValue::Number(n + delta),
            InspectorValue::Vec3(v) => {
                let axis = self.cursor_axis.unwrap_or(EditorAxis::X);
                let mut arr = *v;
                match axis {
                    EditorAxis::X => arr[0] += delta,
                    EditorAxis::Y => arr[1] += delta,
                    EditorAxis::Z => arr[2] += delta,
                }
                InspectorValue::Vec3(arr)
            }
            InspectorValue::Text(_) => {
                return Err(EditorCommandError::UnsupportedFieldKind {
                    field: field_name.clone(),
                })
            }
            InspectorValue::Bool(_) => {
                return Err(EditorCommandError::UnsupportedFieldKind {
                    field: field_name.clone(),
                })
            }
        };

        // Mutate scene document.
        self.session
            .set_selected_value(&comp_name, &field_name, new_value.clone())
            .map_err(EditorCommandError::Session)?;

        // Persistent dirty — only in Editing; Paused edits are transient PIE overrides.
        if self.play_state() == EditorPlayState::Editing {
            self.persistent_dirty = true;
        }

        let field_event = EditorEvent::FieldEdited {
            cursor: EditorInspectorCursor {
                axis: self.cursor_axis,
                ..cursor
            },
            new_value,
        };

        let mut events = vec![field_event];
        events.extend(self.auto_patch_events()?);
        Ok(events)
    }

    fn cmd_enter_play_mode(&mut self) -> Result<Vec<EditorEvent>, EditorCommandError> {
        if self.play_state() != EditorPlayState::Editing {
            return Err(EditorCommandError::UnsupportedCommandState {
                command: "EnterPlayMode",
                state: self.play_state(),
            });
        }
        self.pre_play_persistent_dirty = self.persistent_dirty;
        self.session
            .enter_play_mode(&mut self.world)
            .map_err(EditorCommandError::Session)?;
        self.world_requires_rebuild = false;
        Ok(vec![EditorEvent::PlayModeChanged(EditorPlayState::Playing)])
    }

    fn cmd_pause_play_mode(&mut self) -> Result<Vec<EditorEvent>, EditorCommandError> {
        if self.play_state() != EditorPlayState::Playing {
            return Err(EditorCommandError::UnsupportedCommandState {
                command: "PausePlayMode",
                state: self.play_state(),
            });
        }
        self.session
            .pause_play_mode()
            .map_err(EditorCommandError::Session)?;
        Ok(vec![EditorEvent::PlayModeChanged(EditorPlayState::Paused)])
    }

    fn cmd_resume_play_mode(&mut self) -> Result<Vec<EditorEvent>, EditorCommandError> {
        if self.play_state() != EditorPlayState::Paused {
            return Err(EditorCommandError::UnsupportedCommandState {
                command: "ResumePlayMode",
                state: self.play_state(),
            });
        }
        self.session
            .resume_play_mode()
            .map_err(EditorCommandError::Session)?;
        Ok(vec![EditorEvent::PlayModeChanged(EditorPlayState::Playing)])
    }

    fn cmd_stop_play_mode(&mut self) -> Result<Vec<EditorEvent>, EditorCommandError> {
        let state = self.play_state();
        if state == EditorPlayState::Editing {
            return Err(EditorCommandError::UnsupportedCommandState {
                command: "StopPlayMode",
                state,
            });
        }

        // Preserve cursor identity for restoration.
        let (saved_entity_id, saved_comp, saved_field) = self
            .cursor()
            .map(|c| (Some(c.entity_id), c.component_name, c.field_name))
            .unwrap_or((None, String::new(), String::new()));

        self.session
            .stop_play_mode(&mut self.world)
            .map_err(EditorCommandError::Session)?;

        // Restore pre-play dirty flag; discard transient play edits.
        self.persistent_dirty = self.pre_play_persistent_dirty;
        self.world_requires_rebuild = false;

        // Restore cursor (clamped if the entity/component/field no longer exists).
        self.restore_cursor(saved_entity_id, &saved_comp, &saved_field);

        Ok(vec![
            EditorEvent::PlayModeChanged(EditorPlayState::Editing),
            EditorEvent::WorldRebuilt,
        ])
    }

    fn cmd_save(&mut self) -> Result<Vec<EditorEvent>, EditorCommandError> {
        if self.play_state() != EditorPlayState::Editing {
            return Err(EditorCommandError::UnsupportedCommandState {
                command: "Save",
                state: self.play_state(),
            });
        }
        if self.session.source_path().is_none() {
            return Err(EditorCommandError::MissingSourcePath);
        }
        self.session
            .save_source_hsc()
            .map_err(EditorCommandError::Session)?;
        self.persistent_dirty = false;
        Ok(vec![EditorEvent::Saved])
    }

    fn cmd_reload_source(&mut self) -> Result<Vec<EditorEvent>, EditorCommandError> {
        if self.play_state() != EditorPlayState::Editing {
            return Err(EditorCommandError::UnsupportedCommandState {
                command: "ReloadSource",
                state: self.play_state(),
            });
        }
        let source_path = self
            .session
            .source_path()
            .ok_or(EditorCommandError::MissingSourcePath)?
            .to_path_buf();

        let (saved_entity_id, saved_comp, saved_field) = self
            .cursor()
            .map(|c| (Some(c.entity_id), c.component_name, c.field_name))
            .unwrap_or((None, String::new(), String::new()));

        self.session =
            EditorSceneSession::open_hsc(&source_path).map_err(EditorCommandError::Session)?;
        self.session
            .rebuild_world(&mut self.world)
            .map_err(EditorCommandError::Session)?;
        self.persistent_dirty = false;
        self.pre_play_persistent_dirty = false;
        self.world_requires_rebuild = false;

        self.restore_cursor(saved_entity_id, &saved_comp, &saved_field);
        Ok(vec![EditorEvent::Reloaded, EditorEvent::WorldRebuilt])
    }

    fn cmd_force_patch(&mut self) -> Result<Vec<EditorEvent>, EditorCommandError> {
        self.auto_patch_events()
    }

    fn cmd_force_rebuild(&mut self) -> Result<Vec<EditorEvent>, EditorCommandError> {
        // Preserve cursor for restoration.
        let (saved_entity_id, saved_comp, saved_field) = self
            .cursor()
            .map(|c| (Some(c.entity_id), c.component_name, c.field_name))
            .unwrap_or((None, String::new(), String::new()));

        self.session
            .rebuild_world(&mut self.world)
            .map_err(EditorCommandError::Session)?;
        self.world_requires_rebuild = false;

        self.restore_cursor(saved_entity_id, &saved_comp, &saved_field);

        Ok(vec![EditorEvent::WorldRebuilt])
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ActiveCamera, Camera, Transform, Vec3};
    use std::time::{SystemTime, UNIX_EPOCH};

    // ── Helpers ───────────────────────────────────────────────────────────

    fn nanos() -> u128 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    }

    fn tmp_dir(suffix: &str) -> String {
        format!(
            "{}/hold_host_m2_{}_{}",
            std::env::temp_dir().display(),
            suffix,
            nanos()
        )
    }

    fn write_two_entity_scene(suffix: &str) -> String {
        let root = tmp_dir(suffix);
        std::fs::create_dir_all(&root).unwrap();
        let path = format!("{root}/test.hsc");
        std::fs::write(
            &path,
            r#"
scene "TestScene"

entity "camera" {
  transform { pos 0 2 10 rot 0 0 0 1 scale 1 1 1 }
  camera { fov 60 near 0.1 far 1000 }
  active_camera
}

entity "player" {
  transform { pos 0 1 0 rot 0 0 0 1 scale 1 1 1 }
  stats { health 100 mana 25 attack 8 defense 4 speed 3 }
}
"#,
        )
        .unwrap();
        path
    }

    fn write_single_entity_scene(suffix: &str) -> String {
        let root = tmp_dir(suffix);
        std::fs::create_dir_all(&root).unwrap();
        let path = format!("{root}/test.hsc");
        std::fs::write(
            &path,
            r#"
scene "Single"
entity "cam" {
  transform { pos 0 5 0 rot 0 0 0 1 scale 1 1 1 }
  camera { fov 75 near 0.1 far 500 }
  active_camera
}
"#,
        )
        .unwrap();
        path
    }

    // ── Open / basic ──────────────────────────────────────────────────────

    #[test]
    fn host_opens_and_populates_world() {
        let path = write_two_entity_scene("open");
        let host = EditorRuntimeHost::open_hsc(&path).unwrap();
        assert_eq!(host.session().scene().entities.len(), 2);
        assert_eq!(host.session().last_world_map().len(), 2);
    }

    // ── Selection cycling ─────────────────────────────────────────────────

    #[test]
    fn cycle_entity_wraps_around() {
        let path = write_two_entity_scene("cycle");
        let mut host = EditorRuntimeHost::open_hsc(&path).unwrap();

        // Starts at entity 0.
        assert_eq!(host.cursor_entity_idx, Some(0));

        // Forward: 0 → 1.
        host.execute(EditorCommand::SelectNextEntity).unwrap();
        assert_eq!(host.cursor_entity_idx, Some(1));

        // Forward: 1 → 0 (wraps).
        host.execute(EditorCommand::SelectNextEntity).unwrap();
        assert_eq!(host.cursor_entity_idx, Some(0));

        // Backward: 0 → 1 (wraps).
        host.execute(EditorCommand::SelectPrevEntity).unwrap();
        assert_eq!(host.cursor_entity_idx, Some(1));
    }

    #[test]
    fn cycle_entity_emits_selection_changed_event() {
        let path = write_two_entity_scene("ev");
        let mut host = EditorRuntimeHost::open_hsc(&path).unwrap();

        let events = host.execute(EditorCommand::SelectNextEntity).unwrap();
        assert!(
            events
                .iter()
                .any(|e| matches!(e, EditorEvent::SelectionChanged(_))),
            "SelectNextEntity must emit SelectionChanged"
        );
    }

    #[test]
    fn cycle_component_resets_field_index() {
        let path = write_two_entity_scene("comp");
        let mut host = EditorRuntimeHost::open_hsc(&path).unwrap();

        // Manually advance the field.
        host.cursor_field_idx = 2;
        host.execute(EditorCommand::SelectNextComponent).unwrap();
        assert_eq!(
            host.cursor_field_idx, 0,
            "component change must reset field to 0"
        );
    }

    // ── Scalar edits ──────────────────────────────────────────────────────

    #[test]
    fn apply_delta_to_number_field_patches_world() {
        let path = write_two_entity_scene("delta");
        let mut host = EditorRuntimeHost::open_hsc(&path).unwrap();

        // Navigate to camera entity (idx 0), Camera component, fov_deg field.
        // The cursor starts at entity 0, component 0 (Transform), field 0 (position).
        // We need the Camera component and its fov_deg field.
        // Find camera entity.
        let cam_id = host
            .session()
            .scene()
            .entities
            .iter()
            .find(|e| e.name == "camera")
            .unwrap()
            .id;
        host.select_entity(cam_id).unwrap();
        let cam_idx = host
            .session()
            .scene()
            .entities
            .iter()
            .position(|e| e.id == cam_id)
            .unwrap();
        host.cursor_entity_idx = Some(cam_idx);

        // Find Camera component.
        let cam_comp_idx = host.session().scene().entities[cam_idx]
            .components
            .iter()
            .position(|c| c.type_name == "Camera")
            .unwrap();
        host.cursor_component_idx = cam_comp_idx;

        // Find fov_deg field.
        let fov_idx = host.session().scene().entities[cam_idx].components[cam_comp_idx]
            .fields
            .iter()
            .position(|f| f.name == "fov_deg")
            .unwrap();
        host.cursor_field_idx = fov_idx;

        let events = host.execute(EditorCommand::ApplyDelta(5.0)).unwrap();

        // FieldEdited must be present.
        assert!(
            events
                .iter()
                .any(|e| matches!(e, EditorEvent::FieldEdited { .. })),
            "ApplyDelta must emit FieldEdited"
        );

        // Runtime Camera fov must have changed.
        let runtime_cam = host.applied_entity(cam_id).unwrap();
        let camera = host.world().get::<Camera>(runtime_cam).unwrap();
        assert!(
            (camera.fov.to_degrees() - 65.0).abs() < 0.01,
            "fov_deg should be 60+5=65, got {}",
            camera.fov.to_degrees()
        );
    }

    // ── Vec3 axis edits ───────────────────────────────────────────────────

    #[test]
    fn apply_delta_to_vec3_field_uses_active_axis() {
        let path = write_single_entity_scene("vec3");
        let mut host = EditorRuntimeHost::open_hsc(&path).unwrap();

        // Cursor starts at entity 0 (cam), component 0 (Transform), field 0 (position).
        host.cursor_axis = Some(EditorAxis::Y);

        let events = host.execute(EditorCommand::ApplyDelta(3.0)).unwrap();
        assert!(events
            .iter()
            .any(|e| matches!(e, EditorEvent::FieldEdited { .. })));

        // Scene doc position.y should be 5 + 3 = 8.
        let cam_id = host.session().scene().entities[0].id;
        let pos = host
            .session()
            .scene()
            .get_value(cam_id, "Transform", "position")
            .unwrap();
        if let InspectorValue::Vec3(arr) = pos {
            assert!(
                (arr[1] - 8.0).abs() < 0.01,
                "Y should be 8.0, got {}",
                arr[1]
            );
        } else {
            panic!("expected Vec3");
        }
    }

    // ── Text field rejection ──────────────────────────────────────────────

    // (Stats and Camera don't have Text fields in the current scene format,
    //  but we can synthesise an InspectorValue::Text to confirm the error path.)

    // ── Play / pause / resume / stop ─────────────────────────────────────

    #[test]
    fn apply_delta_rejects_text_field_using_field_name() {
        let path = write_single_entity_scene("text_field");
        let mut host = EditorRuntimeHost::open_hsc(&path).unwrap();

        host.session.scene_mut().entities[0].components[0]
            .fields
            .push(crate::editor::InspectorField {
                name: "label".into(),
                value: InspectorValue::Text("camera-root".into()),
            });
        host.cursor_entity_idx = Some(0);
        host.cursor_component_idx = 0;
        host.cursor_field_idx = host.session.scene().entities[0].components[0].fields.len() - 1;

        let err = host.execute(EditorCommand::ApplyDelta(1.0)).unwrap_err();
        match err {
            EditorCommandError::UnsupportedFieldKind { field } => {
                assert_eq!(field, "label");
            }
            other => panic!("expected UnsupportedFieldKind, got {other:?}"),
        }
    }

    #[test]
    fn play_pause_resume_stop_full_cycle() {
        let path = write_two_entity_scene("play");
        let mut host = EditorRuntimeHost::open_hsc(&path).unwrap();

        // Enter play.
        let evs = host.execute(EditorCommand::EnterPlayMode).unwrap();
        assert!(evs
            .iter()
            .any(|e| matches!(e, EditorEvent::PlayModeChanged(EditorPlayState::Playing))));
        assert_eq!(host.play_state(), EditorPlayState::Playing);

        // Pause.
        let evs = host.execute(EditorCommand::PausePlayMode).unwrap();
        assert!(evs
            .iter()
            .any(|e| matches!(e, EditorEvent::PlayModeChanged(EditorPlayState::Paused))));
        assert_eq!(host.play_state(), EditorPlayState::Paused);

        // Resume.
        let evs = host.execute(EditorCommand::ResumePlayMode).unwrap();
        assert!(evs
            .iter()
            .any(|e| matches!(e, EditorEvent::PlayModeChanged(EditorPlayState::Playing))));
        assert_eq!(host.play_state(), EditorPlayState::Playing);

        // Stop.
        let evs = host.execute(EditorCommand::StopPlayMode).unwrap();
        assert!(evs
            .iter()
            .any(|e| matches!(e, EditorEvent::PlayModeChanged(EditorPlayState::Editing))));
        assert_eq!(host.play_state(), EditorPlayState::Editing);
    }

    #[test]
    fn paused_transient_edits_are_discarded_on_stop() {
        let path = write_single_entity_scene("paused");
        let mut host = EditorRuntimeHost::open_hsc(&path).unwrap();

        let cam_id = host.session().scene().entities[0].id;
        let orig_fov = {
            if let Some(InspectorValue::Number(n)) = host
                .session()
                .scene()
                .get_value(cam_id, "Camera", "fov_deg")
            {
                *n
            } else {
                75.0
            }
        };

        host.execute(EditorCommand::EnterPlayMode).unwrap();
        host.execute(EditorCommand::PausePlayMode).unwrap();

        // Apply a transient delta to fov.
        // Navigate to Camera.fov_deg.
        let cam_idx = 0usize;
        host.cursor_entity_idx = Some(cam_idx);
        let cam_comp_idx = host.session().scene().entities[cam_idx]
            .components
            .iter()
            .position(|c| c.type_name == "Camera")
            .unwrap();
        host.cursor_component_idx = cam_comp_idx;
        let fov_idx = host.session().scene().entities[cam_idx].components[cam_comp_idx]
            .fields
            .iter()
            .position(|f| f.name == "fov_deg")
            .unwrap();
        host.cursor_field_idx = fov_idx;
        host.execute(EditorCommand::ApplyDelta(20.0)).unwrap();

        // Stop — authored snapshot must be restored.
        host.execute(EditorCommand::StopPlayMode).unwrap();

        let restored_fov = host
            .session()
            .scene()
            .get_value(cam_id, "Camera", "fov_deg")
            .cloned();
        assert_eq!(
            restored_fov,
            Some(InspectorValue::Number(orig_fov)),
            "fov_deg must revert to {} after stop",
            orig_fov
        );
    }

    #[test]
    fn persistent_dirty_survives_play_cycle() {
        let path = write_single_entity_scene("dirty");
        let mut host = EditorRuntimeHost::open_hsc(&path).unwrap();

        // Dirty the scene in Editing.
        host.cursor_entity_idx = Some(0);
        host.cursor_component_idx = 0; // Transform
        host.cursor_field_idx = 0; // position
        host.cursor_axis = Some(EditorAxis::X);
        host.execute(EditorCommand::ApplyDelta(1.0)).unwrap();
        assert!(host.is_dirty(), "should be dirty after edit");

        host.execute(EditorCommand::EnterPlayMode).unwrap();
        assert!(!host.is_dirty(), "not dirty during play (playing state)");

        host.execute(EditorCommand::StopPlayMode).unwrap();
        assert!(host.is_dirty(), "dirty flag must survive a play cycle");
    }

    // ── Patch / rebuild ───────────────────────────────────────────────────

    #[test]
    fn force_patch_emits_patch_applied() {
        let path = write_single_entity_scene("patch");
        let mut host = EditorRuntimeHost::open_hsc(&path).unwrap();

        let evs = host.execute(EditorCommand::ForcePatch).unwrap();
        assert!(
            evs.iter().any(|e| matches!(e, EditorEvent::PatchApplied)),
            "ForcePatch on a synced world must emit PatchApplied"
        );
    }

    #[test]
    fn force_rebuild_emits_world_rebuilt() {
        let path = write_two_entity_scene("rebuild");
        let mut host = EditorRuntimeHost::open_hsc(&path).unwrap();

        let evs = host.execute(EditorCommand::ForceRebuild).unwrap();
        assert!(
            evs.iter().any(|e| matches!(e, EditorEvent::WorldRebuilt)),
            "ForceRebuild must emit WorldRebuilt"
        );
    }

    // ── Selection preservation across rebuild ─────────────────────────────

    #[test]
    fn cursor_restored_by_name_after_rebuild() {
        let path = write_two_entity_scene("restore");
        let mut host = EditorRuntimeHost::open_hsc(&path).unwrap();

        // Navigate to player entity, Stats component.
        let player_idx = host
            .session()
            .scene()
            .entities
            .iter()
            .position(|e| e.name == "player")
            .unwrap();
        host.cursor_entity_idx = Some(player_idx);
        let stats_comp_idx = host.session().scene().entities[player_idx]
            .components
            .iter()
            .position(|c| c.type_name == "Stats")
            .unwrap();
        host.cursor_component_idx = stats_comp_idx;

        let player_id = host.session().scene().entities[player_idx].id;
        host.select_entity(player_id).unwrap();

        // Record component/field names.
        let comp_name = host.session().scene().entities[player_idx].components[stats_comp_idx]
            .type_name
            .clone();
        let field_name = host.session().scene().entities[player_idx].components[stats_comp_idx]
            .fields[0]
            .name
            .clone();

        host.execute(EditorCommand::ForceRebuild).unwrap();

        let cursor = host.cursor().unwrap();
        assert_eq!(cursor.entity_id, player_id);
        assert_eq!(cursor.component_name, comp_name);
        assert_eq!(cursor.field_name, field_name);
    }

    // ── UnsupportedCommandState errors ────────────────────────────────────

    #[test]
    fn save_rejected_during_play() {
        let path = write_single_entity_scene("save_play");
        let mut host = EditorRuntimeHost::open_hsc(&path).unwrap();

        host.execute(EditorCommand::EnterPlayMode).unwrap();
        let err = host.execute(EditorCommand::Save).unwrap_err();
        assert!(
            matches!(err, EditorCommandError::UnsupportedCommandState { .. }),
            "Save must be rejected in Playing state"
        );
    }

    #[test]
    fn apply_delta_rejected_during_play() {
        let path = write_single_entity_scene("delta_play");
        let mut host = EditorRuntimeHost::open_hsc(&path).unwrap();

        host.execute(EditorCommand::EnterPlayMode).unwrap();
        let err = host.execute(EditorCommand::ApplyDelta(1.0)).unwrap_err();
        assert!(
            matches!(err, EditorCommandError::UnsupportedCommandState { .. }),
            "ApplyDelta must be rejected in Playing state"
        );
    }

    // ── Existing host tests (smoke) ────────────────────────────────────────

    #[test]
    fn reload_source_restores_scene_and_clears_dirty() {
        let path = write_single_entity_scene("reload");
        let mut host = EditorRuntimeHost::open_hsc(&path).unwrap();
        let cursor = host.cursor().unwrap();

        host.execute(EditorCommand::ApplyDelta(2.0)).unwrap();
        assert!(host.is_dirty());

        let events = host.execute(EditorCommand::ReloadSource).unwrap();
        assert!(events
            .iter()
            .any(|event| matches!(event, EditorEvent::Reloaded)));
        assert!(events
            .iter()
            .any(|event| matches!(event, EditorEvent::WorldRebuilt)));
        assert!(!host.is_dirty());
        assert!(!host.world_requires_rebuild());
        assert_eq!(
            host.session()
                .scene()
                .get_value(cursor.entity_id, "Transform", "position"),
            Some(&InspectorValue::Vec3([0.0, 5.0, 0.0]))
        );
    }

    #[test]
    fn stale_force_patch_marks_world_as_rebuild_required() {
        let path = write_single_entity_scene("stale_patch");
        let mut host = EditorRuntimeHost::open_hsc(&path).unwrap();
        let runtime_entity = host.selected_runtime_entity().unwrap();

        host.world_mut().despawn(runtime_entity);
        let events = host.execute(EditorCommand::ForcePatch).unwrap();

        assert!(events
            .iter()
            .any(|event| matches!(event, EditorEvent::PatchRequiresRebuild { .. })));
        assert!(host.world_requires_rebuild());
    }

    #[test]
    fn protocol_message_executes_and_roundtrips_to_encoded_events() {
        let path = write_two_entity_scene("protocol");
        let mut host = EditorRuntimeHost::open_hsc(&path).unwrap();

        let encoded_events = host
            .execute_protocol_message("cmd|SelectNextEntity")
            .unwrap();
        assert_eq!(encoded_events.len(), 1);

        let decoded_event = crate::editor::protocol::decode_event(&encoded_events[0]).unwrap();
        match decoded_event {
            EditorEvent::SelectionChanged(cursor) => {
                assert_eq!(cursor.entity_id, host.cursor().unwrap().entity_id);
            }
            other => panic!("expected SelectionChanged, got {other:?}"),
        }
    }

    #[test]
    fn host_opens_scene_and_rebuilds_runtime_world() {
        let path = write_two_entity_scene("host_open");
        let mut host = EditorRuntimeHost::open_hsc(&path).unwrap();
        let camera_id = host.select_first_with_component("Camera").unwrap();
        let runtime_camera = host.applied_entity(camera_id).unwrap();

        assert!(host.world().has_component::<Transform>(runtime_camera));
        assert!(host.world().has_component::<Camera>(runtime_camera));
        assert!(host.world().has_component::<ActiveCamera>(runtime_camera));
    }

    #[test]
    fn host_mutates_and_patches_live_world() {
        let path = write_two_entity_scene("host_patch");
        let mut host = EditorRuntimeHost::open_hsc(&path).unwrap();
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
        let path = write_two_entity_scene("host_play");
        let mut host = EditorRuntimeHost::open_hsc(&path).unwrap();
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
