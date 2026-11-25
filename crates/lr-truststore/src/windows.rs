//! Windows: the current user's `Root` store, minus `Disallowed`.
//!
//! The current user's `Root` is a logical store that also shows the machine's
//! roots and those pushed by Group Policy. Windows refuses any certificate in
//! the `Disallowed` store (including Microsoft's own distrust updates), and a
//! certificate whose purposes were disabled in the certificate manager no
//! longer lists server authentication, so both are left out here too.
//!
//! Windows fetches some Microsoft-program roots on first use rather than
//! shipping them all in the store; a root that has never been needed on this
//! machine may therefore be absent until Windows downloads it, at which point
//! the next refresh picks it up.

use std::collections::HashSet;

use schannel::cert_context::ValidUses;
use schannel::cert_store::CertStore;

const PKIX_SERVER_AUTH: &str = "1.3.6.1.5.5.7.3.1";

pub fn load() -> Result<Vec<Vec<u8>>, String> {
    let root = CertStore::open_current_user("ROOT")
        .map_err(|e| format!("Failed to open the Root certificate store: {e}"))?;
    // A missing Disallowed store just means nothing is distrusted.
    let disallowed: HashSet<Vec<u8>> = CertStore::open_current_user("Disallowed")
        .map(|s| s.certs().map(|c| c.to_der().to_vec()).collect())
        .unwrap_or_default();

    Ok(root
        .certs()
        .filter(|c| {
            let server_auth = match c.valid_uses() {
                Ok(ValidUses::All) => true,
                Ok(ValidUses::Oids(oids)) => oids.iter().any(|o| o == PKIX_SERVER_AUTH),
                Err(_) => false,
            };
            server_auth && c.is_time_valid().unwrap_or(false)
        })
        .map(|c| c.to_der().to_vec())
        .filter(|der| !disallowed.contains(der))
        .collect())
}
