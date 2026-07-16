//! Atomic, durable file replacement (M11 — ADR 0003 Pillar 3).
//!
//! The charter's promise: *"A power loss during save must leave either the
//! previous scene or the new scene on disk — never a half-written one."*
//!
//! A bare `std::fs::write` cannot deliver that. It opens the destination
//! with truncate — zeroing the user's existing file — and *then* streams
//! the new bytes. A crash in between destroys the old file and leaves a
//! partial new one, with no copy to fall back to.
//!
//! ## The sequence
//!
//! 1. Serialize fully in memory (the caller's job — this function takes a
//!    complete `&[u8]`, so a serialization failure never reaches the disk).
//! 2. `CreateFileW` a uniquely-named temp file **in the destination's own
//!    directory**, with `CREATE_NEW` so a name collision is an error rather
//!    than a silent clobber.
//! 3. `WriteFile` in a loop until every byte lands.
//! 4. `FlushFileBuffers` — the temp's *contents* are now on physical media.
//! 5. `CloseHandle`.
//! 6. `MoveFileExW(MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH)` —
//!    a single journalled directory-entry swap, flushed through.
//!
//! ## Why the temp must share the destination's directory
//!
//! `MoveFileExW` is only an atomic metadata swap when source and destination
//! are on the same volume. Across volumes it silently degrades into
//! copy-then-delete, which is exactly the non-atomic behaviour we are here to
//! eliminate. Deriving the temp path from `dest.parent()` guarantees the same
//! volume; there is deliberately no option to place the temp elsewhere.
//!
//! ## Ordering guarantees
//!
//! - Crash **before** the rename: the destination still holds the complete
//!   *old* file. A temp file may be orphaned (see [`sweep_orphan_temps`]).
//! - Crash **during or after** the rename: NTFS journals the directory-entry
//!   update, so the destination is the complete *old* or complete *new* file.
//!
//! At no point does the destination path observe a partial write, because it
//! is never opened for writing at all.
//!
//! ## Durability
//!
//! `FlushFileBuffers` makes the temp's data durable before the swap; without
//! it the directory entry could point at a file whose contents never reached
//! disk. Windows has no POSIX directory-fsync, so `MOVEFILE_WRITE_THROUGH`
//! supplies the equivalent guarantee for the rename itself.

use std::ffi::c_void;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::ptr::null_mut;
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(test)]
use std::sync::atomic::AtomicUsize;

use super::ffi::{
    CloseHandle, CreateFileW, DeleteFileW, FlushFileBuffers, GetLastError, MoveFileExW, WriteFile,
    CREATE_NEW, DWORD, ERROR_FILE_EXISTS, FILE_ATTRIBUTE_NORMAL, GENERIC_WRITE, HANDLE,
    INVALID_HANDLE_VALUE, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
};

/// Filename prefix reserved for in-flight temp files. Never collides with a
/// real asset name, and lets [`sweep_orphan_temps`] recognise litter left by
/// a process that died between temp-create and rename.
pub const TEMP_PREFIX: &str = ".hold-tmp-";

/// Monotonic counter making temp names unique within a process, so two
/// concurrent saves in the same directory cannot pick the same path.
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Failure of an atomic write, named by the phase that failed so a caller can
/// tell a permission problem from a durability problem from a sharing
/// violation. Every Win32 variant carries the raw `GetLastError()` code,
/// mirroring [`crate::hot_reload::WatcherError`].
///
/// In every failure case the destination file is left **untouched** — it
/// still holds its previous complete contents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AtomicWriteError {
    /// The destination has no parent directory (e.g. a bare relative name
    /// with no components). Without a parent we cannot place the temp on the
    /// destination's volume, so we refuse rather than silently degrade.
    NoParentDirectory { dest: PathBuf },
    /// `CreateFileW` on the temp path failed — directory missing, read-only,
    /// or permission denied.
    TempCreate { temp: PathBuf, win32_error: DWORD },
    /// `WriteFile` failed partway through the payload (disk full, IO error).
    /// The temp has been deleted.
    Write { temp: PathBuf, win32_error: DWORD },
    /// `FlushFileBuffers` failed — the bytes are not durable, so the rename
    /// is not attempted. The temp has been deleted.
    Flush { temp: PathBuf, win32_error: DWORD },
    /// `MoveFileExW` failed. Commonly `ERROR_SHARING_VIOLATION` (32) when an
    /// antivirus scanner or indexer holds the destination open. The old
    /// destination is intact and the temp has been deleted; the caller should
    /// treat this as retryable and must NOT consider the save successful.
    Replace {
        temp: PathBuf,
        dest: PathBuf,
        win32_error: DWORD,
    },
    /// A test-injected crash fired. Unreachable from [`atomic_write`], which
    /// always passes [`AtomicWriteFault::None`]; exists so the power-loss
    /// tests can distinguish a simulated abort from a genuine failure.
    SimulatedCrash { phase: AtomicWriteFault },
}

impl std::fmt::Display for AtomicWriteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoParentDirectory { dest } => write!(
                f,
                "atomic write: destination {dest:?} has no parent directory to place a temp file in"
            ),
            Self::TempCreate { temp, win32_error } => write!(
                f,
                "atomic write: could not create temp file {temp:?} (Win32 error {win32_error})"
            ),
            Self::Write { temp, win32_error } => write!(
                f,
                "atomic write: failed writing temp file {temp:?} (Win32 error {win32_error})"
            ),
            Self::Flush { temp, win32_error } => write!(
                f,
                "atomic write: failed flushing temp file {temp:?} to disk (Win32 error {win32_error})"
            ),
            Self::Replace {
                temp,
                dest,
                win32_error,
            } => write!(
                f,
                "atomic write: failed replacing {dest:?} with {temp:?} (Win32 error {win32_error}); \
                 the previous file is intact"
            ),
            Self::SimulatedCrash { phase } => {
                write!(f, "atomic write: simulated crash at {phase:?}")
            }
        }
    }
}

impl std::error::Error for AtomicWriteError {}

/// A crash-injection point. "Power-loss tests in CI" cannot literally cut
/// power, so the write path takes a fault marker naming a phase at which to
/// abort exactly as a dying process would: stop immediately, and — crucially
/// — do **not** clean up the temp file, because a real crash never gets to.
///
/// Only the test-only [`atomic_write_with_fault`] can pass a non-`None`
/// value; [`atomic_write`] hardcodes `None` at its single call site, so the
/// optimizer folds every fault branch away in release builds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AtomicWriteFault {
    /// Normal operation. The only value production code can produce.
    #[default]
    None,
    /// Write only the first `n` bytes of the payload, then abort — a torn
    /// write. Proves a partially-written temp can never become the
    /// destination, because the rename is never reached.
    TornWrite(usize),
    /// Abort after all bytes are written, before `FlushFileBuffers`.
    AfterTempWrite,
    /// Abort after `FlushFileBuffers`, before `CloseHandle`.
    AfterFlush,
    /// Abort after `CloseHandle`, immediately before `MoveFileExW`. The last
    /// moment at which the destination still holds the old file.
    BeforeRename,
    /// Abort immediately after `MoveFileExW` returns. The destination now
    /// holds the complete new file.
    AfterRename,
}

/// Atomically replace `dest` with `bytes`, durably.
///
/// On success the destination holds exactly `bytes`. On *any* error the
/// destination is unchanged and still holds its previous complete contents —
/// callers must therefore not clear a dirty flag unless this returns `Ok`.
///
/// `bytes` is a complete in-memory payload by design: serialization cannot
/// fail partway through a file.
pub fn atomic_write(dest: &Path, bytes: &[u8]) -> Result<(), AtomicWriteError> {
    atomic_write_impl(dest, bytes, AtomicWriteFault::None)
}

/// Test-only entry point for the power-loss suite. Deliberately not compiled
/// into release builds: a fault seam reachable from production code would be
/// a liability that could corrupt a real save.
#[cfg(test)]
pub(crate) fn atomic_write_with_fault(
    dest: &Path,
    bytes: &[u8],
    fault: AtomicWriteFault,
) -> Result<(), AtomicWriteError> {
    atomic_write_impl(dest, bytes, fault)
}

#[inline]
fn atomic_write_impl(
    dest: &Path,
    bytes: &[u8],
    fault: AtomicWriteFault,
) -> Result<(), AtomicWriteError> {
    let parent = temp_parent(dest)?;

    let (handle, temp) = create_temp_file(dest, parent)?;

    // A dying process does not tidy up. Simulated crashes therefore close the
    // handle (so the test process doesn't leak one) but leave the temp on
    // disk, exactly as a real crash would.
    macro_rules! simulated_crash {
        () => {{
            unsafe { CloseHandle(handle) };
            return Err(AtomicWriteError::SimulatedCrash { phase: fault });
        }};
    }

    // ── Write ────────────────────────────────────────────────────────────
    let to_write = match fault {
        AtomicWriteFault::TornWrite(n) => n.min(bytes.len()),
        _ => bytes.len(),
    };
    if let Err(win32_error) = unsafe { write_all(handle, &bytes[..to_write]) } {
        unsafe { CloseHandle(handle) };
        delete_file(&temp);
        return Err(AtomicWriteError::Write {
            temp,
            win32_error,
        });
    }
    if matches!(fault, AtomicWriteFault::TornWrite(_)) {
        simulated_crash!();
    }
    if fault == AtomicWriteFault::AfterTempWrite {
        simulated_crash!();
    }

    // ── Flush (durability of the DATA) ───────────────────────────────────
    if unsafe { FlushFileBuffers(handle) } == 0 {
        let win32_error = unsafe { GetLastError() };
        unsafe { CloseHandle(handle) };
        delete_file(&temp);
        return Err(AtomicWriteError::Flush { temp, win32_error });
    }
    if fault == AtomicWriteFault::AfterFlush {
        simulated_crash!();
    }

    unsafe { CloseHandle(handle) };
    if fault == AtomicWriteFault::BeforeRename {
        // Handle already closed; a real crash here leaves the temp orphaned.
        return Err(AtomicWriteError::SimulatedCrash { phase: fault });
    }

    // ── Replace (atomicity + durability of the RENAME) ───────────────────
    let temp_wide = path_to_wide(&temp);
    let dest_wide = path_to_wide(dest);
    let replaced = unsafe {
        MoveFileExW(
            temp_wide.as_ptr(),
            dest_wide.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if replaced == 0 {
        let win32_error = unsafe { GetLastError() };
        delete_file(&temp);
        return Err(AtomicWriteError::Replace {
            temp,
            dest: dest.to_path_buf(),
            win32_error,
        });
    }

    if fault == AtomicWriteFault::AfterRename {
        // The swap already happened; the destination holds the new file.
        return Err(AtomicWriteError::SimulatedCrash { phase: fault });
    }

    Ok(())
}

/// The directory the temp file must be created in: the destination's own
/// directory, so the subsequent `MoveFileExW` is a same-volume atomic swap.
///
/// A bare filename (`"scene.hsc"`) has an *empty* parent but resolves against
/// the current directory — still the destination's directory — so it maps to
/// `"."`. Only a genuinely parentless path (empty string, or a filesystem
/// root like `C:\`) is refused, since there is no directory to place a temp
/// in. Pure function so the path logic is testable without mutating the
/// process-wide current directory.
fn temp_parent(dest: &Path) -> Result<&Path, AtomicWriteError> {
    match dest.parent() {
        Some(p) if !p.as_os_str().is_empty() => Ok(p),
        Some(_) => Ok(Path::new(".")),
        None => Err(AtomicWriteError::NoParentDirectory {
            dest: dest.to_path_buf(),
        }),
    }
}

/// Create a uniquely-named temp file next to `dest`. `CREATE_NEW` makes a
/// name collision an explicit failure; we retry a bounded number of times
/// against the (vanishingly unlikely) case of a leftover temp with the same
/// counter value from a previous run.
fn create_temp_file(dest: &Path, parent: &Path) -> Result<(HANDLE, PathBuf), AtomicWriteError> {
    let stem = dest
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "unnamed".to_string());

    let mut last_error: DWORD = 0;
    let mut last_temp = PathBuf::new();
    for _ in 0..8 {
        let n = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let temp = temp_candidate(parent, &stem, n);
        let wide = path_to_wide(&temp);
        let handle = unsafe {
            CreateFileW(
                wide.as_ptr(),
                GENERIC_WRITE,
                0, // no sharing while we write
                null_mut(),
                CREATE_NEW,
                FILE_ATTRIBUTE_NORMAL,
                null_mut(),
            )
        };
        if handle != INVALID_HANDLE_VALUE {
            return Ok((handle, temp));
        }
        last_error = unsafe { GetLastError() };
        last_temp = temp;
        // CREATE_NEW reports ERROR_FILE_EXISTS (80) on a name collision, NOT
        // ERROR_ALREADY_EXISTS (183) — the latter only comes from
        // CREATE_ALWAYS/OPEN_ALWAYS, which succeed. Only a collision is worth
        // retrying with a fresh counter; any other error (missing dir,
        // permissions) is terminal.
        if last_error != ERROR_FILE_EXISTS {
            break;
        }
    }
    Err(AtomicWriteError::TempCreate {
        temp: last_temp,
        win32_error: last_error,
    })
}

/// The temp path for a given counter value. Extracted so a test can compute
/// the exact next candidate and pre-seed a collision.
fn temp_candidate(parent: &Path, stem: &str, n: u64) -> PathBuf {
    parent.join(format!("{TEMP_PREFIX}{}-{}-{}", std::process::id(), n, stem))
}

/// Peek the next counter value without consuming it. Test-only; the counter
/// is shared across threads, so a peeked value is only a hint under
/// parallelism.
#[cfg(test)]
fn peek_temp_counter() -> u64 {
    TEMP_COUNTER.load(Ordering::Relaxed)
}

/// Largest single `WriteFile` request. `nNumberOfBytesToWrite` is a `DWORD`
/// (u32), so a payload larger than 4 GiB must be written across multiple
/// calls. Tests can lower this via [`TEST_MAX_CHUNK`] to force the multi-chunk
/// path on a small payload, since a real 4 GiB buffer is impractical in CI.
#[cfg(test)]
static TEST_MAX_CHUNK: AtomicUsize = AtomicUsize::new(0);

#[inline]
fn max_write_chunk() -> usize {
    #[cfg(test)]
    {
        let capped = TEST_MAX_CHUNK.load(Ordering::Relaxed);
        if capped != 0 {
            return capped;
        }
    }
    u32::MAX as usize
}

/// `WriteFile` until every byte is written. A short write is not an error on
/// Windows, so the loop advances by the reported count.
unsafe fn write_all(handle: HANDLE, bytes: &[u8]) -> Result<(), DWORD> {
    let mut offset: usize = 0;
    while offset < bytes.len() {
        let remaining = bytes.len() - offset;
        let chunk = remaining.min(max_write_chunk()) as DWORD;
        let mut written: DWORD = 0;
        let ok = WriteFile(
            handle,
            bytes[offset..].as_ptr() as *const c_void,
            chunk,
            &mut written,
            null_mut(),
        );
        if ok == 0 {
            return Err(GetLastError());
        }
        if written == 0 {
            // No progress and no error reported: treat as a hard failure
            // rather than spinning forever.
            return Err(GetLastError());
        }
        offset += written as usize;
    }
    Ok(())
}

/// Best-effort temp removal on a failure path. The error is deliberately
/// ignored: we are already returning a more meaningful failure, and an
/// undeleted temp is litter, not data loss.
fn delete_file(path: &Path) {
    let wide = path_to_wide(path);
    unsafe { DeleteFileW(wide.as_ptr()) };
}

fn path_to_wide(path: &Path) -> Vec<u16> {
    let mut wide: Vec<u16> = path.as_os_str().encode_wide().collect();
    wide.push(0);
    wide
}

/// Remove in-flight temp files orphaned in `dir` by a process that died
/// between temp-create and rename. Such a file can never be mistaken for a
/// real one (the [`TEMP_PREFIX`] is reserved), so this is litter collection,
/// not recovery. Best-effort: unreadable directories and locked files are
/// skipped. Returns how many were removed.
pub fn sweep_orphan_temps(dir: &Path) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    let mut removed = 0;
    for entry in entries.flatten() {
        let name = entry.file_name();
        if name.to_string_lossy().starts_with(TEMP_PREFIX)
            && std::fs::remove_file(entry.path()).is_ok()
        {
            removed += 1;
        }
    }
    removed
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    const OLD: &[u8] = b"PREVIOUS SCENE CONTENTS - complete and valid";
    const NEW: &[u8] = b"NEW SCENE CONTENTS - longer, complete, and valid, with more entities";

    fn test_dir(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "hold_atomic_write_{}_{}_{}",
            label,
            std::process::id(),
            nonce
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn seed_old(dir: &Path) -> PathBuf {
        let dest = dir.join("scene.hsc");
        std::fs::write(&dest, OLD).unwrap();
        dest
    }

    /// Count temp files left behind in `dir`.
    fn orphan_temps(dir: &Path) -> usize {
        std::fs::read_dir(dir)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with(TEMP_PREFIX))
            .count()
    }

    // ── Happy paths ──────────────────────────────────────────────────────

    #[test]
    fn atomic_write_replaces_existing_file() {
        let dir = test_dir("replace");
        let dest = seed_old(&dir);
        atomic_write(&dest, NEW).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), NEW);
        assert_eq!(orphan_temps(&dir), 0, "successful write must leave no temp");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn atomic_write_creates_file_that_does_not_exist() {
        let dir = test_dir("create");
        let dest = dir.join("fresh.hsc");
        atomic_write(&dest, NEW).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), NEW);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn atomic_write_handles_empty_payload() {
        let dir = test_dir("empty");
        let dest = seed_old(&dir);
        atomic_write(&dest, b"").unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn atomic_write_handles_payload_larger_than_one_write() {
        let dir = test_dir("large");
        let dest = dir.join("big.bin");
        // 3 MiB of varying bytes — exercises the WriteFile loop.
        let payload: Vec<u8> = (0..3 * 1024 * 1024).map(|i| (i % 251) as u8).collect();
        atomic_write(&dest, &payload).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), payload);
        std::fs::remove_dir_all(dir).ok();
    }

    // ── The load-bearing invariant: old-or-new, never partial ────────────
    //
    // "Power-loss tests in CI" (ADR 0003 Pillar 3). We cannot cut power in a
    // unit test, so the fault seam aborts the write exactly where a dying
    // process would. For EVERY abort point the destination must be readable
    // and byte-exactly the complete OLD file or the complete NEW file. A
    // zero-length or truncated destination is the failure this milestone
    // exists to make impossible.

    /// Every pre-rename fault leaves the destination holding the complete
    /// OLD file. The rename is the commit point; nothing before it may be
    /// observable at the destination path.
    #[test]
    fn crash_before_rename_leaves_complete_old_file() {
        for fault in [
            AtomicWriteFault::TornWrite(7),
            AtomicWriteFault::AfterTempWrite,
            AtomicWriteFault::AfterFlush,
            AtomicWriteFault::BeforeRename,
        ] {
            let dir = test_dir("pre_rename");
            let dest = seed_old(&dir);

            let err = atomic_write_with_fault(&dest, NEW, fault).unwrap_err();
            assert_eq!(err, AtomicWriteError::SimulatedCrash { phase: fault });

            let actual = std::fs::read(&dest).unwrap();
            assert_eq!(
                actual, OLD,
                "after a crash at {fault:?} the destination must still be the complete OLD file"
            );
            std::fs::remove_dir_all(dir).ok();
        }
    }

    /// A crash after the rename leaves the destination holding the complete
    /// NEW file — the swap already committed.
    #[test]
    fn crash_after_rename_leaves_complete_new_file() {
        let dir = test_dir("post_rename");
        let dest = seed_old(&dir);

        let err = atomic_write_with_fault(&dest, NEW, AtomicWriteFault::AfterRename).unwrap_err();
        assert_eq!(
            err,
            AtomicWriteError::SimulatedCrash {
                phase: AtomicWriteFault::AfterRename
            }
        );
        assert_eq!(std::fs::read(&dest).unwrap(), NEW);
        std::fs::remove_dir_all(dir).ok();
    }

    /// The whole invariant in one assertion: across every fault point, the
    /// destination is ALWAYS one of exactly two complete values. Never
    /// empty, never a prefix of either.
    #[test]
    fn destination_is_always_complete_old_or_complete_new() {
        for fault in [
            AtomicWriteFault::TornWrite(0),
            AtomicWriteFault::TornWrite(1),
            AtomicWriteFault::TornWrite(NEW.len() - 1),
            AtomicWriteFault::AfterTempWrite,
            AtomicWriteFault::AfterFlush,
            AtomicWriteFault::BeforeRename,
            AtomicWriteFault::AfterRename,
        ] {
            let dir = test_dir("invariant");
            let dest = seed_old(&dir);
            let _ = atomic_write_with_fault(&dest, NEW, fault);

            let actual = std::fs::read(&dest).unwrap();
            assert!(
                actual == OLD || actual == NEW,
                "at {fault:?} destination was neither complete OLD nor complete NEW \
                 (len {}, content {:?})",
                actual.len(),
                String::from_utf8_lossy(&actual)
            );
            assert!(!actual.is_empty(), "destination was truncated to zero at {fault:?}");
            std::fs::remove_dir_all(dir).ok();
        }
    }

    /// A torn temp (only some bytes written) must never be promoted to the
    /// destination: the rename is unreachable from a torn write. The partial
    /// bytes are confined to the temp file, which is orphaned by the crash.
    #[test]
    fn torn_temp_never_becomes_the_destination() {
        let dir = test_dir("torn");
        let dest = seed_old(&dir);

        let partial = 5;
        let err =
            atomic_write_with_fault(&dest, NEW, AtomicWriteFault::TornWrite(partial)).unwrap_err();
        assert_eq!(
            err,
            AtomicWriteError::SimulatedCrash {
                phase: AtomicWriteFault::TornWrite(partial)
            }
        );

        // Destination untouched.
        assert_eq!(std::fs::read(&dest).unwrap(), OLD);
        // The partial bytes exist ONLY in an orphaned temp, never at dest.
        assert_eq!(orphan_temps(&dir), 1, "a crashed write orphans its temp");
        std::fs::remove_dir_all(dir).ok();
    }

    // ── Orphan temps: litter, never data loss ────────────────────────────

    /// A real crash cannot clean up after itself, so a pre-rename fault
    /// leaves a temp behind. It is recognisable by the reserved prefix and
    /// reclaimable; it can never be confused with the destination.
    #[test]
    fn crashed_write_orphans_temp_and_sweep_reclaims_it() {
        let dir = test_dir("orphan");
        let dest = seed_old(&dir);

        let _ = atomic_write_with_fault(&dest, NEW, AtomicWriteFault::BeforeRename);
        assert_eq!(orphan_temps(&dir), 1);

        let removed = sweep_orphan_temps(&dir);
        assert_eq!(removed, 1);
        assert_eq!(orphan_temps(&dir), 0);
        // The sweep must not touch the real file.
        assert_eq!(std::fs::read(&dest).unwrap(), OLD);
        std::fs::remove_dir_all(dir).ok();
    }

    // ── Genuine failure paths leave the destination intact ───────────────

    /// A create failure (destination directory does not exist) surfaces a
    /// typed TempCreate error and never touches any file.
    #[test]
    fn missing_directory_surfaces_typed_temp_create_error() {
        let dir = test_dir("missing_dir");
        let dest = dir.join("nope").join("scene.hsc");
        match atomic_write(&dest, NEW).unwrap_err() {
            AtomicWriteError::TempCreate { win32_error, .. } => {
                assert_ne!(win32_error, 0, "a real Win32 error code must be carried");
            }
            other => panic!("expected TempCreate, got {other:?}"),
        }
        std::fs::remove_dir_all(dir).ok();
    }

    /// A genuinely parentless path (empty, or a filesystem root) is refused
    /// rather than silently placing the temp somewhere else, which would
    /// break same-volume atomicity.
    #[test]
    fn destination_without_parent_is_rejected() {
        for p in ["", "\\", "C:\\"] {
            let err = atomic_write(Path::new(p), NEW).unwrap_err();
            assert!(
                matches!(err, AtomicWriteError::NoParentDirectory { .. }),
                "expected NoParentDirectory for {p:?}, got {err:?}"
            );
        }
    }

    /// Temp placement resolution, tested as a pure function — mutating the
    /// process-wide current directory in a test would race with every other
    /// test in the binary.
    ///
    /// A bare filename resolves against the current directory (still the
    /// destination's own directory, so same-volume atomicity holds) rather
    /// than being refused.
    #[test]
    fn temp_parent_resolves_bare_filename_to_current_directory() {
        assert_eq!(temp_parent(Path::new("scene.hsc")).unwrap(), Path::new("."));
        assert_eq!(
            temp_parent(Path::new("dir/scene.hsc")).unwrap(),
            Path::new("dir")
        );
        assert_eq!(
            temp_parent(Path::new("C:\\proj\\scene.hsc")).unwrap(),
            Path::new("C:\\proj")
        );
    }

    /// Temp files are created in the destination's OWN directory. Placing
    /// them elsewhere (e.g. the system temp dir) would silently degrade
    /// MoveFileExW into a non-atomic copy+delete across volumes.
    #[test]
    fn temp_file_is_created_beside_the_destination() {
        let dir = test_dir("same_dir");
        let dest = seed_old(&dir);
        let _ = atomic_write_with_fault(&dest, NEW, AtomicWriteFault::BeforeRename);
        // The orphaned temp proves placement: it is in `dir`, not %TEMP%.
        assert_eq!(orphan_temps(&dir), 1);
        std::fs::remove_dir_all(dir).ok();
    }

    /// Concurrent writes into the same directory pick distinct temp names,
    /// so one save can never clobber another's in-flight temp.
    #[test]
    fn concurrent_writes_use_distinct_temp_names() {
        let dir = test_dir("unique");
        let a = dir.join("a.hsc");
        let b = dir.join("b.hsc");
        // Crash both before rename so both temps survive.
        let _ = atomic_write_with_fault(&a, b"aaaa", AtomicWriteFault::BeforeRename);
        let _ = atomic_write_with_fault(&b, b"bbbb", AtomicWriteFault::BeforeRename);
        assert_eq!(orphan_temps(&dir), 2, "temp names must not collide");
        std::fs::remove_dir_all(dir).ok();
    }

    /// The Replace failure path (review): when MoveFileExW cannot replace the
    /// destination — here because the destination is read-only, the same
    /// old-file-intact outcome as a sharing violation — the error is a typed
    /// Replace carrying the Win32 code, the destination still holds the OLD
    /// file, and the temp is cleaned up (no litter).
    #[test]
    fn replace_failure_leaves_old_file_and_cleans_temp() {
        let dir = test_dir("replace_fail");
        let dest = seed_old(&dir);

        // Mark the destination read-only so MoveFileExW(REPLACE_EXISTING)
        // cannot delete/replace it.
        let mut perms = std::fs::metadata(&dest).unwrap().permissions();
        perms.set_readonly(true);
        std::fs::set_permissions(&dest, perms).unwrap();

        match atomic_write(&dest, NEW) {
            Err(AtomicWriteError::Replace { win32_error, .. }) => {
                assert_ne!(win32_error, 0, "must carry a real Win32 error code");
            }
            other => panic!("expected Replace error, got {other:?}"),
        }

        // Old file intact, no temp litter.
        let mut perms = std::fs::metadata(&dest).unwrap().permissions();
        perms.set_readonly(false);
        std::fs::set_permissions(&dest, perms).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), OLD, "old file must be intact");
        assert_eq!(orphan_temps(&dir), 0, "failed replace must clean its temp");
        std::fs::remove_dir_all(dir).ok();
    }

    /// The temp-name collision retry (review): a leftover temp occupying the
    /// next candidate name does not fail the save — the loop retries with a
    /// fresh counter and still writes. Guards the ERROR_FILE_EXISTS fix (the
    /// loop previously broke on the wrong Win32 code and never retried).
    #[test]
    fn temp_name_collision_is_retried() {
        let dir = test_dir("collision");
        let dest = dir.join("scene.hsc");
        // Pre-seed the next several temp candidates so the first create
        // attempt collides (the counter is shared, so seed a small range to
        // survive concurrent allocations from other tests).
        let base = peek_temp_counter();
        for n in base..base + 6 {
            std::fs::write(temp_candidate(&dir, "scene.hsc", n), b"leftover").unwrap();
        }
        atomic_write(&dest, NEW).expect("collision must be retried, not fatal");
        assert_eq!(std::fs::read(&dest).unwrap(), NEW);
        std::fs::remove_dir_all(dir).ok();
    }

    /// The write_all multi-chunk loop (review): a payload that exceeds one
    /// WriteFile request must be written across calls, byte-exactly. A real
    /// >4 GiB buffer is impractical in CI, so a test-only chunk cap forces the
    /// same offset-advance path on a small payload.
    #[test]
    fn write_all_multi_chunk_loop_is_byte_exact() {
        let dir = test_dir("chunked");
        let dest = dir.join("chunked.bin");
        let payload: Vec<u8> = (0..1000u32).map(|i| (i % 251) as u8).collect();

        // 7-byte chunks => ~143 loop iterations for a 1000-byte payload.
        TEST_MAX_CHUNK.store(7, Ordering::Relaxed);
        let result = atomic_write(&dest, &payload);
        TEST_MAX_CHUNK.store(0, Ordering::Relaxed); // restore for other tests

        result.expect("chunked write must succeed");
        assert_eq!(
            std::fs::read(&dest).unwrap(),
            payload,
            "a multi-chunk write must land byte-exactly with no gaps or dupes"
        );
        std::fs::remove_dir_all(dir).ok();
    }
}
