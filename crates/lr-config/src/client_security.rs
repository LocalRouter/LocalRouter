//! Credential lifetime and advisory review policy. No secret values live here.
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Explicit grants for standalone native provider operations.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum NativeOperation {
    Ocr,
}

/// Default-off constraints on native LLM traffic. These do not grant access.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct ClientPrivacyConfig {
    pub local_only: bool,
    pub no_retention: bool,
    pub metadata_only: bool,
}
impl ClientPrivacyConfig {
    pub fn is_default(&self) -> bool {
        self == &Self::default()
    }
    pub fn redact_payloads(&self) -> bool {
        self.no_retention || self.metadata_only
    }
    pub fn restricted(&self) -> bool {
        self.local_only || self.no_retention
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ClientSecurityConfig {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub native_operations: Vec<NativeOperation>,
    #[serde(
        default,
        skip_serializing_if = "crate::UpstreamToolsPolicy::is_default"
    )]
    pub upstream_tools: crate::UpstreamToolsPolicy,
    #[serde(default, skip_serializing_if = "ClientPrivacyConfig::is_default")]
    pub privacy: ClientPrivacyConfig,
    pub credential_expires_at: Option<DateTime<Utc>>,
    /// Advisory review only: inactivity never silently revokes a credential.
    pub review_idle_after_days: Option<u32>,
}
impl ClientSecurityConfig {
    pub fn is_default(&self) -> bool {
        self == &Self::default()
    }
    pub fn validate(&self) -> Result<(), String> {
        self.upstream_tools.validate()?;
        let mut unique = std::collections::HashSet::new();
        if self
            .native_operations
            .iter()
            .any(|operation| !unique.insert(*operation))
        {
            return Err("Native operation grants must be unique".into());
        }

        if self
            .review_idle_after_days
            .is_some_and(|days| !(1..=3650).contains(&days))
        {
            return Err("Idle review must be between 1 and 3650 days".into());
        }
        if self
            .credential_expires_at
            .is_some_and(|date| !(0..=253402300799).contains(&date.timestamp()))
        {
            return Err("Credential expiry must be between 1970 and 9999".into());
        }
        Ok(())
    }
}
impl crate::Client {
    pub fn credentials_active_at(&self, now: DateTime<Utc>) -> bool {
        self.enabled
            && self
                .security
                .credential_expires_at
                .is_none_or(|expiry| now < expiry)
    }
    pub fn credentials_active(&self) -> bool {
        self.credentials_active_at(Utc::now())
    }
    pub fn idle_review_due_at(&self, now: DateTime<Utc>) -> bool {
        self.security.review_idle_after_days.is_some_and(|days| {
            now.signed_duration_since(self.last_used.unwrap_or(self.created_at))
                >= chrono::Duration::days(days as i64)
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_operations_are_default_off_and_duplicate_or_unknown_grants_reject() {
        let old: ClientSecurityConfig = serde_json::from_str("{}").unwrap();
        assert!(old.native_operations.is_empty());
        assert!(serde_json::to_value(&old)
            .unwrap()
            .get("native_operations")
            .is_none());
        let mut config = old;
        config.native_operations = vec![NativeOperation::Ocr];
        assert!(config.validate().is_ok());
        config.native_operations.push(NativeOperation::Ocr);
        assert!(config.validate().is_err());
        assert!(serde_json::from_str::<ClientSecurityConfig>(
            r#"{"native_operations":["unknown"]}"#
        )
        .is_err());
    }

    #[test]
    fn expiry_has_an_exact_boundary_and_review_is_advisory() {
        let now = Utc::now();
        let mut client = crate::Client::new_with_strategy("test".into(), "strategy".into());
        assert!(client.credentials_active_at(now));
        client.security.credential_expires_at = Some(now);
        assert!(!client.credentials_active_at(now));
        client.security.credential_expires_at = Some(now + chrono::Duration::nanoseconds(1));
        assert!(client.credentials_active_at(now));
        client.security.credential_expires_at = None;
        client.security.review_idle_after_days = Some(30);
        client.created_at = now - chrono::Duration::days(30);
        assert!(client.idle_review_due_at(now));
        assert!(client.credentials_active_at(now));
        client.last_used = Some(now - chrono::Duration::days(29));
        assert!(!client.idle_review_due_at(now));
        client.enabled = false;
        assert!(!client.credentials_active_at(now));
    }
    #[test]
    fn old_configs_default_to_no_new_expiry_and_invalid_review_is_rejected() {
        let config: ClientSecurityConfig = serde_json::from_str("{}").unwrap();
        assert!(config.is_default());
        for days in [0, 3651, u32::MAX] {
            assert!(ClientSecurityConfig {
                review_idle_after_days: Some(days),
                ..Default::default()
            }
            .validate()
            .is_err());
        }
    }
}
