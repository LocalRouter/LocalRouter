//! Keychain storage module
//!
//! Provides keychain storage functionality for securely storing secrets.
//! Used by the clients module to store client secrets.

mod keychain;
pub mod keychain_trait;

pub use keychain_trait::{CachedKeychain, KeychainStorage, MockKeychain};

#[cfg(not(debug_assertions))]
pub use keychain_trait::migrate_secrets_file_to_keychain;
