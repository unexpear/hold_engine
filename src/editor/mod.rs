//! Editor-side data model for projects, scenes, and inspector editing.
//! This module is runtime-agnostic and keeps a dynamic editing layer on top
//! of typed ECS/runtime data.

pub mod bridge;
pub mod command;
pub mod drawlist;
mod host;
pub mod native_ui;
pub mod protocol;
mod session;

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

use crate::ecs::components::{
    ActiveCamera, Camera, DirectionalLight, GlobalTransform, ParticleEmitter, RigidBody,
    SphereCollider, Stats, Transform, Velocity,
};
use crate::ecs::{Entity, World};
use crate::math::{Quat, Vec3};
use crate::renderer::Color;
use crate::scene::{
    load_scene, parse_scene, save_scene, serialize_scene, CameraDef, EntityDef, LightDef,
    ParticleEmitterDef, RigidBodyDef, SceneData, SceneError, SphereColliderDef, StatsDef,
    TransformDef, VelocityDef,
};

pub use bridge::EditorBridgeServer;
pub use command::{
    EditorAxis, EditorCommand, EditorCommandError, EditorEvent, EditorInspectorCursor,
};
pub use drawlist::{generate_draw_list, EditorDrawList};
pub use host::{EditorCamera, EditorRuntimeHost};
pub use native_ui::{build_native_ui, EditorNativeUi, EditorUiHitRegion};
pub use protocol::{
    decode_command as decode_editor_command_protocol, decode_event as decode_editor_event_protocol,
    encode_command as encode_editor_command_protocol, encode_event as encode_editor_event_protocol,
    EditorProtocolError,
};
pub use session::{EditorPlayState, EditorSceneSession, EditorSessionError};

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
    pub fn from_scene_data(scene: &SceneData) -> Self {
        let mut name_to_id = HashMap::with_capacity(scene.entities.len());
        for (index, entity) in scene.entities.iter().enumerate() {
            name_to_id.insert(entity.name.clone(), index as u64 + 1);
        }

        let entities = scene
            .entities
            .iter()
            .enumerate()
            .map(|(index, entity)| EditorEntity {
                id: index as u64 + 1,
                name: entity.name.clone(),
                components: components_from_scene_entity(entity, &name_to_id),
            })
            .collect();

        Self {
            name: scene.name.clone(),
            entities,
        }
    }

    pub fn to_scene_data(&self) -> Result<SceneData, SceneBridgeError> {
        let unique_names = unique_scene_names(&self.entities);

        let entities = self
            .entities
            .iter()
            .map(|entity| {
                Ok(EntityDef {
                    name: unique_names
                        .get(&entity.id)
                        .cloned()
                        .unwrap_or_else(|| format!("Entity{}", entity.id)),
                    transform: entity
                        .component("Transform")
                        .map(|component| bridge_transform(entity.id, component))
                        .transpose()?,
                    velocity: entity
                        .component("Velocity")
                        .map(|component| bridge_velocity(entity.id, component))
                        .transpose()?,
                    rigid_body: entity
                        .component("RigidBody")
                        .map(|component| bridge_rigid_body(entity.id, component))
                        .transpose()?,
                    sphere_collider: entity
                        .component("SphereCollider")
                        .map(|component| bridge_sphere_collider(entity.id, component))
                        .transpose()?,
                    particle_emitter: entity
                        .component("ParticleEmitter")
                        .map(|component| bridge_particle_emitter(entity.id, component))
                        .transpose()?,
                    camera: entity
                        .component("Camera")
                        .map(|component| bridge_camera(entity.id, component))
                        .transpose()?,
                    active_camera: entity.component("ActiveCamera").is_some(),
                    light: entity
                        .component("DirectionalLight")
                        .or_else(|| entity.component("Light"))
                        .map(|component| bridge_light(entity.id, component))
                        .transpose()?,
                    stats: entity
                        .component("Stats")
                        .map(|component| bridge_stats(entity.id, component))
                        .transpose()?,
                    parent_name: entity
                        .component("Parent")
                        .map(|component| bridge_parent_name(entity.id, component, &unique_names))
                        .transpose()?
                        .flatten(),
                })
            })
            .collect::<Result<Vec<_>, SceneBridgeError>>()?;

        Ok(SceneData {
            name: self.name.clone(),
            entities,
        })
    }

    /// Parse a runtime `.hsc` payload into the editor-facing scene document.
    pub fn from_hsc_str(src: &str) -> Result<Self, SceneError> {
        let scene = parse_scene(src)?;
        Ok(Self::from_scene_data(&scene))
    }

    /// Export the editor-facing scene document into runtime `.hsc` text.
    pub fn to_hsc_string(&self) -> Result<String, SceneBridgeError> {
        let scene = self.to_scene_data()?;
        Ok(serialize_scene(&scene))
    }

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
            out.push_str(&format!(
                "entity|{}|{}\n",
                entity.id,
                escape_text(&entity.name)
            ));
            for component in &entity.components {
                out.push_str(&format!(
                    "component|{}\n",
                    escape_text(&component.type_name)
                ));
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SceneBridgeError {
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
    UnknownParentEntityId {
        entity_id: u64,
        parent_id: u64,
    },
}

impl std::fmt::Display for SceneBridgeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingField {
                entity_id,
                component,
                field,
            } => write!(
                f,
                "scene bridge error: entity {} missing {}.{}",
                entity_id, component, field
            ),
            Self::InvalidFieldType {
                entity_id,
                component,
                field,
            } => write!(
                f,
                "scene bridge error: entity {} invalid type for {}.{}",
                entity_id, component, field
            ),
            Self::UnknownParentEntityId {
                entity_id,
                parent_id,
            } => write!(
                f,
                "scene bridge error: entity {} references unknown parent id {}",
                entity_id, parent_id
            ),
        }
    }
}

impl std::error::Error for SceneBridgeError {}

fn components_from_scene_entity(
    entity: &EntityDef,
    name_to_id: &HashMap<String, u64>,
) -> Vec<InspectorComponent> {
    let mut components = Vec::new();

    if let Some(transform) = &entity.transform {
        components.push(InspectorComponent {
            type_name: "Transform".into(),
            fields: vec![
                vec3_field(
                    "position",
                    [
                        transform.position.x,
                        transform.position.y,
                        transform.position.z,
                    ],
                ),
                vec3_field(
                    "scale",
                    [transform.scale.x, transform.scale.y, transform.scale.z],
                ),
                number_field("rotation_x", transform.rotation.x),
                number_field("rotation_y", transform.rotation.y),
                number_field("rotation_z", transform.rotation.z),
                number_field("rotation_w", transform.rotation.w),
            ],
        });
    }

    if let Some(velocity) = &entity.velocity {
        components.push(InspectorComponent {
            type_name: "Velocity".into(),
            fields: vec![
                vec3_field(
                    "linear",
                    [velocity.linear.x, velocity.linear.y, velocity.linear.z],
                ),
                vec3_field(
                    "angular",
                    [velocity.angular.x, velocity.angular.y, velocity.angular.z],
                ),
            ],
        });
    }

    if let Some(rigid_body) = &entity.rigid_body {
        components.push(InspectorComponent {
            type_name: "RigidBody".into(),
            fields: vec![
                number_field("gravity_scale", rigid_body.gravity_scale),
                number_field("linear_damping", rigid_body.linear_damping),
                number_field("angular_damping", rigid_body.angular_damping),
                number_field("restitution", rigid_body.restitution),
            ],
        });
    }

    if let Some(collider) = &entity.sphere_collider {
        components.push(InspectorComponent {
            type_name: "SphereCollider".into(),
            fields: vec![number_field("radius", collider.radius)],
        });
    }

    if let Some(emitter) = &entity.particle_emitter {
        components.push(InspectorComponent {
            type_name: "ParticleEmitter".into(),
            fields: vec![
                number_field("emitting", if emitter.emitting { 1.0 } else { 0.0 }),
                number_field("rate_per_second", emitter.rate_per_second),
                number_field("max_particles", emitter.max_particles as f32),
                number_field("spawn_radius", emitter.spawn_radius),
                vec3_field(
                    "direction",
                    [
                        emitter.direction.x,
                        emitter.direction.y,
                        emitter.direction.z,
                    ],
                ),
                number_field("spread", emitter.spread),
                number_field("speed_min", emitter.speed_min),
                number_field("speed_max", emitter.speed_max),
                number_field("lifetime_min", emitter.lifetime_min),
                number_field("lifetime_max", emitter.lifetime_max),
                number_field("gravity_scale", emitter.gravity_scale),
                number_field("linear_damping", emitter.linear_damping),
                number_field("start_size", emitter.start_size),
                number_field("end_size", emitter.end_size),
                vec3_field(
                    "start_color",
                    [
                        emitter.start_color.r,
                        emitter.start_color.g,
                        emitter.start_color.b,
                    ],
                ),
                number_field("start_color_alpha", emitter.start_color.a),
                vec3_field(
                    "end_color",
                    [
                        emitter.end_color.r,
                        emitter.end_color.g,
                        emitter.end_color.b,
                    ],
                ),
                number_field("end_color_alpha", emitter.end_color.a),
                number_field("seed", emitter.seed as f32),
            ],
        });
    }

    if let Some(camera) = &entity.camera {
        components.push(InspectorComponent {
            type_name: "Camera".into(),
            fields: vec![
                number_field("fov_deg", camera.fov),
                number_field("near", camera.near),
                number_field("far", camera.far),
            ],
        });
    }

    if entity.active_camera {
        components.push(InspectorComponent {
            type_name: "ActiveCamera".into(),
            fields: Vec::new(),
        });
    }

    if let Some(light) = &entity.light {
        components.push(InspectorComponent {
            type_name: "DirectionalLight".into(),
            fields: vec![
                vec3_field(
                    "direction",
                    [light.direction.x, light.direction.y, light.direction.z],
                ),
                vec3_field("color", [light.color.r, light.color.g, light.color.b]),
                number_field("color_alpha", light.color.a),
                number_field("intensity", light.intensity),
                number_field("ambient", light.ambient),
            ],
        });
    }

    if let Some(stats) = &entity.stats {
        components.push(InspectorComponent {
            type_name: "Stats".into(),
            fields: vec![
                number_field("health", stats.health),
                number_field("mana", stats.mana),
                number_field("attack", stats.attack),
                number_field("defense", stats.defense),
                number_field("speed", stats.speed),
            ],
        });
    }

    if let Some(parent_name) = &entity.parent_name {
        let mut fields = vec![text_field("entity_name", parent_name)];
        if let Some(parent_id) = name_to_id.get(parent_name) {
            fields.push(number_field("entity_id", *parent_id as f32));
        }
        components.push(InspectorComponent {
            type_name: "Parent".into(),
            fields,
        });
    }

    components
}

fn unique_scene_names(entities: &[EditorEntity]) -> HashMap<u64, String> {
    let mut used = HashSet::new();
    let mut names = HashMap::with_capacity(entities.len());

    for entity in entities {
        let base_name = if entity.name.trim().is_empty() {
            format!("Entity{}", entity.id)
        } else {
            entity.name.trim().to_string()
        };
        let mut name = base_name.clone();
        let mut suffix = 2;
        while used.contains(&name) {
            name = format!("{} {}", base_name, suffix);
            suffix += 1;
        }
        used.insert(name.clone());
        names.insert(entity.id, name);
    }

    names
}

fn number_field(name: &str, value: f32) -> InspectorField {
    InspectorField {
        name: name.into(),
        value: InspectorValue::Number(value),
    }
}

fn text_field(name: &str, value: &str) -> InspectorField {
    InspectorField {
        name: name.into(),
        value: InspectorValue::Text(value.into()),
    }
}

fn vec3_field(name: &str, value: [f32; 3]) -> InspectorField {
    InspectorField {
        name: name.into(),
        value: InspectorValue::Vec3(value),
    }
}

fn bridge_transform(
    entity_id: u64,
    component: &InspectorComponent,
) -> Result<TransformDef, SceneBridgeError> {
    let position = required_vec3_bridge(entity_id, component, "position")?;
    let scale = optional_vec3(component, "scale").unwrap_or(Vec3::ONE);
    let rotation = bridge_rotation(entity_id, component)?;

    Ok(TransformDef {
        position,
        rotation,
        scale,
    })
}

fn bridge_rotation(
    entity_id: u64,
    component: &InspectorComponent,
) -> Result<Quat, SceneBridgeError> {
    let quat_fields = [
        optional_number(component, "rotation_x"),
        optional_number(component, "rotation_y"),
        optional_number(component, "rotation_z"),
        optional_number(component, "rotation_w"),
    ];
    if quat_fields.iter().any(|value| value.is_some()) {
        return Ok(Quat::new(
            required_number_bridge(entity_id, component, "rotation_x")?,
            required_number_bridge(entity_id, component, "rotation_y")?,
            required_number_bridge(entity_id, component, "rotation_z")?,
            required_number_bridge(entity_id, component, "rotation_w")?,
        ));
    }

    Ok(optional_vec3(component, "rotation_euler_deg")
        .map(|e| Quat::from_euler_xyz(e.x.to_radians(), e.y.to_radians(), e.z.to_radians()))
        .unwrap_or(Quat::IDENTITY))
}

fn bridge_camera(
    entity_id: u64,
    component: &InspectorComponent,
) -> Result<CameraDef, SceneBridgeError> {
    Ok(CameraDef {
        fov: required_number_bridge(entity_id, component, "fov_deg")?,
        near: optional_number(component, "near").unwrap_or(0.1),
        far: optional_number(component, "far").unwrap_or(1000.0),
    })
}

fn bridge_velocity(
    _entity_id: u64,
    component: &InspectorComponent,
) -> Result<VelocityDef, SceneBridgeError> {
    Ok(VelocityDef {
        linear: optional_vec3(component, "linear").unwrap_or(Vec3::ZERO),
        angular: optional_vec3(component, "angular").unwrap_or(Vec3::ZERO),
    })
}

fn bridge_rigid_body(
    _entity_id: u64,
    component: &InspectorComponent,
) -> Result<RigidBodyDef, SceneBridgeError> {
    let default = RigidBody::default();
    Ok(RigidBodyDef {
        gravity_scale: optional_number(component, "gravity_scale").unwrap_or(default.gravity_scale),
        linear_damping: optional_number(component, "linear_damping")
            .unwrap_or(default.linear_damping),
        angular_damping: optional_number(component, "angular_damping")
            .unwrap_or(default.angular_damping),
        restitution: optional_number(component, "restitution").unwrap_or(default.restitution),
    })
}

fn bridge_sphere_collider(
    _entity_id: u64,
    component: &InspectorComponent,
) -> Result<SphereColliderDef, SceneBridgeError> {
    Ok(SphereColliderDef {
        radius: optional_number(component, "radius").unwrap_or(0.5),
    })
}

fn bridge_particle_emitter(
    _entity_id: u64,
    component: &InspectorComponent,
) -> Result<ParticleEmitterDef, SceneBridgeError> {
    let default = ParticleEmitter::spark_fountain();
    Ok(ParticleEmitterDef {
        emitting: optional_number(component, "emitting").unwrap_or(if default.emitting {
            1.0
        } else {
            0.0
        }) > 0.0,
        rate_per_second: optional_number(component, "rate_per_second")
            .or_else(|| optional_number(component, "rate"))
            .unwrap_or(default.rate_per_second),
        max_particles: optional_number(component, "max_particles")
            .or_else(|| optional_number(component, "max"))
            .unwrap_or(default.max_particles as f32)
            .max(0.0) as u32,
        spawn_radius: optional_number(component, "spawn_radius").unwrap_or(default.spawn_radius),
        direction: optional_vec3(component, "direction").unwrap_or(default.direction),
        spread: optional_number(component, "spread").unwrap_or(default.spread),
        speed_min: optional_number(component, "speed_min").unwrap_or(default.speed_min),
        speed_max: optional_number(component, "speed_max").unwrap_or(default.speed_max),
        lifetime_min: optional_number(component, "lifetime_min").unwrap_or(default.lifetime_min),
        lifetime_max: optional_number(component, "lifetime_max").unwrap_or(default.lifetime_max),
        gravity_scale: optional_number(component, "gravity_scale").unwrap_or(default.gravity_scale),
        linear_damping: optional_number(component, "linear_damping")
            .unwrap_or(default.linear_damping),
        start_size: optional_number(component, "start_size").unwrap_or(default.start_size),
        end_size: optional_number(component, "end_size").unwrap_or(default.end_size),
        start_color: optional_vec3(component, "start_color")
            .map(|color| {
                Color::new(
                    color.x,
                    color.y,
                    color.z,
                    optional_number(component, "start_color_alpha")
                        .unwrap_or(default.start_color.a),
                )
            })
            .unwrap_or(default.start_color),
        end_color: optional_vec3(component, "end_color")
            .map(|color| {
                Color::new(
                    color.x,
                    color.y,
                    color.z,
                    optional_number(component, "end_color_alpha").unwrap_or(default.end_color.a),
                )
            })
            .unwrap_or(default.end_color),
        seed: optional_number(component, "seed")
            .unwrap_or(default.seed as f32)
            .max(0.0) as u32,
    })
}

fn bridge_light(
    entity_id: u64,
    component: &InspectorComponent,
) -> Result<LightDef, SceneBridgeError> {
    let direction = required_vec3_bridge(entity_id, component, "direction")?;
    let color = required_vec3_bridge(entity_id, component, "color")?;

    Ok(LightDef {
        direction,
        color: Color::new(
            color.x,
            color.y,
            color.z,
            optional_number(component, "color_alpha").unwrap_or(1.0),
        ),
        intensity: optional_number(component, "intensity").unwrap_or(1.0),
        ambient: optional_number(component, "ambient").unwrap_or(0.15),
    })
}

fn bridge_stats(
    entity_id: u64,
    component: &InspectorComponent,
) -> Result<StatsDef, SceneBridgeError> {
    Ok(StatsDef {
        health: required_number_bridge(entity_id, component, "health")?,
        mana: optional_number(component, "mana").unwrap_or(0.0),
        attack: optional_number(component, "attack").unwrap_or(0.0),
        defense: optional_number(component, "defense").unwrap_or(0.0),
        speed: optional_number(component, "speed").unwrap_or(0.0),
    })
}

fn bridge_parent_name(
    entity_id: u64,
    component: &InspectorComponent,
    unique_names: &HashMap<u64, String>,
) -> Result<Option<String>, SceneBridgeError> {
    if let Some(parent_id) = optional_number(component, "entity_id") {
        let parent_id = parent_id as u64;
        return unique_names.get(&parent_id).cloned().map(Some).ok_or(
            SceneBridgeError::UnknownParentEntityId {
                entity_id,
                parent_id,
            },
        );
    }

    Ok(optional_text(component, "entity_name"))
}

fn required_number_bridge(
    entity_id: u64,
    component: &InspectorComponent,
    field: &'static str,
) -> Result<f32, SceneBridgeError> {
    let value = component
        .get_field(field)
        .ok_or(SceneBridgeError::MissingField {
            entity_id,
            component: component.type_name.clone(),
            field,
        })?;
    match value {
        InspectorValue::Number(v) => Ok(*v),
        _ => Err(SceneBridgeError::InvalidFieldType {
            entity_id,
            component: component.type_name.clone(),
            field,
        }),
    }
}

fn required_vec3_bridge(
    entity_id: u64,
    component: &InspectorComponent,
    field: &'static str,
) -> Result<Vec3, SceneBridgeError> {
    let value = component
        .get_field(field)
        .ok_or(SceneBridgeError::MissingField {
            entity_id,
            component: component.type_name.clone(),
            field,
        })?;
    match value {
        InspectorValue::Vec3(v) => Ok(Vec3::new(v[0], v[1], v[2])),
        _ => Err(SceneBridgeError::InvalidFieldType {
            entity_id,
            component: component.type_name.clone(),
            field,
        }),
    }
}

fn encode_field(field: &InspectorField) -> String {
    match &field.value {
        InspectorValue::Bool(v) => {
            format!(
                "field|{}|bool|{}",
                escape_text(&field.name),
                if *v { 1 } else { 0 }
            )
        }
        InspectorValue::Number(v) => format!("field|{}|number|{}", escape_text(&field.name), v),
        InspectorValue::Text(v) => {
            format!("field|{}|text|{}", escape_text(&field.name), escape_text(v))
        }
        InspectorValue::Vec3(v) => {
            format!(
                "field|{}|vec3|{}|{}|{}",
                escape_text(&field.name),
                v[0],
                v[1],
                v[2]
            )
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
    Scene(SceneError),
    Bridge(SceneBridgeError),
}

impl std::fmt::Display for EditorIoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "editor io error: {e}"),
            Self::Parse(msg) => write!(f, "editor parse error: {msg}"),
            Self::Scene(err) => write!(f, "editor scene error: {err}"),
            Self::Bridge(err) => write!(f, "editor scene bridge error: {err}"),
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

/// Load a runtime `.hsc` file into the editor-facing scene document.
pub fn load_scene_doc_from_hsc<P: AsRef<Path>>(path: P) -> Result<SceneDocument, EditorIoError> {
    let scene = load_scene(path).map_err(EditorIoError::Scene)?;
    Ok(SceneDocument::from_scene_data(&scene))
}

/// Export an editor scene document back into a runtime `.hsc` file.
pub fn save_scene_doc_to_hsc<P: AsRef<Path>>(
    path: P,
    scene: &SceneDocument,
) -> Result<(), EditorIoError> {
    let runtime_scene = scene.to_scene_data().map_err(EditorIoError::Bridge)?;
    save_scene(path, &runtime_scene).map_err(EditorIoError::Scene)
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
    UnknownParentReference {
        entity_id: u64,
        reference: String,
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
            Self::UnknownParentReference {
                entity_id,
                reference,
            } => write!(
                f,
                "editor apply error: entity {} references unknown parent {}",
                entity_id, reference
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

/// Load a runtime `.hsc` file through the editor bridge, then apply it to a world.
pub fn load_and_apply_scene_from_hsc<P: AsRef<Path>>(
    scene_path: P,
    world: &mut World,
) -> Result<Vec<(u64, Entity)>, EditorPipelineError> {
    let scene = load_scene_doc_from_hsc(scene_path).map_err(EditorPipelineError::Io)?;
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
    let mut entity_by_id = HashMap::with_capacity(scene.entities.len());
    let mut entity_by_name = HashMap::with_capacity(scene.entities.len());
    let mut pending_parents = Vec::new();

    for editor_entity in &scene.entities {
        let entity = world.spawn();
        map.push((editor_entity.id, entity));
        entity_by_id.insert(editor_entity.id, entity);
        entity_by_name.insert(editor_entity.name.clone(), entity);

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
                    let rotation = if has_any_field(
                        component,
                        &["rotation_x", "rotation_y", "rotation_z", "rotation_w"],
                    ) {
                        Quat::new(
                            required_number(
                                editor_entity.id,
                                component,
                                component.type_name.as_str(),
                                "rotation_x",
                            )?,
                            required_number(
                                editor_entity.id,
                                component,
                                component.type_name.as_str(),
                                "rotation_y",
                            )?,
                            required_number(
                                editor_entity.id,
                                component,
                                component.type_name.as_str(),
                                "rotation_z",
                            )?,
                            required_number(
                                editor_entity.id,
                                component,
                                component.type_name.as_str(),
                                "rotation_w",
                            )?,
                        )
                    } else {
                        optional_vec3(component, "rotation_euler_deg")
                            .map(|e| {
                                Quat::from_euler_xyz(
                                    e.x.to_radians(),
                                    e.y.to_radians(),
                                    e.z.to_radians(),
                                )
                            })
                            .unwrap_or(Quat::IDENTITY)
                    };
                    world.add_component(
                        entity,
                        Transform {
                            position,
                            rotation,
                            scale,
                        },
                    );
                    world.add_component(entity, GlobalTransform::identity());
                }
                "Velocity" => {
                    world.add_component(
                        entity,
                        Velocity {
                            linear: optional_vec3(component, "linear").unwrap_or(Vec3::ZERO),
                            angular: optional_vec3(component, "angular").unwrap_or(Vec3::ZERO),
                        },
                    );
                }
                "RigidBody" => {
                    let default = RigidBody::default();
                    world.add_component(
                        entity,
                        RigidBody {
                            gravity_scale: optional_number(component, "gravity_scale")
                                .unwrap_or(default.gravity_scale),
                            linear_damping: optional_number(component, "linear_damping")
                                .unwrap_or(default.linear_damping),
                            angular_damping: optional_number(component, "angular_damping")
                                .unwrap_or(default.angular_damping),
                            restitution: optional_number(component, "restitution")
                                .unwrap_or(default.restitution),
                        },
                    );
                }
                "SphereCollider" => {
                    world.add_component(
                        entity,
                        SphereCollider::new(optional_number(component, "radius").unwrap_or(0.5)),
                    );
                }
                "ParticleEmitter" => {
                    let default = ParticleEmitter::spark_fountain();
                    world.add_component(
                        entity,
                        ParticleEmitter {
                            emitting: optional_number(component, "emitting")
                                .unwrap_or(if default.emitting { 1.0 } else { 0.0 })
                                > 0.0,
                            rate_per_second: optional_number(component, "rate_per_second")
                                .or_else(|| optional_number(component, "rate"))
                                .unwrap_or(default.rate_per_second),
                            max_particles: optional_number(component, "max_particles")
                                .or_else(|| optional_number(component, "max"))
                                .unwrap_or(default.max_particles as f32)
                                .max(0.0) as u32,
                            spawn_radius: optional_number(component, "spawn_radius")
                                .unwrap_or(default.spawn_radius),
                            direction: optional_vec3(component, "direction")
                                .unwrap_or(default.direction),
                            spread: optional_number(component, "spread").unwrap_or(default.spread),
                            speed_min: optional_number(component, "speed_min")
                                .unwrap_or(default.speed_min),
                            speed_max: optional_number(component, "speed_max")
                                .unwrap_or(default.speed_max),
                            lifetime_min: optional_number(component, "lifetime_min")
                                .unwrap_or(default.lifetime_min),
                            lifetime_max: optional_number(component, "lifetime_max")
                                .unwrap_or(default.lifetime_max),
                            gravity_scale: optional_number(component, "gravity_scale")
                                .unwrap_or(default.gravity_scale),
                            linear_damping: optional_number(component, "linear_damping")
                                .unwrap_or(default.linear_damping),
                            start_size: optional_number(component, "start_size")
                                .unwrap_or(default.start_size),
                            end_size: optional_number(component, "end_size")
                                .unwrap_or(default.end_size),
                            start_color: optional_vec3(component, "start_color")
                                .map(|color| {
                                    Color::new(
                                        color.x,
                                        color.y,
                                        color.z,
                                        optional_number(component, "start_color_alpha")
                                            .unwrap_or(default.start_color.a),
                                    )
                                })
                                .unwrap_or(default.start_color),
                            end_color: optional_vec3(component, "end_color")
                                .map(|color| {
                                    Color::new(
                                        color.x,
                                        color.y,
                                        color.z,
                                        optional_number(component, "end_color_alpha")
                                            .unwrap_or(default.end_color.a),
                                    )
                                })
                                .unwrap_or(default.end_color),
                            accumulator: 0.0,
                            seed: optional_number(component, "seed")
                                .unwrap_or(default.seed as f32)
                                .max(0.0) as u32,
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
                    stats.health_max =
                        optional_number(component, "health_max").unwrap_or(stats.health_max);
                    stats.mana_max =
                        optional_number(component, "mana_max").unwrap_or(stats.mana_max);
                    stats.level = optional_number(component, "level")
                        .unwrap_or(stats.level as f32)
                        .max(1.0) as u32;
                    stats.xp = optional_number(component, "xp").unwrap_or(0.0).max(0.0) as u32;
                    stats.xp_next = optional_number(component, "xp_next")
                        .unwrap_or(100.0)
                        .max(1.0) as u32;
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
                        .map(|v| {
                            Color::new(
                                v.x,
                                v.y,
                                v.z,
                                optional_number(component, "color_alpha").unwrap_or(1.0),
                            )
                        })
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
                "ActiveCamera" => {
                    world.add_component(entity, ActiveCamera);
                }
                "Parent" => {
                    pending_parents.push((
                        editor_entity.id,
                        entity,
                        optional_number(component, "entity_id").map(|value| value as u64),
                        optional_text(component, "entity_name"),
                    ));
                }
                _ => {}
            }
        }
    }

    for (child_id, child, parent_id, parent_name) in pending_parents {
        let parent = if let Some(parent_id) = parent_id {
            entity_by_id.get(&parent_id).copied().ok_or(
                EditorApplyError::UnknownParentReference {
                    entity_id: child_id,
                    reference: format!("id {}", parent_id),
                },
            )?
        } else if let Some(parent_name) = parent_name {
            entity_by_name.get(parent_name.as_str()).copied().ok_or(
                EditorApplyError::UnknownParentReference {
                    entity_id: child_id,
                    reference: format!("name '{}'", parent_name),
                },
            )?
        } else {
            return Err(EditorApplyError::MissingField {
                entity_id: child_id,
                component: "Parent".into(),
                field: "entity_id",
            });
        };

        world.set_parent(child, parent);
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

fn optional_text(component: &InspectorComponent, field: &str) -> Option<String> {
    match component.get_field(field)? {
        InspectorValue::Text(v) => Some(v.clone()),
        _ => None,
    }
}

fn has_any_field(component: &InspectorComponent, fields: &[&str]) -> bool {
    fields
        .iter()
        .any(|field| component.get_field(field).is_some())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::parse_scene;
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
    fn scene_document_hsc_roundtrip_supports_editor_mutation() {
        let source = r#"
scene "Workflow"

entity "player" {
  transform { pos 0 1 0 rot 0 0 0 1 scale 1 1 1 }
  stats { health 100 mana 25 attack 8 defense 4 speed 3 }
}

entity "camera" {
  transform { pos 0 3 8 rot 0 0 0 1 scale 1 1 1 }
  camera { fov 60 near 0.1 far 1000 }
  active_camera
}
"#;

        let mut doc = SceneDocument::from_hsc_str(source).unwrap();
        assert_eq!(doc.name, "Workflow");
        assert_eq!(
            doc.get_value(1, "Stats", "health"),
            Some(&InspectorValue::Number(100.0))
        );

        doc.set_value(1, "Stats", "health", InspectorValue::Number(135.0));
        doc.set_value(
            2,
            "Transform",
            "position",
            InspectorValue::Vec3([4.0, 6.0, 12.0]),
        );

        let exported = doc.to_hsc_string().unwrap();
        let scene = parse_scene(&exported).unwrap();

        assert_eq!(scene.name, "Workflow");
        assert_eq!(scene.entities.len(), 2);
        assert_eq!(scene.entities[0].stats.as_ref().unwrap().health, 135.0);
        assert_eq!(
            scene.entities[1].transform.as_ref().unwrap().position,
            Vec3::new(4.0, 6.0, 12.0)
        );
    }

    #[test]
    fn scene_bridge_roundtrip_preserves_runtime_scene_data() {
        let scene = SceneData {
            name: "Bridge".into(),
            entities: vec![
                EntityDef {
                    name: "player".into(),
                    transform: Some(TransformDef {
                        position: Vec3::new(1.0, 2.0, 3.0),
                        rotation: Quat::new(0.0, 0.70710677, 0.0, 0.70710677),
                        scale: Vec3::new(1.0, 1.5, 1.0),
                    }),
                    velocity: Some(VelocityDef {
                        linear: Vec3::new(0.0, 0.5, 0.0),
                        angular: Vec3::new(0.0, 1.25, 0.0),
                    }),
                    rigid_body: Some(RigidBodyDef {
                        gravity_scale: 1.0,
                        linear_damping: 0.2,
                        angular_damping: 0.1,
                        restitution: 0.6,
                    }),
                    sphere_collider: Some(SphereColliderDef { radius: 0.75 }),
                    particle_emitter: None,
                    camera: Some(CameraDef {
                        fov: 75.0,
                        near: 0.2,
                        far: 500.0,
                    }),
                    active_camera: true,
                    light: None,
                    stats: Some(StatsDef {
                        health: 100.0,
                        mana: 50.0,
                        attack: 12.0,
                        defense: 6.0,
                        speed: 4.0,
                    }),
                    parent_name: None,
                },
                EntityDef {
                    name: "sun".into(),
                    transform: None,
                    velocity: None,
                    rigid_body: None,
                    sphere_collider: None,
                    particle_emitter: None,
                    camera: None,
                    active_camera: false,
                    light: Some(LightDef {
                        direction: Vec3::new(0.4, -1.0, 0.2),
                        color: Color::new(1.0, 0.9, 0.8, 0.75),
                        intensity: 1.5,
                        ambient: 0.2,
                    }),
                    stats: None,
                    parent_name: None,
                },
                EntityDef {
                    name: "crate".into(),
                    transform: Some(TransformDef {
                        position: Vec3::new(5.0, 0.0, -3.0),
                        rotation: Quat::IDENTITY,
                        scale: Vec3::ONE,
                    }),
                    velocity: None,
                    rigid_body: None,
                    sphere_collider: None,
                    particle_emitter: Some(ParticleEmitterDef {
                        emitting: true,
                        rate_per_second: 20.0,
                        max_particles: 64,
                        spawn_radius: 0.2,
                        direction: Vec3::Y,
                        spread: 0.3,
                        speed_min: 1.5,
                        speed_max: 2.5,
                        lifetime_min: 0.4,
                        lifetime_max: 0.9,
                        gravity_scale: 0.7,
                        linear_damping: 1.1,
                        start_size: 0.15,
                        end_size: 0.03,
                        start_color: Color::new(1.0, 0.8, 0.2, 1.0),
                        end_color: Color::new(1.0, 0.1, 0.05, 0.0),
                        seed: 7,
                    }),
                    camera: None,
                    active_camera: false,
                    light: None,
                    stats: None,
                    parent_name: Some("player".into()),
                },
            ],
        };

        let doc = SceneDocument::from_scene_data(&scene);
        let roundtrip = doc.to_scene_data().unwrap();

        assert_eq!(roundtrip.name, scene.name);
        assert_eq!(roundtrip.entities.len(), 3);

        let player = &roundtrip.entities[0];
        assert_eq!(player.name, "player");
        assert!(player.active_camera);
        let player_transform = player.transform.as_ref().unwrap();
        assert_eq!(player_transform.position, Vec3::new(1.0, 2.0, 3.0));
        assert!((player_transform.rotation.y - 0.70710677).abs() < 1e-5);
        assert_eq!(player.velocity.as_ref().unwrap().angular.y, 1.25);
        assert_eq!(player.sphere_collider.as_ref().unwrap().radius, 0.75);
        assert_eq!(player.stats.as_ref().unwrap().attack, 12.0);

        let sun = &roundtrip.entities[1];
        let light = sun.light.as_ref().unwrap();
        assert!((light.color.a - 0.75).abs() < 1e-5);
        assert!((light.intensity - 1.5).abs() < 1e-5);

        let crate_entity = &roundtrip.entities[2];
        assert_eq!(crate_entity.parent_name.as_deref(), Some("player"));
        assert_eq!(
            crate_entity
                .particle_emitter
                .as_ref()
                .unwrap()
                .max_particles,
            64
        );
    }

    #[test]
    fn scene_document_to_scene_data_deduplicates_names_and_resolves_parent_ids() {
        let doc = SceneDocument {
            name: "Deduped".into(),
            entities: vec![
                EditorEntity {
                    id: 1,
                    name: "Entity".into(),
                    components: vec![InspectorComponent {
                        type_name: "Transform".into(),
                        fields: vec![InspectorField {
                            name: "position".into(),
                            value: InspectorValue::Vec3([0.0, 0.0, 0.0]),
                        }],
                    }],
                },
                EditorEntity {
                    id: 2,
                    name: "Entity".into(),
                    components: vec![InspectorComponent {
                        type_name: "Parent".into(),
                        fields: vec![InspectorField {
                            name: "entity_id".into(),
                            value: InspectorValue::Number(1.0),
                        }],
                    }],
                },
            ],
        };

        let scene = doc.to_scene_data().unwrap();

        assert_eq!(scene.entities[0].name, "Entity");
        assert_eq!(scene.entities[1].name, "Entity 2");
        assert_eq!(scene.entities[1].parent_name.as_deref(), Some("Entity"));
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
    fn scene_document_hsc_file_io_roundtrip() {
        let root = unique_test_dir("scene_hsc");
        fs::create_dir_all(&root).unwrap();
        let scene_path = format!("{root}/level01.hsc");

        let doc = SceneDocument {
            name: "Level01".into(),
            entities: vec![EditorEntity {
                id: 1,
                name: "Sun".into(),
                components: vec![InspectorComponent {
                    type_name: "DirectionalLight".into(),
                    fields: vec![
                        InspectorField {
                            name: "direction".into(),
                            value: InspectorValue::Vec3([0.3, -1.0, 0.2]),
                        },
                        InspectorField {
                            name: "color".into(),
                            value: InspectorValue::Vec3([1.0, 0.95, 0.8]),
                        },
                        InspectorField {
                            name: "color_alpha".into(),
                            value: InspectorValue::Number(1.0),
                        },
                        InspectorField {
                            name: "intensity".into(),
                            value: InspectorValue::Number(1.0),
                        },
                        InspectorField {
                            name: "ambient".into(),
                            value: InspectorValue::Number(0.15),
                        },
                    ],
                }],
            }],
        };

        save_scene_doc_to_hsc(&scene_path, &doc).unwrap();
        let loaded = load_scene_doc_from_hsc(&scene_path).unwrap();
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
        assert!(world.has_component::<GlobalTransform>(entity));

        let stats = world.get::<Stats>(entity).unwrap();
        assert_eq!(stats.health, 120.0);
    }

    #[test]
    fn apply_scene_document_to_world_supports_scene_bridge_components() {
        let scene = SceneData {
            name: "ApplyBridge".into(),
            entities: vec![
                EntityDef {
                    name: "camera_root".into(),
                    transform: Some(TransformDef {
                        position: Vec3::new(0.0, 3.0, 8.0),
                        rotation: Quat::new(0.0, 0.0, 0.0, 1.0),
                        scale: Vec3::ONE,
                    }),
                    velocity: None,
                    rigid_body: None,
                    sphere_collider: None,
                    particle_emitter: None,
                    camera: Some(CameraDef {
                        fov: 70.0,
                        near: 0.1,
                        far: 250.0,
                    }),
                    active_camera: true,
                    light: None,
                    stats: None,
                    parent_name: None,
                },
                EntityDef {
                    name: "child".into(),
                    transform: Some(TransformDef {
                        position: Vec3::new(1.0, 0.0, 0.0),
                        rotation: Quat::new(0.0, 0.38268343, 0.0, 0.9238795),
                        scale: Vec3::new(2.0, 2.0, 2.0),
                    }),
                    velocity: Some(VelocityDef {
                        linear: Vec3::new(0.0, 0.0, 1.0),
                        angular: Vec3::new(0.0, 1.0, 0.0),
                    }),
                    rigid_body: Some(RigidBodyDef {
                        gravity_scale: 1.0,
                        linear_damping: 0.15,
                        angular_damping: 0.05,
                        restitution: 0.4,
                    }),
                    sphere_collider: Some(SphereColliderDef { radius: 0.5 }),
                    particle_emitter: Some(ParticleEmitterDef {
                        emitting: true,
                        rate_per_second: 12.0,
                        max_particles: 48,
                        spawn_radius: 0.1,
                        direction: Vec3::new(0.0, 1.0, 0.0),
                        spread: 0.2,
                        speed_min: 1.0,
                        speed_max: 2.0,
                        lifetime_min: 0.3,
                        lifetime_max: 0.8,
                        gravity_scale: 0.6,
                        linear_damping: 1.0,
                        start_size: 0.1,
                        end_size: 0.02,
                        start_color: Color::new(0.8, 0.9, 1.0, 1.0),
                        end_color: Color::new(0.3, 0.4, 1.0, 0.0),
                        seed: 11,
                    }),
                    camera: None,
                    active_camera: false,
                    light: Some(LightDef {
                        direction: Vec3::new(0.0, -1.0, 0.0),
                        color: Color::new(0.5, 0.6, 0.7, 0.8),
                        intensity: 2.0,
                        ambient: 0.25,
                    }),
                    stats: Some(StatsDef {
                        health: 80.0,
                        mana: 10.0,
                        attack: 5.0,
                        defense: 2.0,
                        speed: 1.0,
                    }),
                    parent_name: Some("camera_root".into()),
                },
            ],
        };

        let doc = SceneDocument::from_scene_data(&scene);
        let mut world = World::new();
        let map = apply_scene_document_to_world(&mut world, &doc).unwrap();
        let id_to_entity: HashMap<u64, Entity> = map.into_iter().collect();

        let root = id_to_entity[&1];
        assert!(world.has_component::<GlobalTransform>(root));
        assert!(world.has_component::<Camera>(root));
        assert!(world.has_component::<ActiveCamera>(root));

        let child = id_to_entity[&2];
        let transform = world.get::<Transform>(child).unwrap();
        assert!((transform.rotation.y - 0.38268343).abs() < 1e-5);
        assert!(world.has_component::<GlobalTransform>(child));
        assert!(world.has_component::<Velocity>(child));
        assert!(world.has_component::<RigidBody>(child));
        assert!(world.has_component::<SphereCollider>(child));
        assert!(world.has_component::<ParticleEmitter>(child));
        assert!(world.has_component::<DirectionalLight>(child));
        assert!(world.has_component::<Stats>(child));

        let parent = world.get::<crate::Parent>(child).unwrap();
        assert_eq!(parent.entity, root);
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
        let map = load_and_apply_scene_from_project(project.project_file(), scene_file, &mut world)
            .unwrap();
        assert_eq!(map.len(), 1);
        let entity = map[0].1;
        assert!(world.has_component::<Transform>(entity));
        assert!(world.has_component::<Camera>(entity));
    }

    #[test]
    fn load_and_apply_scene_from_hsc_pipeline() {
        let root = unique_test_dir("pipeline_hsc");
        fs::create_dir_all(&root).unwrap();
        let scene_path = format!("{root}/level01.hsc");

        let source = r#"
scene "Pipeline"

entity "camera" {
  transform { pos 0 2 10 rot 0 0 0 1 scale 1 1 1 }
  camera { fov 75 near 0.5 far 300 }
  active_camera
}

entity "sun" {
  light { dir 0.4 -1 0.3 color 1 1 1 1 intensity 1.2 ambient 0.2 }
}
"#;

        fs::write(&scene_path, source).unwrap();

        let mut world = World::new();
        let map = load_and_apply_scene_from_hsc(&scene_path, &mut world).unwrap();
        assert_eq!(map.len(), 2);

        let id_to_entity: HashMap<u64, Entity> = map.into_iter().collect();
        let camera = id_to_entity[&1];
        assert!(world.has_component::<Transform>(camera));
        assert!(world.has_component::<Camera>(camera));
        assert!(world.has_component::<ActiveCamera>(camera));

        let sun = id_to_entity[&2];
        assert!(world.has_component::<DirectionalLight>(sun));
    }
}
