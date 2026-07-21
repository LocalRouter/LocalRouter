//! Certificate authority for the HTTPS inspection proxy.
//!
//! Generates (and persists) a single LocalRouter Proxy Root CA, and mints
//! short-lived leaf certificates on demand for each intercepted host, signed by
//! that root. Clients trust the root via `NODE_EXTRA_CA_CERTS` (or their OS
//! trust store); the proxy then presents forged leaves for the hosts it MITMs.
//!
//! The root CA private key is the sensitive artifact — it is written with
//! `0600` permissions and never leaves this machine.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use parking_lot::Mutex;
use rcgen::{
    BasicConstraints, CertificateParams, DistinguishedName, DnType, ExtendedKeyUsagePurpose, IsCa,
    Issuer, KeyPair, KeyUsagePurpose,
};
use rustls_pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};

use crate::error::ProxyError;

const ROOT_CA_CERT_FILE: &str = "root-ca.pem";
const ROOT_CA_KEY_FILE: &str = "root-ca.key";
const CA_COMMON_NAME: &str = "LocalRouter Proxy Root CA";

/// A minted leaf certificate (forged for a specific host) plus its key,
/// in both DER (for rustls) and PEM (for diagnostics) form.
pub struct LeafCert {
    /// Leaf certificate in DER form (what rustls serves).
    pub cert_der: CertificateDer<'static>,
    /// Leaf private key in DER (PKCS#8) form.
    pub key_der: PrivateKeyDer<'static>,
    /// Leaf + root CA chain in PEM form.
    pub chain_pem: String,
}

/// The proxy's certificate authority: a persisted root CA plus an in-memory
/// cache of per-host leaf certificates.
pub struct CertAuthority {
    /// Root CA certificate in PEM (handed to clients as the CA to trust).
    ca_pem: String,
    /// Path the root CA PEM is persisted at (for `NODE_EXTRA_CA_CERTS`).
    ca_path: PathBuf,
    /// Issuer used to sign leaves (owns the parsed CA params + key).
    issuer: Issuer<'static, KeyPair>,
    /// host -> minted leaf, so repeated connections reuse one cert.
    leaf_cache: Mutex<HashMap<String, Arc<LeafCert>>>,
}

impl CertAuthority {
    /// Load the root CA from `dir`, generating and persisting it on first use.
    ///
    /// Files: `<dir>/root-ca.pem` and `<dir>/root-ca.key`. If both exist they are
    /// reused; otherwise a fresh CA is generated (idempotent across restarts).
    pub fn load_or_create(dir: &Path) -> Result<Self, ProxyError> {
        std::fs::create_dir_all(dir)?;
        let cert_path = dir.join(ROOT_CA_CERT_FILE);
        let key_path = dir.join(ROOT_CA_KEY_FILE);

        let (ca_pem, ca_key) = if cert_path.exists() && key_path.exists() {
            let ca_pem = std::fs::read_to_string(&cert_path)?;
            let key_pem = std::fs::read_to_string(&key_path)?;
            let ca_key = KeyPair::from_pem(&key_pem)
                .map_err(|e| ProxyError::Cert(format!("parse root CA key: {e}")))?;
            (ca_pem, ca_key)
        } else {
            let (ca_pem, key_pem, ca_key) = generate_root_ca()?;
            write_secret(&key_path, key_pem.as_bytes())?;
            std::fs::write(&cert_path, ca_pem.as_bytes())?;
            (ca_pem, ca_key)
        };

        let issuer = Issuer::from_ca_cert_pem(&ca_pem, ca_key)
            .map_err(|e| ProxyError::Cert(format!("build issuer from root CA: {e}")))?;

        Ok(Self {
            ca_pem,
            ca_path: cert_path,
            issuer,
            leaf_cache: Mutex::new(HashMap::new()),
        })
    }

    /// Root CA certificate in PEM form (the value clients must trust).
    pub fn ca_pem(&self) -> &str {
        &self.ca_pem
    }

    /// Filesystem path of the persisted root CA PEM (`NODE_EXTRA_CA_CERTS`).
    pub fn ca_cert_path(&self) -> &Path {
        &self.ca_path
    }

    /// Mint (or reuse) a leaf certificate for `host`, signed by the root CA.
    pub fn leaf_for(&self, host: &str) -> Result<Arc<LeafCert>, ProxyError> {
        if let Some(existing) = self.leaf_cache.lock().get(host) {
            return Ok(existing.clone());
        }

        let leaf = Arc::new(self.mint_leaf(host)?);
        self.leaf_cache
            .lock()
            .insert(host.to_string(), leaf.clone());
        Ok(leaf)
    }

    fn mint_leaf(&self, host: &str) -> Result<LeafCert, ProxyError> {
        let leaf_key =
            KeyPair::generate().map_err(|e| ProxyError::Cert(format!("gen leaf key: {e}")))?;

        let mut params = CertificateParams::new(vec![host.to_string()])
            .map_err(|e| ProxyError::Cert(format!("leaf params for {host}: {e}")))?;
        let mut dn = DistinguishedName::new();
        dn.push(DnType::CommonName, host);
        params.distinguished_name = dn;
        params.is_ca = IsCa::NoCa;
        params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];

        let leaf_cert = params
            .signed_by(&leaf_key, &self.issuer)
            .map_err(|e| ProxyError::Cert(format!("sign leaf for {host}: {e}")))?;

        let cert_der = leaf_cert.der().clone();
        let key_der =
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(leaf_key.serialize_der()).clone_key());
        let chain_pem = format!("{}{}", leaf_cert.pem(), self.ca_pem);

        Ok(LeafCert {
            cert_der,
            key_der,
            chain_pem,
        })
    }
}

/// Generate a fresh root CA. Returns (cert PEM, key PEM, key pair).
fn generate_root_ca() -> Result<(String, String, KeyPair), ProxyError> {
    let ca_key =
        KeyPair::generate().map_err(|e| ProxyError::Cert(format!("gen root CA key: {e}")))?;

    let mut params = CertificateParams::new(Vec::new())
        .map_err(|e| ProxyError::Cert(format!("root CA params: {e}")))?;
    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, CA_COMMON_NAME);
    dn.push(DnType::OrganizationName, "LocalRouter");
    params.distinguished_name = dn;
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    params.key_usages = vec![
        KeyUsagePurpose::KeyCertSign,
        KeyUsagePurpose::CrlSign,
        KeyUsagePurpose::DigitalSignature,
    ];

    let ca_cert = params
        .self_signed(&ca_key)
        .map_err(|e| ProxyError::Cert(format!("self-sign root CA: {e}")))?;

    Ok((ca_cert.pem(), ca_key.serialize_pem(), ca_key))
}

/// Write a secret file with owner-only permissions where the OS supports it.
fn write_secret(path: &Path, bytes: &[u8]) -> Result<(), ProxyError> {
    std::fs::write(path, bytes)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generates_and_reuses_root_ca() {
        let dir = tempdir();
        let ca1 = CertAuthority::load_or_create(&dir).unwrap();
        let pem1 = ca1.ca_pem().to_string();
        assert!(pem1.contains("BEGIN CERTIFICATE"));
        assert!(ca1.ca_cert_path().exists());

        // Reload should reuse the same persisted CA, not regenerate.
        let ca2 = CertAuthority::load_or_create(&dir).unwrap();
        assert_eq!(pem1, ca2.ca_pem(), "root CA must be stable across reloads");
    }

    #[test]
    fn mints_and_caches_leaf_certs() {
        let dir = tempdir();
        let ca = CertAuthority::load_or_create(&dir).unwrap();

        let leaf = ca.leaf_for("api.anthropic.com").unwrap();
        assert!(leaf.chain_pem.contains("BEGIN CERTIFICATE"));
        assert!(!leaf.cert_der.as_ref().is_empty());

        // Same host returns the cached Arc (pointer-equal).
        let leaf2 = ca.leaf_for("api.anthropic.com").unwrap();
        assert!(Arc::ptr_eq(&leaf, &leaf2));

        // Different host mints a distinct cert.
        let other = ca.leaf_for("api.openai.com").unwrap();
        assert!(!Arc::ptr_eq(&leaf, &other));
    }

    #[cfg(unix)]
    #[test]
    fn root_ca_key_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempdir();
        let _ca = CertAuthority::load_or_create(&dir).unwrap();
        let mode = std::fs::metadata(dir.join(ROOT_CA_KEY_FILE))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    /// Minimal unique temp dir without pulling in a dev-dependency.
    /// Combines a nanosecond stamp with a process-unique counter so parallel
    /// test threads never share a directory.
    fn tempdir() -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let mut p = std::env::temp_dir();
        p.push(format!("lr-proxy-test-{nanos}-{n}"));
        std::fs::create_dir_all(&p).unwrap();
        p
    }
}
