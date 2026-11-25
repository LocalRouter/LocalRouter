//! macOS: certificates with trust settings in the user, admin and system
//! domains.
//!
//! Apple documents the precedence: "Per-user Trust Settings override locally
//! administered Trust Settings, which in turn override the System Trust
//! Settings." So each certificate is decided by the first domain, in that
//! order, that decides it — a root the system trusts but the user marked
//! "Never Trust" is left out, and a root the user added and trusts is kept.
//!
//! A bundle file can only say "trusted for every host, for every program". So
//! only trust that is that broad on macOS is carried over: a setting limited
//! to one host name (`kSecTrustSettingsPolicyString`), to one application, or
//! to overlooking a specific error (an expired certificate, a host-name
//! mismatch) grants nothing here. Distrust is applied however narrowly it was
//! scoped, so a bundle never trusts more than macOS does.

use std::collections::HashMap;
use std::ptr;

use core_foundation::array::{CFArray, CFArrayRef};
use core_foundation::base::{CFType, TCFType};
use core_foundation::dictionary::{CFDictionary, CFDictionaryRef};
use core_foundation::number::CFNumber;
use core_foundation::string::{CFString, CFStringRef};
use security_framework::certificate::SecCertificate;
use security_framework::trust_settings::{Domain, TrustSettings};
use security_framework_sys::base::SecPolicyRef;
use security_framework_sys::trust_settings::{
    SecTrustSettingsCopyTrustSettings, SecTrustSettingsDomain,
};

#[link(name = "Security", kind = "framework")]
extern "C" {
    fn SecPolicyCopyProperties(policy: SecPolicyRef) -> CFDictionaryRef;
    static kSecPolicyOid: CFStringRef;
    static kSecPolicyAppleSSL: CFStringRef;
}

// kSecTrustSettingsResult values (SecTrustSettings.h).
const RESULT_TRUST_ROOT: i64 = 1;
const RESULT_TRUST_AS_ROOT: i64 = 2;
const RESULT_DENY: i64 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Decision {
    Trust,
    Deny,
    /// This domain has settings for the certificate, but none that decide
    /// general TLS trust; a lower-precedence domain may.
    Undecided,
}

pub fn load() -> Result<Vec<Vec<u8>>, String> {
    let mut decided: HashMap<Vec<u8>, Decision> = HashMap::new();
    for domain in [Domain::User, Domain::Admin, Domain::System] {
        let iter = match TrustSettings::new(domain).iter() {
            Ok(iter) => iter,
            // The user and admin domains are commonly empty, which the API
            // reports as an error.
            Err(_) if domain != Domain::System => continue,
            Err(e) => return Err(format!("Failed to read the system trust settings: {e}")),
        };
        for cert in iter {
            let der = cert.to_der();
            if matches!(decided.get(&der), Some(Decision::Trust | Decision::Deny)) {
                continue; // a higher-precedence domain already decided it
            }
            let decision = match copy_trust_settings(&cert, domain) {
                Ok(settings) => decide(&settings),
                // Unreadable settings: never guess trust, and do not let a
                // lower domain grant it either.
                Err(_) => Decision::Deny,
            };
            decided.insert(der, decision);
        }
    }
    Ok(decided
        .into_iter()
        .filter(|(_, d)| *d == Decision::Trust)
        .map(|(der, _)| der)
        .collect())
}

fn copy_trust_settings(
    cert: &SecCertificate,
    domain: Domain,
) -> Result<CFArray<CFDictionary>, i32> {
    let mut array: CFArrayRef = ptr::null();
    let status = unsafe {
        SecTrustSettingsCopyTrustSettings(
            cert.as_concrete_TypeRef(),
            SecTrustSettingsDomain::from(domain),
            &mut array,
        )
    };
    if status != 0 || array.is_null() {
        return Err(status);
    }
    Ok(unsafe { CFArray::wrap_under_create_rule(array) })
}

/// One trust-settings entry, reduced to what decides general TLS trust.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Entry {
    /// The entry's policy is TLS (or it names no policy, i.e. applies to all).
    tls: bool,
    /// Limited to a host name, an application, or a specific error.
    constrained: bool,
    /// `kSecTrustSettingsResult` (Apple: absent means TrustRoot).
    result: i64,
}

/// Apple: "an empty Trust Settings array means always trust this cert";
/// otherwise the entries are evaluated in order and the first one that
/// applies with a result other than "unspecified" decides.
fn decide_entries(entries: &[Entry]) -> Decision {
    if entries.is_empty() {
        return Decision::Trust;
    }
    for e in entries.iter().filter(|e| e.tls) {
        match e.result {
            // Distrust counts however narrowly it is scoped.
            RESULT_DENY => return Decision::Deny,
            RESULT_TRUST_ROOT | RESULT_TRUST_AS_ROOT if !e.constrained => return Decision::Trust,
            _ => {}
        }
    }
    Decision::Undecided
}

fn decide(settings: &CFArray<CFDictionary>) -> Decision {
    let entries: Vec<Entry> = settings.iter().map(|d| entry(&d)).collect();
    decide_entries(&entries)
}

fn entry(dict: &CFDictionary) -> Entry {
    let get = |key: &'static str| -> Option<CFType> {
        let key = CFString::from_static_string(key);
        dict.find(key.as_CFTypeRef().cast())
            .map(|v| unsafe { CFType::wrap_under_get_rule((*v).cast()) })
    };
    let result = get("kSecTrustSettingsResult")
        .and_then(|v| v.downcast::<CFNumber>())
        .and_then(|n| n.to_i64())
        .unwrap_or(RESULT_TRUST_ROOT);
    let constrained = get("kSecTrustSettingsPolicyString").is_some()
        || get("kSecTrustSettingsApplication").is_some()
        || get("kSecTrustSettingsAllowedError").is_some();
    let tls = match get("kSecTrustSettingsPolicy") {
        None => true, // no policy: applies to every policy, TLS included
        Some(policy) => policy_is_tls(&policy),
    };
    Entry {
        tls,
        constrained,
        result,
    }
}

fn policy_is_tls(policy: &CFType) -> bool {
    unsafe {
        let props = SecPolicyCopyProperties(policy.as_CFTypeRef() as SecPolicyRef);
        if props.is_null() {
            return false;
        }
        let props: CFDictionary = CFDictionary::wrap_under_create_rule(props);
        let oid = props
            .find(kSecPolicyOid.cast())
            .map(|v| CFType::wrap_under_get_rule((*v).cast()))
            .and_then(|v| v.downcast::<CFString>());
        oid.is_some_and(|oid| oid == CFString::wrap_under_get_rule(kSecPolicyAppleSSL))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(tls: bool, constrained: bool, result: i64) -> Entry {
        Entry {
            tls,
            constrained,
            result,
        }
    }

    #[test]
    fn no_settings_means_trusted() {
        assert_eq!(decide_entries(&[]), Decision::Trust);
    }

    #[test]
    fn unconstrained_tls_trust_is_trusted() {
        assert_eq!(
            decide_entries(&[e(true, false, RESULT_TRUST_ROOT)]),
            Decision::Trust
        );
        assert_eq!(
            decide_entries(&[e(true, false, RESULT_TRUST_AS_ROOT)]),
            Decision::Trust
        );
    }

    /// A certificate trusted only for one host (or app, or despite one
    /// error) must not become trusted for every host.
    #[test]
    fn constrained_trust_grants_nothing() {
        assert_eq!(
            decide_entries(&[
                e(true, true, RESULT_TRUST_ROOT),
                e(true, true, RESULT_TRUST_ROOT)
            ]),
            Decision::Undecided
        );
    }

    #[test]
    fn any_tls_distrust_denies() {
        assert_eq!(
            decide_entries(&[e(true, false, RESULT_DENY)]),
            Decision::Deny
        );
        assert_eq!(
            decide_entries(&[e(true, true, RESULT_DENY)]),
            Decision::Deny
        );
    }

    #[test]
    fn settings_for_other_policies_do_not_decide_tls() {
        assert_eq!(
            decide_entries(&[e(false, false, RESULT_DENY)]),
            Decision::Undecided
        );
        assert_eq!(
            decide_entries(&[e(false, false, RESULT_TRUST_ROOT)]),
            Decision::Undecided
        );
    }

    #[test]
    fn unspecified_entries_are_skipped() {
        assert_eq!(
            decide_entries(&[e(true, false, 4), e(true, false, RESULT_TRUST_ROOT)]),
            Decision::Trust
        );
    }
}
