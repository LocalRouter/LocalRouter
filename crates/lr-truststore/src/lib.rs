//! The operating system's trusted TLS root certificates, as the OS itself
//! decides them.
//!
//! Some tools only take a CA bundle file (`SSL_CERT_FILE`,
//! `REQUESTS_CA_BUNDLE`) and treat it as their *entire* trust store. To add
//! one root for such a tool without changing anything else it trusts, the file
//! has to mirror the OS's trust exactly. This crate produces that set:
//!
//! - **macOS**: every certificate with trust settings in the user, admin and
//!   system domains, resolved the way the OS resolves them (user overrides
//!   admin overrides system). A root the user or an administrator marked
//!   "Never Trust" for SSL is left out; one they marked trusted is included.
//! - **Windows**: the current user's `Root` store (which includes the machine's
//!   roots and Group Policy roots), minus anything in the `Disallowed` store,
//!   anything whose enabled purposes exclude server authentication, and
//!   anything outside its validity period.
//! - **Linux and other Unix**: the bundle the distribution compiles from its
//!   trust configuration (`update-ca-certificates` / `update-ca-trust`), which
//!   already applies the distribution's and the administrator's blocklists.
//!
//! Environment variables such as `SSL_CERT_FILE` are deliberately ignored:
//! they may point at a bundle generated from this very function.
//!
//! The result is a snapshot. Callers that keep a file in step with the OS
//! re-read it periodically.

#[cfg(target_os = "macos")]
mod macos;
#[cfg(all(unix, not(target_os = "macos")))]
mod unix;
#[cfg(windows)]
mod windows;

use base64::Engine;
use rustls_pki_types::pem::PemObject;
use rustls_pki_types::CertificateDer;

/// DER-encoded certificates, sorted and without duplicates.
pub type DerCerts = Vec<Vec<u8>>;

/// Load the roots the OS trusts for TLS server authentication.
///
/// Errors when the platform store cannot be read or is empty: an empty or
/// partial set written into a replacing bundle would make the tool distrust
/// every host, which is worse than not producing a bundle at all.
pub fn os_trusted_roots() -> Result<DerCerts, String> {
    #[cfg(target_os = "macos")]
    let certs = macos::load()?;
    #[cfg(all(unix, not(target_os = "macos")))]
    let certs = unix::load()?;
    #[cfg(windows)]
    let certs = windows::load()?;

    let certs = normalize(certs);
    if certs.is_empty() {
        return Err("The operating system reported no trusted root certificates".into());
    }
    Ok(certs)
}

/// Sort and deduplicate, so the same trust state always yields the same
/// bytes (callers compare bundles to decide whether to rewrite them).
pub fn normalize(mut certs: DerCerts) -> DerCerts {
    certs.sort_unstable();
    certs.dedup();
    certs
}

/// Parse every `CERTIFICATE` block in a PEM document, skipping anything else.
pub fn parse_pem_certs(pem: &[u8]) -> DerCerts {
    CertificateDer::pem_slice_iter(pem)
        .filter_map(Result::ok)
        .map(|c| c.as_ref().to_vec())
        .collect()
}

/// Encode one DER certificate as a PEM block (64-column base64, trailing
/// newline).
pub fn der_to_pem(der: &[u8]) -> String {
    let b64 = base64::engine::general_purpose::STANDARD.encode(der);
    let mut out = String::with_capacity(b64.len() + b64.len() / 64 + 64);
    out.push_str("-----BEGIN CERTIFICATE-----\n");
    for chunk in b64.as_bytes().chunks(64) {
        // base64 output is ASCII, so every chunk is valid UTF-8.
        out.push_str(std::str::from_utf8(chunk).unwrap_or_default());
        out.push('\n');
    }
    out.push_str("-----END CERTIFICATE-----\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pem_round_trips() {
        let der: Vec<u8> = (0..=255u8).cycle().take(1000).collect();
        let pem = der_to_pem(&der);
        assert!(pem.lines().all(|l| l.len() <= 64));
        assert_eq!(parse_pem_certs(pem.as_bytes()), vec![der]);
    }

    #[test]
    fn parsing_skips_comments_and_other_blocks() {
        let a = der_to_pem(&[1, 2, 3]);
        let b = der_to_pem(&[4, 5, 6]);
        let doc = format!(
            "# a comment\n{a}-----BEGIN PRIVATE KEY-----\nAAAA\n-----END PRIVATE KEY-----\n{b}"
        );
        assert_eq!(
            parse_pem_certs(doc.as_bytes()),
            vec![vec![1, 2, 3], vec![4, 5, 6]]
        );
    }

    #[test]
    fn normalize_sorts_and_dedups() {
        assert_eq!(
            normalize(vec![vec![3], vec![1], vec![3], vec![2]]),
            vec![vec![1], vec![2], vec![3]]
        );
    }

    /// The real store on the machine running the tests: must be non-empty
    /// and parse back. Not a fixture — it exercises the platform loader.
    #[test]
    fn the_os_store_loads() {
        let roots = os_trusted_roots().expect("OS trust store");
        assert!(roots.len() > 10, "only {} roots", roots.len());
    }
}
