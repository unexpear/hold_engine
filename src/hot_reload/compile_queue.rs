//! Compile queue worker that turns coalesced [`super::FileChange`]
//! events into compiled-asset payloads ready for the reload
//! dispatcher (M7.3) to apply.
//!
//! ## Pipeline
//!
//! ```text
//!  Receiver<FileChange>           (from the FileWatcher in M7.1)
//!         │
//!         ▼
//!  reverse_index lookup       (path → which logical names recompile)
//!         │
//!         ▼
//!  per-asset generation bump  (last-wins; stale results filtered out)
//!         │
//!         ▼
//!  compile_asset_from_source  (existing assets/compiler.rs API)
//!         │
//!         ▼
//!  Sender<CompileResult>      (consumed by the dispatcher next milestone)
//! ```
//!
//! ## Design discipline
//!
//! - **Reverse index** built explicitly by the queue rather than
//!   borrowed from `CompiledAssetMetadata.dependencies` — the
//!   metadata holds *forward* dependency lists, not the inverse the
//!   queue needs. Confirmed by the codebase audit; see ADR 0004 Q4.
//! - **Generation counters** are per-logical-name and monotonic.
//!   The worker never blocks on a compile; the dispatcher checks
//!   `generation == latest_for(logical_name)` to discard stale
//!   results.
//! - **Compile function is injectable** so the worker can be
//!   integration-tested without temp directories on disk. Production
//!   wires the existing `compile_asset_from_source`; tests wire a
//!   closure that returns canned results.
//! - **Worker thread terminates cleanly** on `Drop` or explicit
//!   `stop`, draining no extra work after the stop signal.
//!
//! See ADR 0004 Q4 + Q9.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::assets::{
    compile_asset_from_source, AssetCompilerError, CompiledAssetBlob, CompiledAssetKind,
};

use super::event::FileChange;
use super::path_filter::canonicalise_for_index;

// ── Public types ──────────────────────────────────────────────────────────

/// Monotonic per-logical-name compile counter. The dispatcher uses
/// this to filter stale results — when two file changes arrive for
/// the same asset in quick succession and the worker emits two
/// compile results, only the one whose generation matches the
/// counter's latest value at apply time wins.
///
/// Wraps `u64` for clarity; not equality-compared with raw integers
/// at any call site, which makes the type tag a discipline gate
/// (you can't accidentally compare a generation against a slot
/// index, etc.).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Generation(pub u64);

/// One compile target the queue knows about. Registered at queue
/// construction; tells the worker "when *this* path changes,
/// recompile the asset of *this* kind under *this* logical name."
#[derive(Debug, Clone)]
pub struct CompileTarget {
    pub logical_name: String,
    pub kind: CompiledAssetKind,
    /// The primary source file for this asset. When the watcher
    /// reports a change to this path (or any path in
    /// `extra_source_paths`), the worker queues a recompile.
    pub source_path: PathBuf,
    /// Additional source files whose changes should also trigger a
    /// recompile of this asset. Useful when an asset's compilation
    /// reads multiple files (e.g. a material that includes a
    /// shader source). The first time `extra_source_paths` is
    /// touched, the worker still passes `source_path` as the
    /// argument to `compile_asset_from_source`; the queue does not
    /// yet support multi-input compile.
    pub extra_source_paths: Vec<PathBuf>,
}

impl CompileTarget {
    pub fn new(
        logical_name: impl Into<String>,
        kind: CompiledAssetKind,
        source_path: impl Into<PathBuf>,
    ) -> Self {
        Self {
            logical_name: logical_name.into(),
            kind,
            source_path: source_path.into(),
            extra_source_paths: Vec::new(),
        }
    }

    pub fn with_extra_source_paths<I>(mut self, paths: I) -> Self
    where
        I: IntoIterator<Item = PathBuf>,
    {
        self.extra_source_paths.extend(paths);
        self
    }
}

/// Successful compile output. Carries the original logical name,
/// the kind it was compiled as, the resulting `.hcmp` payload, and
/// the generation counter at the moment the compile was scheduled.
///
/// The `generation` field is the load-bearing piece for last-wins
/// dispatcher logic. M7.3's dispatcher will check
/// `generation == queue.latest_generation_for(logical_name)` before
/// applying.
#[derive(Debug, Clone)]
pub struct CompileOk {
    pub logical_name: String,
    pub kind: CompiledAssetKind,
    pub payload: Vec<u8>,
    pub generation: Generation,
}

/// Failed compile. The `error` field carries a stringified version
/// of the underlying `AssetCompilerError` because the latter holds
/// IO error references that aren't `Send + Sync`. The string is
/// formatted at the boundary so it survives the channel hop.
#[derive(Debug, Clone)]
pub struct CompileErr {
    pub logical_name: String,
    pub kind: CompiledAssetKind,
    pub error: String,
    pub generation: Generation,
}

#[derive(Debug, Clone)]
pub enum CompileResult {
    Ok(CompileOk),
    Err(CompileErr),
}

impl CompileResult {
    pub fn logical_name(&self) -> &str {
        match self {
            Self::Ok(ok) => &ok.logical_name,
            Self::Err(err) => &err.logical_name,
        }
    }

    pub fn generation(&self) -> Generation {
        match self {
            Self::Ok(ok) => ok.generation,
            Self::Err(err) => err.generation,
        }
    }
}

/// Pluggable compile function. Production wires
/// [`crate::assets::compile_asset_from_source`]; tests wire a
/// closure that returns canned results without touching the disk.
///
/// `Send + Sync` because the worker thread holds it across thread
/// boundaries; `'static` because the function lives as long as the
/// `CompileQueue` does.
pub type CompileFn = Box<
    dyn Fn(CompiledAssetKind, &Path, &str) -> Result<CompiledAssetBlob, AssetCompilerError>
        + Send
        + Sync
        + 'static,
>;

/// Construct the production compile function (delegates straight to
/// `compile_asset_from_source`).
pub fn default_compile_fn() -> CompileFn {
    Box::new(|kind, source, logical_name| compile_asset_from_source(kind, source, logical_name))
}

/// Configuration the caller passes to [`CompileQueue::start`].
pub struct CompileQueueConfig {
    /// Channel receiving coalesced [`FileChange`] events. Typically
    /// produced by `FileWatcher::events().clone()` — except the
    /// std mpsc receiver is not Cloneable, so the actual integration
    /// either moves the watcher's receiver into the queue or wires
    /// up a small fan-out forwarder. M7.1 + M7.2 demos move the
    /// receiver in; that pattern is the v1 default.
    pub file_changes: Receiver<FileChange>,
    /// Asset compile targets registered with the queue.
    pub targets: Vec<CompileTarget>,
    /// Pluggable compile function. Use [`default_compile_fn`] for
    /// production; pass a closure for testing.
    pub compile_fn: CompileFn,
}

/// Errors surfaced by [`CompileQueue::start`]. Errors during
/// operation are surfaced as `CompileResult::Err` on the result
/// channel rather than as recoverable values.
#[derive(Debug)]
pub enum CompileQueueError {
    /// Two registered targets share a logical name. Logical names
    /// are the queue's primary key for generation counters and
    /// reverse-index lookups; duplicates would silently shadow.
    DuplicateLogicalName(String),
}

impl std::fmt::Display for CompileQueueError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DuplicateLogicalName(name) => write!(
                f,
                "CompileQueue: duplicate logical name {:?}; registration order is the discipline gate",
                name
            ),
        }
    }
}

impl std::error::Error for CompileQueueError {}

/// A running compile queue with its worker thread.
pub struct CompileQueue {
    receiver: Option<Receiver<CompileResult>>,
    stop_flag: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl CompileQueue {
    /// Start the worker with an internal result channel; access
    /// results via [`Self::recv`] / [`Self::try_recv`].
    pub fn start(config: CompileQueueConfig) -> Result<Self, CompileQueueError> {
        let (tx, rx) = channel::<CompileResult>();
        let mut queue = Self::start_with_sender(config, tx)?;
        queue.receiver = Some(rx);
        Ok(queue)
    }

    /// Start the worker with an external result sender. The caller
    /// owns the receiver — typically the [`super::ReloadDispatcher`]
    /// in M7.3+ which consumes [`CompileResult`]s and applies them
    /// to the live `AssetStore`.
    ///
    /// Mirrors [`super::FileWatcher::start_with_sender`]; the two
    /// together let the user assemble the full
    /// watcher → queue → dispatcher chain via two ordinary `mpsc`
    /// channels.
    pub fn start_with_sender(
        config: CompileQueueConfig,
        sender: Sender<CompileResult>,
    ) -> Result<Self, CompileQueueError> {
        // Validate: no duplicate logical names.
        let mut seen: HashMap<String, ()> = HashMap::new();
        for t in &config.targets {
            if seen.insert(t.logical_name.clone(), ()).is_some() {
                return Err(CompileQueueError::DuplicateLogicalName(
                    t.logical_name.clone(),
                ));
            }
        }

        // Build reverse index + generation counters from the targets.
        let mut reverse = ReverseIndex::new();
        let mut generations = GenerationCounter::new();
        for target in &config.targets {
            reverse.register(target);
            generations.ensure(&target.logical_name);
        }

        let stop_flag = Arc::new(AtomicBool::new(false));
        let stop_flag_thread = Arc::clone(&stop_flag);

        let CompileQueueConfig {
            file_changes,
            compile_fn,
            ..
        } = config;

        let thread = thread::spawn(move || {
            worker_loop(
                file_changes,
                reverse,
                generations,
                compile_fn,
                sender,
                stop_flag_thread,
            );
        });

        Ok(Self {
            receiver: None,
            stop_flag,
            thread: Some(thread),
        })
    }

    /// Read-only access to the internal result channel. Returns
    /// `None` if the queue was started with
    /// [`Self::start_with_sender`] — results flow to the caller's
    /// channel in that case.
    pub fn results(&self) -> Option<&Receiver<CompileResult>> {
        self.receiver.as_ref()
    }

    /// Try to read a result without blocking. Returns
    /// `Disconnected` if the queue was started with an external
    /// sender.
    pub fn try_recv(&self) -> Result<CompileResult, std::sync::mpsc::TryRecvError> {
        match self.receiver.as_ref() {
            Some(rx) => rx.try_recv(),
            None => Err(std::sync::mpsc::TryRecvError::Disconnected),
        }
    }

    /// Block until a result arrives or the worker shuts down.
    /// Returns `Err` if started with an external sender.
    pub fn recv(&self) -> Result<CompileResult, std::sync::mpsc::RecvError> {
        match self.receiver.as_ref() {
            Some(rx) => rx.recv(),
            None => Err(std::sync::mpsc::RecvError),
        }
    }

    /// Block with a timeout. Returns `Disconnected` if started with
    /// an external sender.
    pub fn recv_timeout(
        &self,
        timeout: Duration,
    ) -> Result<CompileResult, RecvTimeoutError> {
        match self.receiver.as_ref() {
            Some(rx) => rx.recv_timeout(timeout),
            None => Err(RecvTimeoutError::Disconnected),
        }
    }

    /// Signal the worker thread to stop and join it. Idempotent.
    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        self.stop_flag.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for CompileQueue {
    fn drop(&mut self) {
        self.shutdown();
    }
}

// ── Worker thread ─────────────────────────────────────────────────────────

fn worker_loop(
    file_changes: Receiver<FileChange>,
    reverse: ReverseIndex,
    mut generations: GenerationCounter,
    compile_fn: CompileFn,
    output: Sender<CompileResult>,
    stop_flag: Arc<AtomicBool>,
) {
    // Index targets by logical name for fast lookup of (kind,
    // source_path) when a recompile is scheduled.
    let target_by_name: HashMap<String, (CompiledAssetKind, PathBuf)> = reverse
        .targets_by_name
        .iter()
        .map(|(name, info)| (name.clone(), (info.kind, info.source_path.clone())))
        .collect();

    while !stop_flag.load(Ordering::SeqCst) {
        let change = match file_changes.recv_timeout(Duration::from_millis(100)) {
            Ok(c) => c,
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => break,
        };

        let affected = reverse.lookup(&change.path);
        if affected.is_empty() {
            // No registered target depends on this path; nothing to
            // do. Common case for files outside the asset tree.
            continue;
        }

        for logical_name in affected {
            let (kind, source_path) = match target_by_name.get(&logical_name) {
                Some(v) => v.clone(),
                None => continue, // shouldn't happen but be defensive
            };
            let generation = generations.bump(&logical_name);
            let result = match compile_fn(kind, &source_path, &logical_name) {
                Ok(blob) => CompileResult::Ok(CompileOk {
                    logical_name: logical_name.clone(),
                    kind,
                    payload: blob.payload,
                    generation,
                }),
                Err(err) => CompileResult::Err(CompileErr {
                    logical_name: logical_name.clone(),
                    kind,
                    error: format!("{}", err),
                    generation,
                }),
            };
            if output.send(result).is_err() {
                // Receiver dropped — caller stopped listening.
                return;
            }
        }
    }
}

// ── ReverseIndex (pure logic) ─────────────────────────────────────────────

#[derive(Debug, Clone)]
struct ReverseIndexTargetInfo {
    kind: CompiledAssetKind,
    source_path: PathBuf,
}

/// Maps `canonical_path → Vec<logical_name>`. Built explicitly from
/// the registered targets at queue construction; the asset
/// compiler's per-asset `dependencies` list is *forward* and can't
/// be borrowed for reverse lookup. This struct owns the inversion.
#[derive(Debug, Clone)]
struct ReverseIndex {
    /// Path → list of logical names whose recompile this path
    /// triggers. Multiple logical names can share a path (rare but
    /// possible — multiple assets descending from the same source).
    by_path: HashMap<PathBuf, Vec<String>>,
    /// Lookup-by-name index used by the worker after the reverse
    /// lookup hits.
    targets_by_name: HashMap<String, ReverseIndexTargetInfo>,
}

impl ReverseIndex {
    fn new() -> Self {
        Self {
            by_path: HashMap::new(),
            targets_by_name: HashMap::new(),
        }
    }

    fn register(&mut self, target: &CompileTarget) {
        // Canonicalise every path the target watches so lookup
        // matches regardless of how the watcher reported its case
        // or separator style.
        let primary = canonicalise_for_index(&target.source_path);
        self.by_path
            .entry(primary)
            .or_default()
            .push(target.logical_name.clone());
        for extra in &target.extra_source_paths {
            let canon = canonicalise_for_index(extra);
            self.by_path
                .entry(canon)
                .or_default()
                .push(target.logical_name.clone());
        }
        self.targets_by_name.insert(
            target.logical_name.clone(),
            ReverseIndexTargetInfo {
                kind: target.kind,
                source_path: target.source_path.clone(),
            },
        );
    }

    /// Return logical names whose recompile is triggered by changes
    /// to `path`. The caller's path is canonicalised before lookup,
    /// so case / separator drift between the watcher boundary and
    /// the index doesn't cause silent misses.
    fn lookup(&self, path: &Path) -> Vec<String> {
        let canon = canonicalise_for_index(path);
        self.by_path
            .get(&canon)
            .map(|v| v.clone())
            .unwrap_or_default()
    }
}

// ── GenerationCounter (pure logic) ────────────────────────────────────────

/// Per-logical-name monotonic counter. `bump` increments and
/// returns; `latest` reads without mutating. The worker bumps
/// before scheduling a compile; the dispatcher reads `latest` to
/// decide whether a returned result is fresh.
#[derive(Debug, Clone)]
struct GenerationCounter {
    counts: HashMap<String, u64>,
}

impl GenerationCounter {
    fn new() -> Self {
        Self {
            counts: HashMap::new(),
        }
    }

    /// Ensure a counter exists for `logical_name`, defaulting to 0.
    /// Idempotent — calling twice for the same name is a no-op.
    fn ensure(&mut self, logical_name: &str) {
        self.counts.entry(logical_name.to_string()).or_insert(0);
    }

    /// Increment the counter for `logical_name` and return the new
    /// generation. Counters not previously seen start at 0 and
    /// return 1 on first bump.
    fn bump(&mut self, logical_name: &str) -> Generation {
        let n = self.counts.entry(logical_name.to_string()).or_insert(0);
        *n = n.wrapping_add(1);
        Generation(*n)
    }

    #[allow(dead_code)]
    fn latest(&self, logical_name: &str) -> Option<Generation> {
        self.counts.get(logical_name).copied().map(Generation)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::channel;
    use std::sync::Mutex;

    // ── ReverseIndex tests ────────────────────────────────────────────────

    fn target(name: &str, kind: CompiledAssetKind, path: &str) -> CompileTarget {
        CompileTarget::new(name, kind, PathBuf::from(path))
    }

    /// Empty index returns no matches. Baseline.
    #[test]
    fn reverse_index_empty_returns_no_matches() {
        let idx = ReverseIndex::new();
        assert!(idx.lookup(Path::new("foo.png")).is_empty());
    }

    /// Single registration: lookup returns the registered name when
    /// the path matches, and empty when it doesn't.
    #[test]
    fn reverse_index_lookup_matches_registered_path() {
        let mut idx = ReverseIndex::new();
        idx.register(&target("textures/crate", CompiledAssetKind::Texture, "C:\\proj\\src\\textures\\crate.bmp"));
        let hits = idx.lookup(Path::new("C:\\proj\\src\\textures\\crate.bmp"));
        assert_eq!(hits, vec!["textures/crate".to_string()]);
        let misses = idx.lookup(Path::new("C:\\proj\\src\\textures\\other.bmp"));
        assert!(misses.is_empty());
    }

    /// Path canonicalisation is applied at lookup time too — the
    /// watcher and the registration call may have different case or
    /// separator conventions; canonicalising both sides at the
    /// boundary prevents silent misses.
    #[test]
    fn reverse_index_lookup_is_case_and_separator_insensitive() {
        let mut idx = ReverseIndex::new();
        idx.register(&target(
            "textures/crate",
            CompiledAssetKind::Texture,
            "C:/Project/Source/Textures/CRATE.BMP",
        ));
        // Different cases + separators must all hit the same entry.
        let hits = idx.lookup(Path::new("c:\\project\\source\\textures\\crate.bmp"));
        assert_eq!(hits, vec!["textures/crate".to_string()]);
    }

    /// A path that triggers two assets returns both names. Rare in
    /// practice but the data structure must support it for future
    /// asset kinds.
    #[test]
    fn reverse_index_supports_multiple_targets_per_path() {
        let mut idx = ReverseIndex::new();
        idx.register(&target("a", CompiledAssetKind::Texture, "shared.bmp"));
        idx.register(&target("b", CompiledAssetKind::Texture, "shared.bmp"));
        let mut hits = idx.lookup(Path::new("shared.bmp"));
        hits.sort();
        assert_eq!(hits, vec!["a".to_string(), "b".to_string()]);
    }

    /// `extra_source_paths` register additional triggers for the
    /// same logical name. A material whose compilation reads a
    /// shader file should recompile when that shader changes.
    #[test]
    fn reverse_index_includes_extra_source_paths() {
        let mut idx = ReverseIndex::new();
        idx.register(&target("materials/wall", CompiledAssetKind::Material, "wall.hmat")
            .with_extra_source_paths(vec![PathBuf::from("wall_albedo.bmp")]));
        assert_eq!(idx.lookup(Path::new("wall.hmat")), vec!["materials/wall".to_string()]);
        assert_eq!(idx.lookup(Path::new("wall_albedo.bmp")), vec!["materials/wall".to_string()]);
    }

    // ── GenerationCounter tests ───────────────────────────────────────────

    #[test]
    fn generation_first_bump_returns_one() {
        let mut gen = GenerationCounter::new();
        assert_eq!(gen.bump("foo"), Generation(1));
    }

    #[test]
    fn generation_subsequent_bumps_are_monotonic() {
        let mut gen = GenerationCounter::new();
        let g1 = gen.bump("foo");
        let g2 = gen.bump("foo");
        let g3 = gen.bump("foo");
        assert!(g1 < g2);
        assert!(g2 < g3);
    }

    /// Different logical names have independent counters. Without
    /// this property, bumping one asset would advance everyone
    /// else's generation and last-wins filtering would corrupt.
    #[test]
    fn generation_counters_are_independent_per_name() {
        let mut gen = GenerationCounter::new();
        let a = gen.bump("a");
        let b = gen.bump("b");
        let a2 = gen.bump("a");
        assert_eq!(a, Generation(1));
        assert_eq!(b, Generation(1));
        assert_eq!(a2, Generation(2));
    }

    #[test]
    fn generation_latest_reflects_most_recent_bump() {
        let mut gen = GenerationCounter::new();
        assert_eq!(gen.latest("foo"), None);
        let g1 = gen.bump("foo");
        assert_eq!(gen.latest("foo"), Some(g1));
        let g2 = gen.bump("foo");
        assert_eq!(gen.latest("foo"), Some(g2));
    }

    #[test]
    fn generation_ensure_creates_counter_at_zero() {
        let mut gen = GenerationCounter::new();
        gen.ensure("foo");
        assert_eq!(gen.latest("foo"), Some(Generation(0)));
        assert_eq!(gen.bump("foo"), Generation(1));
    }

    // ── CompileQueue worker tests ─────────────────────────────────────────

    /// Helper: build a `CompileFn` from a closure plus a shared
    /// counter so tests can observe how many times the worker
    /// invoked the compiler.
    fn counting_compile_fn(
        observed: Arc<Mutex<Vec<String>>>,
        outcome: impl Fn(&str) -> Result<Vec<u8>, AssetCompilerError> + Send + Sync + 'static,
    ) -> CompileFn {
        Box::new(move |_kind, _source, logical_name| {
            observed
                .lock()
                .unwrap()
                .push(logical_name.to_string());
            outcome(logical_name).map(|payload| CompiledAssetBlob {
                metadata: crate::assets::CompiledAssetMetadata {
                    kind: CompiledAssetKind::Texture,
                    logical_name: logical_name.to_string(),
                    source_path: "test/source".to_string(),
                    compiler_tag: "test-compiler".to_string(),
                    runtime_path: "test/runtime".to_string(),
                    dependencies: Vec::new(),
                },
                payload,
            })
        })
    }

    /// Duplicate logical names get rejected at construction. The
    /// counter and reverse-index keys would silently shadow
    /// otherwise.
    #[test]
    fn start_rejects_duplicate_logical_names() {
        let (_tx, rx) = channel::<FileChange>();
        let result = CompileQueue::start(CompileQueueConfig {
            file_changes: rx,
            targets: vec![
                target("dup", CompiledAssetKind::Texture, "a.bmp"),
                target("dup", CompiledAssetKind::Texture, "b.bmp"),
            ],
            compile_fn: default_compile_fn(),
        });
        match result {
            Err(CompileQueueError::DuplicateLogicalName(name)) => {
                assert_eq!(name, "dup");
            }
            Ok(_) => panic!("expected DuplicateLogicalName, got Ok"),
        }
    }

    /// File change for a registered path triggers exactly one
    /// invocation of the compile function, with the registered
    /// kind and logical name.
    #[test]
    fn file_change_for_registered_path_compiles() {
        let (tx, rx) = channel::<FileChange>();
        let observed = Arc::new(Mutex::new(Vec::<String>::new()));
        let queue = CompileQueue::start(CompileQueueConfig {
            file_changes: rx,
            targets: vec![target(
                "textures/crate",
                CompiledAssetKind::Texture,
                "crate.bmp",
            )],
            compile_fn: counting_compile_fn(Arc::clone(&observed), |_| Ok(vec![1, 2, 3])),
        })
        .unwrap();

        tx.send(FileChange::modified(canonicalise_for_index(Path::new(
            "crate.bmp",
        ))))
        .unwrap();

        let result = queue
            .recv_timeout(Duration::from_millis(500))
            .expect("expected a CompileResult");
        match result {
            CompileResult::Ok(ok) => {
                assert_eq!(ok.logical_name, "textures/crate");
                assert_eq!(ok.payload, vec![1, 2, 3]);
                assert_eq!(ok.generation, Generation(1));
            }
            CompileResult::Err(e) => panic!("expected Ok, got {:?}", e),
        }

        let invocations = observed.lock().unwrap().clone();
        assert_eq!(invocations, vec!["textures/crate".to_string()]);
    }

    /// File change for an unregistered path is silently ignored —
    /// no compile invocation, no result emitted. The watcher's
    /// extension filter is the first noise-defence; the queue's
    /// reverse-index miss is the second.
    #[test]
    fn file_change_for_unregistered_path_is_ignored() {
        let (tx, rx) = channel::<FileChange>();
        let observed = Arc::new(Mutex::new(Vec::<String>::new()));
        let queue = CompileQueue::start(CompileQueueConfig {
            file_changes: rx,
            targets: vec![target(
                "textures/crate",
                CompiledAssetKind::Texture,
                "crate.bmp",
            )],
            compile_fn: counting_compile_fn(Arc::clone(&observed), |_| Ok(vec![])),
        })
        .unwrap();

        tx.send(FileChange::modified(canonicalise_for_index(Path::new(
            "stranger.bmp",
        ))))
        .unwrap();

        // Worker must not emit anything. recv_timeout below the
        // worker's loop period of 100ms confirms quiet.
        let outcome = queue.recv_timeout(Duration::from_millis(250));
        assert!(outcome.is_err(), "expected timeout, got {:?}", outcome);
        assert!(observed.lock().unwrap().is_empty());
    }

    /// Compile errors surface as `CompileResult::Err` rather than
    /// panicking. The error string carries enough context to be
    /// useful in the editor pane.
    #[test]
    fn compile_failure_surfaces_as_err_result() {
        let (tx, rx) = channel::<FileChange>();
        let observed = Arc::new(Mutex::new(Vec::<String>::new()));
        let queue = CompileQueue::start(CompileQueueConfig {
            file_changes: rx,
            targets: vec![target(
                "textures/broken",
                CompiledAssetKind::Texture,
                "broken.bmp",
            )],
            compile_fn: counting_compile_fn(Arc::clone(&observed), |name| {
                Err(AssetCompilerError::InvalidMaterialLine {
                    line: 0,
                    message: format!("synthetic failure for {}", name),
                })
            }),
        })
        .unwrap();

        tx.send(FileChange::modified(canonicalise_for_index(Path::new(
            "broken.bmp",
        ))))
        .unwrap();

        let result = queue.recv_timeout(Duration::from_millis(500)).unwrap();
        match result {
            CompileResult::Err(err) => {
                assert_eq!(err.logical_name, "textures/broken");
                assert!(
                    err.error.contains("synthetic failure"),
                    "error message missing context: {}",
                    err.error
                );
                assert_eq!(err.generation, Generation(1));
            }
            CompileResult::Ok(_) => panic!("expected Err"),
        }
    }

    /// Two consecutive changes for the same asset bump the
    /// generation counter monotonically. The dispatcher uses this
    /// to filter stale results per ADR 0004 Q4.
    #[test]
    fn consecutive_changes_bump_generation_monotonically() {
        let (tx, rx) = channel::<FileChange>();
        let observed = Arc::new(Mutex::new(Vec::<String>::new()));
        let queue = CompileQueue::start(CompileQueueConfig {
            file_changes: rx,
            targets: vec![target(
                "textures/crate",
                CompiledAssetKind::Texture,
                "crate.bmp",
            )],
            compile_fn: counting_compile_fn(Arc::clone(&observed), |_| Ok(vec![])),
        })
        .unwrap();

        let path = canonicalise_for_index(Path::new("crate.bmp"));
        tx.send(FileChange::modified(path.clone())).unwrap();
        tx.send(FileChange::modified(path.clone())).unwrap();
        tx.send(FileChange::modified(path)).unwrap();

        let mut generations = Vec::new();
        for _ in 0..3 {
            let r = queue
                .recv_timeout(Duration::from_millis(500))
                .expect("expected a result");
            generations.push(r.generation());
        }
        assert_eq!(
            generations,
            vec![Generation(1), Generation(2), Generation(3)]
        );
    }

    /// Stop signal terminates the worker thread cleanly. Drop on
    /// the queue must not hang.
    #[test]
    fn stop_terminates_worker_thread() {
        let (_tx, rx) = channel::<FileChange>();
        let observed = Arc::new(Mutex::new(Vec::<String>::new()));
        let queue = CompileQueue::start(CompileQueueConfig {
            file_changes: rx,
            targets: vec![target(
                "textures/crate",
                CompiledAssetKind::Texture,
                "crate.bmp",
            )],
            compile_fn: counting_compile_fn(Arc::clone(&observed), |_| Ok(vec![])),
        })
        .unwrap();

        // Explicit stop. If the worker doesn't exit, the test
        // hangs (cargo test will timeout). Implicit Drop has the
        // same property.
        queue.stop();
    }
}
