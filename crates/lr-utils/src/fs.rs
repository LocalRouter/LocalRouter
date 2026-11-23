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
