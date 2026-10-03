//! Lifetime of the temp files that hold uploaded `multipart/form-data` file
//! parts (GH #386).
//!
//! Each part is written to `cfupload_<pid>_<counter>.upload` in the system temp
//! directory and exposed to CFML as `form.<field>.tempFilePath`. Nothing used
//! to delete them, so a server taking uploads filled its temp volume.
//!
//! Three layers, each covering what the one before it cannot:
//!
//! 1. **Request end.** The host owns the paths a request created through an
//!    [`UploadTempFiles`] guard and deletes them when the request is over —
//!    Lucee's `FormImpl.release()`. Unlike Lucee, a request that started
//!    background CFML work (`cfthread`, `runAsync`, an executor task) is NOT
//!    simply skipped forever: its files are kept, because that work may still
//!    be reading `tempFilePath`, and left to layer 2.
//! 2. **Age reaper.** [`reap_stale_upload_temp_files`] sweeps our own files
//!    older than a configured age, plus any left by a process that is no longer
//!    running (a crash, a `kill -9`, a previous run). An age sweep cannot race a
//!    still-running thread the way completion tracking can.
//! 3. **Process exit.** [`remove_outstanding_upload_temp_files`] deletes every
//!    file this process created and has not yet deleted — nothing can be
//!    reading them once the process is gone.
//!
//! Only names matching our exact pattern are ever touched; nothing else in the
//! temp directory is.

use std::cell::Cell;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

const PREFIX: &str = "cfupload_";
const SUFFIX: &str = ".upload";

/// Paths this process created and has not deleted yet.
static OUTSTANDING: Mutex<Option<HashSet<PathBuf>>> = Mutex::new(None);

thread_local! {
    /// Set when the VM running on this thread hands a CFML body to another
    /// thread (see [`note_background_body_spawned`]).
    static BACKGROUND_BODY_SPAWNED: Cell<bool> = const { Cell::new(false) };
}

/// Record that the VM on this thread started CFML work that may outlive the
/// request (a `cfthread`, `runAsync`, an executor task). Called from
/// `build_thread_seed`, which every such path goes through on the submitting
/// thread. Over-reporting is safe: it only defers deletion to the reaper.
pub fn note_background_body_spawned() {
    BACKGROUND_BODY_SPAWNED.with(|c| c.set(true));
}

/// Read and clear this thread's background-work mark. A host calls it once
/// before running a request (to clear a mark a previous request on the same
/// pooled thread left behind) and once after.
pub fn take_background_body_spawned() -> bool {
    BACKGROUND_BODY_SPAWNED.with(|c| c.replace(false))
}

pub(crate) fn register(path: &Path) {
    let mut g = OUTSTANDING.lock().unwrap_or_else(|e| e.into_inner());
    g.get_or_insert_with(HashSet::new).insert(path.to_path_buf());
}

fn unregister(path: &Path) {
    let mut g = OUTSTANDING.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(set) = g.as_mut() {
        set.remove(path);
    }
}

/// Number of upload temp files this process created and has not deleted.
pub fn outstanding_upload_temp_files() -> usize {
    let g = OUTSTANDING.lock().unwrap_or_else(|e| e.into_inner());
    g.as_ref().map_or(0, |s| s.len())
}

fn delete(path: &Path) {
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => log::warn!("upload: could not delete temp file {}: {e}", path.display()),
    }
    unregister(path);
}

/// Delete every upload temp file this process created and still owns. For
/// process exit: no request or thread can be reading them afterwards.
pub fn remove_outstanding_upload_temp_files() -> usize {
    let paths: Vec<PathBuf> = {
        let mut g = OUTSTANDING.lock().unwrap_or_else(|e| e.into_inner());
        g.take().map(|s| s.into_iter().collect()).unwrap_or_default()
    };
    for p in &paths {
        let _ = std::fs::remove_file(p);
    }
    paths.len()
}

/// The upload temp files one request created, deleted when it is dropped
/// unless [`keep`](Self::keep) was called.
///
/// Held by the host for the life of the request, so every early return (a 404,
/// a blocked path, a failed parse) cleans up as well as a normal one.
#[derive(Debug, Default)]
pub struct UploadTempFiles {
    paths: Vec<PathBuf>,
    keep: bool,
}

impl UploadTempFiles {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, path: PathBuf) {
        self.paths.push(path);
    }

    pub fn len(&self) -> usize {
        self.paths.len()
    }

    pub fn is_empty(&self) -> bool {
        self.paths.is_empty()
    }

    /// Leave the files in place: background work this request started may
    /// still read them. The age reaper (or process exit) removes them later.
    pub fn keep(&mut self) {
        self.keep = true;
    }
}

impl Drop for UploadTempFiles {
    fn drop(&mut self) {
        if self.keep {
            if !self.paths.is_empty() {
                log::debug!(
                    "upload: keeping {} temp file(s) past request end — the request \
                     started background work; the age reaper will remove them",
                    self.paths.len()
                );
            }
            return;
        }
        for p in self.paths.drain(..) {
            delete(&p);
        }
    }
}

/// The pid embedded in one of our temp file names, or `None` if `name` is not
/// exactly `cfupload_<pid>_<counter>.upload`.
pub fn upload_temp_file_pid(name: &str) -> Option<u32> {
    let mid = name.strip_prefix(PREFIX)?.strip_suffix(SUFFIX)?;
    let (pid, counter) = mid.split_once('_')?;
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    if !digits(pid) || !digits(counter) {
        return None;
    }
    pid.parse().ok()
}

/// Sweep `dir` for stale upload temp files and delete them. Returns the
/// number deleted.
///
/// A file is stale when:
/// - it was created by THIS process and is older than `max_age`; or
/// - the process that created it is no longer running (`pid_alive` says no) —
///   nothing can still be reading it, whatever its age.
///
/// Files from another process that is still alive are left to that process's
/// own reaper. `max_age` of zero disables the age rule (dead-process files are
/// still removed).
pub fn reap_stale_upload_temp_files(
    dir: &Path,
    max_age: std::time::Duration,
    now: std::time::SystemTime,
    pid_alive: &dyn Fn(u32) -> bool,
) -> usize {
    let own_pid = std::process::id();
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) => {
            log::warn!("upload reaper: cannot read {}: {e}", dir.display());
            return 0;
        }
    };
    let mut removed = 0;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(pid) = name.to_str().and_then(upload_temp_file_pid) else {
            continue;
        };
        // Only plain files; never follow a symlink someone planted.
        match entry.file_type() {
            Ok(t) if t.is_file() => {}
            _ => continue,
        }
        let stale = if pid == own_pid {
            !max_age.is_zero()
                && entry
                    .metadata()
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|m| now.duration_since(m).ok())
                    .is_some_and(|age| age >= max_age)
        } else {
            !pid_alive(pid)
        };
        if stale {
            let path = entry.path();
            match std::fs::remove_file(&path) {
                Ok(()) => removed += 1,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => {
                    log::warn!("upload reaper: could not delete {}: {e}", path.display())
                }
            }
            unregister(&path);
        }
    }
    removed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_our_exact_name_pattern_is_recognised() {
        assert_eq!(upload_temp_file_pid("cfupload_123_0.upload"), Some(123));
        assert_eq!(upload_temp_file_pid("cfupload_9_42.upload"), Some(9));
        for bad in [
            "cfupload_123.upload",
            "cfupload__1.upload",
            "cfupload_1_.upload",
            "cfupload_a_1.upload",
            "cfupload_1_1.upload.bak",
            "xcfupload_1_1.upload",
            "cfupload_avatar.png",
            "cfupload_1_1_1.upload",
            "tmp-1.upload",
        ] {
            assert_eq!(upload_temp_file_pid(bad), None, "{bad}");
        }
    }

    #[test]
    fn guard_deletes_on_drop_unless_kept() {
        let dir = std::env::temp_dir();
        let a = dir.join(format!("cfupload_{}_guardtest_a.tmp", std::process::id()));
        let b = dir.join(format!("cfupload_{}_guardtest_b.tmp", std::process::id()));
        std::fs::write(&a, b"a").unwrap();
        std::fs::write(&b, b"b").unwrap();
        {
            let mut g = UploadTempFiles::new();
            g.push(a.clone());
        }
        assert!(!a.exists(), "a dropped guard deletes its files");
        {
            let mut g = UploadTempFiles::new();
            g.push(b.clone());
            g.keep();
        }
        assert!(b.exists(), "a kept guard leaves its files for the reaper");
        let _ = std::fs::remove_file(&b);
    }

    #[test]
    fn reaper_removes_old_own_files_and_dead_process_files_only() {
        let dir = std::env::temp_dir().join(format!("rcfml_reap_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let me = std::process::id();
        let own = dir.join(format!("cfupload_{me}_1.upload"));
        let dead = dir.join("cfupload_4000000001_1.upload");
        let alive_other = dir.join("cfupload_4000000002_1.upload");
        let unrelated = dir.join("something_else.upload");
        for p in [&own, &dead, &alive_other, &unrelated] {
            std::fs::write(p, b"x").unwrap();
        }
        let alive = |pid: u32| pid != 4_000_000_001;
        let now = std::time::SystemTime::now();

        // Young own file stays; the dead process's goes immediately.
        let n = reap_stale_upload_temp_files(&dir, std::time::Duration::from_secs(3600), now, &alive);
        assert_eq!(n, 1);
        assert!(own.exists() && !dead.exists() && alive_other.exists() && unrelated.exists());

        // Once old enough, our own file goes; a live process's and an
        // unrelated file never do.
        let later = now + std::time::Duration::from_secs(7200);
        let n = reap_stale_upload_temp_files(&dir, std::time::Duration::from_secs(3600), later, &alive);
        assert_eq!(n, 1);
        assert!(!own.exists() && alive_other.exists() && unrelated.exists());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
