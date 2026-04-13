//! Viewport-side selection and gizmo interaction helpers for the native shell.
//!
//! This module projects the live editor world into screen space so the shell
//! can:
//! - click-select entities directly in the viewport,
//! - show hover/selection feedback in screen space, and
//! - manipulate `Transform.position`, `Transform.rotation_euler_deg`, and
//!   `Transform.scale` through simple axis gizmos.

use crate::ecs::components::Transform;
use crate::math::{Mat4, Vec3};
use crate::renderer::{Color, DebugLine, ScreenQuad};

use super::{EditorAxis, EditorCommand, EditorPlayState, EditorRuntimeHost};

const ENTITY_PICK_RADIUS_PX: f32 = 14.0;
const GIZMO_HANDLE_HALF_PX: f32 = 7.0;
const GIZMO_DISTANCE_SCALE: f32 = 0.08;
const GIZMO_MIN_WORLD_SIZE: f32 = 0.8;
const GIZMO_MAX_WORLD_SIZE: f32 = 3.5;
const ROTATE_DEGREES_PER_DRAG: f32 = 90.0;

const AXIS_X_COLOR: Color = Color::new(1.0, 0.28, 0.28, 1.0);
const AXIS_Y_COLOR: Color = Color::new(0.30, 0.95, 0.36, 1.0);
const AXIS_Z_COLOR: Color = Color::new(0.30, 0.66, 1.0, 1.0);
const AXIS_ACTIVE_TINT: f32 = 0.25;
const AXIS_HOVER_TINT: f32 = 0.40;
const HOVER_OUTLINE_COLOR: Color = Color::new(0.95, 0.95, 1.0, 1.0);
const SELECT_OUTLINE_COLOR: Color = Color::new(0.18, 1.0, 0.28, 1.0);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorViewportGizmoMode {
    Translate,
    Rotate,
    Scale,
}

impl std::fmt::Display for EditorViewportGizmoMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Translate => f.write_str("MOVE"),
            Self::Rotate => f.write_str("ROT"),
            Self::Scale => f.write_str("SCALE"),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct EditorViewportOverlay {
    pub debug_lines: Vec<DebugLine>,
    pub quads: Vec<ScreenQuad>,
    entity_hits: Vec<EditorViewportEntityHit>,
    gizmo_handles: Vec<EditorViewportGizmoHandle>,
}

impl EditorViewportOverlay {
    pub fn pick_entity(&self, x: i32, y: i32) -> Option<u64> {
        let mx = x as f32;
        let my = y as f32;
        self.entity_hits
            .iter()
            .filter_map(|hit| {
                let dx = hit.screen_x - mx;
                let dy = hit.screen_y - my;
                let dist_sq = dx * dx + dy * dy;
                (dist_sq <= ENTITY_PICK_RADIUS_PX * ENTITY_PICK_RADIUS_PX).then_some((
                    hit.entity_id,
                    dist_sq,
                    hit.depth,
                ))
            })
            .min_by(|a, b| {
                a.1.partial_cmp(&b.1)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal))
            })
            .map(|(entity_id, _, _)| entity_id)
    }

    pub fn pick_gizmo_handle(&self, x: i32, y: i32) -> Option<EditorViewportGizmoHandle> {
        let mx = x as f32;
        let my = y as f32;
        self.gizmo_handles
            .iter()
            .rev()
            .find(|handle| handle.contains(mx, my))
            .copied()
    }
}

#[derive(Debug, Clone, Copy)]
pub struct EditorViewportGizmoHandle {
    pub entity_id: u64,
    pub axis: EditorAxis,
    pub mode: EditorViewportGizmoMode,
    pick_screen: [f32; 2],
    drag_vector: [f32; 2],
    delta_scale: f32,
    field_name: &'static str,
}

impl EditorViewportGizmoHandle {
    pub fn activation_commands(self) -> Vec<EditorCommand> {
        vec![
            EditorCommand::SelectField {
                entity_id: self.entity_id,
                component_name: "Transform".into(),
                field_name: self.field_name.into(),
            },
            EditorCommand::SetAxis(self.axis),
        ]
    }

    pub fn world_delta_for_mouse_delta(self, dx: i32, dy: i32) -> Option<f32> {
        let axis_x = self.drag_vector[0];
        let axis_y = self.drag_vector[1];
        let axis_len = (axis_x * axis_x + axis_y * axis_y).sqrt();
        if axis_len < 8.0 {
            return None;
        }

        let motion = dx as f32 * (axis_x / axis_len) + dy as f32 * (axis_y / axis_len);
        Some((motion / axis_len) * self.delta_scale)
    }

    pub fn field_name(self) -> &'static str {
        self.field_name
    }

    fn contains(self, x: f32, y: f32) -> bool {
        x >= self.pick_screen[0] - GIZMO_HANDLE_HALF_PX
            && x <= self.pick_screen[0] + GIZMO_HANDLE_HALF_PX
            && y >= self.pick_screen[1] - GIZMO_HANDLE_HALF_PX
            && y <= self.pick_screen[1] + GIZMO_HANDLE_HALF_PX
    }
}

#[derive(Debug, Clone, Copy)]
struct EditorViewportEntityHit {
    entity_id: u64,
    screen_x: f32,
    screen_y: f32,
    depth: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct ProjectedPoint {
    x: f32,
    y: f32,
    depth: f32,
}

pub fn build_viewport_overlay(
    host: &EditorRuntimeHost,
    vp: &Mat4,
    width: u32,
    height: u32,
    mode: EditorViewportGizmoMode,
    hovered_entity: Option<u64>,
    hovered_axis: Option<EditorAxis>,
) -> EditorViewportOverlay {
    let mut overlay = EditorViewportOverlay::default();
    collect_entity_hits(host, vp, width, height, &mut overlay);
    append_selection_feedback(host, hovered_entity, &mut overlay);
    append_transform_gizmo(host, vp, width, height, mode, hovered_axis, &mut overlay);
    overlay
}

fn collect_entity_hits(
    host: &EditorRuntimeHost,
    vp: &Mat4,
    width: u32,
    height: u32,
    overlay: &mut EditorViewportOverlay,
) {
    for &(editor_id, runtime_entity) in host.session().last_world_map() {
        if !host.world().is_alive(runtime_entity) {
            continue;
        }
        let Some(transform) = host.world().get::<Transform>(runtime_entity) else {
            continue;
        };
        let Some(projected) = project_world_to_screen(vp, width, height, transform.position) else {
            continue;
        };
        overlay.entity_hits.push(EditorViewportEntityHit {
            entity_id: editor_id,
            screen_x: projected.x,
            screen_y: projected.y,
            depth: projected.depth,
        });
    }
}

fn append_selection_feedback(
    host: &EditorRuntimeHost,
    hovered_entity: Option<u64>,
    overlay: &mut EditorViewportOverlay,
) {
    let selected_entity = host.cursor().map(|cursor| cursor.entity_id);
    if let Some(entity_id) = selected_entity.and_then(|id| find_entity_hit(overlay, id)) {
        push_outline_rect(
            overlay,
            entity_id.screen_x - 11.0,
            entity_id.screen_y - 11.0,
            22.0,
            22.0,
            SELECT_OUTLINE_COLOR,
        );
    }

    if let Some(hovered) = hovered_entity
        .filter(|hovered| Some(*hovered) != selected_entity)
        .and_then(|id| find_entity_hit(overlay, id))
    {
        push_outline_rect(
            overlay,
            hovered.screen_x - 9.0,
            hovered.screen_y - 9.0,
            18.0,
            18.0,
            HOVER_OUTLINE_COLOR,
        );
    }
}

fn find_entity_hit(
    overlay: &EditorViewportOverlay,
    entity_id: u64,
) -> Option<EditorViewportEntityHit> {
    overlay
        .entity_hits
        .iter()
        .find(|hit| hit.entity_id == entity_id)
        .copied()
}

fn append_transform_gizmo(
    host: &EditorRuntimeHost,
    vp: &Mat4,
    width: u32,
    height: u32,
    mode: EditorViewportGizmoMode,
    hovered_axis: Option<EditorAxis>,
    overlay: &mut EditorViewportOverlay,
) {
    if host.play_state() == EditorPlayState::Playing {
        return;
    }

    let Some(cursor) = host.cursor() else {
        return;
    };
    let Some(runtime_entity) = host
        .session()
        .last_world_map()
        .iter()
        .find(|(entity_id, _)| *entity_id == cursor.entity_id)
        .map(|(_, entity)| *entity)
    else {
        return;
    };
    let Some(transform) = host.world().get::<Transform>(runtime_entity) else {
        return;
    };
    let Some(origin) = project_world_to_screen(vp, width, height, transform.position) else {
        return;
    };

    let distance = (transform.position - host.editor_camera.position).length();
    let gizmo_world_size =
        (distance * GIZMO_DISTANCE_SCALE).clamp(GIZMO_MIN_WORLD_SIZE, GIZMO_MAX_WORLD_SIZE);

    match mode {
        EditorViewportGizmoMode::Translate => append_translate_or_scale_gizmo(
            host,
            vp,
            width,
            height,
            transform.position,
            origin,
            gizmo_world_size,
            1.0,
            "position",
            mode,
            hovered_axis,
            overlay,
        ),
        EditorViewportGizmoMode::Scale => append_translate_or_scale_gizmo(
            host,
            vp,
            width,
            height,
            transform.position,
            origin,
            gizmo_world_size * 0.85,
            1.0,
            "scale",
            mode,
            hovered_axis,
            overlay,
        ),
        EditorViewportGizmoMode::Rotate => append_rotate_gizmo(
            host,
            vp,
            width,
            height,
            transform.position,
            gizmo_world_size,
            hovered_axis,
            overlay,
        ),
    }
}

#[allow(clippy::too_many_arguments)]
fn append_translate_or_scale_gizmo(
    host: &EditorRuntimeHost,
    vp: &Mat4,
    width: u32,
    height: u32,
    origin_world: Vec3,
    origin_screen: ProjectedPoint,
    gizmo_world_size: f32,
    delta_scale: f32,
    field_name: &'static str,
    mode: EditorViewportGizmoMode,
    hovered_axis: Option<EditorAxis>,
    overlay: &mut EditorViewportOverlay,
) {
    let active_axis = cursor_active_axis(host, field_name);

    for (axis, world_dir, color) in [
        (EditorAxis::X, Vec3::X, AXIS_X_COLOR),
        (EditorAxis::Y, Vec3::Y, AXIS_Y_COLOR),
        (EditorAxis::Z, Vec3::Z, AXIS_Z_COLOR),
    ] {
        let end_world = origin_world + world_dir * gizmo_world_size;
        let Some(end) = project_world_to_screen(vp, width, height, end_world) else {
            continue;
        };

        let drag_vector = [end.x - origin_screen.x, end.y - origin_screen.y];
        let axis_screen_len = (drag_vector[0] * drag_vector[0] + drag_vector[1] * drag_vector[1]).sqrt();
        if axis_screen_len < 12.0 {
            continue;
        }

        let tinted = tint_axis_color(color, active_axis == Some(axis), hovered_axis == Some(axis));
        overlay
            .debug_lines
            .push(DebugLine::new(origin_world, end_world, tinted));
        overlay.quads.push(ScreenQuad::new(
            end.x - GIZMO_HANDLE_HALF_PX,
            end.y - GIZMO_HANDLE_HALF_PX,
            GIZMO_HANDLE_HALF_PX * 2.0,
            GIZMO_HANDLE_HALF_PX * 2.0,
            tinted,
        ));
        overlay.gizmo_handles.push(EditorViewportGizmoHandle {
            entity_id: host.cursor().map(|cursor| cursor.entity_id).unwrap_or_default(),
            axis,
            mode,
            pick_screen: [end.x, end.y],
            drag_vector,
            delta_scale: delta_scale
                * if field_name == "position" {
                    gizmo_world_size
                } else {
                    1.0
                },
            field_name,
        });
    }
}

fn append_rotate_gizmo(
    host: &EditorRuntimeHost,
    vp: &Mat4,
    width: u32,
    height: u32,
    origin_world: Vec3,
    radius: f32,
    hovered_axis: Option<EditorAxis>,
    overlay: &mut EditorViewportOverlay,
) {
    let entity_id = host.cursor().map(|cursor| cursor.entity_id).unwrap_or_default();
    let active_axis = cursor_active_axis(host, "rotation_euler_deg");

    for (axis, basis_a, basis_b, handle_dir, color) in [
        (EditorAxis::X, Vec3::Y, Vec3::Z, Vec3::Y, AXIS_X_COLOR),
        (EditorAxis::Y, Vec3::Z, Vec3::X, Vec3::Z, AXIS_Y_COLOR),
        (EditorAxis::Z, Vec3::X, Vec3::Y, Vec3::X, AXIS_Z_COLOR),
    ] {
        let tinted = tint_axis_color(color, active_axis == Some(axis), hovered_axis == Some(axis));
        append_ring_lines(origin_world, basis_a, basis_b, radius, tinted, overlay);

        let handle_world = origin_world + handle_dir * radius;
        let tangent_world = basis_b * radius * 0.35;
        let Some(handle_screen) = project_world_to_screen(vp, width, height, handle_world) else {
            continue;
        };
        let Some(tangent_screen) =
            project_world_to_screen(vp, width, height, handle_world + tangent_world)
        else {
            continue;
        };

        let drag_vector = [tangent_screen.x - handle_screen.x, tangent_screen.y - handle_screen.y];
        let axis_screen_len = (drag_vector[0] * drag_vector[0] + drag_vector[1] * drag_vector[1]).sqrt();
        if axis_screen_len < 8.0 {
            continue;
        }

        overlay.quads.push(ScreenQuad::new(
            handle_screen.x - GIZMO_HANDLE_HALF_PX,
            handle_screen.y - GIZMO_HANDLE_HALF_PX,
            GIZMO_HANDLE_HALF_PX * 2.0,
            GIZMO_HANDLE_HALF_PX * 2.0,
            tinted,
        ));
        overlay.gizmo_handles.push(EditorViewportGizmoHandle {
            entity_id,
            axis,
            mode: EditorViewportGizmoMode::Rotate,
            pick_screen: [handle_screen.x, handle_screen.y],
            drag_vector,
            delta_scale: ROTATE_DEGREES_PER_DRAG,
            field_name: "rotation_euler_deg",
        });
    }
}

fn append_ring_lines(
    origin_world: Vec3,
    basis_a: Vec3,
    basis_b: Vec3,
    radius: f32,
    color: Color,
    overlay: &mut EditorViewportOverlay,
) {
    let segments = 24;
    let mut prev = origin_world + basis_a * radius;
    for step in 1..=segments {
        let t = step as f32 / segments as f32;
        let angle = t * std::f32::consts::TAU;
        let point = origin_world + (basis_a * angle.cos() + basis_b * angle.sin()) * radius;
        overlay.debug_lines.push(DebugLine::new(prev, point, color));
        prev = point;
    }
}

fn cursor_active_axis(host: &EditorRuntimeHost, field_name: &str) -> Option<EditorAxis> {
    host.cursor().and_then(|cursor| {
        (cursor.component_name == "Transform" && cursor.field_name == field_name)
            .then_some(cursor.axis.unwrap_or(EditorAxis::X))
    })
}

fn tint_axis_color(color: Color, active: bool, hovered: bool) -> Color {
    let color = if active {
        color.lerp(Color::WHITE, AXIS_ACTIVE_TINT)
    } else {
        color
    };
    if hovered {
        color.lerp(Color::WHITE, AXIS_HOVER_TINT)
    } else {
        color
    }
}

fn push_outline_rect(
    overlay: &mut EditorViewportOverlay,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    color: Color,
) {
    overlay.quads.push(ScreenQuad::new(x, y, w, 1.0, color));
    overlay.quads.push(ScreenQuad::new(x, y + h - 1.0, w, 1.0, color));
    overlay.quads.push(ScreenQuad::new(x, y, 1.0, h, color));
    overlay.quads.push(ScreenQuad::new(x + w - 1.0, y, 1.0, h, color));
}

fn project_world_to_screen(
    vp: &Mat4,
    width: u32,
    height: u32,
    point: Vec3,
) -> Option<ProjectedPoint> {
    let clip = vp.transform_point(point);
    if clip.w.abs() <= 1e-5 {
        return None;
    }

    let ndc = clip.perspective_divide();
    if ndc.z < -1.0 || ndc.z > 1.0 {
        return None;
    }
    if ndc.x < -1.15 || ndc.x > 1.15 || ndc.y < -1.15 || ndc.y > 1.15 {
        return None;
    }

    Some(ProjectedPoint {
        x: ((ndc.x * 0.5) + 0.5) * width as f32,
        y: (1.0 - ((ndc.y * 0.5) + 0.5)) * height as f32,
        depth: ndc.z,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::{
        EditorEntity, EditorSceneSession, InspectorComponent, InspectorField, InspectorValue,
        SceneDocument,
    };

    fn transform_component(position: [f32; 3]) -> InspectorComponent {
        InspectorComponent {
            type_name: "Transform".into(),
            fields: vec![
                InspectorField {
                    name: "position".into(),
                    value: InspectorValue::Vec3(position),
                },
                InspectorField {
                    name: "scale".into(),
                    value: InspectorValue::Vec3([1.0, 1.0, 1.0]),
                },
                InspectorField {
                    name: "rotation_euler_deg".into(),
                    value: InspectorValue::Vec3([0.0, 0.0, 0.0]),
                },
            ],
        }
    }

    fn test_host() -> EditorRuntimeHost {
        let scene = SceneDocument {
            name: "Viewport".into(),
            entities: vec![
                EditorEntity {
                    id: 1,
                    name: "Root".into(),
                    components: vec![transform_component([0.0, 0.0, 0.0])],
                },
                EditorEntity {
                    id: 2,
                    name: "Offset".into(),
                    components: vec![transform_component([2.0, 0.0, 0.0])],
                },
            ],
        };

        let mut host = EditorRuntimeHost::new(EditorSceneSession::new(scene)).unwrap();
        host.editor_camera.position = Vec3::new(0.0, 2.0, 10.0);
        host.editor_camera.yaw = 0.0;
        host.editor_camera.pitch = -0.15;
        host
    }

    #[test]
    fn viewport_pick_selects_projected_entity() {
        let host = test_host();
        let vp = host.active_view_proj(60.0, 16.0 / 9.0);
        let overlay = build_viewport_overlay(
            &host,
            &vp,
            1280,
            720,
            EditorViewportGizmoMode::Translate,
            None,
            None,
        );

        let hit = overlay
            .entity_hits
            .iter()
            .find(|hit| hit.entity_id == 1)
            .copied()
            .unwrap();
        assert_eq!(
            overlay.pick_entity(hit.screen_x as i32, hit.screen_y as i32),
            Some(1)
        );
    }

    #[test]
    fn translate_gizmo_selects_position_field() {
        let mut host = test_host();
        host.execute(EditorCommand::SelectEntity { entity_id: 1 }).unwrap();

        let vp = host.active_view_proj(60.0, 16.0 / 9.0);
        let overlay = build_viewport_overlay(
            &host,
            &vp,
            1280,
            720,
            EditorViewportGizmoMode::Translate,
            None,
            None,
        );

        let handle = overlay.gizmo_handles[0];
        let commands = handle.activation_commands();
        assert!(matches!(
            &commands[0],
            EditorCommand::SelectField { field_name, .. } if field_name == "position"
        ));
    }

    #[test]
    fn rotate_gizmo_selects_rotation_euler_field() {
        let mut host = test_host();
        host.execute(EditorCommand::SelectEntity { entity_id: 1 }).unwrap();

        let vp = host.active_view_proj(60.0, 16.0 / 9.0);
        let overlay = build_viewport_overlay(
            &host,
            &vp,
            1280,
            720,
            EditorViewportGizmoMode::Rotate,
            None,
            None,
        );

        assert!(overlay.gizmo_handles.len() >= 2);
        let handle = overlay.gizmo_handles[0];
        let commands = handle.activation_commands();
        assert!(matches!(
            &commands[0],
            EditorCommand::SelectField { field_name, .. } if field_name == "rotation_euler_deg"
        ));
        assert!(handle.world_delta_for_mouse_delta(10, 0).is_some());
    }

    #[test]
    fn scale_gizmo_selects_scale_field() {
        let mut host = test_host();
        host.execute(EditorCommand::SelectEntity { entity_id: 1 }).unwrap();

        let vp = host.active_view_proj(60.0, 16.0 / 9.0);
        let overlay = build_viewport_overlay(
            &host,
            &vp,
            1280,
            720,
            EditorViewportGizmoMode::Scale,
            Some(2),
            None,
        );

        let handle = overlay.gizmo_handles[0];
        let commands = handle.activation_commands();
        assert!(matches!(
            &commands[0],
            EditorCommand::SelectField { field_name, .. } if field_name == "scale"
        ));
        assert!(!overlay.quads.is_empty(), "hover feedback should add outline quads");
    }
}
