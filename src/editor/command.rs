//! Editor command/event surface, the authoritative shell and bridge contract.
//!
//! The shell (or any future bridge) sends [`EditorCommand`] values to
//! [`super::host::EditorRuntimeHost::execute`] and receives [`EditorEvent`]
//! payloads in return. All types use editor entity IDs, component names,
//! field names, and primitive values. No ECS handles, renderer objects, or
//! platform types are exposed through this surface.

use crate::math::Vec3;

use super::{EditorPlayState, EditorSessionError, InspectorValue};

/// Which component of a [`Vec3`][InspectorValue::Vec3] field is being edited.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorAxis {
    X,
    Y,
    Z,
}

impl std::fmt::Display for EditorAxis {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::X => f.write_str("X"),
            Self::Y => f.write_str("Y"),
            Self::Z => f.write_str("Z"),
        }
    }
}

/// Identifies the currently inspectable item in the editor.
///
/// Uses stable, user-visible identifiers so the shell, tests, and future
/// browser bridge can all refer to the same item without holding ECS handles.
#[derive(Debug, Clone, PartialEq)]
pub struct EditorInspectorCursor {
    pub entity_id: u64,
    pub component_name: String,
    pub field_name: String,
    pub axis: Option<EditorAxis>,
}

/// Commands sent to [`EditorRuntimeHost::execute`] by the shell or any bridge.
#[derive(Debug, Clone)]
pub enum EditorCommand {
    SelectNextEntity,
    SelectPrevEntity,
    SelectEntity {
        entity_id: u64,
    },
    SelectNextComponent,
    SelectPrevComponent,
    SelectComponent {
        entity_id: u64,
        component_name: String,
    },
    SelectNextField,
    SelectPrevField,
    SelectField {
        entity_id: u64,
        component_name: String,
        field_name: String,
    },
    SetAxis(EditorAxis),
    ApplyDelta(f32),
    EnterPlayMode,
    PausePlayMode,
    ResumePlayMode,
    StopPlayMode,
    Save,
    Undo,
    Redo,
    ReloadSource,
    ForcePatch,
    ForceRebuild,
    /// Load the autosave recovery sidecar into the live scene, replacing the
    /// document loaded from the on-disk source. Recovery is always explicit:
    /// silently loading it would overwrite whatever the user just opened.
    /// Result is dirty — the recovered work is still unsaved (M11).
    RecoverAutosave,
    /// Delete the recovery sidecar and keep the on-disk source as loaded. The
    /// only thing lost is the unsaved delta the user chose to discard (M11).
    DiscardAutosave,
    SetTerrainSettings {
        size_meters: Vec3,
        chunk_size_meters: u32,
        height_scale_meters: f32,
    },
    UpsertTerrainPaintLayer {
        name: String,
        material: Option<String>,
        tiling: f32,
        weightmap_channel: u8,
    },
    RemoveTerrainPaintLayer {
        name: String,
    },
    PaintVegetation {
        prototype: String,
        center: Vec3,
        radius: f32,
        density: f32,
    },
    EraseVegetation {
        prototype: Option<String>,
        center: Vec3,
        radius: f32,
    },
    UpsertPatrolRoute {
        name: String,
        points: Vec<Vec3>,
        looped: bool,
    },
    RemovePatrolRoute {
        name: String,
    },
    UpsertVehicleSpawner {
        name: String,
        mesh: Option<String>,
        material: Option<String>,
        position: Vec3,
        facing_yaw_deg: f32,
    },
    RemoveVehicleSpawner {
        name: String,
    },
}

/// Events emitted by [`EditorRuntimeHost::execute`].
#[derive(Debug, Clone)]
pub enum EditorEvent {
    SelectionChanged(EditorInspectorCursor),
    FieldEdited {
        cursor: EditorInspectorCursor,
        new_value: InspectorValue,
    },
    PlayModeChanged(EditorPlayState),
    PatchApplied,
    PatchRequiresRebuild {
        reason: String,
    },
    WorldRebuilt,
    Saved,
    UndoApplied,
    RedoApplied,
    Reloaded,
    RuntimeSliceUpdated {
        path: String,
    },
    /// An asset was hot-reloaded into the live AssetStore. Carries
    /// the asset kind label, the slot index of the underlying
    /// handle, and the logical name. Lets a remote editor client
    /// (browser or IDE plugin) refresh the asset preview and
    /// invalidate any client-side caches without needing access to
    /// the runtime AssetStore. See ADR 0004 Q12 + Q13.
    AssetInvalidated {
        kind: String,
        handle_index: u32,
        logical_name: String,
    },
    /// A compile attempt failed during hot-reload. The dispatcher
    /// keeps the previous asset live; the editor displays this in
    /// the Hot-Reload Status pane (M7.4.1+) so the developer can
    /// see compile errors without alt-tabbing to a terminal.
    CompileFailed {
        kind: String,
        logical_name: String,
        error: String,
    },
    /// Unsaved work from a previous session was recovered from its autosave
    /// sidecar and installed into the live scene. The scene is now dirty
    /// relative to the on-disk source (M11).
    AutosaveRecovered { path: String },
    /// The user declined recovery; the sidecar was deleted and the on-disk
    /// source is authoritative (M11).
    AutosaveDiscarded,
    /// The frame graph produced a new per-pass thumbnail capture
    /// (M9 — ADR 0002 Q8, ADR 0003 Pillar 4). Carries pass metadata
    /// only; the RGBA pixel data stays on the native side. Bridge
    /// clients treat `generation` as a monotonic cache key so a
    /// browser-side coalescer can skip stale captures when it falls
    /// behind.
    FrameCaptured {
        generation: u64,
        passes: Vec<super::FrameInspectorPass>,
    },
}

/// Errors that [`EditorRuntimeHost::execute`] can return.
#[derive(Debug)]
pub enum EditorCommandError {
    UnsupportedFieldKind {
        field: String,
    },
    UnsupportedCommandState {
        command: &'static str,
        state: EditorPlayState,
    },
    Session(EditorSessionError),
    NoSelection,
    NoField,
    HistoryEmpty {
        command: &'static str,
    },
    MissingSourcePath,
    MissingRuntimeSliceItem {
        kind: &'static str,
        name: String,
    },
    InvalidRuntimeSliceEdit {
        reason: String,
    },
}

impl std::fmt::Display for EditorCommandError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedFieldKind { field } => {
                write!(
                    f,
                    "field '{field}' is not editable (only Number and Vec3 are supported)"
                )
            }
            Self::UnsupportedCommandState { command, state } => {
                write!(f, "command '{command}' is not valid in {:?} state", state)
            }
            Self::Session(err) => write!(f, "session error: {err}"),
            Self::NoSelection => write!(f, "no entity is selected"),
            Self::NoField => write!(f, "cursor does not resolve to a valid field"),
            Self::HistoryEmpty { command } => {
                write!(f, "command '{command}' has no history entry to restore")
            }
            Self::MissingSourcePath => {
                write!(
                    f,
                    "session has no source path; use save_hsc() with an explicit path"
                )
            }
            Self::MissingRuntimeSliceItem { kind, name } => {
                write!(f, "runtime-slice {kind} '{name}' does not exist")
            }
            Self::InvalidRuntimeSliceEdit { reason } => write!(f, "{reason}"),
        }
    }
}

impl std::error::Error for EditorCommandError {}
