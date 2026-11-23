//! Encrypted disk spill for request bytes the proxy has to hold before it may
//! forward them.
//!
//! A [`SpillBuffer`] is an append-only byte log addressed by absolute offset.
//! It keeps the newest bytes in RAM up to a threshold; older bytes move to an
//! anonymous temporary file. The held bytes can contain secrets that were not
//! yet masked, so nothing is written in the clear: every 64 KiB chunk is
//! sealed with ChaCha20-Poly1305 under a random per-buffer key that exists only
//! in memory (zeroized on drop), with a counter nonce that is never reused and
//! the chunk's position as associated data, so a chunk cannot be replayed at
//! another offset.
//!
//! The file is created with `tempfile::tempfile_in` in a 0700 directory: on
//! Unix it is unlinked at once, on Windows it is deleted when closed, so a
//! crash leaves nothing readable behind (and [`SpillConfig::cleanup_stale`]
//! removes whatever a crash on a platform without unlinked files left). Disk
//! use is bounded per buffer ([`SpillConfig::request_cap`]) and across all
//! buffers ([`SpillConfig::global_budget`]); exceeding either is a
//! [`SpillError`] with status 413. Slots of discarded chunks are reused, so a
//! long body that streams through costs only what is held at once.

use std::collections::VecDeque;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, CHACHA20_POLY1305};
use zeroize::{Zeroize, Zeroizing};

/// Plaintext bytes per chunk.
pub const CHUNK: usize = 64 * 1024;
const TAG: usize = 16;
/// Bytes one chunk occupies on disk.
const SLOT: u64 = (CHUNK + TAG) as u64;
/// Bytes a buffer keeps in RAM before it spills.
pub const DEFAULT_RAM_LIMIT: usize = 64 * 1024 * 1024;
/// Most bytes one buffer holds at once.
pub const DEFAULT_REQUEST_CAP: u64 = 2 * 1024 * 1024 * 1024;
/// Most bytes all buffers keep on disk together.
pub const DEFAULT_GLOBAL_BUDGET: u64 = 4 * 1024 * 1024 * 1024;

/// Why a [`SpillBuffer`] refused bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpillError {
    /// The buffer would hold more than its cap.
    RequestCap(u64),
    /// All buffers together would hold more on disk than the global budget.
    Budget,
    /// The spill file could not be created, written or read.
    Io(String),
    /// A chunk read back failed authentication.
    Corrupt,
}

impl SpillError {
    pub fn status(&self) -> u16 {
        match self {
            Self::RequestCap(_) | Self::Budget => 413,
            Self::Io(_) | Self::Corrupt => 500,
        }
    }

    pub fn message(&self) -> String {
        match self {
            Self::RequestCap(cap) => format!(
                "request body exceeds LocalRouter's {} MiB proxy limit for content it cannot yet forward",
                cap / (1024 * 1024)
            ),
            Self::Budget => {
                "LocalRouter is holding too much request content on disk; retry later".into()
            }
            Self::Io(error) => format!("request spill storage failed: {error}"),
            Self::Corrupt => "request spill storage returned corrupt data".into(),
        }
    }
}

impl std::fmt::Display for SpillError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message())
    }
}

impl std::error::Error for SpillError {}

/// Where and how much request content may spill. Shared by every buffer of
/// one proxy; [`in_flight`](Self::in_flight) is what they hold on disk now.
#[derive(Debug)]
pub struct SpillConfig {
    dir: PathBuf,
    ram_limit: usize,
    request_cap: u64,
    global_budget: u64,
    used: AtomicU64,
}

impl SpillConfig {
    pub fn new(dir: PathBuf) -> Self {
        Self::with_limits(
            dir,
            DEFAULT_RAM_LIMIT,
            DEFAULT_REQUEST_CAP,
            DEFAULT_GLOBAL_BUDGET,
        )
    }

    pub fn with_limits(
        dir: PathBuf,
        ram_limit: usize,
        request_cap: u64,
        global_budget: u64,
    ) -> Self {
        Self {
            dir,
            ram_limit,
            request_cap,
            global_budget,
            used: AtomicU64::new(0),
        }
    }

    /// The proxy's spill directory under the app's data directory.
    pub fn default_dir() -> PathBuf {
        lr_utils::paths::config_dir()
            .unwrap_or_else(|_| std::env::temp_dir().join("localrouter"))
            .join("proxy")
            .join("spill")
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn ram_limit(&self) -> usize {
        self.ram_limit
    }

    pub fn request_cap(&self) -> u64 {
        self.request_cap
    }

    pub fn global_budget(&self) -> u64 {
        self.global_budget
    }

    /// Bytes every buffer of this configuration keeps on disk right now.
    pub fn in_flight(&self) -> u64 {
        self.used.load(Ordering::Acquire)
    }

    fn reserve(&self, bytes: u64) -> bool {
        let mut used = self.used.load(Ordering::Acquire);
        loop {
            let Some(total) = used
                .checked_add(bytes)
                .filter(|total| *total <= self.global_budget)
            else {
                return false;
            };
            match self
                .used
                .compare_exchange_weak(used, total, Ordering::AcqRel, Ordering::Acquire)
            {
                Ok(_) => return true,
                Err(current) => used = current,
            }
        }
    }

    fn release(&self, bytes: u64) {
        self.used.fetch_sub(bytes, Ordering::AcqRel);
    }

    /// Create the spill directory with owner-only permissions.
    fn ensure_dir(&self) -> std::io::Result<()> {
        let mut builder = std::fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(&self.dir)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&self.dir, std::fs::Permissions::from_mode(0o700))?;
        }
        Ok(())
    }

    /// Remove files a crashed process left in the spill directory. Files
    /// of a running buffer are already unlinked (Unix) or open for delete on
    /// close (Windows), so this never touches live data. Returns the number
    /// of files removed.
    pub fn cleanup_stale(&self) -> usize {
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return 0;
        };
        let mut removed = 0;
        for entry in entries.flatten() {
            let is_file = entry.file_type().map(|t| t.is_file()).unwrap_or(false);
            if is_file && std::fs::remove_file(entry.path()).is_ok() {
                removed += 1;
            }
        }
        removed
    }
}

enum Chunk {
    Ram(Vec<u8>),
    Disk { slot: u64, nonce: u64, len: usize },
}

/// The per-buffer key, generated at the first spill.
struct Key(Zeroizing<[u8; 32]>);

impl Key {
    fn generate() -> Result<Self, SpillError> {
        use ring::rand::SecureRandom;
        let mut bytes = Zeroizing::new([0u8; 32]);
        ring::rand::SystemRandom::new()
            .fill(&mut bytes[..])
            .map_err(|_| SpillError::Io("no system randomness".into()))?;
        Ok(Self(bytes))
    }

    /// A ring key for one operation; the long-lived copy stays in the
    /// zeroizing buffer.
    fn aead(&self) -> LessSafeKey {
        LessSafeKey::new(
            UnboundKey::new(&CHACHA20_POLY1305, &self.0[..]).expect("32-byte ChaCha20 key"),
        )
    }
}

fn nonce(counter: u64) -> Nonce {
    let mut bytes = [0u8; 12];
    bytes[4..].copy_from_slice(&counter.to_be_bytes());
    Nonce::assume_unique_for_key(bytes)
}

/// An append-only byte log held in RAM up to a threshold and beyond it in an
/// encrypted temporary file. See the module documentation.
pub struct SpillBuffer {
    config: Option<Arc<SpillConfig>>,
    ram_limit: usize,
    cap: u64,
    /// Absolute offset of the first byte of `chunks[0]`, a multiple of
    /// [`CHUNK`].
    start: u64,
    /// One past the last byte appended.
    end: u64,
    chunks: VecDeque<Chunk>,
    /// Index of the first chunk that may still be in RAM.
    ram_from: usize,
    ram: usize,
    file: Option<File>,
    key: Option<Key>,
    free: Vec<u64>,
    slots: u64,
    nonces: u64,
    max_live: u64,
    spilled: u64,
}

impl std::fmt::Debug for SpillBuffer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SpillBuffer")
            .field("start", &self.start)
            .field("end", &self.end)
            .field("ram", &self.ram)
            .field("slots", &self.slots)
            .finish_non_exhaustive()
    }
}

impl SpillBuffer {
    /// A buffer that spills under `config`.
    pub fn new(config: Arc<SpillConfig>) -> Self {
        let (ram_limit, cap) = (config.ram_limit, config.request_cap);
        Self::build(Some(config), ram_limit, cap)
    }

    /// A buffer that never spills and holds at most `cap` bytes.
    pub fn ram_only(cap: u64) -> Self {
        Self::build(None, usize::MAX, cap)
    }

    fn build(config: Option<Arc<SpillConfig>>, ram_limit: usize, cap: u64) -> Self {
        Self {
            config,
            ram_limit,
            cap,
            start: 0,
            end: 0,
            chunks: VecDeque::new(),
            ram_from: 0,
            ram: 0,
            file: None,
            key: None,
            free: Vec::new(),
            slots: 0,
            nonces: 0,
            max_live: 0,
            spilled: 0,
        }
    }

    /// One past the last byte appended (the total ever appended).
    pub fn end(&self) -> u64 {
        self.end
    }

    /// First offset still held.
    pub fn start(&self) -> u64 {
        self.start
    }

    /// Bytes held (including the discarded part of the first chunk).
    pub fn held(&self) -> u64 {
        self.end - self.start
    }

    /// Most bytes held at once.
    pub fn max_held(&self) -> u64 {
        self.max_live
    }

    /// Bytes held in RAM.
    pub fn ram_bytes(&self) -> usize {
        self.ram
    }

    /// Bytes ever written to disk.
    pub fn spilled_bytes(&self) -> u64 {
        self.spilled
    }

    /// Bytes this buffer occupies on disk now.
    pub fn disk_bytes(&self) -> u64 {
        self.slots * SLOT
    }

    pub fn is_empty(&self) -> bool {
        self.end == self.start
    }

    /// Append bytes. A [`SpillError::RequestCap`] appends nothing; after any
    /// other error the buffer is unusable and the request must be aborted.
    pub fn append(&mut self, mut data: &[u8]) -> Result<(), SpillError> {
        if self.held() + data.len() as u64 > self.cap {
            return Err(SpillError::RequestCap(self.cap));
        }
        while !data.is_empty() {
            let fill = match self.chunks.back_mut() {
                Some(Chunk::Ram(last)) if last.len() < CHUNK => last,
                _ => {
                    self.chunks.push_back(Chunk::Ram(Vec::with_capacity(CHUNK)));
                    match self.chunks.back_mut() {
                        Some(Chunk::Ram(last)) => last,
                        _ => unreachable!("just pushed a RAM chunk"),
                    }
                }
            };
            let take = (CHUNK - fill.len()).min(data.len());
            fill.extend_from_slice(&data[..take]);
            self.ram += take;
            self.end += take as u64;
            data = &data[take..];
        }
        self.max_live = self.max_live.max(self.held());
        self.spill()
    }

    /// Move the oldest full RAM chunks to disk until RAM is within its limit.
    fn spill(&mut self) -> Result<(), SpillError> {
        while self.ram > self.ram_limit {
            let Some(config) = self.config.clone() else {
                return Ok(());
            };
            // Never the last chunk: it is still being filled.
            let last = self.chunks.len().saturating_sub(1);
            while self.ram_from < last && matches!(self.chunks[self.ram_from], Chunk::Disk { .. }) {
                self.ram_from += 1;
            }
            if self.ram_from >= last {
                return Ok(());
            }
            let index = self.ram_from;
            let slot = match self.free.pop() {
                Some(slot) => slot,
                None => {
                    if !config.reserve(SLOT) {
                        return Err(SpillError::Budget);
                    }
                    self.slots += 1;
                    self.slots - 1
                }
            };
            let absolute = self.start / CHUNK as u64 + index as u64;
            let counter = self.nonces;
            self.nonces += 1;
            let Chunk::Ram(plain) = std::mem::replace(
                &mut self.chunks[index],
                Chunk::Disk {
                    slot,
                    nonce: counter,
                    len: 0,
                },
            ) else {
                unreachable!("checked above");
            };
            let len = plain.len();
            let mut sealed = plain;
            let written = self.write_slot(&config, slot, counter, absolute, &mut sealed);
            sealed.zeroize();
            // On failure the chunk is lost; the buffer is unusable from here
            // on, so the caller aborts the request.
            written?;
            self.chunks[index] = Chunk::Disk {
                slot,
                nonce: counter,
                len,
            };
            self.ram -= len;
            self.spilled += len as u64;
            self.ram_from += 1;
        }
        Ok(())
    }

    fn write_slot(
        &mut self,
        config: &SpillConfig,
        slot: u64,
        counter: u64,
        absolute: u64,
        buffer: &mut Vec<u8>,
    ) -> Result<(), SpillError> {
        if self.key.is_none() {
            self.key = Some(Key::generate()?);
        }
        if self.file.is_none() {
            config
                .ensure_dir()
                .map_err(|e| SpillError::Io(e.to_string()))?;
            self.file = Some(
                tempfile::tempfile_in(config.dir()).map_err(|e| SpillError::Io(e.to_string()))?,
            );
        }
        let key = self.key.as_ref().expect("key created above").aead();
        key.seal_in_place_append_tag(nonce(counter), Aad::from(absolute.to_be_bytes()), buffer)
            .map_err(|_| SpillError::Corrupt)?;
        let file = self.file.as_mut().expect("file created above");
        file.seek(SeekFrom::Start(slot * SLOT))
            .and_then(|_| file.write_all(buffer))
            .map_err(|e| SpillError::Io(e.to_string()))
    }

    /// The plaintext of chunk `index`, appended to `out`.
    fn chunk_into(&mut self, index: usize, out: &mut Vec<u8>) -> Result<(), SpillError> {
        let absolute = self.start / CHUNK as u64 + index as u64;
        match &self.chunks[index] {
            Chunk::Ram(bytes) => {
                out.extend_from_slice(bytes);
                Ok(())
            }
            &Chunk::Disk {
                slot,
                nonce: n,
                len,
            } => {
                let file = self
                    .file
                    .as_mut()
                    .ok_or_else(|| SpillError::Io("spill file missing".into()))?;
                let mut sealed = vec![0u8; len + TAG];
                file.seek(SeekFrom::Start(slot * SLOT))
                    .and_then(|_| file.read_exact(&mut sealed))
                    .map_err(|e| SpillError::Io(e.to_string()))?;
                let key = self.key.as_ref().ok_or(SpillError::Corrupt)?.aead();
                let plain = key
                    .open_in_place(nonce(n), Aad::from(absolute.to_be_bytes()), &mut sealed)
                    .map_err(|_| SpillError::Corrupt)?;
                out.extend_from_slice(plain);
                sealed.zeroize();
                Ok(())
            }
        }
    }

    /// Append the bytes of `[from, to)` to `out`. The range must be held.
    pub fn read_into(&mut self, from: u64, to: u64, out: &mut Vec<u8>) -> Result<(), SpillError> {
        assert!(
            self.start <= from && from <= to && to <= self.end,
            "spill range {from}..{to} outside {}..{}",
            self.start,
            self.end
        );
        if from == to {
            return Ok(());
        }
        out.reserve((to - from) as usize);
        let first = ((from - self.start) / CHUNK as u64) as usize;
        let last = ((to - 1 - self.start) / CHUNK as u64) as usize;
        let mut scratch = Vec::new();
        for index in first..=last {
            let chunk_start = self.start + index as u64 * CHUNK as u64;
            let lo = from.max(chunk_start) - chunk_start;
            let hi = to.min(chunk_start + CHUNK as u64) - chunk_start;
            if let Chunk::Ram(bytes) = &self.chunks[index] {
                out.extend_from_slice(&bytes[lo as usize..hi as usize]);
                continue;
            }
            scratch.clear();
            self.chunk_into(index, &mut scratch)?;
            out.extend_from_slice(&scratch[lo as usize..hi as usize]);
        }
        scratch.zeroize();
        Ok(())
    }

    /// The bytes of `[from, to)` as one vector.
    pub fn read(&mut self, from: u64, to: u64) -> Result<Vec<u8>, SpillError> {
        let mut out = Vec::new();
        self.read_into(from, to, &mut out)?;
        Ok(out)
    }

    /// A reader over `[from, to)` that decrypts one chunk at a time, for
    /// ranges too large to copy at once.
    pub fn reader(&mut self, from: u64, to: u64) -> SpillReader<'_> {
        assert!(self.start <= from && from <= to && to <= self.end);
        SpillReader {
            buffer: self,
            next: from,
            to,
            current: Vec::new(),
            offset: 0,
        }
    }

    /// Forget everything before `offset`. Whole chunks are freed; their disk
    /// slots are reused, and once no chunk is on disk the file is emptied.
    pub fn discard_before(&mut self, offset: u64) {
        let offset = offset.min(self.end);
        while self.start + CHUNK as u64 <= offset {
            let Some(chunk) = self.chunks.pop_front() else {
                break;
            };
            match chunk {
                Chunk::Ram(mut bytes) => {
                    self.ram -= bytes.len();
                    bytes.zeroize();
                }
                Chunk::Disk { slot, .. } => self.free.push(slot),
            }
            self.start += CHUNK as u64;
            self.ram_from = self.ram_from.saturating_sub(1);
        }
        // A partly filled last chunk is never popped (it ends past `offset`),
        // so an empty buffer always ends on a chunk boundary.
        debug_assert!(!self.chunks.is_empty() || self.start == self.end);
        if self.free.len() as u64 == self.slots && self.slots > 0 {
            self.release_disk();
        }
    }

    fn release_disk(&mut self) {
        if let Some(file) = &self.file {
            let _ = file.set_len(0);
        }
        if let Some(config) = &self.config {
            config.release(self.slots * SLOT);
        }
        self.slots = 0;
        self.free.clear();
    }
}

impl Drop for SpillBuffer {
    fn drop(&mut self) {
        for chunk in &mut self.chunks {
            if let Chunk::Ram(bytes) = chunk {
                bytes.zeroize();
            }
        }
        if let Some(config) = &self.config {
            config.release(self.slots * SLOT);
        }
        self.slots = 0;
        // The key zeroizes itself; the unlinked file disappears on close.
    }
}

/// Reads a range of a [`SpillBuffer`], one decrypted chunk at a time.
pub struct SpillReader<'a> {
    buffer: &'a mut SpillBuffer,
    next: u64,
    to: u64,
    current: Vec<u8>,
    offset: usize,
}

impl Read for SpillReader<'_> {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        if self.offset == self.current.len() {
            if self.next >= self.to {
                return Ok(0);
            }
            let chunk_end = (self.next / CHUNK as u64 + 1) * CHUNK as u64;
            let until = chunk_end.min(self.to);
            self.current.zeroize();
            self.current.clear();
            self.buffer
                .read_into(self.next, until, &mut self.current)
                .map_err(std::io::Error::other)?;
            self.next = until;
            self.offset = 0;
        }
        let n = out.len().min(self.current.len() - self.offset);
        out[..n].copy_from_slice(&self.current[self.offset..self.offset + n]);
        self.offset += n;
        Ok(n)
    }
}

impl Drop for SpillReader<'_> {
    fn drop(&mut self) {
        self.current.zeroize();
    }
}

#[cfg(test)]
#[path = "spill_tests.rs"]
mod tests;
