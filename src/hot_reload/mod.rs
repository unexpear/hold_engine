//! Hot-reload pipeline: file-system change detection, coalescing,
//! compile dispatch, and asset invalidation.
//!
//! This module owns the "watcher → coalescer → compile queue →
//! reload dispatcher → invalidation bus" pipeline described in
//! ADR 0004. Sub-modules:
//!
//! - [`event`]: `FileChange` and `FileChangeKind` — settled
//!   file-system events the rest of the pipeline consumes.
//! - [`path_filter`]: pure path-canonicalisation and extension
//!   allowlist logic.
//! - [`coalescer`]: pure state machine that collapses raw
//!   file-system event bursts into one settled change per path.
//! - [`watcher`]: Win32 `ReadDirectoryChangesW` worker thread that
//!   drives the coalescer and emits coalesced events on an mpsc
//!   `Receiver<FileChange>`.
//!
//! The compile queue, reload dispatcher, and invalidation-bus
//! integration land in M7.2–M7.3. M7.1 ships only the watcher half
//! of the pipeline plus the testable pure-logic modules.
//!
//! See `docs/adr/0004-hot-reload-architecture.md`.

pub mod coalescer;
pub mod compile_queue;
pub mod dispatcher;
#[cfg(test)]
mod equivalence;
pub mod event;
pub mod path_filter;
pub mod watcher;

pub use coalescer::{Coalescer, RawFileEvent};
pub use compile_queue::{
    default_compile_fn, CompileErr, CompileFn, CompileOk, CompileQueue, CompileQueueConfig,
    CompileQueueError, CompileResult, CompileTarget, Generation,
};
pub use dispatcher::{
    DispatchError, DispatchSummary, DroppedResult, ReloadDispatcher, SceneReloadResult,
};
pub use event::{FileChange, FileChangeKind};
pub use path_filter::{canonicalise_for_index, ExtensionFilter};
pub use watcher::{FileWatcher, WatcherConfig, WatcherError};
