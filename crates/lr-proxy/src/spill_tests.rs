//! Unit tests of the encrypted spill buffer.

use super::*;

/// A spill directory under the system temp dir, removed on drop.
struct Dir(PathBuf);

impl Dir {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("lr-spill-test-{}", uuid::Uuid::new_v4())))
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn pattern(len: usize, seed: u64) -> Vec<u8> {
    let mut state = seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1;
    (0..len)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state as u8
        })
        .collect()
}

fn config(dir: &Dir, ram: usize, cap: u64, budget: u64) -> Arc<SpillConfig> {
    Arc::new(SpillConfig::with_limits(dir.0.clone(), ram, cap, budget))
}

#[test]
fn bytes_read_back_whatever_spilled_and_however_they_were_appended() {
    let dir = Dir::new();
    let config = config(&dir, 3 * CHUNK, 1 << 30, 1 << 30);
    let data = pattern(20 * CHUNK + 777, 1);
    for piece in [1, 999, CHUNK, 3 * CHUNK + 5] {
        let mut buffer = SpillBuffer::new(config.clone());
        for part in data.chunks(piece) {
            buffer.append(part).unwrap();
        }
        assert_eq!(buffer.end(), data.len() as u64);
        assert!(buffer.ram_bytes() <= 3 * CHUNK + CHUNK);
        assert!(buffer.spilled_bytes() > 0);
        assert_eq!(buffer.read(0, data.len() as u64).unwrap(), data);
        for (from, to) in [(0, 1), (CHUNK - 3, CHUNK + 3), (5 * CHUNK, 9 * CHUNK + 1)] {
            assert_eq!(
                buffer.read(from as u64, to as u64).unwrap(),
                &data[from..to]
            );
        }
        let mut streamed = Vec::new();
        buffer
            .reader(100, data.len() as u64 - 100)
            .read_to_end(&mut streamed)
            .unwrap();
        assert_eq!(streamed, &data[100..data.len() - 100]);
    }
    assert_eq!(config.in_flight(), 0, "dropped buffers release the budget");
}

#[test]
fn the_spill_file_holds_no_plaintext_and_is_not_visible_in_the_directory() {
    let dir = Dir::new();
    let config = config(&dir, CHUNK, 1 << 30, 1 << 30);
    let secret = b"sk-live-SPILLED-SECRET-0123456789";
    let mut data = Vec::new();
    while data.len() < 8 * CHUNK {
        data.extend_from_slice(secret);
        data.extend_from_slice(b" filler text ");
    }
    let mut buffer = SpillBuffer::new(config.clone());
    buffer.append(&data).unwrap();
    assert!(buffer.spilled_bytes() >= 6 * CHUNK as u64);
    // The file is anonymous: nothing in the directory.
    let entries: Vec<_> = std::fs::read_dir(&dir.0).unwrap().collect();
    assert!(entries.is_empty(), "{entries:?}");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&dir.0).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o700);
    }
    // Read the raw file through the open handle.
    let file = buffer.file.as_mut().unwrap();
    let mut raw = Vec::new();
    file.seek(SeekFrom::Start(0)).unwrap();
    file.read_to_end(&mut raw).unwrap();
    assert!(raw.len() as u64 >= buffer.spilled_bytes());
    assert!(!raw.windows(secret.len()).any(|w| w == secret));
    assert!(!raw.windows(12).any(|w| w == b" filler text"));
    assert_eq!(buffer.read(0, data.len() as u64).unwrap(), data);
}

#[test]
fn tampered_or_moved_chunks_fail_authentication() {
    let dir = Dir::new();
    let config = config(&dir, CHUNK, 1 << 30, 1 << 30);
    let data = pattern(6 * CHUNK, 2);
    let mut buffer = SpillBuffer::new(config);
    buffer.append(&data).unwrap();
    // Flip one byte of the first slot.
    {
        let file = buffer.file.as_mut().unwrap();
        file.seek(SeekFrom::Start(10)).unwrap();
        let mut byte = [0u8];
        file.read_exact(&mut byte).unwrap();
        file.seek(SeekFrom::Start(10)).unwrap();
        file.write_all(&[byte[0] ^ 1]).unwrap();
    }
    assert_eq!(buffer.read(0, 10), Err(SpillError::Corrupt));
    // Copy slot 1 over slot 2: the position is authenticated too.
    {
        let file = buffer.file.as_mut().unwrap();
        let mut slot = vec![0u8; SLOT as usize];
        file.seek(SeekFrom::Start(SLOT)).unwrap();
        file.read_exact(&mut slot).unwrap();
        file.seek(SeekFrom::Start(2 * SLOT)).unwrap();
        file.write_all(&slot).unwrap();
    }
    assert_eq!(
        buffer.read(2 * CHUNK as u64, 2 * CHUNK as u64 + 1),
        Err(SpillError::Corrupt)
    );
}

#[test]
fn discarded_chunks_free_their_slots_and_the_budget() {
    let dir = Dir::new();
    let config = config(&dir, 2 * CHUNK, 1 << 30, 1 << 30);
    let mut buffer = SpillBuffer::new(config.clone());
    let data = pattern(200 * CHUNK, 3);
    let mut max_disk = 0;
    // A sliding window: append, then forget what is more than 8 chunks old.
    for (i, part) in data.chunks(CHUNK / 2).enumerate() {
        buffer.append(part).unwrap();
        let end = buffer.end();
        buffer.discard_before(end.saturating_sub(8 * CHUNK as u64));
        max_disk = max_disk.max(buffer.disk_bytes());
        if i % 37 == 0 {
            let from = buffer.start().max(end.saturating_sub(5 * CHUNK as u64));
            assert_eq!(
                buffer.read(from, end).unwrap(),
                &data[from as usize..end as usize]
            );
        }
    }
    assert!(max_disk <= 10 * SLOT, "slots are reused: {max_disk}");
    assert!(buffer.max_held() <= 10 * CHUNK as u64);
    // Discarding everything empties the file and returns the budget.
    let end = buffer.end();
    buffer.discard_before(end);
    assert_eq!(buffer.disk_bytes(), 0);
    assert_eq!(config.in_flight(), 0);
    // It keeps working afterwards.
    buffer.append(b"more").unwrap();
    assert_eq!(buffer.read(end, end + 4).unwrap(), b"more");
}

#[test]
fn the_request_cap_and_the_global_budget_refuse_more_bytes() {
    let dir = Dir::new();
    let config = config(&dir, CHUNK, 10 * CHUNK as u64, 9 * SLOT);
    let mut buffer = SpillBuffer::new(config.clone());
    buffer.append(&pattern(9 * CHUNK, 4)).unwrap();
    let refused = buffer.append(&pattern(2 * CHUNK, 5)).unwrap_err();
    assert_eq!(refused, SpillError::RequestCap(10 * CHUNK as u64));
    assert_eq!(refused.status(), 413);
    assert_eq!(buffer.end(), 9 * CHUNK as u64, "nothing was appended");

    // A second buffer finds the global budget mostly used by the first.
    let mut other = SpillBuffer::new(config.clone());
    let error = other.append(&pattern(9 * CHUNK, 6)).unwrap_err();
    assert_eq!(error, SpillError::Budget);
    assert_eq!(error.status(), 413);
    assert!(config.in_flight() <= config.global_budget());
    drop(buffer);
    drop(other);
    assert_eq!(config.in_flight(), 0);

    // A RAM-only buffer never touches the disk and stops at its cap.
    let mut ram = SpillBuffer::ram_only(3 * CHUNK as u64);
    ram.append(&pattern(3 * CHUNK, 7)).unwrap();
    assert_eq!(ram.disk_bytes(), 0);
    assert!(ram.append(b"x").is_err());
}

#[test]
fn every_chunk_gets_a_fresh_nonce_even_when_a_slot_is_reused() {
    let dir = Dir::new();
    let config = config(&dir, CHUNK, 1 << 30, 1 << 30);
    let mut buffer = SpillBuffer::new(config);
    let mut seen = std::collections::HashSet::new();
    for round in 0..20u64 {
        buffer.append(&pattern(3 * CHUNK, round)).unwrap();
        for chunk in &buffer.chunks {
            if let Chunk::Disk { nonce, slot, .. } = chunk {
                seen.insert((*nonce, *slot));
            }
        }
        let end = buffer.end();
        buffer.discard_before(end - CHUNK as u64);
    }
    let nonces: std::collections::HashSet<_> = seen.iter().map(|(n, _)| *n).collect();
    assert_eq!(nonces.len(), seen.len(), "a nonce was used twice");
    assert!(buffer.nonces >= 20);
}

#[test]
fn stale_files_are_removed_at_startup() {
    let dir = Dir::new();
    let config = config(&dir, CHUNK, 1 << 30, 1 << 30);
    config.ensure_dir().unwrap();
    std::fs::write(dir.0.join(".tmpstale1"), b"old").unwrap();
    std::fs::write(dir.0.join(".tmpstale2"), b"old").unwrap();
    assert_eq!(config.cleanup_stale(), 2);
    assert_eq!(std::fs::read_dir(&dir.0).unwrap().count(), 0);
    // A missing directory is not an error.
    let missing = SpillConfig::new(dir.0.join("missing"));
    assert_eq!(missing.cleanup_stale(), 0);
}
