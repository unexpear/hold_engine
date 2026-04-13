//! Viewport-side selection and gizmo interaction helpers for the native shell.
//!
//! This module projects the live editor world into screen space so the shell
//! can:
//! - click-select entities directly in the viewport, and
//! - manipulate `Transform.position` through a simple axis gizmo.

use crate::ecs::components::Transform;
use crate::math::{Mat4, Vec3};
use crate::renderer::{Color, DebugLine, ScreenQuad};

use super::{EditorAxis, EditorCommand, EditorPlayState, EditorRuntimeHost};

const ENTITY_PICK_RADIUS_PX: f32 = 14.0;
const GIZMO_HANDLE_HALF_PX: f32 = 7.0;
const GIZMO_DISTANCE_SCALE: f32 = 0.08;
const GIZMO_MIN_WORLD_SIZE: f32 = 0.8;
const GIZMO_MAX_WORLD_SIZE: f32 = 3.5;

const AXIS_X_COLOR: Color = Color::new(1.0, 0.28, 0.28, 1.0);
const AXIS_Y_COLOR: Color = Color::new(0.30, 0.95, 0.36, 1.0);
const AXIS_Z_COLOR: Color = Color::new(0.30, 0.66, 1.0, 1.0);
const AXIS_ACTIVE_TINT: f32 = 0.25;

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
    pub origin_screen: [f32; 2],
    pub end_screen: [f32; 2],
    pub world_size: f32,
}

impl EditorViewportGizmoHandle {
    pub fn activation_commands(self) -> Vec<EditorCommand> {
        vec![
            EditorCommand::SelectField {
                entity_id: self.entity_id,
                component_name: "Transform".into(),
                field_name: "position".into(),
            },
            EditorCommand::SetAxis(self.axis),
        ]
    }

    pub fn world_delta_for_mouse_delta(self, dx: i32, dy: i32) -> Option<f32> {
        let axis_x = self.end_screen[0] - self.origin_screen[0];
        let axis_y = self.end_screen[1] - self.origin_screen[1];
        let axis_len = (axis_x * axis_x + axis_y * axis_y).sqrt();
        if axis_len < 8.0 {
            return None;
        }

        let motion = dx as f32 * (axis_x / axis_len) + dy as f32 * (axis_y / axis_len);
        Some((motion / axis_len) * self.world_size)
    }

    fn contains(self, x: f32, y: f32) -> bool {
        x >= self.end_screen[0] - GIZMO_HANDLE_HALF_PX
            && x <= self.end_screen[0] + GIZMO_HANDLE_HALF_PX
            && y >= self.end_screen[1] - GIZMO_HANDLE_HALF_PX
            && y <= self.end_screen[1] + GIZMO_HANDLE_HALF_PX
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
) -> EditorViewportOverlay {
    let mut overlay = EditorViewportOverlay::default();
    collect_entity_hits(host, vp, width, height, &mut overlay);
    append_transform_gizmo(host, vp, width, height, &mut overlay);
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

fn append_transform_gizmo(
    host: &EditorRuntimeHost,
    vp: &Mat4,
    width: u32,
    height: u32,
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
    let active_axis = (cursor.component_name == "Transform" && cursor.field_name == "position")
        .then_some(cursor.axis.unwrap_or(EditorAxis::X));

    for (axis, world_dir, color) in [
        (EditorAxis::X, Vec3::X, AXIS_X_COLOR),
        (EditorAxis::Y, Vec3::Y, AXIS_Y_COLOR),
        (EditorAxis::Z, Vec3::Z, AXIS_Z_COLOR),
    ] {
        let end_world = transform.position + world_dir * gizmo_world_size;
        let Some(end) = project_world_to_screen(vp, width, height, end_world) else {
            continue;
        };

        let axis_screen_len = ((end.x - origin.x).powi(2) + (end.y - origin.y).powi(2)).sqrt();
        if axis_screen_len < 12.0 {
            continue;
        }

        let tinted = tint_axis_color(color, active_axis == Some(axis));
        overlay
            .debug_lines
            .push(DebugLine::new(transform.position, end_world, tinted));
        overlay.quads.push(ScreenQuad::new(
            end.x - GIZMO_HANDLE_HALF_PX,
            end.y - GIZMO_HANDLE_HALF_PX,
            GIZMO_HANDLE_HALF_PX * 2.0,
            GIZMO_HANDLE_HALF_PX * 2.0,
            tinted,
        ));
        overlay.gizmo_handles.push(EditorViewportGizmoHandle {
            entity_id: cursor.entity_id,
            axis,
            origin_screen: [origin.x, origin.y],
            end_screen: [end.x, end.y],
            world_size: gizmo_world_size,
        });
    }
}

fn tint_axis_color(color: Color, active: bool) -> Color {
    if !active {
        return color;
    }
    color.lerp(Color::WHITE, AXIS_ACTIVE_TINT)
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
        EditorEntity, EditorSceneSession, InspectorComponent, InspectorField, SceneDocument,
    };
    fn transform_component(position: [f32; 3]) -> InspectorComponent {
        InspectorComponent {
            type_name: "Transform".into(),
            fields: vec![
                InspectorField {
                    name: "position".into(),
                    value: super::super::InspectorValue::Vec3(position),
                },
                InspectorField {
                    name: "rotation_x".into(),
                    value: super::super::InspectorValue::Number(0.0),
                },
                InspectorField {
                    name: "rotation_y".into(),
                    value: super::super::InspectorValue::Number(0.0),
                },
                InspectorField {
                    name: "rotation_z".into(),
                    value: super::super::InspectorValue::Number(0.0),
                },
                InspectorField {
                    name: "rotation_w".into(),
                    value: super::super::InspectorValue::Number(1.0),
                },
                InspectorField {
                    name: "scale".into(),
                    value: super::super::InspectorValue::Vec3([1.0, 1.0, 1.0]),
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
        let overlay = build_viewport_overlay(&host, &vp, 1280, 720);

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
    fn viewport_gizmo_exposes_axis_handle_and_delta() {
        let mut host = test_host();
        host.execute(EditorCommand::SelectField {
            entity_id: 1,
            component_name: "Transform".into(),
            field_name: "position".into(),
        })
        .unwrap();

        let vp = host.active_view_proj(60.0, 16.0 / 9.0);
        let overlay = build_viewport_overlay(&host, &vp, 1280, 720);

        assert!(overlay.gizmo_handles.len() >= 2);
        let handle = overlay.gizmo_handles[0];
        assert_eq!(
            overlay
                .pick_gizmo_handle(handle.end_screen[0] as i32, handle.end_screen[1] as i32)
                .map(|picked| picked.axis),
            Some(handle.axis)
        );
        assert!(handle.world_delta_for_mouse_delta(12, 0).is_some());
    }
}
