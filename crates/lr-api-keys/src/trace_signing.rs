//! This install's request-trace signing key (see `lr_types::trace`).
//!
//! 32 random bytes generated on first run and kept with the app's other
//! secrets. The key only ever signs trace headers locally; it is never sent
//! anywhere.

use crate::KeychainStorage;
use lr_types::AppResult;
use zeroize::Zeroizing;

/// Keychain service of the trace signing key.
pub const TRACE_SIGNING_SERVICE: &str = "LocalRouter-TraceSigning";
/// Keychain account of the trace signing key.
pub const TRACE_SIGNING_ACCOUNT: &str = "install";

/// The stored key, or a new one stored for later runs. A stored value that
/// is not a 32-byte hex key is replaced.
pub fn load_or_create(storage: &dyn KeychainStorage) -> AppResult<[u8; 32]> {
    if let Some(stored) = storage
        .get(TRACE_SIGNING_SERVICE, TRACE_SIGNING_ACCOUNT)?
        .map(Zeroizing::new)
    {
        if let Some(key) = decode(&stored) {
            return Ok(key);
        }
        tracing::warn!("Stored trace signing key is unreadable; generating a new one");
    }
    let key: [u8; 32] = rand::random();
    storage.store(
        TRACE_SIGNING_SERVICE,
        TRACE_SIGNING_ACCOUNT,
        &Zeroizing::new(hex::encode(key)),
    )?;
    Ok(key)
}

/// The key for this process: the stored one, or (when the secret store is
/// unavailable) an ephemeral one, so signing still works between the hops of
/// this process.
pub fn load_or_ephemeral(storage: &dyn KeychainStorage) -> [u8; 32] {
    load_or_create(storage).unwrap_or_else(|e| {
        tracing::warn!("Trace signing key unavailable ({e}); using a key for this run only");
        rand::random()
    })
}

fn decode(stored: &str) -> Option<[u8; 32]> {
    hex::decode(stored.trim()).ok()?.try_into().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MockKeychain;

    #[test]
    fn generated_once_then_reused() {
        let storage = MockKeychain::new();
        let first = load_or_create(&storage).unwrap();
        assert_eq!(load_or_create(&storage).unwrap(), first);
        let stored = storage
            .get(TRACE_SIGNING_SERVICE, TRACE_SIGNING_ACCOUNT)
            .unwrap()
            .unwrap();
        assert_eq!(stored, hex::encode(first));
        // Another install has another key.
        assert_ne!(load_or_create(&MockKeychain::new()).unwrap(), first);
    }

    #[test]
    fn an_unreadable_stored_key_is_replaced() {
        let storage = MockKeychain::new();
        storage
            .store(TRACE_SIGNING_SERVICE, TRACE_SIGNING_ACCOUNT, "not hex")
            .unwrap();
        let key = load_or_create(&storage).unwrap();
        assert_eq!(load_or_create(&storage).unwrap(), key);
    }

    #[test]
    fn an_unavailable_store_still_yields_a_key() {
        struct Broken;
        impl KeychainStorage for Broken {
            fn store(&self, _: &str, _: &str, _: &str) -> AppResult<()> {
                Err(lr_types::AppError::Storage("locked".into()))
            }
            fn get(&self, _: &str, _: &str) -> AppResult<Option<String>> {
                Err(lr_types::AppError::Storage("locked".into()))
            }
            fn delete(&self, _: &str, _: &str) -> AppResult<()> {
                Ok(())
            }
        }
        assert!(load_or_create(&Broken).is_err());
        assert_ne!(load_or_ephemeral(&Broken), [0; 32]);
    }
}
