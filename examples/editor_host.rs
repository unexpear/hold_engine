//! Minimal editor-facing host for the scene workflow.
//!
//! Usage:
//!   cargo run --example editor_host
//!   cargo run --example editor_host -- examples/demo.hsc target/edited-demo.hsc
//!
//! This intentionally stays smaller than the renderer demo: it opens a runtime
//! `.hsc` scene as a `SceneDocument`, selects and mutates an entity through the
//! inspector path API, saves a runtime `.hsc`, then applies it into a `World`.

use std::path::{Path, PathBuf};

use hold::{
    apply_scene_document_to_world, load_scene_doc_from_hsc, save_scene_doc_to_hsc, ActiveCamera,
    Camera, Entity, InspectorValue, SceneDocument, Transform, World,
};

const DEFAULT_INPUT_SCENE: &str = "examples/demo.hsc";
const DEFAULT_OUTPUT_SCENE: &str = "target/editor_host_preview.hsc";

fn host_error(message: impl Into<String>) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidInput, message.into())
}

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

    let mut scene = load_scene_doc_from_hsc(&input_path)?;
    let selected_entity_id = select_camera_entity(&scene)?;

    println!(
        "Opened '{}' with {} entities.",
        scene.name,
        scene.entities.len()
    );
    print_entity_summary(&scene, selected_entity_id)?;

    scene.set_value(
        selected_entity_id,
        "Camera",
        "fov_deg",
        InspectorValue::Number(70.0),
    );
    scene.set_value(
        selected_entity_id,
        "Transform",
        "position",
        InspectorValue::Vec3([0.0, 24.0, 12.0]),
    );
    println!("Mutated camera fov_deg and transform.position through SceneDocument.");

    save_output_scene(&output_path, &scene)?;

    let applied_scene = load_scene_doc_from_hsc(&output_path)?;
    let mut world = World::new();
    let map = apply_scene_document_to_world(&mut world, &applied_scene)?;
    let camera_entity = lookup_entity(&map, selected_entity_id)?;

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

fn select_camera_entity(scene: &SceneDocument) -> Result<u64, Box<dyn std::error::Error>> {
    scene
        .entities
        .iter()
        .find(|entity| entity.component("Camera").is_some())
        .map(|entity| entity.id)
        .ok_or_else(|| host_error("scene has no Camera entity").into())
}

fn print_entity_summary(
    scene: &SceneDocument,
    entity_id: u64,
) -> Result<(), Box<dyn std::error::Error>> {
    let entity = scene
        .entities
        .iter()
        .find(|entity| entity.id == entity_id)
        .ok_or_else(|| host_error(format!("entity id {} was not found", entity_id)))?;
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

fn save_output_scene(
    output_path: &Path,
    scene: &SceneDocument,
) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(parent) = output_path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }

    save_scene_doc_to_hsc(output_path, scene)?;
    Ok(())
}

fn lookup_entity(
    map: &[(u64, Entity)],
    editor_entity_id: u64,
) -> Result<Entity, Box<dyn std::error::Error>> {
    map.iter()
        .find(|(id, _)| *id == editor_entity_id)
        .map(|(_, entity)| *entity)
        .ok_or_else(|| {
            host_error(format!(
                "editor entity id {} was not applied to the World",
                editor_entity_id
            ))
            .into()
        })
}
