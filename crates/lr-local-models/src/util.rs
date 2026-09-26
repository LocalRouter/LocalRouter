//! Small filesystem helpers.

use std::io::Write;
use std::path::{Path, PathBuf};

/// Write `bytes` to `path` atomically (temp file in the same directory, fsync,
/// rename).
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(dir)?;
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".into());
    let tmp = dir.join(format!(
        ".{file_name}.{}.tmp",
        uuid::Uuid::new_v4().simple()
    ));
    let result = (|| {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        std::fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// `path` with `suffix` appended to its file name (`a.gguf` → `a.gguf.partial`).
pub(crate) fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(suffix);
    PathBuf::from(s)
}

/// Canonicalize the deepest existing ancestor of `path` and re-append the rest.
pub(crate) fn canonicalize_lenient(path: &Path) -> PathBuf {
    let mut existing = path.to_path_buf();
    let mut rest: Vec<std::ffi::OsString> = Vec::new();
    loop {
        if let Ok(c) = existing.canonicalize() {
            let mut out = c;
            for part in rest.iter().rev() {
                out.push(part);
            }
            return out;
        }
        match (
            existing.file_name().map(|n| n.to_owned()),
            existing.parent(),
        ) {
            (Some(name), Some(parent)) => {
                rest.push(name);
                existing = parent.to_path_buf();
            }
            _ => return path.to_path_buf(),
        }
    }
}

/// Human-readable size in GB (decimal, one decimal place).
pub(crate) fn gb(bytes: u64) -> String {
    format!("{:.1} GB", bytes as f64 / 1e9)
}

/// Lowercase slug of `[a-z0-9._-]`, other characters collapse to `-`.
pub(crate) fn slugify(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        let c = c.to_ascii_lowercase();
        if c.is_ascii_alphanumeric() || c == '.' || c == '_' {
            out.push(c);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let trimmed = out.trim_matches(|c| c == '-' || c == '.' || c == '_');
    let mut s: String = trimmed.chars().take(80).collect();
    while s.ends_with(['-', '.', '_']) {
        s.pop();
    }
    if s.is_empty() {
        "model".to_string()
    } else {
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs() {
        assert_eq!(slugify("Qwen3-8B Q4_K_M"), "qwen3-8b-q4_k_m");
        assert_eq!(slugify("  ..weird//name!!  "), "weird-name");
        assert_eq!(slugify("€€€"), "model");
    }

    #[test]
    fn atomic_write_and_suffix() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("sub/x.json");
        write_atomic(&p, b"{}").unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"{}");
        assert_eq!(
            with_suffix(&p, ".partial").file_name().unwrap(),
            "x.json.partial"
        );
        let lenient = canonicalize_lenient(&dir.path().join("a/b"));
        assert!(lenient.ends_with("a/b"));
    }
}
