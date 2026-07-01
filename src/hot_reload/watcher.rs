//! Win32 `ReadDirectoryChangesW` file watcher.
//!
//! One thread per watched root, blocking on overlapped I/O with a
//! timeout matched to the coalescer's quiet window. Raw notifications
//! flow through the [`super::Coalescer`] before being emitted on the
//! public `Receiver<FileChange>`.
//!
//! ## Lifecycle
//!
//! ```ignore
//! let watcher = FileWatcher::start(WatcherConfig {
//!     root: PathBuf::from("./source"),
//!     extensions: ExtensionFilter::default_engine_extensions(),
//!     coalesce_window: Duration::from_millis(50),
//! })?;
//!
//! while let Ok(event) = watcher.events().recv() {
//!     println!("{:?}", event);
//! }
//!
//! watcher.stop(); // joins the worker thread cleanly.
//! ```
//!
//! ## Failure posture
//!
//! - `start` returns `Err(WatcherError::OpenRoot { ... })` if the
//!   directory can't be opened. No thread is spawned in that case.
//! - Win32 errors *during* watching are logged to stderr and the
//!   thread keeps running. The watcher does not panic on transient
//!   failures.
//! - `Drop` on `FileWatcher` triggers a graceful stop with thread
//!   join, so leaking a watcher does not leak the worker thread.
//!
//! See ADR 0004 Q4.

use std::ffi::OsString;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::platform::ffi::{
    CancelIoEx, CloseHandle, CreateEventW, CreateFileW, GetOverlappedResult, ReadDirectoryChangesW,
    ResetEvent, SetEvent, WaitForMultipleObjects, BOOL, DWORD, FILE_ACTION_ADDED,
    FILE_ACTION_MODIFIED, FILE_ACTION_REMOVED, FILE_ACTION_RENAMED_NEW_NAME,
    FILE_ACTION_RENAMED_OLD_NAME, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OVERLAPPED,
    FILE_LIST_DIRECTORY, FILE_NOTIFY_CHANGE_CREATION, FILE_NOTIFY_CHANGE_DIR_NAME,
    FILE_NOTIFY_CHANGE_FILE_NAME, FILE_NOTIFY_CHANGE_LAST_WRITE, FILE_NOTIFY_CHANGE_SIZE,
    FILE_NOTIFY_INFORMATION, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, HANDLE,
    INVALID_HANDLE_VALUE, OPEN_EXISTING, OVERLAPPED, WAIT_FAILED, WAIT_OBJECT_0,
};

use super::coalescer::{Coalescer, RawFileEvent};
use super::event::FileChange;
use super::path_filter::{canonicalise_for_index, ExtensionFilter};

/// Configuration the caller passes to [`FileWatcher::start`].
pub struct WatcherConfig {
    /// Directory to watch recursively. Must exist on disk.
    pub root: PathBuf,
    /// Extension allowlist applied after coalescing — only events
    /// whose path's extension matches one of these reach the public
    /// receiver. Use [`ExtensionFilter::default_engine_extensions`]
    /// for the engine's standard set.
    pub extensions: ExtensionFilter,
    /// Coalescer quiet-window duration. ADR 0004 Q4 sets the engine
    /// default at 50ms; pass [`Duration::from_millis(50)`] unless
    /// the M7.1 calibration capture has produced a different value.
    pub coalesce_window: Duration,
}

impl Default for WatcherConfig {
    fn default() -> Self {
        Self {
            root: PathBuf::from("."),
            extensions: ExtensionFilter::default_engine_extensions(),
            coalesce_window: Duration::from_millis(50),
        }
    }
}

/// Errors surfaced by [`FileWatcher::start`]. Errors that happen
/// *during* watching are logged to stderr and not surfaced as
/// recoverable values — the watcher's contract is "keep delivering
/// events," and a transient OS failure shouldn't take down the
/// pipeline.
#[derive(Debug)]
pub enum WatcherError {
    /// `CreateFileW` on the watch root failed. The root probably
    /// doesn't exist or the process lacks read permission.
    OpenRoot {
        root: PathBuf,
        win32_error: DWORD,
    },
    /// `CreateEventW` for the overlapped completion event failed.
    /// Effectively impossible barring system-level resource
    /// exhaustion; surfaced for completeness.
    CreateEvent { win32_error: DWORD },
}

impl std::fmt::Display for WatcherError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OpenRoot { root, win32_error } => write!(
                f,
                "FileWatcher: failed to open watch root {:?} (Win32 error {})",
                root, win32_error
            ),
            Self::CreateEvent { win32_error } => write!(
                f,
                "FileWatcher: failed to create overlapped event (Win32 error {})",
                win32_error
            ),
        }
    }
}

impl std::error::Error for WatcherError {}

/// A running file-system watcher. Drop or [`Self::stop`] to
/// terminate the worker thread cleanly.
///
/// Two construction paths:
/// - [`Self::start`] returns the watcher with an internal
///   `Receiver<FileChange>`; access events via [`Self::events`].
///   Best for callers that read events directly.
/// - [`Self::start_with_sender`] takes an external `Sender<FileChange>`
///   so the caller can route events into a downstream channel they
///   own (e.g. `CompileQueue::start` consuming the receiver). The
///   watcher's internal `events`/`recv` accessors return `None` /
///   error in this mode.
pub struct FileWatcher {
    receiver: Option<Receiver<FileChange>>,
    /// Set to `true` to ask the worker thread to exit at the next
    /// `WaitForMultipleObjects` wakeup.
    stop_flag: Arc<AtomicBool>,
    /// Owned event handle the worker waits on alongside the
    /// overlapped completion event. Setting it forces an immediate
    /// wakeup so shutdown doesn't have to wait for the next file
    /// change.
    stop_event: HandleWrapper,
    thread: Option<JoinHandle<()>>,
}

impl FileWatcher {
    /// Start a watcher and spawn the worker thread. Events flow
    /// into an internal channel; access them via
    /// [`Self::events`] / [`Self::try_recv`] / [`Self::recv`].
    pub fn start(config: WatcherConfig) -> Result<Self, WatcherError> {
        let (tx, rx) = channel::<FileChange>();
        let mut watcher = Self::start_with_sender(config, tx)?;
        watcher.receiver = Some(rx);
        Ok(watcher)
    }

    /// Start a watcher that delivers events to an external
    /// [`Sender<FileChange>`] supplied by the caller. Use this when
    /// the receiver needs to be owned by something else — typically
    /// a [`super::CompileQueue`] that consumes file changes.
    ///
    /// The watcher's [`Self::events`] / [`Self::recv`] / [`Self::try_recv`]
    /// methods return `Err` / `None` in this mode because no
    /// internal receiver exists.
    pub fn start_with_sender(
        config: WatcherConfig,
        sender: Sender<FileChange>,
    ) -> Result<Self, WatcherError> {
        let stop_flag = Arc::new(AtomicBool::new(false));
        let stop_event = HandleWrapper::create_event()?;

        let stop_flag_thread = Arc::clone(&stop_flag);
        let stop_event_handle = SendHandle(stop_event.0);

        // Open the root directory before spawning the thread so the
        // caller learns about open failures synchronously.
        let dir_handle = SendHandle(open_directory_for_watching(&config.root)?);

        let thread = thread::spawn(move || {
            let dir = dir_handle;
            let stop_evt = stop_event_handle;
            watch_loop(dir.0, stop_evt.0, stop_flag_thread, config, sender);
            unsafe {
                CloseHandle(dir.0);
            }
        });

        Ok(Self {
            receiver: None,
            stop_flag,
            stop_event,
            thread: Some(thread),
        })
    }

    /// Read-only access to the internal channel of coalesced file
    /// changes. Returns `None` if the watcher was started with
    /// [`Self::start_with_sender`] — events flow to the caller's
    /// channel in that case.
    pub fn events(&self) -> Option<&Receiver<FileChange>> {
        self.receiver.as_ref()
    }

    /// Try to read a coalesced event without blocking. Returns
    /// `Disconnected` if the watcher was started with an external
    /// sender.
    pub fn try_recv(&self) -> Result<FileChange, std::sync::mpsc::TryRecvError> {
        match self.receiver.as_ref() {
            Some(rx) => rx.try_recv(),
            None => Err(std::sync::mpsc::TryRecvError::Disconnected),
        }
    }

    /// Block until a coalesced event arrives, or the worker thread
    /// shuts down. Returns `Err` if the watcher was started with an
    /// external sender (no internal receiver to block on).
    pub fn recv(&self) -> Result<FileChange, std::sync::mpsc::RecvError> {
        match self.receiver.as_ref() {
            Some(rx) => rx.recv(),
            None => Err(std::sync::mpsc::RecvError),
        }
    }

    /// Block with a timeout. Returns `Disconnected` if the watcher
    /// was started with an external sender.
    pub fn recv_timeout(
        &self,
        timeout: Duration,
    ) -> Result<FileChange, std::sync::mpsc::RecvTimeoutError> {
        match self.receiver.as_ref() {
            Some(rx) => rx.recv_timeout(timeout),
            None => Err(std::sync::mpsc::RecvTimeoutError::Disconnected),
        }
    }

    /// Signal the worker thread to stop and join it. Idempotent —
    /// calling twice is fine; the second call is a no-op.
    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        self.stop_flag.store(true, Ordering::SeqCst);
        unsafe {
            SetEvent(self.stop_event.0);
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for FileWatcher {
    fn drop(&mut self) {
        self.shutdown();
    }
}

// SAFETY: the watcher's only thread-shared state is `stop_flag`
// (atomic) and the `stop_event` HANDLE. The HANDLE is captured by
// value into the worker closure; once the worker is spawned, the
// FileWatcher only signals it via `SetEvent`. Both are safe to share
// across thread boundaries.
unsafe impl Send for FileWatcher {}
unsafe impl Sync for FileWatcher {}

// ── HandleWrapper ─────────────────────────────────────────────────────────

/// RAII for a Win32 HANDLE. CloseHandle on drop unless
/// [`Self::take`] released ownership.
struct HandleWrapper(HANDLE);

impl HandleWrapper {
    fn create_event() -> Result<Self, WatcherError> {
        let handle = unsafe {
            CreateEventW(
                std::ptr::null_mut(),
                1, // bManualReset
                0, // bInitialState
                std::ptr::null(),
            )
        };
        if handle.is_null() {
            return Err(WatcherError::CreateEvent {
                win32_error: unsafe { crate::platform::ffi::GetLastError() },
            });
        }
        Ok(Self(handle))
    }
}

impl Drop for HandleWrapper {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
}

// SAFETY: the HANDLE inside is only set/closed via Win32 calls;
// no Rust-side mutable aliasing is possible.
unsafe impl Send for HandleWrapper {}
unsafe impl Sync for HandleWrapper {}

/// Trivial newtype around a Win32 HANDLE that's safe to send across
/// threads. Used at the `thread::spawn` boundary where the closure
/// must be `Send` — `*mut c_void` isn't, so we wrap.
///
/// Safety: the watcher's HANDLEs (the watched directory + the
/// completion event + the stop event) are only ever accessed by one
/// thread at a time after the worker is spawned. The
/// `FileWatcher::shutdown` path signals the stop event from the
/// owning thread but does not otherwise touch the watcher's
/// directory or completion handles, so there's no concurrent
/// mutation.
struct SendHandle(HANDLE);
unsafe impl Send for SendHandle {}
unsafe impl Sync for SendHandle {}

// ── Worker thread ─────────────────────────────────────────────────────────

const WATCH_BUFFER_BYTES: usize = 16 * 1024;

fn open_directory_for_watching(root: &Path) -> Result<HANDLE, WatcherError> {
    let wide = path_to_wide(root);
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            FILE_LIST_DIRECTORY,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null_mut(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OVERLAPPED,
            std::ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE || handle.is_null() {
        return Err(WatcherError::OpenRoot {
            root: root.to_path_buf(),
            win32_error: unsafe { crate::platform::ffi::GetLastError() },
        });
    }
    Ok(handle)
}

fn path_to_wide(path: &Path) -> Vec<u16> {
    let mut wide: Vec<u16> = path.as_os_str().encode_wide().collect();
    wide.push(0);
    wide
}

#[allow(non_snake_case)]
fn watch_loop(
    dir_handle: HANDLE,
    stop_event: HANDLE,
    stop_flag: Arc<AtomicBool>,
    config: WatcherConfig,
    output: Sender<FileChange>,
) {
    let WatcherConfig {
        root,
        extensions,
        coalesce_window,
    } = config;
    let canonical_root = canonicalise_for_index(&root);

    let mut buffer = vec![0u8; WATCH_BUFFER_BYTES];

    let completion_event = match HandleWrapper::create_event() {
        Ok(h) => h,
        Err(e) => {
            eprintln!("[FileWatcher] could not create completion event: {}", e);
            return;
        }
    };

    let mut overlapped = OVERLAPPED::default();
    overlapped.hEvent = completion_event.0;

    let mut coalescer = Coalescer::new(coalesce_window);

    let notify_filter: DWORD = FILE_NOTIFY_CHANGE_FILE_NAME
        | FILE_NOTIFY_CHANGE_DIR_NAME
        | FILE_NOTIFY_CHANGE_LAST_WRITE
        | FILE_NOTIFY_CHANGE_SIZE
        | FILE_NOTIFY_CHANGE_CREATION;

    // Issue the first read.
    if !issue_read(
        dir_handle,
        buffer.as_mut_ptr(),
        buffer.len() as DWORD,
        &mut overlapped,
        notify_filter,
    ) {
        eprintln!(
            "[FileWatcher] initial ReadDirectoryChangesW failed (Win32 error {}); thread exiting",
            unsafe { crate::platform::ffi::GetLastError() }
        );
        return;
    }

    let handles = [completion_event.0, stop_event];
    let timeout_ms: DWORD = coalesce_window.as_millis().min(DWORD::MAX as u128) as DWORD;

    loop {
        if stop_flag.load(Ordering::SeqCst) {
            break;
        }

        let wait_result = unsafe {
            WaitForMultipleObjects(handles.len() as DWORD, handles.as_ptr(), 0, timeout_ms)
        };

        // The completion event is index 0; the stop event is index 1.
        let completed = wait_result == WAIT_OBJECT_0;
        let stop_signalled = wait_result == WAIT_OBJECT_0 + 1;

        if stop_signalled || stop_flag.load(Ordering::SeqCst) {
            break;
        }

        if completed {
            // Reset the completion event so the next read can fire.
            unsafe {
                ResetEvent(completion_event.0);
            }
            let mut bytes: DWORD = 0;
            let ok = unsafe {
                GetOverlappedResult(dir_handle, &mut overlapped, &mut bytes, 0)
            };
            if ok != 0 && bytes > 0 {
                let now = Instant::now();
                parse_buffer_into_coalescer(&buffer[..bytes as usize], &canonical_root, &mut coalescer, now);
            } else if ok == 0 {
                let err = unsafe { crate::platform::ffi::GetLastError() };
                if err != 0 {
                    eprintln!(
                        "[FileWatcher] GetOverlappedResult failed (Win32 error {}); rearming",
                        err
                    );
                }
            }

            // Re-arm the read.
            if !issue_read(
                dir_handle,
                buffer.as_mut_ptr(),
                buffer.len() as DWORD,
                &mut overlapped,
                notify_filter,
            ) {
                eprintln!(
                    "[FileWatcher] re-issue ReadDirectoryChangesW failed (Win32 error {}); thread exiting",
                    unsafe { crate::platform::ffi::GetLastError() }
                );
                break;
            }
        } else if wait_result == WAIT_FAILED {
            eprintln!(
                "[FileWatcher] WaitForMultipleObjects failed (Win32 error {}); thread exiting",
                unsafe { crate::platform::ffi::GetLastError() }
            );
            break;
        }
        // wait_result == WAIT_TIMEOUT (or any other value): no
        // events arrived this tick; fall through to flush expired
        // coalescer entries.

        let now = Instant::now();
        for change in coalescer.drain_expired(now) {
            if extensions.accepts(&change.path) {
                if output.send(change).is_err() {
                    // Receiver dropped — caller stopped listening,
                    // we should exit.
                    return;
                }
            }
        }
    }

    // Cancel any in-flight read before exit.
    unsafe {
        CancelIoEx(dir_handle, &mut overlapped);
    }
    // Flush any remaining pending events. Subscribers may want
    // to know about half-finished bursts on shutdown.
    for change in coalescer.drain_all() {
        if extensions.accepts(&change.path) {
            let _ = output.send(change);
        }
    }
}

fn issue_read(
    dir_handle: HANDLE,
    buffer: *mut u8,
    buffer_len: DWORD,
    overlapped: &mut OVERLAPPED,
    notify_filter: DWORD,
) -> bool {
    let mut bytes: DWORD = 0;
    let ok = unsafe {
        ReadDirectoryChangesW(
            dir_handle,
            buffer as *mut _,
            buffer_len,
            1, // bWatchSubtree (recursive)
            notify_filter,
            &mut bytes,
            overlapped,
            std::ptr::null_mut(),
        )
    };
    ok != 0
}

fn parse_buffer_into_coalescer(
    buffer: &[u8],
    canonical_root: &Path,
    coalescer: &mut Coalescer,
    now: Instant,
) {
    let mut offset = 0usize;
    while offset < buffer.len() {
        // SAFETY: ReadDirectoryChangesW guarantees aligned writes
        // of FILE_NOTIFY_INFORMATION records into the buffer; we
        // bound by length and walk via `NextEntryOffset`.
        let header: &FILE_NOTIFY_INFORMATION = unsafe {
            &*(buffer.as_ptr().add(offset) as *const FILE_NOTIFY_INFORMATION)
        };
        let name_len_chars = (header.FileNameLength as usize) / 2;
        let name_offset = offset
            + std::mem::offset_of!(FILE_NOTIFY_INFORMATION, FileName);
        if name_offset + (name_len_chars * 2) > buffer.len() {
            // Malformed record — bail rather than read past the
            // buffer.
            break;
        }
        let name_slice: &[u16] = unsafe {
            std::slice::from_raw_parts(
                buffer.as_ptr().add(name_offset) as *const u16,
                name_len_chars,
            )
        };
        let relative = OsString::from_wide(name_slice);
        let absolute = canonicalise_for_index(&canonical_root.join(Path::new(&relative)));

        let raw = match header.Action {
            FILE_ACTION_ADDED => Some(RawFileEvent::Created(absolute)),
            FILE_ACTION_REMOVED => Some(RawFileEvent::Removed(absolute)),
            FILE_ACTION_MODIFIED => Some(RawFileEvent::Modified(absolute)),
            FILE_ACTION_RENAMED_OLD_NAME => Some(RawFileEvent::RenamedOldName(absolute)),
            FILE_ACTION_RENAMED_NEW_NAME => Some(RawFileEvent::RenamedNewName(absolute)),
            _ => None,
        };
        if let Some(event) = raw {
            coalescer.record(event, now);
        }

        if header.NextEntryOffset == 0 {
            break;
        }
        offset += header.NextEntryOffset as usize;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Starting a watcher on a non-existent path returns
    /// `OpenRoot` rather than panicking. The caller then surfaces
    /// the error to the user (editor pane, log) instead of
    /// crashing the engine.
    #[test]
    fn start_returns_error_for_missing_root() {
        let bogus = PathBuf::from("Z:\\definitely\\does\\not\\exist\\for\\real");
        let result = FileWatcher::start(WatcherConfig {
            root: bogus.clone(),
            extensions: ExtensionFilter::default_engine_extensions(),
            coalesce_window: Duration::from_millis(50),
        });
        match result {
            Err(WatcherError::OpenRoot { root, .. }) => {
                assert_eq!(canonicalise_for_index(&root), canonicalise_for_index(&bogus));
            }
            Err(other) => panic!("expected OpenRoot, got {:?}", other),
            Ok(_) => panic!("expected error for non-existent root"),
        }
    }
}

// Suppress unused-import lints for SetEvent/ResetEvent on platforms
// where the hot-reload module is compiled but not exercised. They
// are used by `shutdown` and `watch_loop` respectively; the explicit
// allow keeps the imports honest as the watcher grows.
#[allow(dead_code)]
fn _ffi_smoke() {
    let _ = SetEvent;
    let _ = ResetEvent;
    let _ = WAIT_FAILED;
    let _: BOOL = 0;
}
