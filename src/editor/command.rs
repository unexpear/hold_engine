//! Editor command/event surface — the authoritative shell/bridge entrypoint contract.
//!
//! The shell (or any future bridge) sends [`EditorCommand`] values to
//! [`super::host::EditorRuntimeHost::execute`] and receives [`EditorEvent`]
//! payloads in return.  All types use editor entity IDs, component names,
//! field names, and primitive values — no ECS [`crate::ecs::Entity`] handles,
//! renderer objects, or platform types are exposed through this surface.

use super::{EditorPlayState, EditorSessionError, InspectorValue};

// ── Axis ─────────────────────────────────────────────────────────────────────

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

// ── Cursor ────────────────────────────────────────────────────────────────────

/// Identifies the currently inspectable item in the editor.
///
/// Uses stable, user-visible identifiers (entity ID, component name, field
/// name) so the shell, tests, and future browser bridge can all refer to the
/// same item without holding ECS handles.
///
/// `axis` is `Some` only when the current field is a [`Vec3`][InspectorValue::Vec3]
/// and the user has selected a specific component to increment.
#[derive(Debug, Clone, PartialEq)]
pub struct EditorInspectorCursor {
    pub entity_id: u64,
    pub component_name: String,
    pub field_name: String,
    pub axis: Option<EditorAxis>,
}

// ── Commands ──────────────────────────────────────────────────────────────────

/// Commands sent to [`super::host::EditorRuntimeHost::execute`] by the shell or any bridge.
///
/// Commands map 1:1 to key bindings (documented in comments) but are
/// intentionally decoupled from the input layer so tests can drive the host
/// programmatically.
#[derive(Debug, Clone)]
pub enum EditorCommand {
    /// Advance entity selection by one (Tab).
    SelectNextEntity,
    /// Retreat entity selection by one (Shift+Tab).
    SelectPrevEntity,
    /// Advance component selection within the current entity ([).
    SelectNextComponent,
    /// Retreat component selection within the current entity (]).
    SelectPrevComponent,
    /// Advance field selection within the current component (;).
    SelectNextField,
    /// Retreat field selection within the current component (').
    SelectPrevField,
    /// Set the active Vec3 axis (keys 1/2/3). No-op for non-Vec3 fields.
    SetAxis(EditorAxis),
    /// Apply a numeric delta to the current field (-/=).
    ///
    /// For [`Number`][InspectorValue::Number] fields the delta is added directly.
    /// For [`Vec3`][InspectorValue::Vec3] fields it is added to the active [`EditorAxis`]
    /// component (defaulting to X when no axis is set).
    /// Shift multiplies the step by 10 at the call site before passing it here.
    ApplyDelta(f32),
    /// Enter play mode from Editing (F8 when Editing).
    EnterPlayMode,
    /// Pause play mode (F9 when Playing).
    PausePlayMode,
    /// Resume from Paused back to Playing (F9 when Paused).
    ResumePlayMode,
    /// Stop play mode and restore the authored snapshot (F8 when Playing/Paused).
    StopPlayMode,
    /// Save the current scene to the source `.hsc` file (Ctrl+S).
    Save,
    /// Reload the source `.hsc` file from disk and rebuild the runtime world (F7).
    ReloadSource,
    /// Attempt an incremental world patch without rebuilding (Ctrl+P).
    ForcePatch,
    /// Force a deterministic world rebuild from the scene document (Ctrl+R).
    ForceRebuild,
}

// ── Events ────────────────────────────────────────────────────────────────────

/// Events emitted by [`super::host::EditorRuntimeHost::execute`].
///
/// A single command can emit multiple events (e.g. `ApplyDelta` emits
/// [`FieldEdited`][Self::FieldEdited] plus either [`PatchApplied`][Self::PatchApplied]
/// or [`PatchRequiresRebuild`][Self::PatchRequiresRebuild]).
#[derive(Debug, Clone)]
pub enum EditorEvent {
    /// The inspection cursor moved to a new entity/component/field.
    SelectionChanged(EditorInspectorCursor),
    /// A field value was changed by [`EditorCommand::ApplyDelta`].
    FieldEdited {
        cursor: EditorInspectorCursor,
        new_value: InspectorValue,
    },
    /// The play-mode state changed.
    PlayModeChanged(EditorPlayState),
    /// An incremental patch was applied to the live world without an entity remap.
    PatchApplied,
    /// The incremental patch detected a structural change — an explicit
    /// [`EditorCommand::ForceRebuild`] is required before the world is in sync.
    PatchRequiresRebuild { reason: String },
    /// The world was fully rebuilt from the scene document (entity handles remapped).
    WorldRebuilt,
    /// The scene was serialised to disk.
    Saved,
    /// The scene was reloaded from the source `.hsc` file on disk.
    Reloaded,
}

// ── Errors ────────────────────────────────────────────────────────────────────

/// Errors that [`super::host::EditorRuntimeHost::execute`] can return.
///
/// Note that patch-requires-rebuild is an *event*, not an error — it is
/// expected behaviour when field mutations cause structural changes.
#[derive(Debug)]
pub enum EditorCommandError {
    /// The current field type cannot be edited in this milestone (e.g. Text, Bool).
    UnsupportedFieldKind { field: String },
    /// The command is not valid in the current play state.
    UnsupportedCommandState {
        command: &'static str,
        state: EditorPlayState,
    },
    /// The underlying session produced an error.
    Session(EditorSessionError),
    /// No entity is currently selected / cursor is uninitialised.
    NoSelection,
    /// The cursor's field path does not resolve to a field in the scene document.
    NoField,
    /// [`EditorCommand::Save`] was issued but the session has no source path.
    MissingSourcePath,
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
            Self::MissingSourcePath => write!(
                f,
                "session has no source path — use save_hsc() with an explicit path"
            ),
        }
    }
}

impl std::error::Error for EditorCommandError {}
