//! Bounded, confined extraction of skill archives (`.zip` / `.skill`).
//!
//! An archive is untrusted input. Extraction is strict: the first entry that
//! breaks a rule fails the whole archive, and nothing of a failed archive is
//! left where a later scan could mistake it for an installed skill. Files are
//! written into a staging directory beside the final one and renamed into
//! place only once every entry has been written.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};
use std::time::{Duration, SystemTime};

const MIB: u64 = 1024 * 1024;

/// Most bytes one archive may expand to. Skills are instructions, scripts and
/// a few assets; this leaves room for bundles of many skills with binary
/// assets while keeping a hostile archive from filling the disk.
pub const MAX_ARCHIVE_TOTAL_BYTES: u64 = 256 * MIB;
/// Most bytes a single extracted file may hold.
pub const MAX_ARCHIVE_FILE_BYTES: u64 = 64 * MIB;
/// Most entries (files and directories) an archive may list.
pub const MAX_ARCHIVE_ENTRIES: usize = 10_000;
/// Deepest entry path, in components.
pub const MAX_ARCHIVE_PATH_DEPTH: usize = 32;
/// Longest entry path, in bytes.
pub const MAX_ARCHIVE_PATH_BYTES: usize = 1024;
/// Longest single path component, in bytes (the limit of common filesystems).
pub const MAX_ARCHIVE_COMPONENT_BYTES: usize = 255;
/// Most an archive may expand relative to the size of the archive file.
/// Text compresses about 10x; deflate tops out near 1000x, which is what a
/// decompression bomb uses.
pub const MAX_ARCHIVE_EXPANSION_RATIO: u64 = 100;
/// The expansion ratio is only checked once this much has been written, so a
/// tiny archive of repetitive files is not rejected.
pub const ARCHIVE_EXPANSION_FLOOR_BYTES: u64 = 16 * MIB;

/// Staging directories are named `.staging-<uuid>` beside the final directory.
const STAGING_PREFIX: &str = ".staging-";

/// Written into a finished extraction: `<files> <bytes>` on the first line,
/// the archive's path on the second, and `archive <size> <mtime-ns>` (the
/// archive file as it was when extracted) on the third. Reusing an
/// extraction requires the counts to match the tree (checked once per
/// process). The path tells apart extractions of same-named archives in
/// different folders; the archive stamp lets a scan find the extraction of
/// an unchanged archive without hashing it. The file is also what processes
/// lock to lease the extraction (see [`ExtractionLease`]).
const COMPLETE_MARKER: &str = ".localrouter-extraction";

/// Budgets applied while extracting one archive.
#[derive(Debug, Clone, Copy)]
pub struct ExtractionLimits {
    pub max_total_bytes: u64,
    pub max_file_bytes: u64,
    pub max_entries: usize,
    pub max_path_depth: usize,
    pub max_path_bytes: usize,
    pub max_component_bytes: usize,
    pub max_expansion_ratio: u64,
    pub expansion_floor_bytes: u64,
}

impl Default for ExtractionLimits {
    fn default() -> Self {
        Self {
            max_total_bytes: MAX_ARCHIVE_TOTAL_BYTES,
            max_file_bytes: MAX_ARCHIVE_FILE_BYTES,
            max_entries: MAX_ARCHIVE_ENTRIES,
            max_path_depth: MAX_ARCHIVE_PATH_DEPTH,
            max_path_bytes: MAX_ARCHIVE_PATH_BYTES,
            max_component_bytes: MAX_ARCHIVE_COMPONENT_BYTES,
            max_expansion_ratio: MAX_ARCHIVE_EXPANSION_RATIO,
            expansion_floor_bytes: ARCHIVE_EXPANSION_FLOOR_BYTES,
        }
    }
}

/// Why an archive was not extracted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArchiveError {
    /// The archive could not be opened or is not a valid zip.
    Invalid(String),
    /// A filesystem operation failed.
    Io(String),
    TooManyEntries {
        count: usize,
        limit: usize,
    },
    FileTooLarge {
        path: String,
        limit: u64,
    },
    TotalTooLarge {
        limit: u64,
    },
    ExpansionRatio {
        limit: u64,
    },
    UnsafePath {
        path: String,
        reason: &'static str,
    },
    PathTooDeep {
        path: String,
        limit: usize,
    },
    PathTooLong {
        path: String,
        limit: usize,
    },
    Link {
        path: String,
    },
    SpecialFile {
        path: String,
    },
    Collision {
        path: String,
    },
}

fn display_bytes(bytes: u64) -> String {
    if bytes >= MIB && bytes.is_multiple_of(MIB) {
        format!("{} MiB", bytes / MIB)
    } else if bytes >= 1024 && bytes.is_multiple_of(1024) {
        format!("{} KiB", bytes / 1024)
    } else {
        format!("{bytes} bytes")
    }
}

impl fmt::Display for ArchiveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(detail) => write!(f, "not a readable zip archive: {detail}"),
            Self::Io(detail) => write!(f, "could not extract the archive: {detail}"),
            Self::TooManyEntries { count, limit } => {
                write!(f, "the archive lists {count} entries; the limit is {limit}")
            }
            Self::FileTooLarge { path, limit } => write!(
                f,
                "'{path}' is larger than the {} limit for one file",
                display_bytes(*limit)
            ),
            Self::TotalTooLarge { limit } => write!(
                f,
                "the archive expands to more than the {} limit",
                display_bytes(*limit)
            ),
            Self::ExpansionRatio { limit } => write!(
                f,
                "the archive expands to more than {limit} times its own size"
            ),
            Self::UnsafePath { path, reason } => {
                write!(f, "unsafe path in archive ({reason}): '{path}'")
            }
            Self::PathTooDeep { path, limit } => write!(
                f,
                "'{path}' is nested deeper than the limit of {limit} folders"
            ),
            Self::PathTooLong { path, limit } => {
                write!(f, "'{path}' is longer than the {limit}-byte path limit")
            }
            Self::Link { path } => write!(f, "links are not allowed in a skill archive: '{path}'"),
            Self::SpecialFile { path } => write!(
                f,
                "'{path}' is neither a regular file nor a folder (device, pipe or socket)"
            ),
            Self::Collision { path } => write!(
                f,
                "'{path}' appears more than once, or differs from another entry only by letter case"
            ),
        }
    }
}

impl std::error::Error for ArchiveError {}

fn io_error(context: &str, error: std::io::Error) -> ArchiveError {
    ArchiveError::Io(format!("{context}: {error}"))
}

/// What a successful extraction wrote.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExtractionSummary {
    pub files: usize,
    pub bytes: u64,
}

/// Validate an entry name and split it into components.
///
/// Rejects everything that is not a plain relative path made of ordinary
/// names: absolute paths, `.` and `..`, empty components, backslashes (a
/// separator on Windows), colons (drive prefixes and alternate data streams)
/// and NUL bytes.
fn entry_components<'a>(
    name: &'a str,
    limits: &ExtractionLimits,
) -> Result<Vec<&'a str>, ArchiveError> {
    let unsafe_path = |reason| ArchiveError::UnsafePath {
        path: name.escape_debug().to_string(),
        reason,
    };
    if name.contains('\0') {
        return Err(unsafe_path("NUL byte"));
    }
    if name.contains('\\') {
        return Err(unsafe_path("backslash"));
    }
    if name.contains(':') {
        return Err(unsafe_path("drive prefix or stream name"));
    }
    if name.starts_with('/') {
        return Err(unsafe_path("absolute path"));
    }
    if name.len() > limits.max_path_bytes {
        return Err(ArchiveError::PathTooLong {
            path: name.chars().take(80).collect::<String>() + "...",
            limit: limits.max_path_bytes,
        });
    }
    // A directory entry ends with one separator.
    let trimmed = name.strip_suffix('/').unwrap_or(name);
    if trimmed.is_empty() {
        return Err(unsafe_path("empty path"));
    }
    let components: Vec<&str> = trimmed.split('/').collect();
    for component in &components {
        match *component {
            "" => return Err(unsafe_path("empty path component")),
            "." => return Err(unsafe_path("'.' component")),
            ".." => return Err(unsafe_path("parent directory reference")),
            _ => {}
        }
        if let Some(reason) = lr_utils::fs::non_portable_component(component) {
            return Err(unsafe_path(reason));
        }
        if component.len() > limits.max_component_bytes {
            return Err(ArchiveError::PathTooLong {
                path: name.chars().take(80).collect::<String>() + "...",
                limit: limits.max_component_bytes,
            });
        }
    }
    if components.len() > limits.max_path_depth {
        return Err(ArchiveError::PathTooDeep {
            path: name.to_string(),
            limit: limits.max_path_depth,
        });
    }
    Ok(components)
}

/// Entries already claimed, compared without regard to letter case so an
/// archive extracts to the same tree on case-sensitive and case-insensitive
/// filesystems.
#[derive(Default)]
struct ClaimedPaths {
    files: HashSet<String>,
    /// Folded directory path -> the spelling first used for it.
    directories: HashMap<String, String>,
}

impl ClaimedPaths {
    fn claim_directory(&mut self, path: &str) -> Result<(), ArchiveError> {
        let folded = path.to_lowercase();
        if self.files.contains(&folded) {
            return Err(ArchiveError::Collision {
                path: path.to_string(),
            });
        }
        match self.directories.get(&folded) {
            Some(spelling) if spelling != path => Err(ArchiveError::Collision {
                path: path.to_string(),
            }),
            Some(_) => Ok(()),
            None => {
                self.directories.insert(folded, path.to_string());
                Ok(())
            }
        }
    }

    fn claim(&mut self, components: &[&str], is_directory: bool) -> Result<(), ArchiveError> {
        let parents = if is_directory {
            components.len()
        } else {
            components.len() - 1
        };
        for depth in 1..=parents {
            self.claim_directory(&components[..depth].join("/"))?;
        }
        if !is_directory {
            let path = components.join("/");
            let folded = path.to_lowercase();
            if self.directories.contains_key(&folded) || !self.files.insert(folded) {
                return Err(ArchiveError::Collision { path });
            }
        }
        Ok(())
    }
}

const S_IFMT: u32 = 0o170000;
const S_IFREG: u32 = 0o100000;
const S_IFDIR: u32 = 0o040000;

/// Extract `archive_path` into the existing, empty directory `dest`.
///
/// Byte budgets count what is actually written, not the sizes the archive
/// declares. On error `dest` holds whatever was written so far; the caller
/// owns removing it (see [`extract_archive_atomically`]).
pub fn extract_archive(
    archive_path: &Path,
    dest: &Path,
    limits: &ExtractionLimits,
) -> Result<ExtractionSummary, ArchiveError> {
    let file = std::fs::File::open(archive_path)
        .map_err(|e| ArchiveError::Invalid(format!("cannot open the file: {e}")))?;
    let archive_bytes = file
        .metadata()
        .map_err(|e| ArchiveError::Invalid(format!("cannot inspect the file: {e}")))?
        .len();
    let mut archive = zip::ZipArchive::new(std::io::BufReader::new(file))
        .map_err(|e| ArchiveError::Invalid(e.to_string()))?;

    if archive.len() > limits.max_entries {
        return Err(ArchiveError::TooManyEntries {
            count: archive.len(),
            limit: limits.max_entries,
        });
    }
    let expansion_limit = archive_bytes.saturating_mul(limits.max_expansion_ratio);

    let mut claimed = ClaimedPaths::default();
    let mut summary = ExtractionSummary { files: 0, bytes: 0 };
    let mut buffer = vec![0u8; 64 * 1024];

    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|e| ArchiveError::Invalid(format!("entry {index}: {e}")))?;
        let name = entry.name().to_string();
        let components = entry_components(&name, limits)?;
        if components[0].eq_ignore_ascii_case(COMPLETE_MARKER) {
            return Err(ArchiveError::UnsafePath {
                path: name,
                reason: "name reserved by LocalRouter",
            });
        }

        if entry.is_symlink() {
            return Err(ArchiveError::Link { path: name });
        }
        let file_type = entry.unix_mode().map_or(0, |mode| mode & S_IFMT);
        if file_type != 0 && file_type != S_IFREG && file_type != S_IFDIR {
            return Err(ArchiveError::SpecialFile { path: name });
        }
        let is_directory = entry.is_dir();
        claimed.claim(&components, is_directory)?;

        let out: PathBuf = components
            .iter()
            .fold(dest.to_path_buf(), |path, c| path.join(c));
        if is_directory {
            std::fs::create_dir_all(&out).map_err(|e| io_error(&name, e))?;
            continue;
        }

        // An early, cheap rejection; the loop below is what enforces it.
        if entry.size() > limits.max_file_bytes {
            return Err(ArchiveError::FileTooLarge {
                path: name,
                limit: limits.max_file_bytes,
            });
        }
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent).map_err(|e| io_error(&name, e))?;
        }
        let mut out_file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&out)
            .map_err(|e| io_error(&name, e))?;

        let mut file_bytes: u64 = 0;
        loop {
            let read = entry
                .read(&mut buffer)
                .map_err(|e| ArchiveError::Invalid(format!("'{name}': {e}")))?;
            if read == 0 {
                break;
            }
            file_bytes += read as u64;
            if file_bytes > limits.max_file_bytes {
                return Err(ArchiveError::FileTooLarge {
                    path: name,
                    limit: limits.max_file_bytes,
                });
            }
            summary.bytes += read as u64;
            if summary.bytes > limits.max_total_bytes {
                return Err(ArchiveError::TotalTooLarge {
                    limit: limits.max_total_bytes,
                });
            }
            if summary.bytes > limits.expansion_floor_bytes && summary.bytes > expansion_limit {
                return Err(ArchiveError::ExpansionRatio {
                    limit: limits.max_expansion_ratio,
                });
            }
            out_file
                .write_all(&buffer[..read])
                .map_err(|e| io_error(&name, e))?;
        }
        out_file.flush().map_err(|e| io_error(&name, e))?;
        summary.files += 1;
    }

    Ok(summary)
}

/// One lock per final directory, so two scans in this process never extract
/// the same archive at once. Other processes are kept out by [`RootLock`].
fn target_lock(target: &Path) -> Arc<parking_lot::Mutex<()>> {
    static LOCKS: LazyLock<parking_lot::Mutex<HashMap<PathBuf, Arc<parking_lot::Mutex<()>>>>> =
        LazyLock::new(Default::default);
    LOCKS
        .lock()
        .entry(target.to_path_buf())
        .or_default()
        .clone()
}

/// Name of the lock file in the extraction root.
const ROOT_LOCK_FILE: &str = ".lock";

/// Exclusive, cross-process lock on an extraction root, held while an
/// extraction is checked, written and moved into place, and while any tree is
/// removed. Two LocalRouter processes sharing a root (two instances, or two
/// builds pointed at the same data directory) therefore never interleave
/// those steps. Released when dropped.
pub struct RootLock {
    _file: std::fs::File,
}

impl RootLock {
    /// Block until the lock on `root` is held. Creates `root` if needed.
    pub fn acquire(root: &Path) -> std::io::Result<Self> {
        std::fs::create_dir_all(root)?;
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(root.join(ROOT_LOCK_FILE))?;
        file.lock()?;
        Ok(Self { _file: file })
    }
}

/// A shared lock on an extraction's marker file, held for as long as this
/// process serves skills from that extraction. Removal needs an exclusive
/// lock on the same file, so a tree another process is reading is never
/// deleted under it. Released when dropped.
#[derive(Debug)]
pub struct ExtractionLease {
    dir: PathBuf,
    _file: std::fs::File,
}

impl ExtractionLease {
    /// The extraction this lease keeps alive.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Take a shared lease on the finished extraction `dir`. Call with the
    /// [`RootLock`] held: exclusive locks on markers are only ever taken under
    /// it, so this never waits.
    fn acquire(dir: &Path) -> std::io::Result<Self> {
        let file = std::fs::File::open(dir.join(COMPLETE_MARKER))?;
        file.lock_shared()?;
        Ok(Self {
            dir: dir.to_path_buf(),
            _file: file,
        })
    }
}

/// Removes the staging directory unless the extraction was promoted.
struct StagingGuard(Option<PathBuf>);

impl Drop for StagingGuard {
    fn drop(&mut self) {
        if let Some(path) = self.0.take() {
            if let Err(e) = std::fs::remove_dir_all(&path) {
                tracing::warn!("Failed to remove staging directory {:?}: {}", path, e);
            }
        }
    }
}

/// Regular files under `dir` (the marker excluded) and their total size, or
/// `None` when the tree holds anything else (a link, a special file) or
/// cannot be read.
fn tree_summary(dir: &Path) -> Option<ExtractionSummary> {
    let mut summary = ExtractionSummary { files: 0, bytes: 0 };
    let mut pending = vec![dir.to_path_buf()];
    while let Some(current) = pending.pop() {
        for entry in std::fs::read_dir(&current).ok()? {
            let entry = entry.ok()?;
            let file_type = entry.file_type().ok()?;
            if file_type.is_dir() {
                pending.push(entry.path());
            } else if file_type.is_file() {
                if current == dir && entry.file_name() == COMPLETE_MARKER {
                    continue;
                }
                summary.files += 1;
                summary.bytes += entry.metadata().ok()?.len();
            } else {
                return None;
            }
        }
    }
    Some(summary)
}

/// The size and modification time of a file, as recorded in markers and
/// caches (nanoseconds where the filesystem keeps them).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FileStamp {
    pub len: u64,
    pub modified_ns: u128,
}

impl FileStamp {
    pub fn of(path: &Path) -> Option<Self> {
        let metadata = std::fs::metadata(path).ok()?;
        let modified_ns = metadata
            .modified()
            .ok()?
            .duration_since(SystemTime::UNIX_EPOCH)
            .ok()?
            .as_nanos();
        Some(Self {
            len: metadata.len(),
            modified_ns,
        })
    }
}

/// What a marker records.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Marker {
    summary: ExtractionSummary,
    source: Option<PathBuf>,
    /// The archive file as it was when extracted (absent in markers written
    /// before this was recorded).
    archive: Option<FileStamp>,
}

impl Marker {
    fn render(&self) -> String {
        let mut text = format!("{} {}\n", self.summary.files, self.summary.bytes);
        if let Some(source) = &self.source {
            text.push_str(&source.display().to_string());
        }
        text.push('\n');
        if let Some(stamp) = self.archive {
            text.push_str(&format!("archive {} {}\n", stamp.len, stamp.modified_ns));
        }
        text
    }

    fn parse(text: &str) -> Option<Self> {
        let mut lines = text.lines();
        let mut counts = lines.next()?.split_whitespace();
        let files = counts.next()?.parse::<usize>().ok()?;
        let bytes = counts.next()?.parse::<u64>().ok()?;
        if counts.next().is_some() {
            return None;
        }
        let source = lines.next().filter(|s| !s.is_empty()).map(PathBuf::from);
        let archive = lines.next().and_then(|line| {
            let mut fields = line.split_whitespace();
            (fields.next()? == "archive").then_some(())?;
            Some(FileStamp {
                len: fields.next()?.parse().ok()?,
                modified_ns: fields.next()?.parse().ok()?,
            })
        });
        Some(Self {
            summary: ExtractionSummary { files, bytes },
            source,
            archive,
        })
    }

    fn read(dir: &Path) -> Option<Self> {
        Self::parse(&std::fs::read_to_string(dir.join(COMPLETE_MARKER)).ok()?)
    }
}

/// Extractions this process has already walked and found whole, with the
/// marker's stamp at the time. Extractions live in the data directory, which
/// nothing prunes, so one walk per process is enough; a marker that changes
/// (a new extraction in the same place) is walked again.
static VERIFIED: LazyLock<parking_lot::Mutex<HashMap<PathBuf, FileStamp>>> =
    LazyLock::new(Default::default);

/// Whether `dir` is a finished extraction that is still whole: its marker
/// matches the files actually present. Walks the tree the first time a
/// process asks about an extraction, then trusts the marker.
fn is_complete_extraction(dir: &Path) -> bool {
    let Some(stamp) = FileStamp::of(&dir.join(COMPLETE_MARKER)) else {
        return false;
    };
    if VERIFIED.lock().get(dir) == Some(&stamp) {
        return true;
    }
    let Some(marker) = Marker::read(dir) else {
        return false;
    };
    let whole = tree_summary(dir).is_some_and(|found| found == marker.summary);
    if whole {
        VERIFIED.lock().insert(dir.to_path_buf(), stamp);
    }
    whole
}

/// Forget that `dir` was verified (it is being removed or replaced).
fn forget_verified(dir: &Path) {
    VERIFIED.lock().remove(dir);
}

/// The archive a finished extraction was made from, as recorded in its
/// marker; `None` for an unmarked directory.
pub fn extraction_source(dir: &Path) -> Option<PathBuf> {
    Marker::read(dir)?.source
}

/// The size and modification time the archive had when `dir` was extracted
/// from it, if the marker records them.
pub fn extraction_archive_stamp(dir: &Path) -> Option<FileStamp> {
    Marker::read(dir)?.archive
}

/// How an archive is identified in markers: its canonical path when it can
/// be resolved.
pub fn archive_identity(archive_path: &Path) -> PathBuf {
    std::fs::canonicalize(archive_path).unwrap_or_else(|_| archive_path.to_path_buf())
}

/// Extract `archive_path` so that `final_dir` either holds the complete
/// archive or does not exist, and return a lease that keeps it from being
/// removed while it is in use.
///
/// Files are written to a `.staging-<uuid>` directory beside `final_dir`
/// (same filesystem) and renamed into place once every entry is written. A
/// failure removes the staging directory. An existing `final_dir` whose
/// completion marker matches its files is reused; one that does not match is
/// extracted again, unless another process holds a lease on it.
///
/// The whole check-extract-swap runs under the [`RootLock`] of `final_dir`'s
/// parent.
pub fn extract_archive_atomically(
    archive_path: &Path,
    final_dir: &Path,
    limits: &ExtractionLimits,
) -> Result<ExtractionLease, ArchiveError> {
    let parent = final_dir
        .parent()
        .ok_or_else(|| ArchiveError::Io("extraction directory has no parent".to_string()))?;

    let lock = target_lock(final_dir);
    let _held = lock.lock();
    let _root =
        RootLock::acquire(parent).map_err(|e| io_error("locking the extraction root", e))?;
    let lease = |dir: &Path| {
        ExtractionLease::acquire(dir).map_err(|e| io_error("holding the extraction open", e))
    };
    if let Ok(metadata) = std::fs::symlink_metadata(final_dir) {
        if metadata.is_dir() && is_complete_extraction(final_dir) {
            return lease(final_dir);
        }
        tracing::info!("Re-extracting incomplete skill archive extraction {final_dir:?}");
        let removed = remove_unleased(final_dir)
            .map_err(|e| io_error("removing an incomplete extraction", e))?;
        if !removed {
            return Err(ArchiveError::Io(
                "an incomplete earlier extraction is still in use by a LocalRouter process; \
                 it is extracted again once it is released"
                    .to_string(),
            ));
        }
    }

    let staging = parent.join(format!("{STAGING_PREFIX}{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&staging).map_err(|e| io_error("creating staging directory", e))?;
    let mut guard = StagingGuard(Some(staging.clone()));

    let summary = extract_archive(archive_path, &staging, limits)?;
    let marker = Marker {
        summary,
        source: Some(archive_identity(archive_path)),
        archive: FileStamp::of(archive_path),
    };
    std::fs::write(staging.join(COMPLETE_MARKER), marker.render())
        .map_err(|e| io_error("marking the extraction complete", e))?;

    std::fs::rename(&staging, final_dir)
        .map_err(|e| io_error("moving the extraction into place", e))?;
    guard.0 = None;
    // Just written whole: no need to walk it before its first reuse.
    match FileStamp::of(&final_dir.join(COMPLETE_MARKER)) {
        Some(stamp) => VERIFIED.lock().insert(final_dir.to_path_buf(), stamp),
        None => VERIFIED.lock().remove(final_dir),
    };
    lease(final_dir)
}

/// Remove the extraction `dir` unless some process (this one included) holds
/// a lease on it. Call with the [`RootLock`] held. Returns whether it is gone.
fn remove_unleased(dir: &Path) -> std::io::Result<bool> {
    let metadata = match std::fs::symlink_metadata(dir) {
        Ok(metadata) => metadata,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(true),
        Err(e) => return Err(e),
    };
    if !metadata.is_dir() {
        std::fs::remove_file(dir)?;
        return Ok(true);
    }
    match std::fs::File::open(dir.join(COMPLETE_MARKER)) {
        Ok(marker) => match marker.try_lock() {
            // Leases are only taken under the root lock, which the caller
            // holds, so none can appear between this check and the removal.
            // The handle is closed first: Windows cannot delete an open file.
            Ok(()) => drop(marker),
            Err(std::fs::TryLockError::WouldBlock) => return Ok(false),
            Err(std::fs::TryLockError::Error(e)) => return Err(e),
        },
        // An unmarked tree cannot be leased.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    forget_verified(dir);
    std::fs::remove_dir_all(dir)?;
    Ok(true)
}

/// Remove the extraction `dir` (under `root`) unless a process is still
/// serving skills from it. Takes the root lock. Returns whether it is gone;
/// `false` means "in use, try again later".
pub fn remove_extraction(root: &Path, dir: &Path) -> std::io::Result<bool> {
    let _root = RootLock::acquire(root)?;
    remove_unleased(dir)
}

/// Lease the existing extraction `dir` (under `root`) if it is still whole,
/// e.g. to keep serving an earlier version when an update fails.
pub fn lease_existing(root: &Path, dir: &Path) -> Option<ExtractionLease> {
    let _root = RootLock::acquire(root).ok()?;
    if !is_complete_extraction(dir) {
        return None;
    }
    ExtractionLease::acquire(dir).ok()
}

/// Whether `name` is the folder name of an extraction (`<stem>-<32 hex>`).
pub fn is_extraction_name(name: &str) -> bool {
    name.rsplit_once('-').is_some_and(|(stem, hash)| {
        !stem.is_empty() && hash.len() == 32 && hash.bytes().all(|b| b.is_ascii_hexdigit())
    })
}

/// Extractions under `root`, with the archive each was made from (`None`
/// when the marker is missing or unreadable).
pub fn list_extractions(root: &Path) -> Vec<(PathBuf, Option<PathBuf>)> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|entry| is_extraction_name(&entry.file_name().to_string_lossy()))
        .filter(|entry| entry.file_type().is_ok_and(|t| t.is_dir()))
        .map(|entry| {
            let path = entry.path();
            let source = extraction_source(&path);
            (path, source)
        })
        .collect()
}

/// Remove staging directories left under `root` by a process that died
/// mid-extraction. Extraction runs under the root lock, which this takes, so
/// a staging directory seen here belongs to no running extraction; `min_age`
/// is kept as a second guard. Returns how many were removed.
pub fn cleanup_stale_staging(root: &Path, min_age: Duration) -> usize {
    let Ok(entries) = std::fs::read_dir(root) else {
        return 0;
    };
    let _root = match RootLock::acquire(root) {
        Ok(lock) => lock,
        Err(e) => {
            tracing::warn!("Could not lock {:?} to clean up staging: {}", root, e);
            return 0;
        }
    };
    let now = SystemTime::now();
    let mut removed = 0;
    for entry in entries.flatten() {
        if !entry
            .file_name()
            .to_string_lossy()
            .starts_with(STAGING_PREFIX)
        {
            continue;
        }
        let Ok(metadata) = std::fs::symlink_metadata(entry.path()) else {
            continue;
        };
        if !metadata.is_dir() {
            continue;
        }
        let age = metadata
            .modified()
            .ok()
            .and_then(|modified| now.duration_since(modified).ok())
            .unwrap_or_default();
        if age < min_age {
            continue;
        }
        match std::fs::remove_dir_all(entry.path()) {
            Ok(()) => {
                tracing::info!("Removed stale staging directory {:?}", entry.path());
                removed += 1;
            }
            Err(e) => tracing::warn!(
                "Failed to remove stale staging directory {:?}: {}",
                entry.path(),
                e
            ),
        }
    }
    removed
}

/// Records, in the extraction root, that the old temp location was cleaned.
const MIGRATED_MARKER: &str = ".migrated-from-temp";

/// Remove the extractions and staging folders that earlier builds left in
/// `legacy_root` (`<temp>/localrouter-skills`), once per `root`. Anything
/// else there (the per-PID output folders of asynchronous scripts) is left
/// alone. Returns how many folders were removed.
///
/// Call after the first scan, so the configured archives are already
/// extracted into `root` and served from there.
pub fn migrate_legacy_root(legacy_root: &Path, root: &Path) -> usize {
    let migrated = root.join(MIGRATED_MARKER);
    if migrated.exists() {
        return 0;
    }
    let mut removed = 0;
    if let Ok(entries) = std::fs::read_dir(legacy_root) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if !(name.starts_with(STAGING_PREFIX) || is_extraction_name(&name)) {
                continue;
            }
            if !entry.file_type().is_ok_and(|t| t.is_dir()) {
                continue;
            }
            match std::fs::remove_dir_all(entry.path()) {
                Ok(()) => removed += 1,
                Err(e) => tracing::warn!(
                    "Failed to remove old skill extraction {:?}: {}",
                    entry.path(),
                    e
                ),
            }
        }
    }
    if removed > 0 {
        tracing::info!(
            "Removed {removed} skill archive extractions from the old location {:?}",
            legacy_root
        );
    }
    if let Err(e) = std::fs::create_dir_all(root).and_then(|()| std::fs::write(&migrated, b"")) {
        tracing::warn!("Could not record the skill extraction migration: {}", e);
    }
    removed
}

#[cfg(test)]
pub(crate) mod test_support {
    use std::io::Write;
    use std::path::{Path, PathBuf};
    use zip::write::SimpleFileOptions;

    pub(crate) enum Entry<'a> {
        File(&'a str, &'a [u8]),
        Directory(&'a str),
        Link(&'a str, &'a str),
        /// A file entry carrying the given full Unix mode (type bits included).
        Mode(&'a str, u32),
    }

    pub(crate) fn zip_bytes(entries: &[Entry<'_>]) -> Vec<u8> {
        let mut bytes = Vec::new();
        {
            let mut writer = zip::ZipWriter::new(std::io::Cursor::new(&mut bytes));
            let options = SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated)
                .unix_permissions(0o644);
            for entry in entries {
                match entry {
                    Entry::File(name, contents) => {
                        writer.start_file(*name, options).unwrap();
                        writer.write_all(contents).unwrap();
                    }
                    Entry::Directory(name) => writer.add_directory(*name, options).unwrap(),
                    Entry::Link(name, target) => {
                        writer.add_symlink(*name, *target, options).unwrap()
                    }
                    Entry::Mode(name, _) => {
                        writer.start_file(*name, options).unwrap();
                        writer.write_all(b"x").unwrap();
                    }
                }
            }
            writer.finish().unwrap();
        }
        // The writer only stores permission bits; patch the file type into the
        // central directory's external attributes (offset 38, high 16 bits).
        for entry in entries {
            if let Entry::Mode(name, mode) = entry {
                let header = central_header(&bytes, name);
                bytes[header + 38..header + 42].copy_from_slice(&(mode << 16).to_le_bytes());
            }
        }
        bytes
    }

    /// Offset of the central directory header of `name`.
    fn central_header(bytes: &[u8], name: &str) -> usize {
        (0..bytes.len() - 46)
            .find(|&at| {
                bytes[at..at + 4] == *b"PK\x01\x02" && bytes[at + 46..].starts_with(name.as_bytes())
            })
            .expect("central directory header")
    }

    /// Overwrite the uncompressed size the archive declares for `name`, in
    /// both its local header and the central directory.
    pub(crate) fn declare_size(bytes: &mut [u8], name: &str, declared: u32) {
        let central = central_header(bytes, name);
        bytes[central + 24..central + 28].copy_from_slice(&declared.to_le_bytes());
        let local = u32::from_le_bytes(bytes[central + 42..central + 46].try_into().unwrap());
        let local = local as usize;
        assert_eq!(bytes[local..local + 4], *b"PK\x03\x04");
        bytes[local + 22..local + 26].copy_from_slice(&declared.to_le_bytes());
    }

    pub(crate) fn write_zip(dir: &Path, file_name: &str, entries: &[Entry<'_>]) -> PathBuf {
        let path = dir.join(file_name);
        std::fs::write(&path, zip_bytes(entries)).unwrap();
        path
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::{declare_size, write_zip, zip_bytes, Entry};
    use super::*;
    use tempfile::TempDir;

    fn extract(
        entries: &[Entry<'_>],
        limits: &ExtractionLimits,
    ) -> (TempDir, Result<ExtractionSummary, ArchiveError>) {
        let tmp = TempDir::new().unwrap();
        let archive = write_zip(tmp.path(), "skill.zip", entries);
        let dest = tmp.path().join("out");
        std::fs::create_dir(&dest).unwrap();
        let result = extract_archive(&archive, &dest, limits);
        (tmp, result)
    }

    /// Names under `dir`, without the root lock file.
    fn entries_under(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .filter(|name| name != ROOT_LOCK_FILE)
            .collect();
        names.sort();
        names
    }

    #[test]
    fn extracts_nested_files() {
        let (tmp, result) = extract(
            &[
                Entry::Directory("skill/"),
                Entry::File("skill/SKILL.md", b"---\nname: a\n---\n"),
                Entry::File("skill/scripts/run.sh", b"echo hi"),
            ],
            &ExtractionLimits::default(),
        );
        assert_eq!(
            result.unwrap(),
            ExtractionSummary {
                files: 2,
                bytes: 23
            }
        );
        let out = tmp.path().join("out/skill");
        assert_eq!(
            std::fs::read_to_string(out.join("scripts/run.sh")).unwrap(),
            "echo hi"
        );
        assert!(out.join("SKILL.md").is_file());
    }

    #[test]
    fn budget_counts_written_bytes_when_the_header_lies() {
        // 200 KiB of zeros that the archive declares as 10 bytes.
        let payload = vec![0u8; 200 * 1024];
        let mut bytes = zip_bytes(&[Entry::File("big.bin", &payload)]);
        declare_size(&mut bytes, "big.bin", 10);

        let per_file = ExtractionLimits {
            max_file_bytes: 64 * 1024,
            ..Default::default()
        };
        let total = ExtractionLimits {
            max_total_bytes: 64 * 1024,
            ..Default::default()
        };
        for (limits, expected) in [
            (
                per_file,
                ArchiveError::FileTooLarge {
                    path: "big.bin".to_string(),
                    limit: 64 * 1024,
                },
            ),
            (total, ArchiveError::TotalTooLarge { limit: 64 * 1024 }),
        ] {
            let tmp = TempDir::new().unwrap();
            let archive = tmp.path().join("lying.zip");
            std::fs::write(&archive, &bytes).unwrap();
            let dest = tmp.path().join("out");
            std::fs::create_dir(&dest).unwrap();
            let error = extract_archive(&archive, &dest, &limits).unwrap_err();
            assert_eq!(error, expected);
            // Nothing past the budget reached the disk.
            let written = std::fs::metadata(dest.join("big.bin")).unwrap().len();
            assert!(written <= 64 * 1024, "wrote {written} bytes");
        }
    }

    #[test]
    fn declared_oversize_is_rejected_before_writing() {
        let (tmp, result) = extract(
            &[Entry::File("big.bin", &[7u8; 4096])],
            &ExtractionLimits {
                max_file_bytes: 1024,
                ..Default::default()
            },
        );
        let error = result.unwrap_err();
        assert_eq!(
            error.to_string(),
            "'big.bin' is larger than the 1 KiB limit for one file"
        );
        assert!(!tmp.path().join("out/big.bin").exists());
    }

    #[test]
    fn total_budget_spans_files() {
        let chunk = [1u8; 600];
        let (_tmp, result) = extract(
            &[Entry::File("a", &chunk), Entry::File("b", &chunk)],
            &ExtractionLimits {
                max_total_bytes: 1000,
                ..Default::default()
            },
        );
        assert_eq!(
            result.unwrap_err(),
            ArchiveError::TotalTooLarge { limit: 1000 }
        );
    }

    #[test]
    fn expansion_ratio_is_enforced_past_the_floor() {
        let payload = vec![0u8; 512 * 1024];
        let limits = ExtractionLimits {
            max_expansion_ratio: 2,
            expansion_floor_bytes: 64 * 1024,
            ..Default::default()
        };
        let (_tmp, result) = extract(&[Entry::File("zeros", &payload)], &limits);
        assert_eq!(
            result.unwrap_err(),
            ArchiveError::ExpansionRatio { limit: 2 }
        );

        // Below the floor the same ratio is tolerated.
        let (_tmp, result) = extract(
            &[Entry::File("zeros", &payload)],
            &ExtractionLimits {
                expansion_floor_bytes: 1024 * 1024,
                ..limits
            },
        );
        assert_eq!(result.unwrap().bytes, 512 * 1024);
    }

    #[test]
    fn entry_count_is_limited() {
        let (tmp, result) = extract(
            &[
                Entry::File("a", b"1"),
                Entry::File("b", b"2"),
                Entry::File("c", b"3"),
            ],
            &ExtractionLimits {
                max_entries: 2,
                ..Default::default()
            },
        );
        let error = result.unwrap_err();
        assert_eq!(error, ArchiveError::TooManyEntries { count: 3, limit: 2 });
        assert_eq!(
            error.to_string(),
            "the archive lists 3 entries; the limit is 2"
        );
        assert!(entries_under(&tmp.path().join("out")).is_empty());
    }

    #[test]
    fn traversal_and_non_portable_paths_are_rejected() {
        for name in [
            "../evil",
            "skill/../../evil",
            "/etc/evil",
            "C:/evil",
            "skill/file:stream",
            r"skill\..\..\evil",
            "skill//evil",
            "./evil",
            "skill/./evil",
            // Windows device names and names Windows would silently change.
            "skill/CON",
            "skill/nul.txt",
            "COM1/evil",
            "skill/evil.",
            "skill/evil ",
        ] {
            let (tmp, result) = extract(
                &[Entry::File("ok.txt", b"ok"), Entry::File(name, b"evil")],
                &ExtractionLimits::default(),
            );
            let error = result.unwrap_err();
            assert!(
                matches!(error, ArchiveError::UnsafePath { .. }),
                "{name:?} gave {error:?}"
            );
            assert!(!tmp.path().join("evil").exists(), "{name:?} escaped");
            assert_eq!(
                entries_under(tmp.path()),
                ["out", "skill.zip"],
                "{name:?} wrote outside the destination"
            );
        }
    }

    #[test]
    fn depth_and_length_are_limited() {
        let limits = ExtractionLimits {
            max_path_depth: 3,
            max_path_bytes: 40,
            max_component_bytes: 20,
            ..Default::default()
        };
        let (_tmp, result) = extract(&[Entry::File("a/b/c/d", b"x")], &limits);
        assert!(matches!(
            result.unwrap_err(),
            ArchiveError::PathTooDeep { limit: 3, .. }
        ));
        let (_tmp, result) = extract(&[Entry::File(&"n".repeat(21), b"x")], &limits);
        assert!(matches!(
            result.unwrap_err(),
            ArchiveError::PathTooLong { limit: 20, .. }
        ));
        let long = format!("{0}/{0}/{0}", "n".repeat(15));
        let (_tmp, result) = extract(&[Entry::File(&long, b"x")], &limits);
        assert!(matches!(
            result.unwrap_err(),
            ArchiveError::PathTooLong { limit: 40, .. }
        ));
        let (_tmp, result) = extract(&[Entry::File("a/b/c", b"x")], &limits);
        assert!(result.is_ok());
    }

    #[test]
    fn links_are_rejected() {
        let (tmp, result) = extract(
            &[
                Entry::File("SKILL.md", b"x"),
                Entry::Link("references", "/etc"),
            ],
            &ExtractionLimits::default(),
        );
        assert_eq!(
            result.unwrap_err(),
            ArchiveError::Link {
                path: "references".to_string()
            }
        );
        assert!(std::fs::symlink_metadata(tmp.path().join("out/references")).is_err());
    }

    #[test]
    fn special_files_are_rejected() {
        // FIFO, character device, block device, socket.
        for file_type in [0o010000, 0o020000, 0o060000, 0o140000] {
            let (_tmp, result) = extract(
                &[Entry::Mode("device", file_type | 0o644)],
                &ExtractionLimits::default(),
            );
            assert_eq!(
                result.unwrap_err(),
                ArchiveError::SpecialFile {
                    path: "device".to_string()
                },
                "type {file_type:o}"
            );
        }
        let (_tmp, result) = extract(
            &[Entry::Mode("regular", S_IFREG | 0o755)],
            &ExtractionLimits::default(),
        );
        assert!(result.is_ok());
    }

    #[test]
    fn colliding_entries_are_rejected() {
        for entries in [
            // Same file apart from letter case.
            vec![
                Entry::File("Readme.md", b"a"),
                Entry::File("README.md", b"b"),
            ],
            // Same folder apart from letter case.
            vec![Entry::File("Docs/a", b"a"), Entry::File("docs/b", b"b")],
            // A file used as a folder, in either order.
            vec![Entry::File("a", b"a"), Entry::File("a/b", b"b")],
            vec![Entry::File("a/b", b"b"), Entry::File("A", b"a")],
            vec![Entry::File("a", b"a"), Entry::Directory("a/")],
        ] {
            let (_tmp, result) = extract(&entries, &ExtractionLimits::default());
            assert!(
                matches!(result, Err(ArchiveError::Collision { .. })),
                "got {result:?}"
            );
        }
        // Repeating a folder with the same spelling is harmless.
        let (_tmp, result) = extract(
            &[
                Entry::Directory("a/"),
                Entry::File("a/b", b"b"),
                Entry::File("a/c", b"c"),
            ],
            &ExtractionLimits::default(),
        );
        assert_eq!(result.unwrap().files, 2);
    }

    #[test]
    fn garbage_is_not_an_archive() {
        let tmp = TempDir::new().unwrap();
        let archive = tmp.path().join("skill.zip");
        std::fs::write(&archive, b"this is not a zip").unwrap();
        let error =
            extract_archive(&archive, tmp.path(), &ExtractionLimits::default()).unwrap_err();
        assert!(matches!(error, ArchiveError::Invalid(_)));
    }

    #[test]
    fn atomic_extraction_leaves_nothing_behind_on_failure() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("root");
        // Valid files first, so the failure happens mid-extraction.
        let archive = write_zip(
            tmp.path(),
            "skill.zip",
            &[
                Entry::File("SKILL.md", b"---\nname: a\n---\n"),
                Entry::File("scripts/run.sh", b"echo hi"),
                Entry::File("../evil", b"evil"),
            ],
        );
        let final_dir = root.join("skill-abc");
        let error = extract_archive_atomically(&archive, &final_dir, &ExtractionLimits::default())
            .unwrap_err();
        assert!(matches!(error, ArchiveError::UnsafePath { .. }));
        assert!(!final_dir.exists());
        assert!(entries_under(&root).is_empty(), "staging was left behind");
    }

    #[test]
    fn atomic_extraction_promotes_a_complete_tree_once() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("root");
        let archive = write_zip(tmp.path(), "skill.zip", &[Entry::File("SKILL.md", b"one")]);
        let final_dir = root.join("skill-abc");
        extract_archive_atomically(&archive, &final_dir, &ExtractionLimits::default()).unwrap();
        assert_eq!(entries_under(&root), ["skill-abc"]);
        assert_eq!(
            std::fs::read_to_string(final_dir.join("SKILL.md")).unwrap(),
            "one"
        );

        // An existing final directory is a finished extraction: not redone.
        let other = write_zip(tmp.path(), "other.zip", &[Entry::File("SKILL.md", b"two")]);
        extract_archive_atomically(&other, &final_dir, &ExtractionLimits::default()).unwrap();
        assert_eq!(
            std::fs::read_to_string(final_dir.join("SKILL.md")).unwrap(),
            "one"
        );
    }

    #[test]
    fn pruned_or_unmarked_extraction_is_extracted_again() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("root");
        let archive = write_zip(
            tmp.path(),
            "skill.zip",
            &[
                Entry::File("SKILL.md", b"one"),
                Entry::File("scripts/run.sh", b"echo hi"),
            ],
        );
        let final_dir = root.join("skill-abc");
        let limits = ExtractionLimits::default();
        drop(extract_archive_atomically(&archive, &final_dir, &limits).unwrap());
        let stamp = FileStamp::of(&archive).unwrap();
        assert_eq!(
            std::fs::read_to_string(final_dir.join(COMPLETE_MARKER)).unwrap(),
            format!(
                "2 10\n{}\narchive {} {}\n",
                archive_identity(&archive).display(),
                stamp.len,
                stamp.modified_ns
            )
        );
        assert_eq!(
            extraction_source(&final_dir),
            Some(archive_identity(&archive))
        );
        assert_eq!(extraction_archive_stamp(&final_dir), Some(stamp));

        // A file went missing. A process that already verified the tree keeps
        // trusting it; a new process (simulated by forgetting) walks it again.
        std::fs::remove_file(final_dir.join("scripts/run.sh")).unwrap();
        assert!(is_complete_extraction(&final_dir));
        forget_verified(&final_dir);
        assert!(!is_complete_extraction(&final_dir));
        drop(extract_archive_atomically(&archive, &final_dir, &limits).unwrap());
        assert!(final_dir.join("scripts/run.sh").is_file());

        // A file shrank in place.
        std::fs::write(final_dir.join("SKILL.md"), b"").unwrap();
        forget_verified(&final_dir);
        drop(extract_archive_atomically(&archive, &final_dir, &limits).unwrap());
        assert_eq!(
            std::fs::read_to_string(final_dir.join("SKILL.md")).unwrap(),
            "one"
        );

        // Written by a version without markers.
        std::fs::remove_file(final_dir.join(COMPLETE_MARKER)).unwrap();
        drop(extract_archive_atomically(&archive, &final_dir, &limits).unwrap());
        assert!(is_complete_extraction(&final_dir));
        assert_eq!(entries_under(&root), ["skill-abc"]);
    }

    #[test]
    fn markers_without_an_archive_stamp_still_parse() {
        let marker = Marker::parse("3 42\n/a/b.zip\n").unwrap();
        assert_eq!(
            marker.summary,
            ExtractionSummary {
                files: 3,
                bytes: 42
            }
        );
        assert_eq!(marker.source, Some(PathBuf::from("/a/b.zip")));
        assert_eq!(marker.archive, None);
        assert_eq!(Marker::parse("3 42\n").unwrap().source, None);
        assert!(Marker::parse("3\n").is_none());
        assert!(Marker::parse("3 42 7\n").is_none());
        let full = Marker {
            summary: ExtractionSummary { files: 1, bytes: 2 },
            source: Some(PathBuf::from("/x.zip")),
            archive: Some(FileStamp {
                len: 9,
                modified_ns: 1_700_000_000_123_456_789,
            }),
        };
        assert_eq!(Marker::parse(&full.render()), Some(full));
    }

    #[test]
    fn a_leased_extraction_is_not_removed() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("root");
        let archive = write_zip(tmp.path(), "skill.zip", &[Entry::File("SKILL.md", b"one")]);
        let final_dir = root.join("skill-abc");
        let lease =
            extract_archive_atomically(&archive, &final_dir, &ExtractionLimits::default()).unwrap();
        assert_eq!(lease.dir(), final_dir);

        assert!(!remove_extraction(&root, &final_dir).unwrap());
        assert!(final_dir.join("SKILL.md").is_file());
        // A second lease (another scan, another process) is compatible.
        let second = lease_existing(&root, &final_dir).expect("shared lease");
        drop(lease);
        assert!(!remove_extraction(&root, &final_dir).unwrap());
        drop(second);
        assert!(remove_extraction(&root, &final_dir).unwrap());
        assert!(!final_dir.exists());
        // Already gone counts as removed.
        assert!(remove_extraction(&root, &final_dir).unwrap());
    }

    #[test]
    fn a_damaged_extraction_in_use_is_not_replaced_under_its_reader() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("root");
        let archive = write_zip(
            tmp.path(),
            "skill.zip",
            &[
                Entry::File("SKILL.md", b"one"),
                Entry::File("scripts/run.sh", b"echo hi"),
            ],
        );
        let final_dir = root.join("skill-abc");
        let limits = ExtractionLimits::default();
        let reader = extract_archive_atomically(&archive, &final_dir, &limits).unwrap();
        std::fs::remove_file(final_dir.join("scripts/run.sh")).unwrap();
        forget_verified(&final_dir);

        let error = extract_archive_atomically(&archive, &final_dir, &limits).unwrap_err();
        assert!(error.to_string().contains("still in use"), "{error}");
        assert!(final_dir.join("SKILL.md").is_file());

        drop(reader);
        drop(extract_archive_atomically(&archive, &final_dir, &limits).unwrap());
        assert!(final_dir.join("scripts/run.sh").is_file());
    }

    /// Run by [`a_lease_held_by_another_process_blocks_removal`] in a child
    /// process: lease the extraction named by the environment, say so, and
    /// hold it until told to let go.
    #[test]
    #[ignore = "helper run in a child process"]
    fn hold_lease_in_child_process() {
        let Ok(dir) = std::env::var("LR_SKILLS_LEASE_DIR") else {
            return;
        };
        let dir = PathBuf::from(dir);
        let root = dir.parent().unwrap().to_path_buf();
        let _lease = lease_existing(&root, &dir).expect("lease in child");
        std::fs::write(root.join("child-ready"), b"").unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        while !root.join("child-release").exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn a_lease_held_by_another_process_blocks_removal() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("root");
        let archive = write_zip(tmp.path(), "skill.zip", &[Entry::File("SKILL.md", b"one")]);
        let final_dir = root.join("skill-abc");
        drop(
            extract_archive_atomically(&archive, &final_dir, &ExtractionLimits::default()).unwrap(),
        );

        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "archive::tests::hold_lease_in_child_process",
                "--ignored",
                "--test-threads=1",
            ])
            .env("LR_SKILLS_LEASE_DIR", &final_dir)
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        while !root.join("child-ready").exists() {
            assert!(std::time::Instant::now() < deadline, "child never leased");
            std::thread::sleep(Duration::from_millis(10));
        }

        assert!(!remove_extraction(&root, &final_dir).unwrap());
        assert!(final_dir.join("SKILL.md").is_file());

        std::fs::write(root.join("child-release"), b"").unwrap();
        assert!(child.wait().unwrap().success());
        assert!(remove_extraction(&root, &final_dir).unwrap());
    }

    #[test]
    fn the_root_lock_is_exclusive() {
        let tmp = TempDir::new().unwrap();
        let held = RootLock::acquire(tmp.path()).unwrap();
        let other = std::fs::File::open(tmp.path().join(ROOT_LOCK_FILE)).unwrap();
        assert!(matches!(
            other.try_lock(),
            Err(std::fs::TryLockError::WouldBlock)
        ));
        drop(held);
        other.try_lock().unwrap();
    }

    #[test]
    fn extraction_names_are_recognized() {
        let hash = "0123456789abcdef0123456789abcdef";
        assert!(is_extraction_name(&format!("pack-{hash}")));
        assert!(is_extraction_name(&format!("my-pack-{hash}")));
        assert!(!is_extraction_name(&format!("-{hash}")));
        assert!(!is_extraction_name("12345"));
        assert!(!is_extraction_name(&format!("{STAGING_PREFIX}x")));
        assert!(!is_extraction_name(&format!("pack-{}", &hash[1..])));
    }

    #[test]
    fn the_old_temp_location_is_cleaned_once() {
        let tmp = TempDir::new().unwrap();
        let legacy = tmp.path().join("legacy");
        let root = tmp.path().join("root");
        let hash = "0123456789abcdef0123456789abcdef";
        std::fs::create_dir_all(legacy.join(format!("pack-{hash}/scripts"))).unwrap();
        std::fs::create_dir_all(legacy.join(format!("{STAGING_PREFIX}dead"))).unwrap();
        // Output of a running asynchronous script, and a stray file.
        std::fs::create_dir_all(legacy.join("4242")).unwrap();
        std::fs::write(legacy.join(format!("loose-{hash}")), b"file").unwrap();

        assert_eq!(migrate_legacy_root(&legacy, &root), 2);
        assert_eq!(
            entries_under(&legacy),
            ["4242".to_string(), format!("loose-{hash}")]
        );
        assert!(root.join(MIGRATED_MARKER).is_file());

        // Once only: a later extraction there (an older build still running)
        // is not touched again.
        std::fs::create_dir_all(legacy.join(format!("pack-{hash}"))).unwrap();
        assert_eq!(migrate_legacy_root(&legacy, &root), 0);
        assert!(legacy.join(format!("pack-{hash}")).is_dir());

        // A missing old location is fine.
        let fresh = tmp.path().join("fresh");
        assert_eq!(migrate_legacy_root(&tmp.path().join("none"), &fresh), 0);
        assert!(fresh.join(MIGRATED_MARKER).is_file());
    }

    #[test]
    fn the_marker_name_is_reserved() {
        for name in [COMPLETE_MARKER, ".LocalRouter-Extraction/x"] {
            let (_tmp, result) =
                extract(&[Entry::File(name, b"1 1")], &ExtractionLimits::default());
            assert!(
                matches!(result, Err(ArchiveError::UnsafePath { .. })),
                "{name}: {result:?}"
            );
        }
    }

    #[test]
    fn concurrent_extractions_of_one_archive_agree() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("root");
        let payload = vec![b'a'; 64 * 1024];
        let archive = write_zip(
            tmp.path(),
            "skill.zip",
            &[
                Entry::File("SKILL.md", b"x"),
                Entry::File("assets/data", &payload),
            ],
        );
        let final_dir = root.join("skill-abc");
        std::thread::scope(|scope| {
            for _ in 0..4 {
                scope.spawn(|| {
                    let _lease = extract_archive_atomically(
                        &archive,
                        &final_dir,
                        &ExtractionLimits::default(),
                    )
                    .unwrap();
                    // Whoever returns sees the whole tree.
                    assert_eq!(
                        std::fs::metadata(final_dir.join("assets/data"))
                            .unwrap()
                            .len(),
                        64 * 1024
                    );
                });
            }
        });
        assert_eq!(entries_under(&root), ["skill-abc"]);
    }

    #[test]
    fn stale_staging_directories_are_removed() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let stale = root.join(format!("{STAGING_PREFIX}dead"));
        std::fs::create_dir(&stale).unwrap();
        std::fs::write(stale.join("partial"), "half").unwrap();
        std::fs::create_dir(root.join("skill-abc")).unwrap();
        std::fs::write(root.join(format!("{STAGING_PREFIX}file")), "not a dir").unwrap();

        // Too young to be considered abandoned.
        assert_eq!(cleanup_stale_staging(root, Duration::from_secs(3600)), 0);
        assert!(stale.exists());

        assert_eq!(cleanup_stale_staging(root, Duration::ZERO), 1);
        assert!(!stale.exists());
        assert!(root.join("skill-abc").is_dir());
        assert!(root.join(format!("{STAGING_PREFIX}file")).is_file());

        assert_eq!(
            cleanup_stale_staging(&root.join("missing"), Duration::ZERO),
            0
        );
    }
}
