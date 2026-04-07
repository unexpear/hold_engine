//! Editor-side data model for projects, scenes, and inspector editing.
//! This module is runtime-agnostic and keeps a dynamic editing layer on top
//! of typed ECS/runtime data.

use std::fs;
use std::path::Path;

use crate::ecs::components::{Camera, DirectionalLight, Stats, Transform};
use crate::ecs::{Entity, World};
use crate::math::{Quat, Vec3};
use crate::renderer::Color;

/// Basic project metadata and folder layout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Project {
    pub name: String,
    pub root: String,
    pub assets_dir: String,
    pub scenes_dir: String,
}

impl Project {
    pub fn new(name: impl Into<String>, root: impl Into<String>) -> Self {
        let root = root.into();
        Self {
            name: name.into(),
            assets_dir: format!("{root}/assets"),
            scenes_dir: format!("{root}/scenes"),
            root,
        }
    }

    pub fn project_file(&self) -> String {
        format!("{}/{}.hproj", self.root, self.name)
    }

    /// Serialize project metadata into a minimal `.hproj` text format.
    pub fn to_hproj_string(&self) -> String {
        format!(
            "HOLD_PROJECT_V1\nname={}\nroot={}\nassets_dir={}\nscenes_dir={}\n",
            escape_text(&self.name),
            escape_text(&self.root),
            escape_text(&self.assets_dir),
            escape_text(&self.scenes_dir)
        )
    }

    /// Parse project metadata from a `.hproj` text payload.
    pub fn from_hproj_str(src: &str) -> Result<Self, &'static str> {
        let mut lines = src.lines();
        if lines.next() != Some("HOLD_PROJECT_V1") {
            return Err("invalid project header");
        }

        let mut name: Option<String> = None;
        let mut root: Option<String> = None;
        let mut assets_dir: Option<String> = None;
        let mut scenes_dir: Option<String> = None;

        for line in lines {
            if line.is_empty() {
                continue;
            }
            let (k, v) = line.split_once('=').ok_or("invalid project line")?;
            match k {
                "name" => name = Some(unescape_text(v)?),
                "root" => root = Some(unescape_text(v)?),
                "assets_dir" => assets_dir = Some(unescape_text(v)?),
                "scenes_dir" => scenes_dir = Some(unescape_text(v)?),
                _ => return Err("unknown project key"),
            }
        }

        let name = name.ok_or("missing project name")?;
        let root = root.ok_or("missing project root")?;
        let assets_dir = assets_dir.unwrap_or_else(|| format!("{root}/assets"));
        let scenes_dir = scenes_dir.unwrap_or_else(|| format!("{root}/scenes"));
        Ok(Self {
            name,
            root,
            assets_dir,
            scenes_dir,
        })
    }
}

/// A scalar/vector value editable through an inspector.
#[derive(Debug, Clone, PartialEq)]
pub enum InspectorValue {
    Bool(bool),
    Number(f32),
    Text(String),
    Vec3([f32; 3]),
}

/// A single editable field under a component.
#[derive(Debug, Clone, PartialEq)]
pub struct InspectorField {
    pub name: String,
    pub value: InspectorValue,
}

/// Dynamic component snapshot used by the editor.
#[derive(Debug, Clone, PartialEq)]
pub struct InspectorComponent {
    pub type_name: String,
    pub fields: Vec<InspectorField>,
}

impl InspectorComponent {
    pub fn get_field(&self, field: &str) -> Option<&InspectorValue> {
        self.fields
            .iter()
            .find(|f| f.name == field)
            .map(|f| &f.value)
    }

    pub fn set_field(&mut self, field: &str, value: InspectorValue) -> bool {
        if let Some(existing) = self.fields.iter_mut().find(|f| f.name == field) {
            existing.value = value;
            true
        } else {
            false
        }
    }
}

/// Editable entity representation in a scene document.
#[derive(Debug, Clone, PartialEq)]
pub struct EditorEntity {
    pub id: u64,
    pub name: String,
    pub components: Vec<InspectorComponent>,
}

impl EditorEntity {
    pub fn component(&self, type_name: &str) -> Option<&InspectorComponent> {
        self.components.iter().find(|c| c.type_name == type_name)
    }

    pub fn component_mut(&mut self, type_name: &str) -> Option<&mut InspectorComponent> {
        self.components
            .iter_mut()
            .find(|c| c.type_name == type_name)
    }
}

/// Scene editing document owned by the editor.
#[derive(Debug, Clone, PartialEq)]
pub struct SceneDocument {
    pub name: String,
    pub entities: Vec<EditorEntity>,
}

impl SceneDocument {
    /// Dynamic path-style getter:
    /// entity -> component -> field -> value
    pub fn get_value(
        &self,
        entity_id: u64,
        component: &str,
        field: &str,
    ) -> Option<&InspectorValue> {
        self.entities
            .iter()
            .find(|e| e.id == entity_id)?
            .component(component)?
            .get_field(field)
    }

    /// Dynamic path-style setter:
    /// entity -> component -> field -> value
    ///
    /// If the target entity/component/field does not exist yet, it is created.
    pub fn set_value(
        &mut self,
        entity_id: u64,
        component: &str,
        field: &str,
        value: InspectorValue,
    ) -> bool {
        let entity = if let Some(existing) = self.entities.iter_mut().find(|e| e.id == entity_id) {
            existing
        } else {
            self.entities.push(EditorEntity {
                id: entity_id,
                name: format!("Entity{entity_id}"),
                components: Vec::new(),
            });
            let idx = self.entities.len() - 1;
            &mut self.entities[idx]
        };

        let component = if let Some(existing) = entity.component_mut(component) {
            existing
        } else {
            entity.components.push(InspectorComponent {
                type_name: component.to_string(),
                fields: Vec::new(),
            });
            let idx = entity.components.len() - 1;
            &mut entity.components[idx]
        };

        if let Some(existing_field) = component.fields.iter_mut().find(|f| f.name == field) {
            existing_field.value = value;
        } else {
            component.fields.push(InspectorField {
                name: field.to_string(),
                value,
            });
        }

        true
    }

    /// Serialize the editor scene document to a stable line-based text format.
    pub fn to_hscene_string(&self) -> String {
        let mut out = String::new();
        out.push_str("HOLD_SCENE_DOC_V1\n");
        out.push_str(&format!("name|{}\n", escape_text(&self.name)));
        for entity in &self.entities {
            out.push_str(&format!("entity|{}|{}\n", entity.id, escape_text(&entity.name)));
            for component in &entity.components {
                out.push_str(&format!("component|{}\n", escape_text(&component.type_name)));
                for field in &component.fields {
                    out.push_str(&encode_field(field));
                    out.push('\n');
                }
                out.push_str("endcomponent\n");
            }
            out.push_str("endentity\n");
        }
        out
    }

    /// Parse the editor scene document from the line-based text format.
    pub fn from_hscene_str(src: &str) -> Result<Self, &'static str> {
        let mut lines = src.lines();
        if lines.next() != Some("HOLD_SCENE_DOC_V1") {
            return Err("invalid scene document header");
        }

        let name_line = lines.next().ok_or("missing scene name line")?;
        let (_, name) = name_line.split_once('|').ok_or("invalid scene name line")?;
        if !name_line.starts_with("name|") {
            return Err("invalid scene name prefix");
        }

        let mut doc = Self {
            name: unescape_text(name)?,
            entities: Vec::new(),
        };

        let mut current_entity: Option<EditorEntity> = None;
        let mut current_component: Option<InspectorComponent> = None;

        for line in lines {
            if line.is_empty() {
                continue;
            }

            if line == "endcomponent" {
                let component = current_component
                    .take()
                    .ok_or("endcomponent without component")?;
                let entity = current_entity
                    .as_mut()
                    .ok_or("component outside entity in scene doc")?;
                entity.components.push(component);
                continue;
            }

            if line == "endentity" {
                if current_component.is_some() {
                    return Err("entity ended before component closed");
                }
                let entity = current_entity.take().ok_or("endentity without entity")?;
                doc.entities.push(entity);
                continue;
            }

            if let Some(rest) = line.strip_prefix("entity|") {
                if current_entity.is_some() {
                    return Err("nested entity not allowed");
                }
                let mut parts = rest.splitn(2, '|');
                let id = parts
                    .next()
                    .ok_or("missing entity id")?
                    .parse::<u64>()
                    .map_err(|_| "invalid entity id")?;
                let name = unescape_text(parts.next().ok_or("missing entity name")?)?;
                current_entity = Some(EditorEntity {
                    id,
                    name,
                    components: Vec::new(),
                });
                continue;
            }

            if let Some(type_name) = line.strip_prefix("component|") {
                if current_entity.is_none() {
                    return Err("component outside entity");
                }
                if current_component.is_some() {
                    return Err("nested component not allowed");
                }
                current_component = Some(InspectorComponent {
                    type_name: unescape_text(type_name)?,
                    fields: Vec::new(),
                });
                continue;
            }

            if let Some(rest) = line.strip_prefix("field|") {
                let component = current_component
                    .as_mut()
                    .ok_or("field outside component in scene doc")?;
                component.fields.push(decode_field(rest)?);
                continue;
            }

            return Err("unknown scene document line");
        }

        if current_component.is_some() {
            return Err("unterminated component");
        }
        if current_entity.is_some() {
            return Err("unterminated entity");
        }

        Ok(doc)
    }
}

fn encode_field(field: &InspectorField) -> String {
    match &field.value {
        InspectorValue::Bool(v) => {
            format!("field|{}|bool|{}", escape_text(&field.name), if *v { 1 } else { 0 })
        }
        InspectorValue::Number(v) => format!("field|{}|number|{}", escape_text(&field.name), v),
        InspectorValue::Text(v) => {
            format!("field|{}|text|{}", escape_text(&field.name), escape_text(v))
        }
        InspectorValue::Vec3(v) => {
            format!("field|{}|vec3|{}|{}|{}", escape_text(&field.name), v[0], v[1], v[2])
        }
    }
}

fn decode_field(rest: &str) -> Result<InspectorField, &'static str> {
    let mut parts = rest.split('|');
    let name = unescape_text(parts.next().ok_or("missing field name")?)?;
    let kind = parts.next().ok_or("missing field type")?;

    let value = match kind {
        "bool" => {
            let raw = parts.next().ok_or("missing bool value")?;
            InspectorValue::Bool(raw == "1")
        }
        "number" => {
            let raw = parts.next().ok_or("missing number value")?;
            InspectorValue::Number(raw.parse::<f32>().map_err(|_| "invalid number value")?)
        }
        "text" => InspectorValue::Text(unescape_text(parts.next().ok_or("missing text value")?)?),
        "vec3" => {
            let x = parts
                .next()
                .ok_or("missing vec3 x")?
                .parse::<f32>()
                .map_err(|_| "invalid vec3 x")?;
            let y = parts
                .next()
                .ok_or("missing vec3 y")?
                .parse::<f32>()
                .map_err(|_| "invalid vec3 y")?;
            let z = parts
                .next()
                .ok_or("missing vec3 z")?
                .parse::<f32>()
                .map_err(|_| "invalid vec3 z")?;
            InspectorValue::Vec3([x, y, z])
        }
        _ => return Err("unknown field type"),
    };

    Ok(InspectorField { name, value })
}

fn escape_text(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '|' => out.push_str("\\p"),
            _ => out.push(ch),
        }
    }
    out
}

fn unescape_text(input: &str) -> Result<String, &'static str> {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }

        let esc = chars.next().ok_or("dangling escape")?;
        match esc {
            '\\' => out.push('\\'),
            'n' => out.push('\n'),
            'r' => out.push('\r'),
            'p' => out.push('|'),
            _ => return Err("invalid escape sequence"),
        }
    }
    Ok(out)
}

#[derive(Debug)]
pub enum EditorIoError {
    Io(std::io::Error),
    Parse(&'static str),
}

impl std::fmt::Display for EditorIoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "editor io error: {e}"),
            Self::Parse(msg) => write!(f, "editor parse error: {msg}"),
        }
    }
}

impl std::error::Error for EditorIoError {}

/// Save a project document to its `.hproj` location.
pub fn save_project(project: &Project) -> Result<(), EditorIoError> {
    fs::write(project.project_file(), project.to_hproj_string()).map_err(EditorIoError::Io)
}

/// Load a project document from a `.hproj` file path.
pub fn load_project<P: AsRef<Path>>(path: P) -> Result<Project, EditorIoError> {
    let src = fs::read_to_string(path).map_err(EditorIoError::Io)?;
    Project::from_hproj_str(&src).map_err(EditorIoError::Parse)
}

/// Save an editor scene document to a file path.
pub fn save_scene_doc<P: AsRef<Path>>(path: P, scene: &SceneDocument) -> Result<(), EditorIoError> {
    fs::write(path, scene.to_hscene_string()).map_err(EditorIoError::Io)
}

/// Load an editor scene document from a file path.
pub fn load_scene_doc<P: AsRef<Path>>(path: P) -> Result<SceneDocument, EditorIoError> {
    let src = fs::read_to_string(path).map_err(EditorIoError::Io)?;
    SceneDocument::from_hscene_str(&src).map_err(EditorIoError::Parse)
}

#[derive(Debug)]
pub enum EditorApplyError {
    MissingField {
        entity_id: u64,
        component: String,
        field: &'static str,
    },
    InvalidFieldType {
        entity_id: u64,
        component: String,
        field: &'static str,
    },
}

impl std::fmt::Display for EditorApplyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingField {
                entity_id,
                component,
                field,
            } => write!(
                f,
                "editor apply error: entity {} missing {}.{}",
                entity_id, component, field
            ),
            Self::InvalidFieldType {
                entity_id,
                component,
                field,
            } => write!(
                f,
                "editor apply error: entity {} invalid type for {}.{}",
                entity_id, component, field
            ),
        }
    }
}

impl std::error::Error for EditorApplyError {}

#[derive(Debug)]
pub enum EditorPipelineError {
    Io(EditorIoError),
    Apply(EditorApplyError),
}

impl std::fmt::Display for EditorPipelineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "{e}"),
            Self::Apply(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for EditorPipelineError {}

/// Load `.hproj` and `.hscene`, then apply scene entities/components into a world.
pub fn load_and_apply_scene_from_project<P: AsRef<Path>>(
    project_path: P,
    scene_file_name: &str,
    world: &mut World,
) -> Result<Vec<(u64, Entity)>, EditorPipelineError> {
    let project = load_project(project_path).map_err(EditorPipelineError::Io)?;
    let scene_path = format!("{}/{}", project.scenes_dir, scene_file_name);
    let scene = load_scene_doc(scene_path).map_err(EditorPipelineError::Io)?;
    apply_scene_document_to_world(world, &scene).map_err(EditorPipelineError::Apply)
}

/// Apply a dynamic editor scene document to typed ECS components in `world`.
///
/// Returns a vector that maps editor entity ids to spawned ECS entities.
pub fn apply_scene_document_to_world(
    world: &mut World,
    scene: &SceneDocument,
) -> Result<Vec<(u64, Entity)>, EditorApplyError> {
    let mut map = Vec::with_capacity(scene.entities.len());

    for editor_entity in &scene.entities {
        let entity = world.spawn();
        map.push((editor_entity.id, entity));

        for component in &editor_entity.components {
            match component.type_name.as_str() {
                "Transform" => {
                    let position = required_vec3(
                        editor_entity.id,
                        component,
                        component.type_name.as_str(),
                        "position",
                    )?;
                    let scale = optional_vec3(component, "scale").unwrap_or(Vec3::ONE);
                    let rotation = optional_vec3(component, "rotation_euler_deg")
                        .map(|e| Quat::from_euler_xyz(e.x.to_radians(), e.y.to_radians(), e.z.to_radians()))
                        .unwrap_or(Quat::IDENTITY);
                    world.add_component(
                        entity,
                        Transform {
                            position,
                            rotation,
                            scale,
                        },
                    );
                }
                "Stats" => {
                    let health = required_number(
                        editor_entity.id,
                        component,
                        component.type_name.as_str(),
                        "health",
                    )?;
                    let mana = optional_number(component, "mana").unwrap_or(0.0);
                    let attack = optional_number(component, "attack").unwrap_or(0.0);
                    let defense = optional_number(component, "defense").unwrap_or(0.0);
                    let speed = optional_number(component, "speed").unwrap_or(0.0);

                    let mut stats = Stats::new(health, mana, attack, defense, speed);
                    stats.health_max = optional_number(component, "health_max").unwrap_or(stats.health_max);
                    stats.mana_max = optional_number(component, "mana_max").unwrap_or(stats.mana_max);
                    stats.level = optional_number(component, "level").unwrap_or(stats.level as f32).max(1.0) as u32;
                    stats.xp = optional_number(component, "xp").unwrap_or(0.0).max(0.0) as u32;
                    stats.xp_next = optional_number(component, "xp_next").unwrap_or(100.0).max(1.0) as u32;
                    world.add_component(entity, stats);
                }
                "DirectionalLight" | "Light" => {
                    let direction = required_vec3(
                        editor_entity.id,
                        component,
                        component.type_name.as_str(),
                        "direction",
                    )?;
                    let color = optional_vec3(component, "color")
                        .map(|v| Color::rgb(v.x, v.y, v.z))
                        .unwrap_or(Color::WHITE);
                    let intensity = optional_number(component, "intensity").unwrap_or(1.0);
                    let ambient = optional_number(component, "ambient").unwrap_or(0.15);
                    world.add_component(
                        entity,
                        DirectionalLight {
                            direction: direction.normalize(),
                            color,
                            intensity,
                            ambient,
                        },
                    );
                }
                "Camera" => {
                    let fov = optional_number(component, "fov_deg").unwrap_or(60.0);
                    let near = optional_number(component, "near").unwrap_or(0.1);
                    let far = optional_number(component, "far").unwrap_or(1000.0);
                    let aspect = optional_number(component, "aspect").unwrap_or(16.0 / 9.0);
                    world.add_component(entity, Camera::perspective(fov, aspect, near, far));
                }
                _ => {}
            }
        }
    }

    Ok(map)
}

fn required_number(
    entity_id: u64,
    component: &InspectorComponent,
    component_name: &str,
    field: &'static str,
) -> Result<f32, EditorApplyError> {
    let value = component
        .get_field(field)
        .ok_or(EditorApplyError::MissingField {
            entity_id,
            component: component_name.to_string(),
            field,
        })?;
    match value {
        InspectorValue::Number(v) => Ok(*v),
        _ => Err(EditorApplyError::InvalidFieldType {
            entity_id,
            component: component_name.to_string(),
            field,
        }),
    }
}

fn required_vec3(
    entity_id: u64,
    component: &InspectorComponent,
    component_name: &str,
    field: &'static str,
) -> Result<Vec3, EditorApplyError> {
    let value = component
        .get_field(field)
        .ok_or(EditorApplyError::MissingField {
            entity_id,
            component: component_name.to_string(),
            field,
        })?;
    match value {
        InspectorValue::Vec3(v) => Ok(Vec3::new(v[0], v[1], v[2])),
        _ => Err(EditorApplyError::InvalidFieldType {
            entity_id,
            component: component_name.to_string(),
            field,
        }),
    }
}

fn optional_number(component: &InspectorComponent, field: &str) -> Option<f32> {
    match component.get_field(field)? {
        InspectorValue::Number(v) => Some(*v),
        _ => None,
    }
}

fn optional_vec3(component: &InspectorComponent, field: &str) -> Option<Vec3> {
    match component.get_field(field)? {
        InspectorValue::Vec3(v) => Some(Vec3::new(v[0], v[1], v[2])),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_test_dir(suffix: &str) -> String {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        format!(
            "{}/hold_engine_editor_tests_{}_{}",
            std::env::temp_dir().display(),
            suffix,
            nanos
        )
    }

    #[test]
    fn project_layout_paths() {
        let p = Project::new("demo", "C:/games/demo");
        assert_eq!(p.assets_dir, "C:/games/demo/assets");
        assert_eq!(p.scenes_dir, "C:/games/demo/scenes");
        assert_eq!(p.project_file(), "C:/games/demo/demo.hproj");
    }

    #[test]
    fn project_hproj_roundtrip() {
        let p = Project::new("demo", "C:/games/demo");
        let text = p.to_hproj_string();
        let parsed = Project::from_hproj_str(&text).unwrap();
        assert_eq!(parsed, p);
    }

    #[test]
    fn scene_document_set_value_creates_missing_path() {
        let mut doc = SceneDocument {
            name: "Level02".into(),
            entities: Vec::new(),
        };

        assert!(doc.set_value(42, "Stats", "health", InspectorValue::Number(100.0)));

        assert_eq!(doc.entities.len(), 1);
        assert_eq!(doc.entities[0].name, "Entity42");
        assert_eq!(
            doc.get_value(42, "Stats", "health"),
            Some(&InspectorValue::Number(100.0))
        );
    }
    #[test]
    fn scene_document_dynamic_access() {
        let mut doc = SceneDocument {
            name: "Level01".into(),
            entities: vec![EditorEntity {
                id: 1,
                name: "Player".into(),
                components: vec![InspectorComponent {
                    type_name: "Transform".into(),
                    fields: vec![InspectorField {
                        name: "position".into(),
                        value: InspectorValue::Vec3([0.0, 1.0, 2.0]),
                    }],
                }],
            }],
        };

        assert_eq!(
            doc.get_value(1, "Transform", "position"),
            Some(&InspectorValue::Vec3([0.0, 1.0, 2.0]))
        );

        assert!(doc.set_value(
            1,
            "Transform",
            "position",
            InspectorValue::Vec3([3.0, 4.0, 5.0])
        ));

        assert_eq!(
            doc.get_value(1, "Transform", "position"),
            Some(&InspectorValue::Vec3([3.0, 4.0, 5.0]))
        );
    }

    #[test]
    fn scene_document_roundtrip() {
        let doc = SceneDocument {
            name: "Roundtrip".into(),
            entities: vec![EditorEntity {
                id: 7,
                name: "Hero".into(),
                components: vec![
                    InspectorComponent {
                        type_name: "Transform".into(),
                        fields: vec![InspectorField {
                            name: "position".into(),
                            value: InspectorValue::Vec3([1.0, 2.0, 3.0]),
                        }],
                    },
                    InspectorComponent {
                        type_name: "Stats".into(),
                        fields: vec![
                            InspectorField {
                                name: "health".into(),
                                value: InspectorValue::Number(100.0),
                            },
                            InspectorField {
                                name: "alive".into(),
                                value: InspectorValue::Bool(true),
                            },
                            InspectorField {
                                name: "title".into(),
                                value: InspectorValue::Text("Knight".into()),
                            },
                        ],
                    },
                ],
            }],
        };

        let text = doc.to_hscene_string();
        let parsed = SceneDocument::from_hscene_str(&text).unwrap();
        assert_eq!(parsed, doc);
    }

    #[test]
    fn project_file_io_roundtrip() {
        let root = unique_test_dir("project");
        fs::create_dir_all(&root).unwrap();

        let project = Project::new("demo", root.clone());
        save_project(&project).unwrap();

        let loaded = load_project(project.project_file()).unwrap();
        assert_eq!(loaded, project);
    }

    #[test]
    fn scene_document_file_io_roundtrip() {
        let root = unique_test_dir("scene");
        fs::create_dir_all(&root).unwrap();
        let scene_path = format!("{root}/level01.hscene");

        let doc = SceneDocument {
            name: "Level01".into(),
            entities: vec![EditorEntity {
                id: 1,
                name: "Player".into(),
                components: vec![InspectorComponent {
                    type_name: "Stats".into(),
                    fields: vec![InspectorField {
                        name: "health".into(),
                        value: InspectorValue::Number(100.0),
                    }],
                }],
            }],
        };

        save_scene_doc(&scene_path, &doc).unwrap();
        let loaded = load_scene_doc(&scene_path).unwrap();
        assert_eq!(loaded, doc);
    }

    #[test]
    fn serialization_escapes_reserved_characters() {
        let project = Project {
            name: "demo|name".into(),
            root: "C:/games/demo\\test".into(),
            assets_dir: "C:/games/demo/assets".into(),
            scenes_dir: "C:/games/demo/scenes".into(),
        };
        let project_text = project.to_hproj_string();
        let project_loaded = Project::from_hproj_str(&project_text).unwrap();
        assert_eq!(project_loaded, project);

        let doc = SceneDocument {
            name: "Level|One".into(),
            entities: vec![EditorEntity {
                id: 9,
                name: "Boss|Unit".into(),
                components: vec![InspectorComponent {
                    type_name: "Dialogue|State".into(),
                    fields: vec![InspectorField {
                        name: "line|1".into(),
                        value: InspectorValue::Text("slash\\\\pipe|newline\nok".into()),
                    }],
                }],
            }],
        };

        let text = doc.to_hscene_string();
        let loaded = SceneDocument::from_hscene_str(&text).unwrap();
        assert_eq!(loaded, doc);
    }

    #[test]
    fn apply_scene_document_to_world_maps_core_components() {
        let scene = SceneDocument {
            name: "Apply".into(),
            entities: vec![EditorEntity {
                id: 100,
                name: "Player".into(),
                components: vec![
                    InspectorComponent {
                        type_name: "Transform".into(),
                        fields: vec![
                            InspectorField {
                                name: "position".into(),
                                value: InspectorValue::Vec3([1.0, 2.0, 3.0]),
                            },
                            InspectorField {
                                name: "scale".into(),
                                value: InspectorValue::Vec3([2.0, 2.0, 2.0]),
                            },
                        ],
                    },
                    InspectorComponent {
                        type_name: "Stats".into(),
                        fields: vec![InspectorField {
                            name: "health".into(),
                            value: InspectorValue::Number(120.0),
                        }],
                    },
                ],
            }],
        };

        let mut world = World::new();
        let map = apply_scene_document_to_world(&mut world, &scene).unwrap();
        assert_eq!(map.len(), 1);
        assert_eq!(map[0].0, 100);

        let entity = map[0].1;
        let t = world.get::<Transform>(entity).unwrap();
        assert_eq!(t.position, Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(t.scale, Vec3::new(2.0, 2.0, 2.0));

        let stats = world.get::<Stats>(entity).unwrap();
        assert_eq!(stats.health, 120.0);
    }

    #[test]
    fn load_and_apply_scene_from_project_pipeline() {
        let root = unique_test_dir("pipeline");
        let scenes_dir = format!("{root}/scenes");
        fs::create_dir_all(&scenes_dir).unwrap();

        let project = Project::new("demo", root.clone());
        save_project(&project).unwrap();

        let scene = SceneDocument {
            name: "Level01".into(),
            entities: vec![EditorEntity {
                id: 1,
                name: "Cam".into(),
                components: vec![
                    InspectorComponent {
                        type_name: "Transform".into(),
                        fields: vec![InspectorField {
                            name: "position".into(),
                            value: InspectorValue::Vec3([0.0, 1.0, 5.0]),
                        }],
                    },
                    InspectorComponent {
                        type_name: "Camera".into(),
                        fields: vec![InspectorField {
                            name: "fov_deg".into(),
                            value: InspectorValue::Number(75.0),
                        }],
                    },
                ],
            }],
        };

        let scene_file = "level01.hscene";
        let scene_path = format!("{}/{}", project.scenes_dir, scene_file);
        save_scene_doc(scene_path, &scene).unwrap();

        let mut world = World::new();
        let map = load_and_apply_scene_from_project(project.project_file(), scene_file, &mut world).unwrap();
        assert_eq!(map.len(), 1);
        let entity = map[0].1;
        assert!(world.has_component::<Transform>(entity));
        assert!(world.has_component::<Camera>(entity));
    }
}
