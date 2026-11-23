//! Filesystem helpers

use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

/// `<dest>.part`, the sibling a file is written to before it replaces `dest`.
fn part_path(dest: &Path) -> PathBuf {
    let mut name = dest
        .file_name()
        .map(OsString::from)
        .unwrap_or_else(|| OsString::from("file"));
    name.push(".part");
    dest.with_file_name(name)
}

/// Copy `src` to `dest` so that `dest` is either absent, its previous
/// content, or the complete copy — never a truncated file.
///
/// The bytes go to `<dest>.part` in the same directory and are renamed over
/// `dest` once fully written. A `.part` left by an interrupted copy is
/// overwritten by the next one. Blocking: call from `spawn_blocking` in async
/// code. Returns the number of bytes copied.
pub fn copy_atomically(src: &Path, dest: &Path) -> io::Result<u64> {
    let part = part_path(dest);
    let copied =
        std::fs::copy(src, &part).and_then(|bytes| std::fs::rename(&part, dest).map(|()| bytes));
    if copied.is_err() {
        let _ = std::fs::remove_file(&part);
    }
    copied
}

/// Write `bytes` to `dest` so that `dest` is either its previous content or
/// the complete new content — never a torn file.
///
/// The bytes go to `<dest>.<pid>.part` in the same directory (one part file
/// per process, so two processes replacing the same file never interleave
/// their writes) and are renamed over `dest` once fully written. Blocking.
pub fn write_atomically(dest: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut name = dest
        .file_name()
        .map(OsString::from)
        .unwrap_or_else(|| OsString::from("file"));
    name.push(format!(".{}.part", std::process::id()));
    let part = dest.with_file_name(name);
    let written = std::fs::write(&part, bytes).and_then(|()| std::fs::rename(&part, dest));
    if written.is_err() {
        let _ = std::fs::remove_file(&part);
    }
    written
}

/// Attempts and pause of [`retry_while_missing`] during a replace:
/// the gap between two renames is far shorter than this.
const MISSING_RETRIES: u32 = 10;
const MISSING_RETRY_PAUSE: std::time::Duration = std::time::Duration::from_millis(10);

/// Replaces in progress and finished, so a reader can tell whether
/// a missing path may be one being replaced.
struct ReplaceTracker {
    /// Replaces in progress.
    active: AtomicUsize,
    /// Replaces finished (only ever increases, wrapping).
    finished: AtomicUsize,
}

impl ReplaceTracker {
    const fn new() -> Self {
        Self {
            active: AtomicUsize::new(0),
            finished: AtomicUsize::new(0),
        }
    }

    fn begin(&'static self) -> PathReplace {
        self.active.fetch_add(1, Ordering::SeqCst);
        PathReplace(self)
    }

    /// Whether a read that started when `finished` was `before` may have
    /// fallen into a gap: a replace is in progress, or one finished since.
    fn in_gap(&self, before: usize) -> bool {
        self.active.load(Ordering::SeqCst) > 0 || self.finished.load(Ordering::SeqCst) != before
    }
}

/// The replaces of this process.
static REPLACES: ReplaceTracker = ReplaceTracker::new();

/// Held by an updater while it replaces a path (a skill directory) and until
/// the replaced version is removed. While one is held, and for a read that
/// overlapped one, [`retry_while_missing`] in this process waits out a
/// missing path. It covers both ways a reader can miss the path during an
/// update: the gap between two renames where [`exchange_paths`] cannot swap
/// atomically (Windows, or a filesystem without support), and a lookup that
/// resolved the old directory just before an exchange and then finds the
/// file removed along with the old version.
#[must_use = "the gap is covered only while the guard is held"]
pub struct PathReplace(&'static ReplaceTracker);

impl PathReplace {
    pub fn begin() -> Self {
        REPLACES.begin()
    }
}

impl Drop for PathReplace {
    fn drop(&mut self) {
        // Counted as finished before it stops being active, so a reader that
        // saw the path missing sees this replace either active or finished.
        self.0.finished.fetch_add(1, Ordering::SeqCst);
        self.0.active.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Run a filesystem read, retrying it while it fails with `NotFound` and a
/// replace ([`PathReplace`]) in this process was in progress or finished
/// during the attempt: the path may be briefly absent while it is replaced. Bounded: at most `10 × 10 ms` of extra waiting, after which
/// the last error is returned. Otherwise the read runs once, so a path that
/// is simply missing costs two atomic loads, not a wait. Blocking.
pub fn retry_while_missing<T>(read: impl FnMut() -> io::Result<T>) -> io::Result<T> {
    retry_not_found(&REPLACES, MISSING_RETRIES, MISSING_RETRY_PAUSE, read)
}

fn retry_not_found<T>(
    replaces: &ReplaceTracker,
    retries: u32,
    pause: std::time::Duration,
    mut read: impl FnMut() -> io::Result<T>,
) -> io::Result<T> {
    let mut attempt = 0;
    loop {
        let before = replaces.finished.load(Ordering::SeqCst);
        match read() {
            Err(error)
                if error.kind() == io::ErrorKind::NotFound
                    && attempt < retries
                    && replaces.in_gap(before) =>
            {
                attempt += 1;
                std::thread::sleep(pause);
            }
            result => return result,
        }
    }
}

/// Why a path component taken from untrusted input (an archive entry, a
/// downloaded file list) would not name an ordinary file on Windows, if it
/// would not: a reserved device name (`CON`, `nul.txt`, `COM1`, … — opening
/// one opens the device), or a trailing dot or space (Windows drops them, so
/// `a.` and `a` are the same file). Checked on every platform so the same
/// input installs the same files everywhere.
pub fn non_portable_component(name: &str) -> Option<&'static str> {
    if name.ends_with('.') || name.ends_with(' ') {
        return Some("trailing dot or space");
    }
    let stem = name.split('.').next().unwrap_or(name).trim_end();
    let upper = stem.to_ascii_uppercase();
    let reserved = matches!(
        upper.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) || ((upper.starts_with("COM") || upper.starts_with("LPT"))
        && upper.len() == 4
        && matches!(upper.as_bytes()[3], b'1'..=b'9'));
    reserved.then_some("reserved Windows device name")
}

/// Atomically exchange two existing paths (files or directories) on the same
/// filesystem, so that neither name is ever missing: `renamex_np(RENAME_SWAP)`
/// on macOS, `renameat2(RENAME_EXCHANGE)` on Linux.
///
/// Returns `Ok(false)`, with nothing changed, where the platform or the
/// filesystem has no atomic exchange (Windows, older kernels, some network
/// and FUSE filesystems); the caller falls back to two renames.
pub fn exchange_paths(a: &Path, b: &Path) -> io::Result<bool> {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        use std::os::unix::ffi::OsStrExt;
        let to_c = |path: &Path| {
            std::ffi::CString::new(path.as_os_str().as_bytes())
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "path contains NUL"))
        };
        let (a, b) = (to_c(a)?, to_c(b)?);
        // SAFETY: both pointers are valid NUL-terminated strings that outlive
        // the call; the call does not retain them.
        #[cfg(target_os = "macos")]
        let result = unsafe { libc::renamex_np(a.as_ptr(), b.as_ptr(), libc::RENAME_SWAP) };
        // Through `syscall` rather than glibc's wrapper (glibc 2.28+, absent
        // from musl), so the binary runs on older systems too.
        #[cfg(target_os = "linux")]
        let result = unsafe {
            libc::syscall(
                libc::SYS_renameat2,
                libc::AT_FDCWD,
                a.as_ptr(),
                libc::AT_FDCWD,
                b.as_ptr(),
                libc::RENAME_EXCHANGE,
            )
        };
        if result == 0 {
            return Ok(true);
        }
        let error = io::Error::last_os_error();
        match error.raw_os_error() {
            // ENOTSUP is EOPNOTSUPP on Linux.
            Some(libc::ENOTSUP) | Some(libc::EINVAL) | Some(libc::ENOSYS) => Ok(false),
            _ => Err(error),
        }
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = (a, b);
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_device_names_and_trailing_dots_are_not_portable() {
        for bad in [
            "CON",
            "con",
            "nul.txt",
            "Aux.tar.gz",
            "COM1",
            "lpt9.log",
            "a.",
            "b ",
        ] {
            assert!(non_portable_component(bad).is_some(), "{bad}");
        }
        for good in [
            "console", "nul_", "COM0", "COM10", "LPT", "a.b", ".hidden", "SKILL.md",
        ] {
            assert_eq!(non_portable_component(good), None, "{good}");
        }
    }

    #[test]
    fn write_atomically_replaces_the_file_and_leaves_no_part_file() {
        let dir = TestDir::new("write-atomic");
        let dest = dir.0.join("records.json");
        std::fs::write(&dest, b"old").unwrap();
        write_atomically(&dest, b"new contents").unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"new contents");
        let names: Vec<_> = std::fs::read_dir(&dir.0)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, vec![std::ffi::OsString::from("records.json")]);
    }

    #[test]
    fn write_atomically_failure_keeps_the_previous_content() {
        let dir = TestDir::new("write-atomic-fail");
        // The destination is a directory: the rename fails, the part file
        // is removed and the directory is untouched.
        let dest = dir.0.join("busy");
        std::fs::create_dir(&dest).unwrap();
        std::fs::write(dest.join("keep"), b"x").unwrap();
        assert!(write_atomically(&dest, b"new").is_err());
        assert!(dest.join("keep").is_file());
        assert_eq!(std::fs::read_dir(&dir.0).unwrap().count(), 1);
    }

    #[test]
    fn missing_reads_are_retried_a_bounded_number_of_times_during_a_replace() {
        static REPLACES: ReplaceTracker = ReplaceTracker::new();
        let pause = std::time::Duration::from_millis(1);
        let not_found = || io::Error::from(io::ErrorKind::NotFound);
        let replace = REPLACES.begin();
        // Appears after two misses: found on the third attempt.
        let mut calls = 0;
        let found = retry_not_found(&REPLACES, 10, pause, || {
            calls += 1;
            if calls < 3 {
                Err(not_found())
            } else {
                Ok(calls)
            }
        });
        assert_eq!(found.unwrap(), 3);
        // Never appears: one attempt plus the retries, then the error.
        let mut calls = 0;
        let missing: io::Result<()> = retry_not_found(&REPLACES, 4, pause, || {
            calls += 1;
            Err(not_found())
        });
        assert_eq!(missing.unwrap_err().kind(), io::ErrorKind::NotFound);
        assert_eq!(calls, 5);
        // Other errors are not retried.
        let mut calls = 0;
        let denied: io::Result<()> = retry_not_found(&REPLACES, 4, pause, || {
            calls += 1;
            Err(io::Error::from(io::ErrorKind::PermissionDenied))
        });
        assert!(denied.is_err());
        assert_eq!(calls, 1);
        drop(replace);
    }

    #[test]
    fn a_missing_path_fails_at_once_without_a_replace() {
        static REPLACES: ReplaceTracker = ReplaceTracker::new();
        let mut calls = 0;
        let result: io::Result<()> =
            retry_not_found(&REPLACES, 10, std::time::Duration::from_secs(1), || {
                calls += 1;
                Err(io::Error::from(io::ErrorKind::NotFound))
            });
        assert!(result.is_err());
        assert_eq!(calls, 1);
        // A replace that began and finished before the read does not count.
        drop(REPLACES.begin());
        let mut calls = 0;
        let result: io::Result<()> =
            retry_not_found(&REPLACES, 10, std::time::Duration::from_secs(1), || {
                calls += 1;
                Err(io::Error::from(io::ErrorKind::NotFound))
            });
        assert!(result.is_err());
        assert_eq!(calls, 1);
    }

    /// A read that misses the path in the gap is retried even when the
    /// replace finishes before the read looks at the tracker.
    #[test]
    fn a_read_overlapping_a_finished_replace_is_retried() {
        static REPLACES: ReplaceTracker = ReplaceTracker::new();
        let mut replace = Some(REPLACES.begin());
        let mut calls = 0;
        let result = retry_not_found(&REPLACES, 10, std::time::Duration::from_millis(1), || {
            calls += 1;
            if calls == 1 {
                // The second rename lands and the guard is dropped between
                // the failed read and the check.
                drop(replace.take());
                Err(io::Error::from(io::ErrorKind::NotFound))
            } else {
                Ok("v2")
            }
        });
        assert_eq!(result.unwrap(), "v2");
        assert_eq!(calls, 2);
    }

    /// The real path through the process tracker: a directory replaced in
    /// two renames under [`PathReplace`] is waited for, on every platform.
    #[test]
    fn retry_while_missing_waits_out_a_two_step_replace() {
        let dir = TestDir::new("two-step");
        let target = dir.0.join("skill");
        let aside = dir.0.join("previous");
        std::fs::create_dir(&aside).unwrap();
        std::fs::write(aside.join("file"), b"v2").unwrap();
        let replace = PathReplace::begin();
        let (from, to) = (aside.clone(), target.clone());
        let mover = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(20));
            std::fs::rename(&from, &to).unwrap();
            drop(replace);
        });
        let read = retry_while_missing(|| std::fs::read(target.join("file")));
        mover.join().unwrap();
        assert_eq!(read.unwrap(), b"v2");
    }

    #[test]
    fn exchange_swaps_directories_or_reports_no_support() {
        let dir = TestDir::new("exchange");
        let a = dir.0.join("a");
        let b = dir.0.join("b");
        std::fs::create_dir(&a).unwrap();
        std::fs::create_dir(&b).unwrap();
        std::fs::write(a.join("which"), b"a").unwrap();
        std::fs::write(b.join("which"), b"b").unwrap();

        let swapped = exchange_paths(&a, &b).unwrap();
        // APFS, ext4 and tmpfs all support it.
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        assert!(
            swapped,
            "the temp filesystem should support an atomic exchange"
        );
        let (in_a, in_b) = if swapped { ("b", "a") } else { ("a", "b") };
        assert_eq!(std::fs::read(a.join("which")).unwrap(), in_a.as_bytes());
        assert_eq!(std::fs::read(b.join("which")).unwrap(), in_b.as_bytes());
    }

    #[test]
    fn exchange_with_a_missing_path_fails_and_changes_nothing() {
        let dir = TestDir::new("exchange-missing");
        let a = dir.0.join("a");
        std::fs::create_dir(&a).unwrap();
        let result = exchange_paths(&a, &dir.0.join("missing"));
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        assert!(result.is_err());
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        assert!(!result.unwrap());
        assert!(a.is_dir());
    }

    struct TestDir(PathBuf);
    impl TestDir {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "localrouter-utils-fs-{}-{}",
                std::process::id(),
                name
            ));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn copy_replaces_the_destination_and_leaves_no_part_file() {
        let dir = TestDir::new("replace");
        let src = dir.0.join("src.bin");
        let dest = dir.0.join("model.safetensors");
        std::fs::write(&src, b"new contents").unwrap();
        std::fs::write(&dest, b"old").unwrap();
        // Left behind by an interrupted copy.
        std::fs::write(part_path(&dest), b"trunc").unwrap();

        assert_eq!(copy_atomically(&src, &dest).unwrap(), 12);
        assert_eq!(std::fs::read(&dest).unwrap(), b"new contents");
        assert!(!part_path(&dest).exists());
        assert_eq!(part_path(&dest), dir.0.join("model.safetensors.part"));
    }

    #[test]
    fn failed_copy_keeps_the_destination() {
        let dir = TestDir::new("failed");
        let dest = dir.0.join("tokenizer.json");
        std::fs::write(&dest, b"old").unwrap();

        assert!(copy_atomically(&dir.0.join("missing"), &dest).is_err());
        assert_eq!(std::fs::read(&dest).unwrap(), b"old");
        assert!(!part_path(&dest).exists());
    }
}
