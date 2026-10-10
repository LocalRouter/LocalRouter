//! Secrets that exceed a credential store's per-entry size limit.
//!
//! Windows Credential Manager caps a credential blob at 2560 bytes, and the
//! keyring crate stores a password there as UTF-16, so one entry holds at most
//! 1280 UTF-16 code units — less than an OAuth access token. Linux keyutils
//! rejects payloads over 32767 bytes.
//!
//! A secret that fits is stored in its entry unchanged, so entries written
//! before chunking existed read back as they always did. A secret that does
//! not fit is split on character boundaries into numbered part entries next to
//! the primary one, and the primary entry holds a header with the part count,
//! the total length and a fingerprint of the whole secret. A secret that itself
//! starts with the header prefix is always stored in parts, so a primary entry
//! starting with the prefix is never mistaken for a plain secret.

use lr_types::{AppError, AppResult};
use tracing::warn;
use zeroize::Zeroizing;

/// Start of the header stored in the primary entry of a chunked secret. The
/// leading record separator (U+001E) does not occur in real credentials.
pub(crate) const HEADER_PREFIX: &str = "\u{1e}localrouter-chunked:v1:";

/// Upper bound on parts per secret. Bounds stale-part cleanup and rejects
/// corrupt headers; 128 parts hold over 128 KB even on Windows.
pub(crate) const MAX_CHUNKS: usize = 128;

/// Single-entry operations on a credential store.
pub(crate) trait RawEntryStore {
    fn set(&self, service: &str, account: &str, value: &str) -> AppResult<()>;
    fn get(&self, service: &str, account: &str) -> AppResult<Option<String>>;
    /// Remove an entry, returning whether one existed.
    fn remove(&self, service: &str, account: &str) -> AppResult<bool>;
}

/// How much one entry holds, measured by `char_cost` per character.
#[derive(Clone, Copy, Debug)]
pub(crate) struct EntryLimit {
    pub max_cost: usize,
    pub char_cost: fn(char) -> usize,
}

impl EntryLimit {
    fn cost(&self, value: &str) -> usize {
        value.chars().map(self.char_cost).sum()
    }
}

/// Bytes a character takes in a UTF-16 credential blob.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn utf16_bytes(c: char) -> usize {
    c.len_utf16() * 2
}

/// Bytes a character takes in a UTF-8 payload.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(crate) fn utf8_bytes(c: char) -> usize {
    c.len_utf8()
}

/// Windows: `CRED_MAX_CREDENTIAL_BLOB_SIZE` is 2560 bytes of UTF-16.
#[cfg(windows)]
pub(crate) const PLATFORM_LIMIT: Option<EntryLimit> = Some(EntryLimit {
    max_cost: 2048,
    char_cost: utf16_bytes,
});

/// Linux keyutils: a `user` key payload is at most 32767 bytes.
#[cfg(target_os = "linux")]
pub(crate) const PLATFORM_LIMIT: Option<EntryLimit> = Some(EntryLimit {
    max_cost: 32_000,
    char_cost: utf8_bytes,
});

/// macOS Keychain and other stores have no practical per-entry limit.
#[cfg(not(any(windows, target_os = "linux")))]
pub(crate) const PLATFORM_LIMIT: Option<EntryLimit> = None;

/// Account holding part `index` (1-based) of a chunked secret.
pub(crate) fn chunk_account(account: &str, index: usize) -> String {
    format!("{account}#localrouter-chunk-{index}")
}

/// Store `secret`, splitting it across entries when it exceeds `limit`.
pub(crate) fn store<S: RawEntryStore + ?Sized>(
    raw: &S,
    limit: Option<EntryLimit>,
    service: &str,
    account: &str,
    secret: &str,
) -> AppResult<()> {
    let needs_parts = secret.starts_with(HEADER_PREFIX)
        || limit.is_some_and(|limit| limit.cost(secret) > limit.max_cost);

    let parts_written = if needs_parts {
        let parts = split(secret, limit)?;
        let header = format!(
            "{HEADER_PREFIX}{}:{}:{:016x}",
            parts.len(),
            secret.len(),
            fingerprint(secret)
        );
        if let Some(limit) = limit {
            if limit.cost(&header) > limit.max_cost {
                return Err(AppError::Internal(
                    "Credential store entry limit is too small for a chunked secret header"
                        .to_string(),
                ));
            }
        }
        for (index, part) in parts.iter().enumerate() {
            raw.set(service, &chunk_account(account, index + 1), part)?;
        }
        // Writing the header commits the new secret.
        raw.set(service, account, &header)?;
        parts.len()
    } else {
        raw.set(service, account, secret)?;
        0
    };

    // Parts left over from a previous, longer secret. The new secret is
    // already committed, so failing to tidy up must not fail the store.
    if let Err(e) = remove_parts_from(raw, service, account, parts_written + 1) {
        warn!(
            "Stored {}:{} but could not remove leftover parts of its previous value: {}",
            service, account, e
        );
    }
    Ok(())
}

/// Read a secret, reassembling it when it was stored in parts.
pub(crate) fn load<S: RawEntryStore + ?Sized>(
    raw: &S,
    service: &str,
    account: &str,
) -> AppResult<Option<String>> {
    let Some(primary) = raw.get(service, account)? else {
        return Ok(None);
    };
    let Some(header) = primary.strip_prefix(HEADER_PREFIX) else {
        return Ok(Some(primary));
    };
    let header = parse_header(header).ok_or_else(|| {
        AppError::Internal(format!(
            "Stored secret {}:{} has a malformed header; store it again",
            service, account
        ))
    })?;

    // Reserve up front so growth does not leave unzeroized copies behind, but
    // never trust a corrupt header with an unbounded allocation.
    let mut secret = Zeroizing::new(String::with_capacity(header.byte_len.min(1 << 20)));
    for index in 1..=header.count {
        let part = raw
            .get(service, &chunk_account(account, index))?
            .map(Zeroizing::new)
            .ok_or_else(|| {
                AppError::Internal(format!(
                    "Stored secret {}:{} is incomplete: part {} of {} is missing; store it again",
                    service, account, index, header.count
                ))
            })?;
        secret.push_str(&part);
    }
    if secret.len() != header.byte_len || fingerprint(&secret) != header.fingerprint {
        return Err(AppError::Internal(format!(
            "Stored secret {}:{} does not match its header (a save was likely interrupted); \
             store it again",
            service, account
        )));
    }
    Ok(Some(std::mem::take(&mut *secret)))
}

/// Delete a secret and every part it was stored in.
pub(crate) fn delete<S: RawEntryStore + ?Sized>(
    raw: &S,
    service: &str,
    account: &str,
) -> AppResult<()> {
    // Primary first: an interruption then leaves orphaned parts, not a
    // header pointing at missing ones.
    raw.remove(service, account)?;
    remove_parts_from(raw, service, account, 1)
}

/// Remove parts `first..` until one is absent. Parts are always numbered
/// contiguously from 1, so the first gap ends them.
fn remove_parts_from<S: RawEntryStore + ?Sized>(
    raw: &S,
    service: &str,
    account: &str,
    first: usize,
) -> AppResult<()> {
    for index in first..=MAX_CHUNKS {
        if !raw.remove(service, &chunk_account(account, index))? {
            break;
        }
    }
    Ok(())
}

/// Split `secret` into the fewest parts that each fit `limit`, never inside a
/// character. Without a limit the whole secret is one part.
fn split(secret: &str, limit: Option<EntryLimit>) -> AppResult<Vec<&str>> {
    let Some(limit) = limit else {
        return Ok(vec![secret]);
    };
    let mut parts = Vec::new();
    let mut start = 0;
    let mut cost = 0;
    for (offset, c) in secret.char_indices() {
        let char_cost = (limit.char_cost)(c);
        if char_cost > limit.max_cost {
            return Err(AppError::Internal(
                "Credential store entry limit is smaller than one character".to_string(),
            ));
        }
        if cost + char_cost > limit.max_cost {
            parts.push(&secret[start..offset]);
            start = offset;
            cost = 0;
        }
        cost += char_cost;
    }
    if start < secret.len() {
        parts.push(&secret[start..]);
    }
    if parts.len() > MAX_CHUNKS {
        return Err(AppError::Internal(format!(
            "Secret is too large for the credential store ({} bytes)",
            secret.len()
        )));
    }
    Ok(parts)
}

struct Header {
    count: usize,
    byte_len: usize,
    fingerprint: u64,
}

fn parse_header(header: &str) -> Option<Header> {
    let mut fields = header.split(':');
    let count = fields.next()?.parse().ok()?;
    let byte_len = fields.next()?.parse().ok()?;
    let fingerprint = fields.next()?;
    if fields.next().is_some() || fingerprint.len() != 16 || !(1..=MAX_CHUNKS).contains(&count) {
        return None;
    }
    Some(Header {
        count,
        byte_len,
        fingerprint: u64::from_str_radix(fingerprint, 16).ok()?,
    })
}

/// FNV-1a 64: detects parts that do not belong together. Not a security
/// measure; it sits in the same store as the secret itself.
fn fingerprint(secret: &str) -> u64 {
    secret.bytes().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::BTreeMap;

    /// In-memory store that rejects values over its limit like the real one.
    struct MemoryStore {
        entries: RefCell<BTreeMap<String, String>>,
        limit: Option<EntryLimit>,
    }

    impl MemoryStore {
        fn new(limit: Option<EntryLimit>) -> Self {
            Self {
                entries: RefCell::new(BTreeMap::new()),
                limit,
            }
        }

        fn key(service: &str, account: &str) -> String {
            format!("{service}\n{account}")
        }

        fn raw(&self, account: &str) -> Option<String> {
            self.entries
                .borrow()
                .get(&Self::key("svc", account))
                .cloned()
        }

        fn put_raw(&self, account: &str, value: &str) {
            self.entries
                .borrow_mut()
                .insert(Self::key("svc", account), value.to_string());
        }

        fn len(&self) -> usize {
            self.entries.borrow().len()
        }
    }

    impl RawEntryStore for MemoryStore {
        fn set(&self, service: &str, account: &str, value: &str) -> AppResult<()> {
            if let Some(limit) = self.limit {
                assert!(
                    limit.cost(value) <= limit.max_cost,
                    "entry {account} exceeds the store limit"
                );
            }
            self.entries
                .borrow_mut()
                .insert(Self::key(service, account), value.to_string());
            Ok(())
        }

        fn get(&self, service: &str, account: &str) -> AppResult<Option<String>> {
            Ok(self
                .entries
                .borrow()
                .get(&Self::key(service, account))
                .cloned())
        }

        fn remove(&self, service: &str, account: &str) -> AppResult<bool> {
            Ok(self
                .entries
                .borrow_mut()
                .remove(&Self::key(service, account))
                .is_some())
        }
    }

    const BYTES_64: EntryLimit = EntryLimit {
        max_cost: 64,
        char_cost: utf8_bytes,
    };

    fn round_trip(store: &MemoryStore, secret: &str) {
        super::store(store, store.limit, "svc", "acct", secret).unwrap();
        assert_eq!(load(store, "svc", "acct").unwrap().as_deref(), Some(secret));
    }

    #[test]
    fn short_secret_is_stored_unchanged() {
        let store = MemoryStore::new(Some(BYTES_64));
        round_trip(&store, "sk-short");
        assert_eq!(store.raw("acct").as_deref(), Some("sk-short"));
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn secret_exactly_at_limit_is_one_entry() {
        let store = MemoryStore::new(Some(BYTES_64));
        let secret = "a".repeat(64);
        round_trip(&store, &secret);
        assert_eq!(store.raw("acct"), Some(secret));
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn secret_over_limit_is_split_and_reassembled() {
        let store = MemoryStore::new(Some(BYTES_64));
        let secret: String = (0..65).map(|i| char::from(b'a' + (i % 26) as u8)).collect();
        round_trip(&store, &secret);
        assert!(store.raw("acct").unwrap().starts_with(HEADER_PREFIX));
        assert_eq!(store.raw(&chunk_account("acct", 1)).unwrap().len(), 64);
        assert_eq!(store.raw(&chunk_account("acct", 2)).as_deref(), Some("m"));
        assert_eq!(store.len(), 3);
    }

    #[test]
    fn multibyte_characters_are_never_split() {
        // 'é' is 2 bytes, '😀' is 4: 63 ASCII bytes leave no room for either.
        let store = MemoryStore::new(Some(BYTES_64));
        let secret = format!("{}é{}😀{}", "a".repeat(63), "b".repeat(61), "c".repeat(70));
        round_trip(&store, &secret);
        assert_eq!(
            store.raw(&chunk_account("acct", 1)),
            Some("a".repeat(63)),
            "the 2-byte char moves to the next part"
        );
        assert_eq!(
            store.raw(&chunk_account("acct", 2)),
            Some(format!("é{}", "b".repeat(61)))
        );
    }

    #[test]
    fn utf16_cost_counts_surrogate_pairs() {
        let limit = EntryLimit {
            max_cost: 128,
            char_cost: utf16_bytes,
        };
        let store = MemoryStore::new(Some(limit));
        let secret = format!("{}{}", "x".repeat(63), "😀".repeat(40));
        round_trip(&store, &secret);
        // 63 'x' = 126 bytes; a surrogate pair (4 bytes) does not fit after it.
        assert_eq!(store.raw(&chunk_account("acct", 1)), Some("x".repeat(63)));
        assert_eq!(store.raw(&chunk_account("acct", 2)), Some("😀".repeat(32)));
        assert_eq!(store.raw(&chunk_account("acct", 3)), Some("😀".repeat(8)));
    }

    #[test]
    fn chatgpt_sized_token_fits_windows_credential_manager() {
        // keyring 3.6 rejects a password when its UTF-16 encoding exceeds
        // CRED_MAX_CREDENTIAL_BLOB_SIZE (2560 bytes).
        struct WindowsLike(MemoryStore);
        impl RawEntryStore for WindowsLike {
            fn set(&self, service: &str, account: &str, value: &str) -> AppResult<()> {
                if value.encode_utf16().count() * 2 > 2560 {
                    return Err(AppError::Internal("longer than platform limit".into()));
                }
                self.0.set(service, account, value)
            }
            fn get(&self, service: &str, account: &str) -> AppResult<Option<String>> {
                self.0.get(service, account)
            }
            fn remove(&self, service: &str, account: &str) -> AppResult<bool> {
                self.0.remove(service, account)
            }
        }
        let windows_limit = EntryLimit {
            max_cost: 2048,
            char_cost: utf16_bytes,
        };
        let store = WindowsLike(MemoryStore::new(None));
        let token = format!("eyJhbGciOiJSUzI1NiJ9.{}.sig", "Q".repeat(3500));
        assert!(store.set("svc", "plain", &token).is_err());
        super::store(&store, Some(windows_limit), "svc", "acct", &token).unwrap();
        assert_eq!(load(&store, "svc", "acct").unwrap(), Some(token));
    }

    #[test]
    fn overwriting_long_with_short_removes_parts() {
        let store = MemoryStore::new(Some(BYTES_64));
        round_trip(&store, &"L".repeat(300));
        assert_eq!(store.len(), 6);
        round_trip(&store, "short");
        assert_eq!(store.raw("acct").as_deref(), Some("short"));
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn overwriting_long_with_shorter_long_removes_extra_parts() {
        let store = MemoryStore::new(Some(BYTES_64));
        round_trip(&store, &"L".repeat(300));
        round_trip(&store, &"M".repeat(100));
        assert_eq!(store.len(), 3);
        assert!(store.raw(&chunk_account("acct", 3)).is_none());
    }

    #[test]
    fn overwriting_short_with_long() {
        let store = MemoryStore::new(Some(BYTES_64));
        round_trip(&store, "short");
        round_trip(&store, &"L".repeat(200));
        assert_eq!(store.len(), 5);
    }

    #[test]
    fn delete_removes_every_part() {
        let store = MemoryStore::new(Some(BYTES_64));
        round_trip(&store, &"L".repeat(300));
        store.put_raw("other", "kept");
        delete(&store, "svc", "acct").unwrap();
        assert_eq!(load(&store, "svc", "acct").unwrap(), None);
        assert_eq!(store.len(), 1, "only the unrelated entry remains");
        // Deleting what is not there is not an error.
        delete(&store, "svc", "acct").unwrap();
    }

    #[test]
    fn missing_part_is_an_error_not_a_truncated_secret() {
        let store = MemoryStore::new(Some(BYTES_64));
        round_trip(&store, &"L".repeat(300));
        store.remove("svc", &chunk_account("acct", 3)).unwrap();
        let error = load(&store, "svc", "acct").unwrap_err().to_string();
        assert!(error.contains("part 3 of 5 is missing"), "{error}");
    }

    #[test]
    fn mismatched_parts_are_an_error() {
        let store = MemoryStore::new(Some(BYTES_64));
        round_trip(&store, &"L".repeat(300));
        // An interrupted overwrite: a part replaced without its header.
        store.put_raw(&chunk_account("acct", 2), &"M".repeat(64));
        let error = load(&store, "svc", "acct").unwrap_err().to_string();
        assert!(error.contains("does not match its header"), "{error}");
    }

    #[test]
    fn malformed_header_is_an_error() {
        let store = MemoryStore::new(Some(BYTES_64));
        for header in [
            "",
            "x:1:0",
            "0:1:0000000000000000",
            "999:1:0000000000000000",
        ] {
            store.put_raw("acct", &format!("{HEADER_PREFIX}{header}"));
            let error = load(&store, "svc", "acct").unwrap_err().to_string();
            assert!(error.contains("malformed header"), "{header}: {error}");
        }
    }

    #[test]
    fn header_with_absurd_length_is_an_error_not_an_allocation() {
        let store = MemoryStore::new(Some(BYTES_64));
        round_trip(&store, &"L".repeat(100));
        store.put_raw(
            "acct",
            &format!("{HEADER_PREFIX}2:{}:0000000000000000", usize::MAX),
        );
        let error = load(&store, "svc", "acct").unwrap_err().to_string();
        assert!(error.contains("does not match its header"), "{error}");
    }

    #[test]
    fn legacy_single_entry_reads_unchanged() {
        // Written before chunking existed, even if longer than today's limit.
        let store = MemoryStore::new(Some(BYTES_64));
        let legacy = "z".repeat(500);
        store.put_raw("acct", &legacy);
        assert_eq!(load(&store, "svc", "acct").unwrap(), Some(legacy));
    }

    #[test]
    fn secret_starting_with_marker_is_always_chunked() {
        for limit in [Some(BYTES_64), None] {
            let store = MemoryStore::new(limit);
            let header_lookalike = format!("{HEADER_PREFIX}1:3:0000000000000000");
            round_trip(&store, &header_lookalike);
            round_trip(&store, HEADER_PREFIX);
            assert_ne!(store.raw("acct").as_deref(), Some(HEADER_PREFIX));
        }
    }

    #[test]
    fn platform_limit_round_trips_a_long_token() {
        let store = MemoryStore::new(PLATFORM_LIMIT);
        round_trip(&store, &"Q".repeat(100_000));
        #[cfg(windows)]
        assert!(store
            .entries
            .borrow()
            .values()
            .all(|value| value.encode_utf16().count() * 2 <= 2560));
    }

    #[test]
    fn unlimited_store_keeps_long_secrets_in_one_entry() {
        let store = MemoryStore::new(None);
        let secret = "u".repeat(10_000);
        round_trip(&store, &secret);
        assert_eq!(store.raw("acct"), Some(secret));
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn oversized_secret_is_rejected_without_writing() {
        let store = MemoryStore::new(Some(BYTES_64));
        let secret = "o".repeat(64 * MAX_CHUNKS + 1);
        let error = super::store(&store, store.limit, "svc", "acct", &secret)
            .unwrap_err()
            .to_string();
        assert!(error.contains("too large"), "{error}");
        assert_eq!(store.len(), 0);

        let largest = "o".repeat(64 * MAX_CHUNKS);
        round_trip(&store, &largest);
        assert_eq!(store.len(), MAX_CHUNKS + 1);
        delete(&store, "svc", "acct").unwrap();
        assert_eq!(store.len(), 0);
    }

    #[test]
    fn limit_too_small_for_header_is_an_error() {
        let tiny = EntryLimit {
            max_cost: 8,
            char_cost: utf8_bytes,
        };
        let store = MemoryStore::new(None);
        assert!(super::store(&store, Some(tiny), "svc", "acct", &"t".repeat(9)).is_err());
        assert_eq!(store.len(), 0);
    }

    #[test]
    fn limit_smaller_than_a_character_is_an_error() {
        let one_byte = EntryLimit {
            max_cost: 1,
            char_cost: utf16_bytes,
        };
        let store = MemoryStore::new(None);
        let error = super::store(&store, Some(one_byte), "svc", "acct", "ab")
            .unwrap_err()
            .to_string();
        assert!(error.contains("smaller than one character"), "{error}");
        assert_eq!(store.len(), 0);
    }

    #[test]
    fn failing_to_remove_leftover_parts_does_not_fail_the_store() {
        struct NoRemove(MemoryStore);
        impl RawEntryStore for NoRemove {
            fn set(&self, service: &str, account: &str, value: &str) -> AppResult<()> {
                self.0.set(service, account, value)
            }
            fn get(&self, service: &str, account: &str) -> AppResult<Option<String>> {
                self.0.get(service, account)
            }
            fn remove(&self, _: &str, _: &str) -> AppResult<bool> {
                Err(AppError::Internal("remove failed".into()))
            }
        }
        let store = NoRemove(MemoryStore::new(Some(BYTES_64)));
        super::store(&store, Some(BYTES_64), "svc", "acct", &"L".repeat(300)).unwrap();
        super::store(&store, Some(BYTES_64), "svc", "acct", "short").unwrap();
        assert_eq!(
            load(&store, "svc", "acct").unwrap().as_deref(),
            Some("short")
        );
        assert!(delete(&store, "svc", "acct").is_err());
    }

    #[test]
    fn empty_secret_is_stored_unchanged() {
        let store = MemoryStore::new(Some(BYTES_64));
        round_trip(&store, "");
        assert_eq!(store.raw("acct").as_deref(), Some(""));
    }

    #[test]
    fn accounts_do_not_interfere() {
        let store = MemoryStore::new(Some(BYTES_64));
        super::store(&store, store.limit, "svc", "a", &"A".repeat(100)).unwrap();
        super::store(&store, store.limit, "svc", "b", &"B".repeat(100)).unwrap();
        delete(&store, "svc", "a").unwrap();
        assert_eq!(load(&store, "svc", "b").unwrap(), Some("B".repeat(100)));
    }
}
