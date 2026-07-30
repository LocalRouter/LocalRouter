//! Combined root-CA bundle generation.
//!
//! Most tools we configure trust an extra CA **additively** — Node's
//! `NODE_EXTRA_CA_CERTS`, Continue's `caBundlePath`, Goose's
//! `GOOSE_CA_CERT_PATH`. Python's does not: `SSL_CERT_FILE` and
//! `REQUESTS_CA_BUNDLE` (httpx / aiohttp / requests, i.e. Aider's stack)
//! **replace** the default certifi bundle outright. Pointing those at our root
//! CA alone would break every host the proxy does not intercept — PyPI version
//! checks, model-metadata fetches, and every non-intercepted provider.
//!
//! So for those tools we materialize a combined bundle: the platform's existing
//! trust bundle followed by our root CA, written into LocalRouter's own config
//! directory.

use std::path::{Path, PathBuf};

/// Well-known system CA bundle locations, in probe order. macOS ships
/// `/etc/ssl/cert.pem`; the rest cover common Linux distributions.
const SYSTEM_BUNDLE_CANDIDATES: &[&str] = &[
    "/etc/ssl/cert.pem",                                 // macOS, some BSDs
    "/etc/ssl/certs/ca-certificates.crt",                // Debian/Ubuntu/Alpine
    "/etc/pki/tls/certs/ca-bundle.crt",                  // Fedora/RHEL
    "/etc/ssl/ca-bundle.pem",                            // openSUSE
    "/etc/pki/ca-trust/extracted/pem/tls-ca-bundle.pem", // RHEL variants
];

/// Locate the certifi bundle the user's Python would use, if Python is present.
///
/// Preferred over the system bundle for Python tools because certifi is what
/// their stack defaults to — starting from the same set avoids silently
/// changing which public roots are trusted.
fn certifi_bundle_path() -> Option<PathBuf> {
    for python in ["python3", "python"] {
        // `continue`, not `?`: a missing `python3` must not stop us from
        // trying `python` (venv/pyenv layouts, some Windows installs).
        let Some(bin) = super::integrations::find_binary(python) else {
            continue;
        };
        let out = std::process::Command::new(bin)
            .args(["-c", "import certifi; print(certifi.where())"])
            .output()
            .ok()?;
        if !out.status.success() {
            continue;
        }
        let path = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim().to_string());
        if path.is_file() {
            return Some(path);
        }
    }
    None
}

/// The base trust bundle to build on: certifi if discoverable, else the first
/// existing system bundle.
fn base_bundle_path() -> Option<PathBuf> {
    certifi_bundle_path().or_else(|| {
        SYSTEM_BUNDLE_CANDIDATES
            .iter()
            .map(PathBuf::from)
            .find(|p| p.is_file())
    })
}

/// Where the generated combined bundle lives.
pub fn combined_bundle_path() -> Result<PathBuf, String> {
    Ok(lr_utils::paths::config_dir()
        .map_err(|e| format!("Failed to resolve config dir: {e}"))?
        .join("proxy")
        .join("combined-ca.pem"))
}

/// Concatenate a base trust bundle with our root CA.
///
/// Exposed separately from the filesystem work so the composition rules are
/// unit-testable: the base comes first, our CA last, and a newline always
/// separates them so two PEM blocks never end up on the same line.
pub fn compose_bundle(base_pem: &str, ca_pem: &str) -> String {
    let mut out = String::with_capacity(base_pem.len() + ca_pem.len() + 2);
    out.push_str(base_pem.trim_end());
    out.push('\n');
    out.push_str(ca_pem.trim_start());
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

/// Write (or refresh) the combined bundle and return its path.
///
/// Fails loudly when no base bundle can be found rather than writing a
/// CA-only file: silently narrowing a tool's trust store to just our CA would
/// break all non-intercepted TLS, which is far worse than not configuring the
/// proxy at all.
pub fn ensure_combined_bundle(ca_cert_path: &Path) -> Result<PathBuf, String> {
    let ca_pem = std::fs::read_to_string(ca_cert_path)
        .map_err(|e| format!("Failed to read root CA {}: {e}", ca_cert_path.display()))?;

    // Reuse an existing bundle that already contains this CA. Locating the
    // base bundle shells out to Python, so this keeps the read-only setup
    // path (which renders the bundle's location) cheap to call repeatedly.
    let out_path = combined_bundle_path()?;
    if let Ok(existing) = std::fs::read_to_string(&out_path) {
        if existing.contains(ca_pem.trim()) {
            return Ok(out_path);
        }
    }

    let base_path = base_bundle_path().ok_or_else(|| {
        "Could not locate a system or certifi CA bundle to extend. Without one, setting \
         SSL_CERT_FILE would replace the tool's entire trust store and break every host \
         the proxy does not intercept."
            .to_string()
    })?;
    let base_pem = std::fs::read_to_string(&base_path)
        .map_err(|e| format!("Failed to read CA bundle {}: {e}", base_path.display()))?;

    let combined = compose_bundle(&base_pem, &ca_pem);
    if let Some(parent) = out_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create {}: {e}", parent.display()))?;
    }
    std::fs::write(&out_path, combined.as_bytes())
        .map_err(|e| format!("Failed to write {}: {e}", out_path.display()))?;
    Ok(out_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CA: &str = "-----BEGIN CERTIFICATE-----\nOURCA\n-----END CERTIFICATE-----\n";

    #[test]
    fn compose_puts_base_first_and_ca_last() {
        let base = "-----BEGIN CERTIFICATE-----\nBASE\n-----END CERTIFICATE-----\n";
        let out = compose_bundle(base, CA);
        assert!(out.starts_with("-----BEGIN CERTIFICATE-----\nBASE"));
        assert!(out.trim_end().ends_with("-----END CERTIFICATE-----"));
        assert!(out.contains("BASE"));
        assert!(out.contains("OURCA"));
        assert_eq!(out.matches("BEGIN CERTIFICATE").count(), 2);
    }

    #[test]
    fn compose_separates_blocks_when_base_lacks_trailing_newline() {
        let base = "-----BEGIN CERTIFICATE-----\nBASE\n-----END CERTIFICATE-----";
        let out = compose_bundle(base, CA);
        // The two PEM blocks must not collide on one line.
        assert!(out.contains("-----END CERTIFICATE-----\n-----BEGIN CERTIFICATE-----"));
    }

    #[test]
    fn compose_always_ends_with_newline() {
        let out = compose_bundle("BASE", "CA");
        assert!(out.ends_with('\n'));
    }
}
