//! Linux and other Unix: the bundle the distribution compiles from its trust
//! configuration.
//!
//! Distributions do not keep trust in a queryable service; `update-ca-trust`
//! (Fedora/RHEL), `update-ca-certificates` (Debian/Ubuntu/Alpine/Arch/SUSE) or
//! the package manager regenerate a single PEM bundle from the shipped roots,
//! the administrator's additions and the blocklist. That file *is* the
//! system's TLS trust, so it is read as-is. Probed in order; the first that
//! exists wins. The Fedora path is the server-authentication-only extract and
//! comes first so roots trusted only for e-mail or code signing stay out.

const CANDIDATES: &[&str] = &[
    "/etc/pki/ca-trust/extracted/pem/tls-ca-bundle.pem", // Fedora/RHEL (TLS only)
    "/etc/ssl/certs/ca-certificates.crt",                // Debian/Ubuntu/Alpine/Arch/NixOS
    "/etc/pki/tls/certs/ca-bundle.crt",                  // older RHEL
    "/etc/ssl/ca-bundle.pem",                            // openSUSE
    "/etc/ssl/cert.pem",                                 // BSDs, Alpine
    "/usr/local/share/certs/ca-root-nss.crt",            // FreeBSD
    "/etc/pki/tls/cacert.pem",                           // OpenELEC
];

pub fn load() -> Result<Vec<Vec<u8>>, String> {
    for path in CANDIDATES {
        match std::fs::read(path) {
            Ok(bytes) => {
                let certs = super::parse_pem_certs(&bytes);
                if !certs.is_empty() {
                    return Ok(certs);
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("Failed to read {path}: {e}")),
        }
    }
    Err(format!(
        "No system CA bundle found (looked in {})",
        CANDIDATES.join(", ")
    ))
}
