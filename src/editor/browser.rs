use std::cmp::Ordering;
use std::fs;
use std::path::{Component, Path, PathBuf};

use super::{host::EditorRuntimeHost, session::EditorSceneSession, Project};

/// Top-level browser roots derived from project metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorBrowserRootKind {
    Scenes,
    Assets,
}

impl EditorBrowserRootKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Scenes => "Scenes",
            Self::Assets => "Assets",
        }
    }

    fn configured_path<'a>(self, project: &'a Project) -> &'a str {
        match self {
            Self::Scenes => &project.scenes_dir,
            Self::Assets => &project.assets_dir,
        }
    }
}

/// Read-only entry kind for the browser tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorBrowserEntryKind {
    Directory,
    File,
}

/// Flat browser row suitable for a future native panel tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorBrowserEntry {
    pub root: EditorBrowserRootKind,
    pub kind: EditorBrowserEntryKind,
    pub name: String,
    pub project_path: String,
    pub depth: usize,
}

/// Read-only browser snapshot for one project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorBrowserModel {
    pub project_name: String,
    pub selected_path: Option<String>,
    pub entries: Vec<EditorBrowserEntry>,
}

impl EditorBrowserModel {
    pub fn from_project(project: &Project) -> Result<Self, EditorBrowserError> {
        Ok(Self {
            project_name: project.name.clone(),
            selected_path: None,
            entries: collect_project_entries(project)?,
        })
    }

    pub fn from_session(session: &EditorSceneSession) -> Result<Option<Self>, EditorBrowserError> {
        let project = match session.project() {
            Some(project) => project,
            None => return Ok(None),
        };

        let mut model = Self::from_project(project)?;
        model.selected_path = session
            .source_path()
            .and_then(|path| project_relative_path(Path::new(&project.root), path).ok())
            .filter(|path| {
                model
                    .entries
                    .iter()
                    .any(|entry| entry.project_path == *path)
            });
        Ok(Some(model))
    }

    pub fn from_host(host: &EditorRuntimeHost) -> Result<Option<Self>, EditorBrowserError> {
        Self::from_session(host.session())
    }

    pub fn selected_entry(&self) -> Option<&EditorBrowserEntry> {
        let selected_path = self.selected_path.as_deref()?;
        self.entries
            .iter()
            .find(|entry| entry.project_path == selected_path)
    }
}

#[derive(Debug)]
pub enum EditorBrowserError {
    Io(std::io::Error),
    RootOutsideProject {
        root: EditorBrowserRootKind,
        project_root: String,
        root_path: String,
    },
    RootIsNotDirectory {
        root: EditorBrowserRootKind,
        root_path: String,
    },
}

impl std::fmt::Display for EditorBrowserError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(err) => write!(f, "editor browser io error: {err}"),
            Self::RootOutsideProject {
                root,
                project_root,
                root_path,
            } => write!(
                f,
                "{} root '{}' is outside project root '{}'",
                root.label(),
                root_path,
                project_root
            ),
            Self::RootIsNotDirectory { root, root_path } => {
                write!(
                    f,
                    "{} root '{}' is not a directory",
                    root.label(),
                    root_path
                )
            }
        }
    }
}

impl std::error::Error for EditorBrowserError {}

fn collect_project_entries(
    project: &Project,
) -> Result<Vec<EditorBrowserEntry>, EditorBrowserError> {
    let project_root = Path::new(&project.root);
    let mut entries = Vec::new();

    for root in [EditorBrowserRootKind::Scenes, EditorBrowserRootKind::Assets] {
        let root_path = Path::new(root.configured_path(project));
        let root_project_path = project_relative_path(project_root, root_path).map_err(|_| {
            EditorBrowserError::RootOutsideProject {
                root,
                project_root: project.root.clone(),
                root_path: root.configured_path(project).to_string(),
            }
        })?;

        entries.push(EditorBrowserEntry {
            root,
            kind: EditorBrowserEntryKind::Directory,
            name: entry_name(root_path, &root_project_path),
            project_path: root_project_path,
            depth: 0,
        });

        collect_directory_entries(project_root, root, root_path, 1, &mut entries)?;
    }

    Ok(entries)
}

fn collect_directory_entries(
    project_root: &Path,
    root: EditorBrowserRootKind,
    dir_path: &Path,
    depth: usize,
    entries: &mut Vec<EditorBrowserEntry>,
) -> Result<(), EditorBrowserError> {
    let metadata = match fs::metadata(dir_path) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(EditorBrowserError::Io(err)),
    };

    if !metadata.is_dir() {
        return Err(EditorBrowserError::RootIsNotDirectory {
            root,
            root_path: dir_path.display().to_string(),
        });
    }

    let mut children = Vec::new();
    for entry in fs::read_dir(dir_path).map_err(EditorBrowserError::Io)? {
        let entry = entry.map_err(EditorBrowserError::Io)?;
        let path = entry.path();
        let kind = if entry.file_type().map_err(EditorBrowserError::Io)?.is_dir() {
            EditorBrowserEntryKind::Directory
        } else {
            EditorBrowserEntryKind::File
        };
        let project_path = project_relative_path(project_root, &path).map_err(|_| {
            EditorBrowserError::RootOutsideProject {
                root,
                project_root: project_root.display().to_string(),
                root_path: path.display().to_string(),
            }
        })?;

        children.push(PendingBrowserEntry {
            kind,
            name: entry.file_name().to_string_lossy().into_owned(),
            project_path,
            path,
        });
    }

    children.sort_by(compare_pending_entries);

    for child in children {
        let is_directory = child.kind == EditorBrowserEntryKind::Directory;
        entries.push(EditorBrowserEntry {
            root,
            kind: child.kind,
            name: child.name,
            project_path: child.project_path,
            depth,
        });

        if is_directory {
            let next_path = child.path;
            collect_directory_entries(project_root, root, &next_path, depth + 1, entries)?;
        }
    }

    Ok(())
}

fn compare_pending_entries(left: &PendingBrowserEntry, right: &PendingBrowserEntry) -> Ordering {
    match (left.kind, right.kind) {
        (EditorBrowserEntryKind::Directory, EditorBrowserEntryKind::File) => Ordering::Less,
        (EditorBrowserEntryKind::File, EditorBrowserEntryKind::Directory) => Ordering::Greater,
        _ => left
            .name
            .to_lowercase()
            .cmp(&right.name.to_lowercase())
            .then_with(|| left.project_path.cmp(&right.project_path)),
    }
}

fn project_relative_path(project_root: &Path, path: &Path) -> Result<String, ()> {
    let relative = path.strip_prefix(project_root).map_err(|_| ())?;
    Ok(normalize_relative_path(relative))
}

fn normalize_relative_path(path: &Path) -> String {
    path.components()
        .filter_map(|component| match component {
            Component::CurDir => None,
            Component::Normal(part) => Some(part.to_string_lossy().into_owned()),
            Component::ParentDir => Some("..".to_string()),
            Component::RootDir | Component::Prefix(_) => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn entry_name(path: &Path, project_path: &str) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| {
            Path::new(project_path)
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| project_path.to_string())
        })
}

struct PendingBrowserEntry {
    kind: EditorBrowserEntryKind,
    name: String,
    project_path: String,
    path: PathBuf,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::SceneDocument;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn nanos() -> u128 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    }

    fn tmp_dir(label: &str) -> String {
        format!(
            "{}/hold_browser_model_{}_{}",
            std::env::temp_dir().display(),
            label,
            nanos()
        )
    }

    fn create_file(path: &str) {
        if let Some(parent) = Path::new(path).parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, "test").unwrap();
    }

    fn create_hsc_scene(path: &str) {
        if let Some(parent) = Path::new(path).parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(
            path,
            r#"
scene "Browser"

entity "camera" {
  transform { pos 0 2 10 rot 0 0 0 1 scale 1 1 1 }
  camera { fov 60 near 0.1 far 1000 }
  active_camera
}
"#,
        )
        .unwrap();
    }

    #[test]
    fn browser_model_builds_sorted_project_relative_rows() {
        let root = tmp_dir("rows");
        let project = Project::new("demo", root.clone());

        fs::create_dir_all(format!("{}/campaign", project.scenes_dir)).unwrap();
        fs::create_dir_all(format!("{}/textures", project.assets_dir)).unwrap();
        create_file(&format!("{}/level02.hscene", project.scenes_dir));
        create_file(&format!("{}/campaign/intro.hsc", project.scenes_dir));
        create_file(&format!("{}/textures/brick.png", project.assets_dir));
        create_file(&format!("{}/ambient.wav", project.assets_dir));

        let model = EditorBrowserModel::from_project(&project).unwrap();
        let rows = model
            .entries
            .iter()
            .map(|entry| {
                (
                    entry.root,
                    entry.kind,
                    entry.project_path.as_str(),
                    entry.depth,
                )
            })
            .collect::<Vec<_>>();

        assert_eq!(
            rows,
            vec![
                (
                    EditorBrowserRootKind::Scenes,
                    EditorBrowserEntryKind::Directory,
                    "scenes",
                    0,
                ),
                (
                    EditorBrowserRootKind::Scenes,
                    EditorBrowserEntryKind::Directory,
                    "scenes/campaign",
                    1,
                ),
                (
                    EditorBrowserRootKind::Scenes,
                    EditorBrowserEntryKind::File,
                    "scenes/campaign/intro.hsc",
                    2,
                ),
                (
                    EditorBrowserRootKind::Scenes,
                    EditorBrowserEntryKind::File,
                    "scenes/level02.hscene",
                    1,
                ),
                (
                    EditorBrowserRootKind::Assets,
                    EditorBrowserEntryKind::Directory,
                    "assets",
                    0,
                ),
                (
                    EditorBrowserRootKind::Assets,
                    EditorBrowserEntryKind::Directory,
                    "assets/textures",
                    1,
                ),
                (
                    EditorBrowserRootKind::Assets,
                    EditorBrowserEntryKind::File,
                    "assets/textures/brick.png",
                    2,
                ),
                (
                    EditorBrowserRootKind::Assets,
                    EditorBrowserEntryKind::File,
                    "assets/ambient.wav",
                    1,
                ),
            ]
        );
    }

    #[test]
    fn browser_model_keeps_missing_roots_as_empty_sections() {
        let root = tmp_dir("missing");
        fs::create_dir_all(&root).unwrap();
        let project = Project::new("demo", root);

        let model = EditorBrowserModel::from_project(&project).unwrap();

        assert_eq!(model.entries.len(), 2);
        assert_eq!(model.entries[0].project_path, "scenes");
        assert_eq!(model.entries[1].project_path, "assets");
    }

    #[test]
    fn browser_model_from_session_selects_current_source_path() {
        let root = tmp_dir("session_selection");
        let project = Project::new("demo", root.clone());
        let scene_path = format!("{}/level01.hsc", project.scenes_dir);
        create_hsc_scene(&scene_path);

        let session =
            EditorSceneSession::open_hsc_with_project(&scene_path, project.clone()).unwrap();
        let model = EditorBrowserModel::from_session(&session).unwrap().unwrap();

        assert_eq!(model.selected_path.as_deref(), Some("scenes/level01.hsc"));
        assert_eq!(
            model
                .selected_entry()
                .map(|entry| entry.project_path.as_str()),
            Some("scenes/level01.hsc")
        );
    }

    #[test]
    fn browser_model_from_host_uses_attached_project_context() {
        let root = tmp_dir("host_selection");
        let project = Project::new("demo", root.clone());
        let scene_path = format!("{}/level01.hsc", project.scenes_dir);
        create_hsc_scene(&scene_path);

        let host = EditorRuntimeHost::open_hsc_with_project(&scene_path, project).unwrap();
        let model = EditorBrowserModel::from_host(&host).unwrap().unwrap();

        assert_eq!(model.selected_path.as_deref(), Some("scenes/level01.hsc"));
    }

    #[test]
    fn browser_model_rejects_roots_outside_project_root() {
        let project = Project {
            name: "demo".to_string(),
            root: tmp_dir("outside_root"),
            assets_dir: tmp_dir("external_assets"),
            scenes_dir: tmp_dir("external_scenes"),
        };

        let err = EditorBrowserModel::from_project(&project).unwrap_err();
        assert!(matches!(
            err,
            EditorBrowserError::RootOutsideProject {
                root: EditorBrowserRootKind::Scenes,
                ..
            }
        ));
    }

    #[test]
    fn browser_model_from_session_returns_none_without_project() {
        let session = EditorSceneSession::new(SceneDocument {
            name: "Empty".to_string(),
            runtime_slice: crate::runtime_slice::RuntimeSliceAuthoring::default(),
            entities: Vec::new(),
        });

        assert!(EditorBrowserModel::from_session(&session)
            .unwrap()
            .is_none());
    }
}
