//! Filesystem helpers

use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};

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

/// Whether [`exchange_paths`] can swap two paths atomically on this
/// platform. Where it cannot, a directory being replaced is briefly missing
/// between two renames.
pub const ATOMIC_EXCHANGE_PLATFORM: bool = cfg!(any(target_os = "macos", target_os = "linux"));

/// Attempts and pause of [`retry_while_missing`] on platforms without an
/// atomic exchange: the gap between two renames is far shorter than this.
const MISSING_RETRIES: u32 = 10;
const MISSING_RETRY_PAUSE: std::time::Duration = std::time::Duration::from_millis(10);

/// Run a filesystem read, retrying it while it fails with `NotFound` on
/// platforms where a directory being replaced (a skill update) is briefly
/// absent ([`ATOMIC_EXCHANGE_PLATFORM`] is false). Bounded: at most
/// `10 × 10 ms` of extra waiting, after which the last error is returned.
/// Elsewhere the read runs once. Blocking.
pub fn retry_while_missing<T>(read: impl FnMut() -> io::Result<T>) -> io::Result<T> {
    let retries = if ATOMIC_EXCHANGE_PLATFORM {
        0
    } else {
        MISSING_RETRIES
    };
    retry_not_found(retries, MISSING_RETRY_PAUSE, read)
}

fn retry_not_found<T>(
    retries: u32,
    pause: std::time::Duration,
    mut read: impl FnMut() -> io::Result<T>,
) -> io::Result<T> {
    let mut attempt = 0;
    loop {
        match read() {
            Err(error) if error.kind() == io::ErrorKind::NotFound && attempt < retries => {
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
    fn missing_reads_are_retried_a_bounded_number_of_times() {
        let pause = std::time::Duration::from_millis(1);
        let not_found = || io::Error::from(io::ErrorKind::NotFound);
        // Appears after two misses: found on the third attempt.
        let mut calls = 0;
        let found = retry_not_found(10, pause, || {
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
        let missing: io::Result<()> = retry_not_found(4, pause, || {
            calls += 1;
            Err(not_found())
        });
        assert_eq!(missing.unwrap_err().kind(), io::ErrorKind::NotFound);
        assert_eq!(calls, 5);
        // Other errors are not retried.
        let mut calls = 0;
        let denied: io::Result<()> = retry_not_found(4, pause, || {
            calls += 1;
            Err(io::Error::from(io::ErrorKind::PermissionDenied))
        });
        assert!(denied.is_err());
        assert_eq!(calls, 1);
    }

    #[test]
    fn retry_while_missing_runs_once_where_the_exchange_is_atomic() {
        let mut calls = 0;
        let result: io::Result<()> = retry_while_missing(|| {
            calls += 1;
            Err(io::Error::from(io::ErrorKind::NotFound))
        });
        assert!(result.is_err());
        let expected = if ATOMIC_EXCHANGE_PLATFORM {
            1
        } else {
            MISSING_RETRIES + 1
        };
        assert_eq!(calls, expected);
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
