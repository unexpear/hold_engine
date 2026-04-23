//! Viewport-side selection and gizmo interaction helpers for the native shell.
//!
//! This module projects the live editor world into screen space so the shell
//! can:
//! - click-select entities directly in the viewport,
//! - show hover/selection feedback in screen space, and
//! - manipulate `Transform.position`, `Transform.rotation_euler_deg`, and
//!   `Transform.scale` through axis, plane, and ring gizmos.

use crate::ecs::components::Transform;
use crate::math::{Mat4, Vec3};
use crate::renderer::{Color, DebugLine, ScreenQuad};

use super::{EditorAxis, EditorCommand, EditorPlayState, EditorRuntimeHost};

const ENTITY_PICK_RADIUS_PX: f32 = 14.0;
const PICK_DEPTH_BIAS_PX_SQ: f32 = 52.0;
const GIZMO_HANDLE_HALF_PX: f32 = 7.0;
const GIZMO_PLANE_HALF_PX: f32 = 10.0;
const GIZMO_RING_PICK_RADIUS_PX: f32 = 8.0;
const GIZMO_DISTANCE_SCALE: f32 = 0.08;
const GIZMO_MIN_WORLD_SIZE: f32 = 0.8;
const GIZMO_MAX_WORLD_SIZE: f32 = 3.5;
const PLANE_HANDLE_INNER: f32 = 0.20;
const PLANE_HANDLE_OUTER: f32 = 0.46;
const RING_BACKFACE_TINT: f32 = 0.62;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EditorViewportGizmoHandleId {
    pub mode: EditorViewportGizmoMode,
    pub field_name: &'static str,
    pub primary_axis: EditorAxis,
    pub secondary_axis: Option<EditorAxis>,
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
        let point = [x as f32, y as f32];
        self.entity_hits
            .iter()
            .filter_map(|hit| {
                let dist_sq = screen_distance_sq(point, [hit.screen_x, hit.screen_y]);
                (dist_sq <= ENTITY_PICK_RADIUS_PX * ENTITY_PICK_RADIUS_PX)
                    .then_some((hit.entity_id, depth_pick_score(dist_sq, hit.depth)))
            })
            .min_by(|a, b| compare_f32(a.1, b.1))
            .map(|(entity_id, _)| entity_id)
    }

    pub fn pick_gizmo_handle(&self, x: i32, y: i32) -> Option<EditorViewportGizmoHandle> {
        let point = [x as f32, y as f32];
        self.gizmo_handles
            .iter()
            .filter_map(|handle| handle.hit_test(point))
            .min_by(|a, b| compare_f32(a.score, b.score))
            .map(|candidate| candidate.handle)
    }
}

#[derive(Debug, Clone)]
pub struct EditorViewportGizmoHandle {
    pub entity_id: u64,
    pub axis: EditorAxis,
    pub mode: EditorViewportGizmoMode,
    depth: f32,
    secondary_axis: Option<EditorAxis>,
    shape: EditorViewportHandleShape,
    drag: EditorViewportHandleDrag,
    field_name: &'static str,
}

impl EditorViewportGizmoHandle {
    pub fn id(&self) -> EditorViewportGizmoHandleId {
        EditorViewportGizmoHandleId {
            mode: self.mode,
            field_name: self.field_name,
            primary_axis: self.axis,
            secondary_axis: self.secondary_axis,
        }
    }

    pub fn activation_commands(&self) -> Vec<EditorCommand> {
        vec![
            EditorCommand::SelectField {
                entity_id: self.entity_id,
                component_name: "Transform".into(),
                field_name: self.field_name.into(),
            },
            EditorCommand::SetAxis(self.axis),
        ]
    }

    pub fn world_deltas_for_mouse_delta(&self, dx: i32, dy: i32) -> Vec<(EditorAxis, f32)> {
        let motion = [dx as f32, dy as f32];
        match &self.drag {
            EditorViewportHandleDrag::SingleAxis {
                drag_vector,
                delta_scale,
            } => single_axis_delta(*drag_vector, *delta_scale, motion)
                .map(|delta| vec![(self.axis, delta)])
                .unwrap_or_default(),
            EditorViewportHandleDrag::DualAxis {
                drag_vectors,
                delta_scales,
            } => {
                let Some([coeff_a, coeff_b]) = solve_drag_basis(*drag_vectors, motion) else {
                    return Vec::new();
                };
                let mut out = Vec::with_capacity(2);
                let delta_a = coeff_a * delta_scales[0];
                if delta_a.abs() > 1e-6 {
                    out.push((self.axis, delta_a));
                }
                if let Some(axis_b) = self.secondary_axis {
                    let delta_b = coeff_b * delta_scales[1];
                    if delta_b.abs() > 1e-6 {
                        out.push((axis_b, delta_b));
                    }
                }
                out
            }
        }
    }

    pub fn field_name(&self) -> &'static str {
        self.field_name
    }

    fn hit_test(&self, point: [f32; 2]) -> Option<HandleHitCandidate> {
        match &self.shape {
            EditorViewportHandleShape::Rect {
                center,
                half_extent,
            } => {
                let within_x =
                    point[0] >= center[0] - half_extent && point[0] <= center[0] + half_extent;
                let within_y =
                    point[1] >= center[1] - half_extent && point[1] <= center[1] + half_extent;
                if !within_x || !within_y {
                    return None;
                }
                Some(HandleHitCandidate {
                    score: depth_pick_score(screen_distance_sq(point, *center), self.depth),
                    handle: self.clone(),
                })
            }
            EditorViewportHandleShape::Ring {
                segments,
                pick_radius,
                circumference,
            } => {
                let mut best: Option<(f32, HandlePickSegment)> = None;
                for segment in segments {
                    let dist_sq = point_segment_distance_sq(point, segment.start, segment.end);
                    if dist_sq > pick_radius * pick_radius {
                        continue;
                    }
                    match best {
                        Some((best_dist_sq, _)) if dist_sq >= best_dist_sq => {}
                        _ => best = Some((dist_sq, *segment)),
                    }
                }

                let (dist_sq, segment) = best?;
                let segment_vector = [
                    segment.end[0] - segment.start[0],
                    segment.end[1] - segment.start[1],
                ];
                let segment_len = vector_len(segment_vector);
                if segment_len < 4.0 {
                    return None;
                }

                let mut handle = self.clone();
                handle.drag = EditorViewportHandleDrag::SingleAxis {
                    drag_vector: segment_vector,
                    delta_scale: segment_len * 360.0 / circumference.max(segment_len),
                };
                Some(HandleHitCandidate {
                    score: depth_pick_score(dist_sq, segment.depth),
                    handle,
                })
            }
        }
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

#[derive(Debug, Clone)]
enum EditorViewportHandleShape {
    Rect {
        center: [f32; 2],
        half_extent: f32,
    },
    Ring {
        segments: Vec<HandlePickSegment>,
        pick_radius: f32,
        circumference: f32,
    },
}

#[derive(Debug, Clone)]
enum EditorViewportHandleDrag {
    SingleAxis {
        drag_vector: [f32; 2],
        delta_scale: f32,
    },
    DualAxis {
        drag_vectors: [[f32; 2]; 2],
        delta_scales: [f32; 2],
    },
}

#[derive(Debug, Clone, Copy)]
struct HandlePickSegment {
    start: [f32; 2],
    end: [f32; 2],
    depth: f32,
}

#[derive(Debug, Clone)]
struct HandleHitCandidate {
    score: f32,
    handle: EditorViewportGizmoHandle,
}

pub fn build_viewport_overlay(
    host: &EditorRuntimeHost,
    vp: &Mat4,
    width: u32,
    height: u32,
    mode: EditorViewportGizmoMode,
    hovered_entity: Option<u64>,
    hovered_handle: Option<EditorViewportGizmoHandleId>,
) -> EditorViewportOverlay {
    let mut overlay = EditorViewportOverlay::default();
    collect_entity_hits(host, vp, width, height, &mut overlay);
    append_selection_feedback(host, hovered_entity, &mut overlay);
    append_transform_gizmo(host, vp, width, height, mode, hovered_handle, &mut overlay);
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
    hovered_handle: Option<EditorViewportGizmoHandleId>,
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
            gizmo_world_size,
            "position",
            mode,
            hovered_handle,
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
            hovered_handle,
            overlay,
        ),
        EditorViewportGizmoMode::Rotate => append_rotate_gizmo(
            host,
            vp,
            width,
            height,
            transform.position,
            origin.depth,
            gizmo_world_size,
            hovered_handle,
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
    axis_world_size: f32,
    full_delta_scale: f32,
    field_name: &'static str,
    mode: EditorViewportGizmoMode,
    hovered_handle: Option<EditorViewportGizmoHandleId>,
    overlay: &mut EditorViewportOverlay,
) {
    let active_axis = cursor_active_axis(host, field_name);
    let entity_id = host
        .cursor()
        .map(|cursor| cursor.entity_id)
        .unwrap_or_default();

    for (axis, world_dir, color) in [
        (EditorAxis::X, Vec3::X, AXIS_X_COLOR),
        (EditorAxis::Y, Vec3::Y, AXIS_Y_COLOR),
        (EditorAxis::Z, Vec3::Z, AXIS_Z_COLOR),
    ] {
        let end_world = origin_world + world_dir * axis_world_size;
        let Some(end) = project_world_to_screen(vp, width, height, end_world) else {
            continue;
        };

        let drag_vector = [end.x - origin_screen.x, end.y - origin_screen.y];
        if vector_len(drag_vector) < 12.0 {
            continue;
        }

        let handle = EditorViewportGizmoHandle {
            entity_id,
            axis,
            mode,
            depth: end.depth,
            secondary_axis: None,
            shape: EditorViewportHandleShape::Rect {
                center: [end.x, end.y],
                half_extent: GIZMO_HANDLE_HALF_PX,
            },
            drag: EditorViewportHandleDrag::SingleAxis {
                drag_vector,
                delta_scale: full_delta_scale,
            },
            field_name,
        };
        let tinted = tint_axis_color(
            color,
            active_axis == Some(axis),
            hovered_handle == Some(handle.id()),
        );

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
        overlay.gizmo_handles.push(handle);
    }

    append_plane_handles(
        entity_id,
        vp,
        width,
        height,
        origin_world,
        origin_screen,
        axis_world_size,
        full_delta_scale,
        field_name,
        mode,
        active_axis,
        hovered_handle,
        overlay,
    );
}

#[allow(clippy::too_many_arguments)]
fn append_plane_handles(
    entity_id: u64,
    vp: &Mat4,
    width: u32,
    height: u32,
    origin_world: Vec3,
    origin_screen: ProjectedPoint,
    axis_world_size: f32,
    full_delta_scale: f32,
    field_name: &'static str,
    mode: EditorViewportGizmoMode,
    active_axis: Option<EditorAxis>,
    hovered_handle: Option<EditorViewportGizmoHandleId>,
    overlay: &mut EditorViewportOverlay,
) {
    for (axis_a, dir_a, color_a, axis_b, dir_b, color_b) in [
        (
            EditorAxis::X,
            Vec3::X,
            AXIS_X_COLOR,
            EditorAxis::Y,
            Vec3::Y,
            AXIS_Y_COLOR,
        ),
        (
            EditorAxis::Y,
            Vec3::Y,
            AXIS_Y_COLOR,
            EditorAxis::Z,
            Vec3::Z,
            AXIS_Z_COLOR,
        ),
        (
            EditorAxis::Z,
            Vec3::Z,
            AXIS_Z_COLOR,
            EditorAxis::X,
            Vec3::X,
            AXIS_X_COLOR,
        ),
    ] {
        let p0 = origin_world
            + dir_a * (axis_world_size * PLANE_HANDLE_INNER)
            + dir_b * (axis_world_size * PLANE_HANDLE_INNER);
        let p1 = origin_world
            + dir_a * (axis_world_size * PLANE_HANDLE_OUTER)
            + dir_b * (axis_world_size * PLANE_HANDLE_INNER);
        let p2 = origin_world
            + dir_a * (axis_world_size * PLANE_HANDLE_OUTER)
            + dir_b * (axis_world_size * PLANE_HANDLE_OUTER);
        let p3 = origin_world
            + dir_a * (axis_world_size * PLANE_HANDLE_INNER)
            + dir_b * (axis_world_size * PLANE_HANDLE_OUTER);
        let center_world = origin_world
            + dir_a * (axis_world_size * (PLANE_HANDLE_INNER + PLANE_HANDLE_OUTER) * 0.5)
            + dir_b * (axis_world_size * (PLANE_HANDLE_INNER + PLANE_HANDLE_OUTER) * 0.5);

        let Some(center) = project_world_to_screen(vp, width, height, center_world) else {
            continue;
        };
        let Some(axis_a_end) = project_world_to_screen(
            vp,
            width,
            height,
            origin_world + dir_a * (axis_world_size * PLANE_HANDLE_OUTER),
        ) else {
            continue;
        };
        let Some(axis_b_end) = project_world_to_screen(
            vp,
            width,
            height,
            origin_world + dir_b * (axis_world_size * PLANE_HANDLE_OUTER),
        ) else {
            continue;
        };

        let drag_vectors = [
            [
                axis_a_end.x - origin_screen.x,
                axis_a_end.y - origin_screen.y,
            ],
            [
                axis_b_end.x - origin_screen.x,
                axis_b_end.y - origin_screen.y,
            ],
        ];
        if solve_drag_basis(drag_vectors, [1.0, 1.0]).is_none() {
            continue;
        }

        let handle = EditorViewportGizmoHandle {
            entity_id,
            axis: axis_a,
            mode,
            depth: center.depth,
            secondary_axis: Some(axis_b),
            shape: EditorViewportHandleShape::Rect {
                center: [center.x, center.y],
                half_extent: GIZMO_PLANE_HALF_PX,
            },
            drag: EditorViewportHandleDrag::DualAxis {
                drag_vectors,
                delta_scales: [
                    full_delta_scale * PLANE_HANDLE_OUTER,
                    full_delta_scale * PLANE_HANDLE_OUTER,
                ],
            },
            field_name,
        };
        let tinted = tint_plane_color(
            mix_axis_colors(color_a, color_b),
            active_axis.is_some_and(|axis| axis == axis_a || axis == axis_b),
            hovered_handle == Some(handle.id()),
        );

        overlay.debug_lines.push(DebugLine::new(p0, p1, tinted));
        overlay.debug_lines.push(DebugLine::new(p1, p2, tinted));
        overlay.debug_lines.push(DebugLine::new(p2, p3, tinted));
        overlay.debug_lines.push(DebugLine::new(p3, p0, tinted));
        overlay.quads.push(ScreenQuad::new(
            center.x - GIZMO_PLANE_HALF_PX,
            center.y - GIZMO_PLANE_HALF_PX,
            GIZMO_PLANE_HALF_PX * 2.0,
            GIZMO_PLANE_HALF_PX * 2.0,
            with_alpha(tinted, 0.55),
        ));
        overlay.gizmo_handles.push(handle);
    }
}
#[allow(clippy::too_many_arguments)]
fn append_rotate_gizmo(
    host: &EditorRuntimeHost,
    vp: &Mat4,
    width: u32,
    height: u32,
    origin_world: Vec3,
    origin_depth: f32,
    radius: f32,
    hovered_handle: Option<EditorViewportGizmoHandleId>,
    overlay: &mut EditorViewportOverlay,
) {
    let entity_id = host
        .cursor()
        .map(|cursor| cursor.entity_id)
        .unwrap_or_default();
    let active_axis = cursor_active_axis(host, "rotation_euler_deg");

    for (axis, basis_a, basis_b, color) in [
        (EditorAxis::X, Vec3::Y, Vec3::Z, AXIS_X_COLOR),
        (EditorAxis::Y, Vec3::Z, Vec3::X, AXIS_Y_COLOR),
        (EditorAxis::Z, Vec3::X, Vec3::Y, AXIS_Z_COLOR),
    ] {
        let handle = EditorViewportGizmoHandle {
            entity_id,
            axis,
            mode: EditorViewportGizmoMode::Rotate,
            depth: origin_depth,
            secondary_axis: None,
            shape: EditorViewportHandleShape::Ring {
                segments: Vec::new(),
                pick_radius: GIZMO_RING_PICK_RADIUS_PX,
                circumference: 0.0,
            },
            drag: EditorViewportHandleDrag::SingleAxis {
                drag_vector: [1.0, 0.0],
                delta_scale: 0.0,
            },
            field_name: "rotation_euler_deg",
        };
        let tinted = tint_axis_color(
            color,
            active_axis == Some(axis),
            hovered_handle == Some(handle.id()),
        );

        let (segments, circumference) = append_rotate_ring_lines(
            vp,
            width,
            height,
            origin_world,
            origin_depth,
            basis_a,
            basis_b,
            radius,
            tinted,
            overlay,
        );
        if segments.is_empty() || circumference <= 1.0 {
            continue;
        }

        let mut handle = handle;
        handle.shape = EditorViewportHandleShape::Ring {
            segments,
            pick_radius: GIZMO_RING_PICK_RADIUS_PX,
            circumference,
        };
        overlay.gizmo_handles.push(handle);
    }
}

#[allow(clippy::too_many_arguments)]
fn append_rotate_ring_lines(
    vp: &Mat4,
    width: u32,
    height: u32,
    origin_world: Vec3,
    origin_depth: f32,
    basis_a: Vec3,
    basis_b: Vec3,
    radius: f32,
    color: Color,
    overlay: &mut EditorViewportOverlay,
) -> (Vec<HandlePickSegment>, f32) {
    let segments = 48;
    let mut prev_world = origin_world + basis_a * radius;
    let Some(mut prev_projected) = project_world_to_screen(vp, width, height, prev_world) else {
        return (Vec::new(), 0.0);
    };

    let mut pick_segments = Vec::new();
    let mut circumference = 0.0;

    for step in 1..=segments {
        let t = step as f32 / segments as f32;
        let angle = t * std::f32::consts::TAU;
        let world = origin_world + (basis_a * angle.cos() + basis_b * angle.sin()) * radius;
        let Some(projected) = project_world_to_screen(vp, width, height, world) else {
            prev_world = world;
            continue;
        };

        let segment_depth = (prev_projected.depth + projected.depth) * 0.5;
        let front_facing = segment_depth <= origin_depth;
        let segment_color = if front_facing {
            color
        } else {
            color.lerp(Color::BLACK, RING_BACKFACE_TINT)
        };
        overlay
            .debug_lines
            .push(DebugLine::new(prev_world, world, segment_color));

        let screen_start = [prev_projected.x, prev_projected.y];
        let screen_end = [projected.x, projected.y];
        let screen_len = vector_len([
            screen_end[0] - screen_start[0],
            screen_end[1] - screen_start[1],
        ]);
        circumference += screen_len;
        if front_facing && screen_len > 1.5 {
            pick_segments.push(HandlePickSegment {
                start: screen_start,
                end: screen_end,
                depth: segment_depth,
            });
        }

        prev_world = world;
        prev_projected = projected;
    }

    (pick_segments, circumference)
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

fn tint_plane_color(color: Color, active: bool, hovered: bool) -> Color {
    let color = if active {
        color.lerp(Color::WHITE, 0.18)
    } else {
        color
    };
    if hovered {
        color.lerp(Color::WHITE, 0.35)
    } else {
        color
    }
}

fn mix_axis_colors(a: Color, b: Color) -> Color {
    Color::new((a.r + b.r) * 0.5, (a.g + b.g) * 0.5, (a.b + b.b) * 0.5, 1.0)
}

fn with_alpha(color: Color, alpha: f32) -> Color {
    Color::new(color.r, color.g, color.b, alpha)
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
    overlay
        .quads
        .push(ScreenQuad::new(x, y + h - 1.0, w, 1.0, color));
    overlay.quads.push(ScreenQuad::new(x, y, 1.0, h, color));
    overlay
        .quads
        .push(ScreenQuad::new(x + w - 1.0, y, 1.0, h, color));
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

fn single_axis_delta(drag_vector: [f32; 2], delta_scale: f32, motion: [f32; 2]) -> Option<f32> {
    let axis_len = vector_len(drag_vector);
    if axis_len < 8.0 {
        return None;
    }
    let motion_along_axis =
        motion[0] * (drag_vector[0] / axis_len) + motion[1] * (drag_vector[1] / axis_len);
    Some((motion_along_axis / axis_len) * delta_scale)
}

fn solve_drag_basis(drag_vectors: [[f32; 2]; 2], motion: [f32; 2]) -> Option<[f32; 2]> {
    let det = drag_vectors[0][0] * drag_vectors[1][1] - drag_vectors[0][1] * drag_vectors[1][0];
    if det.abs() < 18.0 {
        return None;
    }

    Some([
        (motion[0] * drag_vectors[1][1] - motion[1] * drag_vectors[1][0]) / det,
        (drag_vectors[0][0] * motion[1] - drag_vectors[0][1] * motion[0]) / det,
    ])
}

fn point_segment_distance_sq(point: [f32; 2], start: [f32; 2], end: [f32; 2]) -> f32 {
    let seg = [end[0] - start[0], end[1] - start[1]];
    let len_sq = seg[0] * seg[0] + seg[1] * seg[1];
    if len_sq <= 1e-6 {
        return screen_distance_sq(point, start);
    }

    let to_point = [point[0] - start[0], point[1] - start[1]];
    let t = ((to_point[0] * seg[0] + to_point[1] * seg[1]) / len_sq).clamp(0.0, 1.0);
    let closest = [start[0] + seg[0] * t, start[1] + seg[1] * t];
    screen_distance_sq(point, closest)
}

fn screen_distance_sq(a: [f32; 2], b: [f32; 2]) -> f32 {
    let dx = a[0] - b[0];
    let dy = a[1] - b[1];
    dx * dx + dy * dy
}

fn vector_len(v: [f32; 2]) -> f32 {
    (v[0] * v[0] + v[1] * v[1]).sqrt()
}

fn depth_pick_score(dist_sq: f32, depth: f32) -> f32 {
    let depth_t = ((depth + 1.0) * 0.5).clamp(0.0, 1.0);
    dist_sq + depth_t * PICK_DEPTH_BIAS_PX_SQ
}

fn compare_f32(a: f32, b: f32) -> std::cmp::Ordering {
    a.partial_cmp(&b).unwrap_or(std::cmp::Ordering::Equal)
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
            runtime_slice: crate::runtime_slice::RuntimeSliceAuthoring::default(),
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

    fn depth_test_host() -> EditorRuntimeHost {
        let scene = SceneDocument {
            name: "Depth".into(),
            runtime_slice: crate::runtime_slice::RuntimeSliceAuthoring::default(),
            entities: vec![
                EditorEntity {
                    id: 1,
                    name: "Back".into(),
                    components: vec![transform_component([0.0, 0.0, 0.0])],
                },
                EditorEntity {
                    id: 2,
                    name: "Front".into(),
                    components: vec![transform_component([0.0, 0.0, 4.0])],
                },
            ],
        };

        let mut host = EditorRuntimeHost::new(EditorSceneSession::new(scene)).unwrap();
        host.editor_camera.position = Vec3::new(0.0, 0.0, 10.0);
        host.editor_camera.yaw = 0.0;
        host.editor_camera.pitch = 0.0;
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
    fn viewport_pick_prefers_frontmost_entity_when_hits_overlap() {
        let host = depth_test_host();
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

        let front = overlay
            .entity_hits
            .iter()
            .find(|hit| hit.entity_id == 2)
            .copied()
            .unwrap();
        assert_eq!(
            overlay.pick_entity(front.screen_x as i32, front.screen_y as i32),
            Some(2)
        );
    }

    #[test]
    fn translate_plane_handle_edits_two_axes() {
        let mut host = test_host();
        host.execute(EditorCommand::SelectEntity { entity_id: 1 })
            .unwrap();

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

        let handle = overlay
            .gizmo_handles
            .iter()
            .find(|handle| handle.secondary_axis.is_some())
            .unwrap();
        let commands = handle.activation_commands();
        assert!(matches!(
            &commands[0],
            EditorCommand::SelectField { field_name, .. } if field_name == "position"
        ));
        let deltas = handle.world_deltas_for_mouse_delta(18, -12);
        assert_eq!(deltas.len(), 2);
    }

    #[test]
    fn rotate_gizmo_is_pickable_from_ring_segments() {
        let mut host = test_host();
        host.execute(EditorCommand::SelectEntity { entity_id: 1 })
            .unwrap();

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

        let ring_handle = overlay.gizmo_handles.first().unwrap();
        let (sample_x, sample_y, drag_dx, drag_dy) = match &ring_handle.shape {
            EditorViewportHandleShape::Ring { segments, .. } => {
                let segment = segments.first().unwrap();
                (
                    ((segment.start[0] + segment.end[0]) * 0.5) as i32,
                    ((segment.start[1] + segment.end[1]) * 0.5) as i32,
                    ((segment.end[0] - segment.start[0]) * 4.0).round() as i32,
                    ((segment.end[1] - segment.start[1]) * 4.0).round() as i32,
                )
            }
            other => panic!("expected ring handle, got {other:?}"),
        };

        let picked = overlay.pick_gizmo_handle(sample_x, sample_y).unwrap();
        let commands = picked.activation_commands();
        assert!(matches!(
            &commands[0],
            EditorCommand::SelectField { field_name, .. } if field_name == "rotation_euler_deg"
        ));
        assert_eq!(picked.field_name(), "rotation_euler_deg");
        assert_eq!(picked.mode, EditorViewportGizmoMode::Rotate);
        assert!(picked.world_deltas_for_mouse_delta(drag_dx, drag_dy).len() <= 1);
    }

    #[test]
    fn scale_gizmo_includes_plane_handles_and_hover_feedback() {
        let mut host = test_host();
        host.execute(EditorCommand::SelectEntity { entity_id: 1 })
            .unwrap();

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

        assert!(
            overlay
                .gizmo_handles
                .iter()
                .any(|handle| handle.secondary_axis.is_some()),
            "scale mode should expose plane handles"
        );
        assert!(
            !overlay.quads.is_empty(),
            "hover feedback should add outline quads"
        );
    }
}
