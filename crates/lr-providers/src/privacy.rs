//! Explicit configured evidence, separate from authorization and certification.
use chrono::Utc;
use lr_config::ClientPrivacyConfig;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use utoipa::ToSchema;

pub const ATTESTATION_CONFIG: &str = "localrouter_privacy_attestation";
pub const ROUTE_CLIENT_EXTENSION: &str = "_localrouter_route_client";
pub const INTERNAL_EXTENSION: &str = "_localrouter_privacy";
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ProviderPrivacyAttestation {
    pub local_execution: bool,
    pub no_retention: bool,
    pub source: String,
    pub reviewed_at: i64,
    pub expires_at: Option<i64>,
}
impl ProviderPrivacyAttestation {
    pub fn validate(&self, now: i64) -> Result<(), String> {
        if self.source.trim().is_empty()
            || self.source.len() > 1024
            || self.source.chars().any(char::is_control)
            || self.reviewed_at < 0
            || self.reviewed_at > now
            || self
                .expires_at
                .is_some_and(|date| date <= self.reviewed_at || date > 253402300799)
        {
            return Err("Privacy attestation requires bounded provenance and a valid review/expiry timestamp".into());
        }
        Ok(())
    }
    pub fn current(&self, now: i64) -> bool {
        self.validate(now).is_ok() && self.expires_at.is_none_or(|date| date > now)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BoundAttestation {
    attestation: ProviderPrivacyAttestation,
    salt: String,
    binding: String,
}
fn binding(kind: &str, config: &HashMap<String, String>, salt: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hash = Sha256::new();
    for value in [env!("CARGO_PKG_VERSION"), kind, salt] {
        hash.update((value.len() as u64).to_be_bytes());
        hash.update(value.as_bytes());
    }
    let ordered = config
        .iter()
        .filter(|(key, _)| key.as_str() != ATTESTATION_CONFIG)
        .collect::<std::collections::BTreeMap<_, _>>();
    for (key, value) in ordered {
        for text in [key, value] {
            hash.update((text.len() as u64).to_be_bytes());
            hash.update(text.as_bytes());
        }
    }
    format!("{:x}", hash.finalize())
}
pub fn seal_attestation(
    kind: &str,
    config: &HashMap<String, String>,
    attestation: ProviderPrivacyAttestation,
) -> lr_types::AppResult<String> {
    attestation
        .validate(Utc::now().timestamp())
        .map_err(lr_types::AppError::InvalidParams)?;
    let salt = uuid::Uuid::new_v4().simple().to_string();
    serde_json::to_string(&BoundAttestation {
        binding: binding(kind, config, &salt),
        salt,
        attestation,
    })
    .map_err(|_| lr_types::AppError::InvalidParams("Could not seal privacy evidence".into()))
}
fn current_attestation(
    kind: &str,
    config: &HashMap<String, String>,
    now: i64,
) -> Option<ProviderPrivacyAttestation> {
    let bound: BoundAttestation = serde_json::from_str(config.get(ATTESTATION_CONFIG)?).ok()?;
    (bound.attestation.current(now) && bound.binding == binding(kind, config, &bound.salt))
        .then_some(bound.attestation)
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum PrivacyEvidenceSource {
    ManagedLocalEngine,
    UserAttestation,
    Unknown,
}
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct ProviderPrivacyAssessment {
    pub allowed: bool,
    pub requires_gateway_zdr: bool,
    pub local_execution: PrivacyEvidenceSource,
    pub no_retention: PrivacyEvidenceSource,
    pub source: Option<String>,
    pub reviewed_at: Option<i64>,
    pub expires_at: Option<i64>,
    pub reason: Option<String>,
}
/// A configured local endpoint is necessary for a manual local-execution
/// attestation. It is never sufficient evidence on its own.
pub fn loopback(config: &HashMap<String, String>) -> bool {
    config
        .get("base_url")
        .and_then(|root| reqwest::Url::parse(root).ok())
        .is_some_and(|url| {
            if !matches!(url.scheme(), "http" | "https")
                || !url.username().is_empty()
                || url.password().is_some()
            {
                return false;
            }
            url.host_str().is_some_and(|host| {
                host.eq_ignore_ascii_case("localhost")
                    || host
                        .trim_matches(['[', ']'])
                        .parse::<std::net::IpAddr>()
                        .is_ok_and(|ip| ip.is_loopback())
            })
        })
}
impl crate::registry::ProviderRegistry {
    pub fn privacy_assessment(
        &self,
        instance: &str,
        policy: &ClientPrivacyConfig,
    ) -> ProviderPrivacyAssessment {
        let now = Utc::now().timestamp();
        let config = self.get_provider_config(instance).unwrap_or_default();
        let managed = self
            .get_provider_type_for_instance(instance)
            .and_then(|kind| self.get_factory(&kind))
            .is_some_and(|factory| {
                factory.category() == crate::factory::ProviderCategory::Embedded
            });
        let kind = self
            .get_provider_type_for_instance(instance)
            .unwrap_or_default();
        let subscription = self.get_factory(&kind).is_some_and(|factory| {
            factory.category() == crate::factory::ProviderCategory::Subscription
        });
        // Dynamic subscription account identity has no verified stable binding.
        let attestation = if subscription {
            None
        } else {
            current_attestation(&kind, &config, now)
        };
        let local = managed
            || (loopback(&config) && attestation.as_ref().is_some_and(|a| a.local_execution));
        // Local execution alone says nothing about a service's storage policy.
        let no_retention = attestation.as_ref().is_some_and(|a| a.no_retention);
        let allowed = (!policy.local_only || local) && (!policy.no_retention || no_retention);
        let source = |proven: bool| {
            if managed {
                PrivacyEvidenceSource::ManagedLocalEngine
            } else if proven {
                PrivacyEvidenceSource::UserAttestation
            } else {
                PrivacyEvidenceSource::Unknown
            }
        };
        ProviderPrivacyAssessment {
            allowed,
            requires_gateway_zdr: policy.no_retention
                && self
                    .get_provider(instance)
                    .is_some_and(|provider| provider.supports_no_retention_filter()),
            local_execution: source(local),
            no_retention: if no_retention {
                PrivacyEvidenceSource::UserAttestation
            } else {
                PrivacyEvidenceSource::Unknown
            },
            source: attestation.as_ref().map(|a| a.source.clone()),
            reviewed_at: attestation.as_ref().map(|a| a.reviewed_at),
            expires_at: attestation.as_ref().and_then(|a| a.expires_at),
            reason: if allowed {
                None
            } else if policy.local_only && !local {
                Some("Local execution is unverified for this provider/account".into())
            } else {
                Some("No-retention policy is unverified for this provider/account".into())
            },
        }
    }
}

pub fn validate_config(config: &HashMap<String, String>) -> lr_types::AppResult<()> {
    if let Some(raw) = config
        .get(ATTESTATION_CONFIG)
        .filter(|text| !text.is_empty())
    {
        let bound: BoundAttestation = serde_json::from_str(raw).map_err(|_| {
            lr_types::AppError::InvalidParams(
                "Privacy evidence must be sealed by the reviewed-attestation action".into(),
            )
        })?;
        bound
            .attestation
            .validate(Utc::now().timestamp())
            .map_err(lr_types::AppError::InvalidParams)?;
        if bound.salt.len() != 32 || bound.binding.len() != 64 {
            return Err(lr_types::AppError::InvalidParams(
                "Invalid sealed privacy evidence".into(),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn evidence_is_bound_to_credentials_origin_settings_and_provider_kind() {
        let now = Utc::now().timestamp();
        let mut config = HashMap::from([
            ("base_url".into(), "http://localhost:8000/v1".into()),
            ("api_key".into(), "first-secret".into()),
        ]);
        let evidence = ProviderPrivacyAttestation {
            local_execution: true,
            no_retention: true,
            source: "Reviewed account and storage configuration".into(),
            reviewed_at: now,
            expires_at: Some(now + 60),
        };
        let sealed = seal_attestation("openai_compatible", &config, evidence).unwrap();
        config.insert(ATTESTATION_CONFIG.into(), sealed);
        assert!(current_attestation("openai_compatible", &config, now).is_some());
        assert!(current_attestation("other", &config, now).is_none());
        assert!(current_attestation("openai_compatible", &config, now + 60).is_none());
        for (key, value) in [
            ("api_key", "rotated-secret"),
            ("base_url", "http://localhost:9000/v1"),
            ("new_setting", "changed"),
        ] {
            let mut changed = config.clone();
            changed.insert(key.into(), value.into());
            assert!(current_attestation("openai_compatible", &changed, now).is_none());
        }
        let raw = config.get(ATTESTATION_CONFIG).unwrap();
        assert!(!raw.contains("first-secret"));
        assert!(!raw.contains("localhost"));
    }
    #[tokio::test]
    async fn loopback_without_review_fails_closed_and_review_enables_only_stated_claims() {
        use std::sync::Arc;
        let registry = crate::registry::ProviderRegistry::new();
        registry.register_factory(Arc::new(crate::factory::OpenAICompatibleProviderFactory));
        let mut config = HashMap::from([("base_url".into(), "http://localhost:8000/v1".into())]);
        registry
            .create_provider("local".into(), "openai_compatible".into(), config.clone())
            .await
            .unwrap();
        let local = ClientPrivacyConfig {
            local_only: true,
            ..Default::default()
        };
        assert!(!registry.privacy_assessment("local", &local).allowed);
        let evidence = ProviderPrivacyAttestation {
            local_execution: true,
            no_retention: false,
            source: "Reviewed local execution".into(),
            reviewed_at: Utc::now().timestamp(),
            expires_at: None,
        };
        let sealed = seal_attestation("openai_compatible", &config, evidence).unwrap();
        config.insert(ATTESTATION_CONFIG.into(), sealed);
        registry
            .update_provider("local".into(), "openai_compatible".into(), config)
            .await
            .unwrap();
        assert!(registry.privacy_assessment("local", &local).allowed);
        assert!(
            !registry
                .privacy_assessment(
                    "local",
                    &ClientPrivacyConfig {
                        no_retention: true,
                        ..Default::default()
                    }
                )
                .allowed
        );
    }
    #[test]
    fn local_address_requires_a_separate_attestation_and_remote_hosts_are_not_local() {
        assert!(loopback(&HashMap::from([(
            "base_url".into(),
            "http://127.0.0.1:9000/v1".into()
        )])));
        for root in [
            "https://example.com/v1",
            "http://127.0.0.1.example.com",
            "http://user@localhost",
            "http://192.168.1.1",
        ] {
            assert!(!loopback(&HashMap::from([(
                "base_url".into(),
                root.into()
            )])));
        }
    }
    #[test]
    fn missing_provenance_future_reviews_and_expired_assertions_are_not_current() {
        let mut value = ProviderPrivacyAttestation {
            local_execution: true,
            no_retention: true,
            source: "Reviewed local server/account configuration".into(),
            reviewed_at: 100,
            expires_at: Some(200),
        };
        assert!(value.current(150));
        assert!(!value.current(200));
        assert!(!value.current(50));
        value.source.clear();
        assert!(!value.current(150));
    }
}
