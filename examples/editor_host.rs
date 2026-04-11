//! Minimal editor-facing host for the scene workflow.
//!
//! Usage:
//!   cargo run --example editor_host
//!   cargo run --example editor_host -- examples/demo.hsc target/edited-demo.hsc
//!
//! This intentionally stays smaller than the renderer demo: it opens a runtime
//! `.hsc` scene as an `EditorSceneSession`, selects and mutates an entity through
//! the inspector path API, patches a live `World`, then does a deterministic
//! rebuild from the same document.

use std::path::PathBuf;

use hold::{ActiveCamera, Camera, EditorSceneSession, InspectorValue, Transform, Vec3, World};

const DEFAULT_INPUT_SCENE: &str = "examples/demo.hsc";
const DEFAULT_OUTPUT_SCENE: &str = "target/editor_host_preview.hsc";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let input_path = args
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_INPUT_SCENE));
    let output_path = args
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_OUTPUT_SCENE));

    let mut session = EditorSceneSession::open_hsc(&input_path)?;
    let selected_entity_id = session.select_first_with_component("Camera")?;

    println!(
        "Opened '{}' with {} entities.",
        session.scene().name,
        session.scene().entities.len()
    );
    print_selected_entity_summary(&session)?;

    let mut world = World::new();
    let initial_map_len = session.rebuild_world(&mut world)?.len();
    let initial_camera_entity = session.applied_entity(selected_entity_id)?;
    assert_camera_world_state(
        &world,
        initial_camera_entity,
        Vec3::new(0.0, 20.0, 0.0),
        60.0,
    );
    println!("Rebuilt a live runtime World with {initial_map_len} editor entities.");

    session.set_selected_value("Camera", "fov_deg", InspectorValue::Number(70.0))?;
    session.set_selected_value(
        "Transform",
        "position",
        InspectorValue::Vec3([0.0, 24.0, 12.0]),
    )?;
    println!("Mutated camera fov_deg and transform.position through EditorSceneSession.");

    session.patch_world(&mut world)?;
    let patched_camera_entity = session.applied_entity(selected_entity_id)?;
    assert_eq!(patched_camera_entity, initial_camera_entity);
    assert_camera_world_state(
        &world,
        patched_camera_entity,
        Vec3::new(0.0, 24.0, 12.0),
        70.0,
    );
    println!("Patched the existing runtime World without rebuilding entity handles.");

    session.save_hsc(&output_path)?;
    let rebuilt_map_len = session.rebuild_world(&mut world)?.len();
    let rebuilt_camera_entity = session.applied_entity(selected_entity_id)?;
    assert_camera_world_state(
        &world,
        rebuilt_camera_entity,
        Vec3::new(0.0, 24.0, 12.0),
        70.0,
    );
    println!(
        "Saved '{}' and rebuilt {} editor entities from the same SceneDocument.",
        output_path.display(),
        rebuilt_map_len
    );

    Ok(())
}

fn print_selected_entity_summary(
    session: &EditorSceneSession,
) -> Result<(), Box<dyn std::error::Error>> {
    let entity = session.selected_entity()?;
    let component_names = entity
        .components
        .iter()
        .map(|component| component.type_name.as_str())
        .collect::<Vec<_>>()
        .join(", ");

    println!(
        "Selected entity {} '{}' with components: {}.",
        entity.id, entity.name, component_names
    );
    Ok(())
}

fn assert_camera_world_state(
    world: &World,
    camera_entity: hold::Entity,
    expected_position: Vec3,
    expected_fov_deg: f32,
) {
    assert!(world.has_component::<Transform>(camera_entity));
    assert!(world.has_component::<Camera>(camera_entity));
    assert!(world.has_component::<ActiveCamera>(camera_entity));

    let transform = world.get::<Transform>(camera_entity).unwrap();
    assert_eq!(transform.position, expected_position);
    let camera = world.get::<Camera>(camera_entity).unwrap();
    assert!((camera.fov.to_degrees() - expected_fov_deg).abs() < 0.001);
}
