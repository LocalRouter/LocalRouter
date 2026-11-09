//! Local round-trip playground using the production scanner and vault.
//! Preview originals live only in a fresh memory store for this one invocation.
use crate::{
    placeholders::Vault, ExtractedText, ScanResult, SecretScanEngine, SecretScanEngineConfig,
};
use lr_api_keys::KeychainStorage;
use lr_types::{secret_context::SecretContext, AppError, AppResult};
use serde::Serialize;
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};
use zeroize::Zeroizing;

#[derive(Serialize)]
pub struct SecretScanPreview {
    #[serde(flatten)]
    pub scan: ScanResult,
    pub redacted_request: String,
    pub model_response: String,
    pub restored_response: String,
    pub round_trip_matches: bool,
    pub restored_references: usize,
    pub warnings: Vec<String>,
}
#[derive(Default)]
struct PreviewStorage(Mutex<BTreeMap<(String, String), Zeroizing<String>>>);
impl KeychainStorage for PreviewStorage {
    fn store(&self, s: &str, a: &str, value: &str) -> AppResult<()> {
        self.0
            .lock()
            .map_err(|_| AppError::Internal("Preview store unavailable".into()))?
            .insert((s.into(), a.into()), Zeroizing::new(value.into()));
        Ok(())
    }
    fn get(&self, s: &str, a: &str) -> AppResult<Option<String>> {
        Ok(self
            .0
            .lock()
            .map_err(|_| AppError::Internal("Preview store unavailable".into()))?
            .get(&(s.into(), a.into()))
            .map(|s| s.to_string()))
    }
    fn delete(&self, s: &str, a: &str) -> AppResult<()> {
        self.0
            .lock()
            .map_err(|_| AppError::Internal("Preview store unavailable".into()))?
            .remove(&(s.into(), a.into()));
        Ok(())
    }
}
fn encode(text: &str, format: &str) -> AppResult<String> {
    match format {
        "plain" => Ok(text.into()),
        "json" => Ok(serde_json::to_string(text)?),
        "nested_json" => Ok(serde_json::to_string(&serde_json::to_string(text)?)?),
        "html" => Ok(text
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
            .replace('\'', "&#39;")
            .replace('-', "&#45;")),
        _ => Err(AppError::InvalidParams(
            "Unsupported preview encoding".into(),
        )),
    }
}
fn render(template: &str, value: &str) -> AppResult<String> {
    let occurrences = template.matches("{{redacted}}").count();
    let size = template
        .len()
        .saturating_add(occurrences.saturating_mul(value.len()));
    if size > 1024 * 1024 {
        return Err(AppError::InvalidParams(
            "Expanded preview must be at most 1 MiB".into(),
        ));
    }
    Ok(template.replace("{{redacted}}", value))
}
pub fn round_trip(
    config: &SecretScanEngineConfig,
    input: String,
    response_template: Option<String>,
    response_encoding: Option<String>,
) -> AppResult<SecretScanPreview> {
    let template = response_template.unwrap_or_else(|| "{{redacted}}".into());
    if input.len() > 256 * 1024 || template.len() > 256 * 1024 {
        return Err(AppError::InvalidParams(
            "Preview input must be at most 256 KiB".into(),
        ));
    }
    let format = response_encoding.as_deref().unwrap_or("plain");
    let expected = render(&template, &encode(&input, format)?)?;
    let scanner = SecretScanEngine::new(config).map_err(AppError::InvalidParams)?;
    let scan = scanner.scan(&[ExtractedText {
        label: "preview".into(),
        text: input.clone(),
        message_index: 0,
    }]);
    let vault = Vault::for_client(Arc::new(PreviewStorage::default()), "local-preview");
    for finding in &scan.findings {
        vault.register(&finding.raw_text)?;
    }
    let restored_ids = Arc::new(Mutex::new(std::collections::BTreeSet::new()));
    let restored_sink = restored_ids.clone();
    let warnings = Arc::new(Mutex::new(Vec::new()));
    let sink = warnings.clone();
    vault.set_operation_observer(Arc::new(move |operation| {
        if operation.action == "restore" {
            if let Ok(mut ids) = restored_sink.lock() {
                ids.insert(operation.reference_id);
            }
        } else if operation.action == "restore_warning" {
            if let Ok(mut warnings) = sink.lock() {
                if !warnings.contains(&operation.location) {
                    warnings.push(operation.location);
                }
            }
        }
    }));
    let mut outbound = serde_json::Value::String(input);
    vault.protect(&mut outbound)?;
    let redacted_request = outbound.as_str().unwrap_or_default().to_string();
    let model_response = render(&template, &encode(&redacted_request, format)?)?;
    let restored_response = vault.restore_text(&model_response, false)?;
    let warnings = warnings
        .lock()
        .map_err(|_| AppError::Internal("Preview diagnostics unavailable".into()))?
        .clone();
    let restored_references = restored_ids
        .lock()
        .map_err(|_| AppError::Internal("Preview diagnostics unavailable".into()))?
        .len();
    Ok(SecretScanPreview {
        scan,
        redacted_request,
        model_response,
        round_trip_matches: restored_response == expected,
        restored_references,
        restored_response,
        warnings,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn playground_uses_real_masking_and_restoration_without_retaining_mappings() {
        let config = SecretScanEngineConfig {
            entropy_threshold: 3.5,
            allowlist: Vec::new(),
            scan_system_messages: true,
        };
        let input = "aws_secret_access_key=6nmCEa00cbNmH0B4zKPSYOPP5z1oYFR26doJldQr";
        for encoding in ["plain", "json", "nested_json", "html"] {
            let preview = round_trip(&config, input.into(), None, Some(encoding.into())).unwrap();
            assert!(!preview.scan.findings.is_empty());
            assert!(!preview.redacted_request.contains("6nmCEa00"));
            assert!(
                preview.round_trip_matches,
                "{encoding}: {}",
                preview.restored_response
            );
        }
        let first = round_trip(&config, input.into(), None, None).unwrap();
        let second = round_trip(
            &config,
            input.into(),
            Some(first.redacted_request.clone()),
            None,
        )
        .unwrap();
        assert_eq!(second.restored_response, first.redacted_request);
        assert!(!second.warnings.is_empty());
    }
}
