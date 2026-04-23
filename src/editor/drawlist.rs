//! Backend-neutral editor debug draw-list generator.
//!
//! [`generate_draw_list`] inspects the live ECS [`World`] and derives a set
//! of 3-D line segments that visualize the scene:
//!
//! - **World grid** — a 20×20 XZ ground plane at 1-unit spacing.
//! - **Entity markers** — a small 3-axis cross at each entity that has a
//!   [`Transform`].
//! - **Selection highlight** — a larger cross in bright green for the
//!   currently selected entity.
//! - **Parent links** — a line from each child to its parent.
//! - **Camera markers** — a short forward-direction arrow for camera entities.
//! - **Light markers** — a direction arrow for directional-light entities.
//!
//! Render the resulting [`EditorDrawList`] with
//! [`Renderer::render_debug_lines`].

use crate::ecs::components::{Camera, DirectionalLight, Parent, Transform};
use crate::ecs::{Entity, World};
use crate::math::Vec3;
use crate::renderer::{Color, DebugLine};

use super::{command::EditorInspectorCursor, SceneDocument};

// ── Colour palette ────────────────────────────────────────────────────────────

const GRID_COLOR: Color = Color::new(0.22, 0.22, 0.25, 1.0);
const ENTITY_COLOR: Color = Color::new(0.55, 0.55, 0.55, 1.0);
const SELECTION_COLOR: Color = Color::new(0.18, 1.0, 0.28, 1.0);
const PARENT_LINK_COLOR: Color = Color::new(0.3, 0.78, 1.0, 1.0);
const CAMERA_COLOR: Color = Color::new(1.0, 1.0, 0.2, 1.0);
const LIGHT_COLOR: Color = Color::new(1.0, 0.58, 0.1, 1.0);

// ── EditorDrawList ────────────────────────────────────────────────────────────

/// A backend-neutral list of 3-D debug line segments derived from the editor
/// world state.
///
/// Generate with [`generate_draw_list`] and render with
/// [`Renderer::render_debug_lines`].
#[derive(Debug, Clone, Default)]
pub struct EditorDrawList {
    pub lines: Vec<DebugLine>,
}

impl EditorDrawList {
    pub fn new() -> Self {
        Self::default()
    }

    fn push(&mut self, start: Vec3, end: Vec3, color: Color) {
        self.lines.push(DebugLine { start, end, color });
    }

    /// Draw a 3-axis cross centred at `pos` with half-extent `r`.
    fn push_cross(&mut self, pos: Vec3, r: f32, color: Color) {
        self.push(
            pos - Vec3::new(r, 0.0, 0.0),
            pos + Vec3::new(r, 0.0, 0.0),
            color,
        );
        self.push(
            pos - Vec3::new(0.0, r, 0.0),
            pos + Vec3::new(0.0, r, 0.0),
            color,
        );
        self.push(
            pos - Vec3::new(0.0, 0.0, r),
            pos + Vec3::new(0.0, 0.0, r),
            color,
        );
    }
}

// ── Generator ─────────────────────────────────────────────────────────────────

/// Generate a debug draw list from the live ECS world plus editor selection.
///
/// # Parameters
/// - `world`       — live ECS world (read-only access to components)
/// - `entity_map`  — editor-id → runtime-[`Entity`] mapping (from the host's
///                   `last_world_map`)
/// - `_scene`      — scene document (reserved for future label queries)
/// - `cursor`      — currently selected entity/component/field; `None` when
///                   nothing is selected
pub fn generate_draw_list(
    world: &World,
    entity_map: &[(u64, Entity)],
    _scene: &SceneDocument,
    cursor: Option<&EditorInspectorCursor>,
) -> EditorDrawList {
    let mut list = EditorDrawList::new();

    // ── World grid (XZ plane, 20×20 cells) ────────────────────────────────
    let half = 10i32;
    let extent = half as f32;
    for i in -half..=half {
        let f = i as f32;
        list.push(
            Vec3::new(f, 0.0, -extent),
            Vec3::new(f, 0.0, extent),
            GRID_COLOR,
        );
        list.push(
            Vec3::new(-extent, 0.0, f),
            Vec3::new(extent, 0.0, f),
            GRID_COLOR,
        );
    }

    let selected_id = cursor.map(|c| c.entity_id);

    // ── Per-entity markers ─────────────────────────────────────────────────
    for &(editor_id, runtime_entity) in entity_map {
        if !world.is_alive(runtime_entity) {
            continue;
        }

        let pos = world
            .get::<Transform>(runtime_entity)
            .map(|t| t.position)
            .unwrap_or(Vec3::ZERO);

        let is_selected = selected_id == Some(editor_id);

        if is_selected {
            // Selection highlight — larger cross in bright green.
            list.push_cross(pos, 0.5, SELECTION_COLOR);
        } else {
            list.push_cross(pos, 0.22, ENTITY_COLOR);
        }

        // ── Camera marker: short forward arrow (-Z by convention) ──────────
        if world.has_component::<Camera>(runtime_entity) {
            let tip = pos + Vec3::new(0.0, 0.0, -1.0);
            list.push(pos, tip, CAMERA_COLOR);
            // Two fin lines to hint at a frustum corner.
            list.push(tip, pos + Vec3::new(0.18, 0.18, -0.7), CAMERA_COLOR);
            list.push(tip, pos + Vec3::new(-0.18, 0.18, -0.7), CAMERA_COLOR);
        }

        // ── Directional-light marker: direction arrow ──────────────────────
        if let Some(light) = world.get::<DirectionalLight>(runtime_entity) {
            let dir = light.direction.normalize();
            list.push(pos, pos + dir * 1.4, LIGHT_COLOR);
        }

        // ── Parent link: line to parent position ───────────────────────────
        if let Some(parent_comp) = world.get::<Parent>(runtime_entity) {
            let parent_entity = parent_comp.entity;
            if world.is_alive(parent_entity) {
                let parent_pos = world
                    .get::<Transform>(parent_entity)
                    .map(|t| t.position)
                    .unwrap_or(Vec3::ZERO);
                list.push(pos, parent_pos, PARENT_LINK_COLOR);
            }
        }
    }

    list
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ecs::components::Transform;
    use crate::math::{Quat, Vec3};

    fn empty_scene() -> SceneDocument {
        SceneDocument {
            name: String::new(),
            runtime_slice: crate::runtime_slice::RuntimeSliceAuthoring::default(),
            entities: vec![],
        }
    }

    fn make_transform(pos: Vec3) -> Transform {
        Transform {
            position: pos,
            rotation: Quat::IDENTITY,
            scale: Vec3::ONE,
        }
    }

    // ── Grid ──────────────────────────────────────────────────────────────

    #[test]
    fn grid_generation_produces_correct_line_count() {
        let world = World::new();
        let list = generate_draw_list(&world, &[], &empty_scene(), None);
        // 21 vertical + 21 horizontal = 42 grid lines.
        let grid_count = list.lines.iter().filter(|l| l.color == GRID_COLOR).count();
        assert_eq!(grid_count, 42);
    }

    #[test]
    fn grid_lines_lie_on_xz_plane() {
        let world = World::new();
        let list = generate_draw_list(&world, &[], &empty_scene(), None);
        for line in list.lines.iter().filter(|l| l.color == GRID_COLOR) {
            assert!((line.start.y).abs() < 1e-6, "grid start.y should be 0");
            assert!((line.end.y).abs() < 1e-6, "grid end.y should be 0");
        }
    }

    // ── Entity markers ────────────────────────────────────────────────────

    #[test]
    fn entity_marker_placed_at_transform_position() {
        let mut world = World::new();
        let entity = world.spawn();
        world.add_component(entity, make_transform(Vec3::new(3.0, 0.0, 0.0)));

        let map = vec![(42u64, entity)];
        let list = generate_draw_list(&world, &map, &empty_scene(), None);

        let entity_lines: Vec<_> = list
            .lines
            .iter()
            .filter(|l| l.color == ENTITY_COLOR)
            .collect();
        assert_eq!(
            entity_lines.len(),
            3,
            "expect 3-axis cross for unselected entity"
        );

        for line in &entity_lines {
            // Each arm of the cross must pass through or near x=3.
            let mid_x = (line.start.x + line.end.x) * 0.5;
            assert!((mid_x - 3.0).abs() < 1e-5, "cross should be centred at x=3");
        }
    }

    // ── Selection highlight ───────────────────────────────────────────────

    #[test]
    fn selection_highlight_uses_selection_color() {
        let mut world = World::new();
        let entity = world.spawn();
        world.add_component(entity, make_transform(Vec3::ZERO));

        let cursor = EditorInspectorCursor {
            entity_id: 99,
            component_name: "Transform".into(),
            field_name: "position".into(),
            axis: None,
        };
        let map = vec![(99u64, entity)];
        let list = generate_draw_list(&world, &map, &empty_scene(), Some(&cursor));

        let sel_count = list
            .lines
            .iter()
            .filter(|l| l.color == SELECTION_COLOR)
            .count();
        assert_eq!(
            sel_count, 3,
            "selected entity should have a 3-arm selection cross"
        );

        let entity_count = list
            .lines
            .iter()
            .filter(|l| l.color == ENTITY_COLOR)
            .count();
        assert_eq!(
            entity_count, 0,
            "selected entity must not also get the default marker"
        );
    }

    // ── Parent links ──────────────────────────────────────────────────────

    #[test]
    fn parent_link_connects_child_to_parent() {
        let mut world = World::new();
        let parent = world.spawn();
        world.add_component(parent, make_transform(Vec3::new(0.0, 5.0, 0.0)));

        let child = world.spawn();
        world.add_component(child, make_transform(Vec3::new(2.0, 5.0, 0.0)));
        world.set_parent(child, parent);

        let map = vec![(1u64, parent), (2u64, child)];
        let list = generate_draw_list(&world, &map, &empty_scene(), None);

        let parent_lines: Vec<_> = list
            .lines
            .iter()
            .filter(|l| l.color == PARENT_LINK_COLOR)
            .collect();
        assert_eq!(parent_lines.len(), 1, "one parent-link line expected");

        let pl = parent_lines[0];
        // The line must connect (2,5,0) and (0,5,0) in some order.
        let connects = (pl.start.distance(Vec3::new(2.0, 5.0, 0.0)) < 1e-4
            && pl.end.distance(Vec3::new(0.0, 5.0, 0.0)) < 1e-4)
            || (pl.end.distance(Vec3::new(2.0, 5.0, 0.0)) < 1e-4
                && pl.start.distance(Vec3::new(0.0, 5.0, 0.0)) < 1e-4);
        assert!(connects, "parent link must connect child pos to parent pos");
    }

    // ── Stable markers ────────────────────────────────────────────────────

    #[test]
    fn marker_position_is_stable_across_calls() {
        let mut world = World::new();
        let entity = world.spawn();
        world.add_component(entity, make_transform(Vec3::new(1.0, 2.0, 3.0)));

        let map = vec![(7u64, entity)];
        let list1 = generate_draw_list(&world, &map, &empty_scene(), None);
        let list2 = generate_draw_list(&world, &map, &empty_scene(), None);

        assert_eq!(list1.lines.len(), list2.lines.len());
        for (a, b) in list1.lines.iter().zip(list2.lines.iter()) {
            assert_eq!(a.start, b.start);
            assert_eq!(a.end, b.end);
            assert_eq!(a.color, b.color);
        }
    }
}
