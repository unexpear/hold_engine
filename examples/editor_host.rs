//! Minimal editor-facing host for the scene workflow.
//!
//! Usage:
//!   cargo run --example editor_host
//!   cargo run --example editor_host -- examples/demo.hsc target/edited-demo.hsc
//!
//! This intentionally stays smaller than the renderer demo: it opens a runtime
//! `.hsc` scene as an `EditorSceneSession`, selects and mutates an entity through
//! the inspector path API, saves a runtime `.hsc`, then applies it into a `World`.

use std::path::PathBuf;

use hold::{ActiveCamera, Camera, EditorSceneSession, InspectorValue, Transform, World};

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

    session.set_selected_value("Camera", "fov_deg", InspectorValue::Number(70.0))?;
    session.set_selected_value(
        "Transform",
        "position",
        InspectorValue::Vec3([0.0, 24.0, 12.0]),
    )?;
    println!("Mutated camera fov_deg and transform.position through EditorSceneSession.");

    session.save_hsc(&output_path)?;

    let applied_session = EditorSceneSession::open_hsc(&output_path)?;
    let mut world = World::new();
    let map = applied_session.apply_to_world(&mut world)?;
    let camera_entity = map
        .iter()
        .find(|(id, _)| *id == selected_entity_id)
        .map(|(_, entity)| *entity)
        .ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!(
                    "editor entity id {} was not applied to the World",
                    selected_entity_id
                ),
            )
        })?;

    assert!(world.has_component::<Transform>(camera_entity));
    assert!(world.has_component::<Camera>(camera_entity));
    assert!(world.has_component::<ActiveCamera>(camera_entity));

    println!(
        "Saved '{}' and applied {} editor entities into a runtime World.",
        output_path.display(),
        map.len()
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
