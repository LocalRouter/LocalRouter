//! Trusting the proxy's root CA in the operating system's trust store.
//!
//! Some tools validate TLS through the platform verifier and expose no CA
//! setting at all — Goose's ChatGPT-Codex client, Zed, and anything running
//! inside VS Code. For those, interception only validates if LocalRouter's
//! root CA is trusted by the OS.
//!
//! This is the most consequential thing LocalRouter can do to a machine: a
//! trusted root CA can vouch for **any** host, so it is only ever performed as
//! an explicit, individually-confirmed user action, and always with a
//! one-click undo. Two deliberate design choices follow from that:
//!
//! - **User keychain, not the system keychain.** On macOS the cert goes into
//!   the user's login keychain, which needs no administrator rights and is
//!   scoped to this user's applications. `-d` (admin/system-wide) is never
//!   passed.
//! - **No silent installation.** Nothing here runs as part of applying a proxy
//!   config; the UI surfaces it separately with its own confirmation.

use std::path::Path;

/// Whether the root CA is currently trusted for TLS by the OS.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TrustState {
    // Trusted/NotTrusted are only constructed on macOS (the one platform whose
    // trust store we can query), but they are part of the serialized contract
    // with the frontend on every platform.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    Trusted,
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    NotTrusted,
    /// We can't determine it (no CA generated yet, or unsupported platform).
    Unknown,
}

/// Trust status plus what the user can do about it on this platform.
#[derive(Debug, Clone, serde::Serialize)]
pub struct CaTrustStatus {
    pub state: TrustState,
    /// Whether LocalRouter can add/remove the trust itself here.
    pub can_manage: bool,
    /// Path of the root CA in question.
    pub ca_cert_path: String,
    /// Manual steps, for platforms we can't manage automatically.
    pub manual_instructions: Option<String>,
}

#[cfg(target_os = "macos")]
fn login_keychain() -> Result<std::path::PathBuf, String> {
    let home = dirs::home_dir().ok_or_else(|| "Could not resolve home directory".to_string())?;
    let modern = home
        .join("Library")
        .join("Keychains")
        .join("login.keychain-db");
    if modern.exists() {
        return Ok(modern);
    }
    // Pre-Sierra naming, still present on some upgraded machines.
    let legacy = home
        .join("Library")
        .join("Keychains")
        .join("login.keychain");
    if legacy.exists() {
        return Ok(legacy);
    }
    Err("Could not find your login keychain".to_string())
}

/// Read the current trust state.
pub fn status(ca_cert_path: &Path) -> CaTrustStatus {
    let path_str = ca_cert_path.display().to_string();

    if !ca_cert_path.exists() {
        // The CA is generated on first proxy start. Platforms we can't manage
        // still need their manual steps here, or the UI would say the CA must
        // be trusted while offering neither a button nor instructions.
        let can_manage = cfg!(target_os = "macos");
        return CaTrustStatus {
            state: TrustState::Unknown,
            can_manage,
            manual_instructions: (!can_manage).then(|| manual_instructions(&path_str)),
            ca_cert_path: path_str,
        };
    }

    #[cfg(target_os = "macos")]
    {
        // `verify-cert` against the SSL policy answers the question we
        // actually care about: would a TLS client accept a chain signed by
        // this CA? A non-zero exit means "not trusted", not an error.
        let verdict = std::process::Command::new("/usr/bin/security")
            .args(["verify-cert", "-p", "ssl", "-c"])
            .arg(ca_cert_path)
            .output();
        let state = match verdict {
            Ok(out) if out.status.success() => TrustState::Trusted,
            Ok(_) => TrustState::NotTrusted,
            Err(_) => TrustState::Unknown,
        };
        CaTrustStatus {
            state,
            can_manage: true,
            ca_cert_path: path_str,
            manual_instructions: None,
        }
    }

    #[cfg(not(target_os = "macos"))]
    {
        CaTrustStatus {
            state: TrustState::Unknown,
            can_manage: false,
            ca_cert_path: path_str.clone(),
            manual_instructions: Some(manual_instructions(&path_str)),
        }
    }
}

/// Platform-specific manual steps, for where we can't manage trust ourselves.
#[cfg_attr(target_os = "macos", allow(dead_code))]
pub fn manual_instructions(ca_cert_path: &str) -> String {
    if cfg!(target_os = "windows") {
        format!(
            "Import {ca_cert_path} into \"Trusted Root Certification Authorities\" for your user \
             account: run `certutil -user -addstore Root \"{ca_cert_path}\"`, or open the file and \
             use the Certificate Import Wizard. Remove it later with \
             `certutil -user -delstore Root \"LocalRouter\"`."
        )
    } else {
        format!(
            "Copy {ca_cert_path} into your distribution's trust store and refresh it — e.g. on \
             Debian/Ubuntu: `sudo cp {ca_cert_path} /usr/local/share/ca-certificates/localrouter.crt \
             && sudo update-ca-certificates`; on Fedora/RHEL: \
             `sudo cp {ca_cert_path} /etc/pki/ca-trust/source/anchors/localrouter.crt \
             && sudo update-ca-trust`. To remove it later, delete that copied file and run the \
             same refresh command again. Note that tools using their own trust store (Firefox, \
             some language runtimes) need to be configured separately."
        )
    }
}

/// Add the root CA to the user's trust store.
///
/// Returns a human-readable confirmation. On unsupported platforms this fails
/// with the manual steps rather than doing something partial.
pub fn trust(ca_cert_path: &Path) -> Result<String, String> {
    if !ca_cert_path.exists() {
        return Err(format!(
            "Root CA not found at {}. Start the inspection proxy once to generate it.",
            ca_cert_path.display()
        ));
    }

    #[cfg(target_os = "macos")]
    {
        let keychain = login_keychain()?;
        // No `-d`: this is the *user's* trust store, so no administrator
        // rights are required and the change is scoped to this user.
        // `-r trustRoot` marks it as a trusted anchor for SSL.
        let out = std::process::Command::new("/usr/bin/security")
            .arg("add-trusted-cert")
            .args(["-r", "trustRoot", "-p", "ssl", "-k"])
            .arg(&keychain)
            .arg(ca_cert_path)
            .output()
            .map_err(|e| format!("Failed to run `security add-trusted-cert`: {e}"))?;

        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
            return Err(if stderr.is_empty() {
                "Adding the certificate to your login keychain was cancelled or failed.".to_string()
            } else {
                format!("Could not trust the certificate: {stderr}")
            });
        }
        Ok(format!(
            "LocalRouter's root CA is now trusted in your login keychain ({}).",
            keychain.display()
        ))
    }

    #[cfg(not(target_os = "macos"))]
    {
        Err(manual_instructions(&ca_cert_path.display().to_string()))
    }
}

/// Remove the root CA from the user's trust store.
pub fn untrust(ca_cert_path: &Path) -> Result<String, String> {
    #[cfg(target_os = "macos")]
    {
        if !ca_cert_path.exists() {
            return Err(format!("Root CA not found at {}.", ca_cert_path.display()));
        }
        // `remove-trusted-cert` drops the trust setting; the cert itself is
        // then harmless. `-d` is not used here either (we never added it
        // system-wide).
        let out = std::process::Command::new("/usr/bin/security")
            .arg("remove-trusted-cert")
            .arg(ca_cert_path)
            .output()
            .map_err(|e| format!("Failed to run `security remove-trusted-cert`: {e}"))?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
            return Err(if stderr.is_empty() {
                "Removing the certificate trust was cancelled or failed.".to_string()
            } else {
                format!("Could not remove the certificate trust: {stderr}")
            });
        }
        Ok("LocalRouter's root CA is no longer trusted.".to_string())
    }

    #[cfg(not(target_os = "macos"))]
    {
        Err(format!(
            "Remove {} from your system trust store manually.",
            ca_cert_path.display()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_ca_is_unknown_not_trusted() {
        let s = status(Path::new("/nonexistent/does-not-exist.pem"));
        assert_eq!(s.state, TrustState::Unknown);
        assert!(s.ca_cert_path.contains("does-not-exist.pem"));
    }

    #[test]
    fn user_always_gets_either_a_button_or_instructions() {
        // Otherwise the UI tells them the CA must be trusted and offers no
        // way to do it.
        for p in ["/nonexistent/nope.pem", "/etc/hosts"] {
            let s = status(Path::new(p));
            assert!(
                s.can_manage || s.manual_instructions.is_some(),
                "no actionable path for {p}"
            );
        }
    }

    #[test]
    fn trusting_a_missing_ca_fails_with_guidance() {
        let err = trust(Path::new("/nonexistent/nope.pem")).unwrap_err();
        assert!(err.contains("not found"));
    }

    #[test]
    fn manual_instructions_name_the_cert_and_a_removal_path() {
        let text = manual_instructions("/tmp/ca.pem");
        assert!(text.contains("/tmp/ca.pem"));
        // Users must be able to undo whatever we tell them to do.
        let lowered = text.to_lowercase();
        assert!(
            lowered.contains("remove") || lowered.contains("delstore"),
            "manual steps must explain removal too"
        );
    }

    /// Guard the security-critical flags: the certificate must go into the
    /// user's login keychain, never the admin/system domain (`-d`).
    #[cfg(target_os = "macos")]
    #[test]
    fn login_keychain_path_is_user_scoped() {
        if let Ok(kc) = login_keychain() {
            let s = kc.display().to_string();
            assert!(s.contains("Library/Keychains/login.keychain"));
            assert!(!s.starts_with("/Library/Keychains/System"));
        }
    }
}
