//! Editor-side data model for projects, scenes, and inspector editing.
//! This module is runtime-agnostic and keeps a dynamic editing layer on top
//! of typed ECS/runtime data.

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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_layout_paths() {
        let p = Project::new("demo", "C:/games/demo");
        assert_eq!(p.assets_dir, "C:/games/demo/assets");
        assert_eq!(p.scenes_dir, "C:/games/demo/scenes");
        assert_eq!(p.project_file(), "C:/games/demo/demo.hproj");
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
}
