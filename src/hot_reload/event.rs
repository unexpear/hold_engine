//! File-system change events emitted by the hot-reload [`super::FileWatcher`].
//!
//! These are *post-coalescing* events: every event in this module
//! represents a settled file-system state change rather than a raw
//! `ReadDirectoryChangesW` notification. Multiple raw notifications
//! that arrive within the coalescing window collapse into one
//! [`FileChange`].
//!
//! See ADR 0004 Q4 for the coalescing rationale.

use std::path::PathBuf;

/// Kind of file-system change that survived the coalescer.
///
/// Maps onto a subset of `FILE_ACTION_*` values from
/// `ReadDirectoryChangesW`, normalised so downstream consumers don't
/// have to think in Win32 terms.
///
/// Renames are reported as a `Renamed` pair rather than as separate
/// `Removed` + `Created` events so the dispatcher can preserve the
/// asset's logical-name ↔ handle mapping when only the filename
/// changed. The OS may still report a true delete + recreate as
/// separate events; the coalescer collapses those into a single
/// settled `Modified` when the path is the same.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileChangeKind {
    /// File appeared on disk (no prior state, or prior state was
    /// `Removed`).
    Created,
    /// File's content was modified in place (or content + metadata,
    /// after the coalescer collapsed any temp-write-rename burst).
    Modified,
    /// File no longer exists on disk.
    Removed,
    /// File was renamed from `old_path` to the path on the event.
    /// Both paths are canonicalised at the watcher boundary so the
    /// reload pipeline can update its source-path → logical-name
    /// reverse index without further normalisation.
    Renamed { old_path: PathBuf },
}

/// A single coalesced file-system change. Path is absolute and
/// canonicalised (lowercased on Windows, normalised separators) at
/// the watcher boundary so downstream comparators don't need their
/// own canonicalisation step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileChange {
    /// Canonicalised absolute path to the file.
    pub path: PathBuf,
    pub kind: FileChangeKind,
}

impl FileChange {
    pub fn new(path: PathBuf, kind: FileChangeKind) -> Self {
        Self { path, kind }
    }

    pub fn created(path: PathBuf) -> Self {
        Self::new(path, FileChangeKind::Created)
    }

    pub fn modified(path: PathBuf) -> Self {
        Self::new(path, FileChangeKind::Modified)
    }

    pub fn removed(path: PathBuf) -> Self {
        Self::new(path, FileChangeKind::Removed)
    }

    pub fn renamed(old_path: PathBuf, new_path: PathBuf) -> Self {
        Self::new(new_path, FileChangeKind::Renamed { old_path })
    }
}
