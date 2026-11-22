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

#[cfg(test)]
mod tests {
    use super::*;

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
