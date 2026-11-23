//! The outbound secret scan split for requests that are inspected one unit at
//! a time (the HTTPS proxy's streamed request bodies).
//!
//! [`scan_request_for_secrets_fresh`](super::scan_request_for_secrets_fresh)
//! resolves a policy context, scans the whole request and handles the
//! findings. Here the policy context is [`SecretScanPolicy`] (shared with the
//! whole-request scan), detection runs per unit ([`StreamSecretScan::inspect`],
//! with the same projection, text extraction and per-value deduplication),
//! and the findings are handled once, at the end of the body
//! ([`StreamSecretScan::finish`]): one aggregated `record_secret_scan` and,
//! for Notify, one popup. Nothing here needs the whole request as a
//! `serde_json::Value`.

use std::sync::Arc;

use lr_types::secret_context::SecretContext;
use serde_json::Value;

use super::{secret_scan_llm_source, secret_scan_popup_preview, secret_scan_warning};
use super::{ClientAuthContext, SecretScanOutcome};
use crate::state::AppState;

/// Characters of context kept on each side of a finding in the monitor.
const EXCERPT: usize = 120;
/// Bytes of context kept on each side of a finding for the popup of a
/// streamed request (the whole text, as the whole-request popup shows, is
/// not held).
const POPUP_CONTEXT: usize = 64 * 1024;

/// What a remembered decision says about one detected value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Remembered {
    /// Always redact it (replaced with the redaction message).
    Redact,
    /// Ignore it.
    Ignore,
    /// No decision: handle it with the configured action.
    New,
}

struct Digest {
    hash: String,
    iterations: u32,
    hint: String,
}

impl Digest {
    fn view(&self) -> lr_secret_scanner::dismissal::DismissedDigest<'_> {
        lr_secret_scanner::dismissal::DismissedDigest {
            hash: &self.hash,
            iterations: self.iterations,
            hint: Some(&self.hint),
        }
    }
}

/// The policy context of one outbound secret scan: everything that does not
/// depend on the request content.
pub struct SecretScanPolicy {
    pub action: lr_config::SecretScanAction,
    /// A time-based bypass is active (and no monitor intercept overrides it).
    pub bypass: bool,
    pub scanner: Arc<lr_secret_scanner::SecretScanEngine>,
    salt: Option<String>,
    redact: Vec<Digest>,
    ignore: Vec<Digest>,
}

impl SecretScanPolicy {
    /// The policy for a client, or `None` when nothing is to be detected:
    /// scanning is off, or there is no usable scanner (a Redact client is
    /// warned, as the whole-request scan warns).
    pub fn resolve(state: &AppState, client_id: &str, session_id: &str) -> Option<Self> {
        let config = state.config_manager.get();
        let client = state.client_manager.get_client(client_id);
        // Resolve effective action: per-client override > global
        let action = client
            .as_ref()
            .and_then(|c| c.secret_scanning.action.clone())
            .unwrap_or_else(|| config.secret_scanning.action.clone());
        if action == lr_config::SecretScanAction::Off {
            return None;
        }
        // Check time-based bypass (unless monitor intercept overrides)
        let bypass = state
            .secret_scan_approval_tracker
            .has_valid_bypass(client_id)
            && !state.mcp_gateway.firewall_manager.should_intercept(
                client_id,
                lr_mcp::gateway::firewall::InterceptCategory::SecretScan,
            );
        // Clone the Arc out of the lock; it is never held across awaits.
        let scanner = state.secret_scanner.read().as_ref().cloned();
        let Some(scanner) = scanner else {
            if action == lr_config::SecretScanAction::Redact {
                secret_scan_warning(
                    state,
                    client_id,
                    Some(session_id),
                    &format!(
                        "{}; request forwarded without new detection",
                        state.secret_scanner_unavailable_reason()
                    ),
                );
            }
            return None;
        };
        if !scanner.has_rules() {
            if action == lr_config::SecretScanAction::Redact {
                secret_scan_warning(
                    state,
                    client_id,
                    Some(session_id),
                    "Secret scanner has no rules; request forwarded without new detection",
                );
            }
            return None;
        }
        let scanning = client.as_ref().map(|c| &c.secret_scanning);
        let digests = |redact: bool| -> Vec<Digest> {
            scanning
                .map(|s| s.dismissed_secrets.as_slice())
                .unwrap_or_default()
                .iter()
                .filter(|e| e.redact == redact)
                .map(|e| Digest {
                    hash: e.hash.clone(),
                    iterations: e.iterations,
                    hint: e.hint.clone(),
                })
                .collect()
        };
        Some(Self {
            action,
            bypass,
            scanner,
            salt: scanning.and_then(|s| s.dismiss_salt.clone()),
            redact: digests(true),
            ignore: digests(false),
        })
    }

    /// The remembered decision for a detected value. Remembered redactions
    /// take priority over ignored values.
    pub fn remembered(&self, state: &AppState, client_id: &str, secret: &str) -> Remembered {
        let salt = self.salt.as_deref();
        let redact: Vec<_> = self.redact.iter().map(Digest::view).collect();
        if state.secret_dismissal_cache.is_dismissed(
            &format!("redact:{client_id}"),
            salt,
            &redact,
            secret,
        ) {
            return Remembered::Redact;
        }
        let ignore: Vec<_> = self.ignore.iter().map(Digest::view).collect();
        if state
            .secret_dismissal_cache
            .is_dismissed(client_id, salt, &ignore, secret)
        {
            return Remembered::Ignore;
        }
        Remembered::New
    }
}

/// The text around the first occurrence of a finding, as the monitor shows it.
pub fn finding_excerpt(text: &str, secret: &str) -> String {
    let position = text.find(secret).unwrap_or_default();
    let mut start = position.saturating_sub(EXCERPT);
    while !text.is_char_boundary(start) {
        start += 1;
    }
    let mut end = (position + secret.len() + EXCERPT).min(text.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[start..end].to_owned()
}

/// The monitor entry of one finding of a recorded secret scan.
pub fn monitor_finding_entry(
    finding: &lr_secret_scanner::SecretFinding,
    location: Option<(&str, String)>,
    outcome: &SecretScanOutcome,
) -> Value {
    let mut entry = serde_json::to_value(finding).unwrap_or_else(|_| serde_json::json!({}));
    let mut value = serde_json::json!({"value":finding.raw_text});
    if let Some(context) = lr_types::secret_context::current() {
        if context.redact_monitor(&mut value).is_err() {
            value["value"] = Value::Null;
        }
    }
    if let SecretScanOutcome::Redact(plan) = outcome {
        plan.apply(&mut value);
    }
    let redacted = value["value"]
        .as_str()
        .is_some_and(|v| v != finding.raw_text);
    entry["value"] = value["value"].clone();
    entry["redacted"] = Value::Bool(redacted);
    entry["replacement"] = if redacted {
        value["value"].clone()
    } else {
        Value::Null
    };
    entry["action"] = Value::String(
        if matches!(outcome, SecretScanOutcome::Deny(_)) {
            "blocked"
        } else if redacted {
            "redacted"
        } else {
            "allowed"
        }
        .into(),
    );
    if let Some((label, excerpt)) = location {
        entry["location"] = Value::String(label.to_owned());
        entry["context_excerpt"] = Value::String(excerpt);
    }
    entry
}

/// Where a finding was seen, kept from the unit it was detected in.
struct Seen {
    label: String,
    message_index: Option<usize>,
    excerpt: String,
    line: usize,
    before: String,
    after: String,
    /// No remembered decision applies.
    new: bool,
}

/// The secret scan of one streamed request.
pub struct StreamSecretScan {
    state: AppState,
    client_id: String,
    session_id: String,
    provider: Option<String>,
    policy: SecretScanPolicy,
    context: Option<Arc<dyn SecretContext>>,
    findings: lr_secret_scanner::FindingAccumulator,
    seen: Vec<Seen>,
    redaction: lr_secret_scanner::redaction::Redaction,
    dismissed: usize,
    texts: usize,
    first_text: Option<String>,
    elapsed: std::time::Duration,
}

impl StreamSecretScan {
    /// Start the scan of a streamed request, or `None` when nothing is to be
    /// detected for this client. A Redact scan registers its findings in
    /// `context`.
    pub fn new(
        state: &AppState,
        client_id: &str,
        session_id: &str,
        provider: Option<&str>,
        context: Option<Arc<dyn SecretContext>>,
    ) -> Option<Self> {
        let policy = SecretScanPolicy::resolve(state, client_id, session_id)?;
        Some(Self {
            state: state.clone(),
            client_id: client_id.to_owned(),
            session_id: session_id.to_owned(),
            provider: provider.map(str::to_owned),
            policy,
            context,
            findings: Default::default(),
            seen: Vec::new(),
            redaction: Default::default(),
            dismissed: 0,
            texts: 0,
            first_text: None,
            elapsed: Default::default(),
        })
    }

    pub fn action(&self) -> &lr_config::SecretScanAction {
        &self.policy.action
    }

    /// Remembered redactions found so far: apply them to every unit before it
    /// is forwarded, as the whole-request path applies its plan.
    pub fn redaction(&self) -> &lr_secret_scanner::redaction::Redaction {
        &self.redaction
    }

    /// Detect secrets in one unit: the value of the top-level member `key`
    /// or, with `index`, element `index` of the top-level array `key`.
    /// Returns the values newly registered for masking (Redact only).
    pub fn inspect(&mut self, key: &str, index: Option<u64>, value: &Value) -> Vec<String> {
        // Warnings reach the request's vault, as inside the whole-request scope.
        let context = self.context.clone();
        lr_types::secret_context::sync_scope(context, || self.inspect_unit(key, index, value))
    }

    fn inspect_unit(&mut self, key: &str, index: Option<u64>, value: &Value) -> Vec<String> {
        let started = std::time::Instant::now();
        let mut unit = serde_json::Map::new();
        unit.insert(
            key.to_owned(),
            match index {
                Some(_) => Value::Array(vec![value.clone()]),
                None => value.clone(),
            },
        );
        let unit = Value::Object(unit);
        let projection = match crate::documents::scan_projection(&unit) {
            Ok((projection, _)) => projection,
            Err(_) => {
                secret_scan_warning(
                    &self.state,
                    &self.client_id,
                    Some(&self.session_id),
                    "Document inspection unavailable; original request inspected where possible",
                );
                unit
            }
        };
        let mut texts = lr_guardrails::text_extractor::extract_request_text(&projection);
        if let Some(index) = index {
            for text in &mut texts {
                if text.message_index.is_some() {
                    text.message_index = usize::try_from(index).ok();
                }
            }
        }
        if texts.is_empty() {
            return Vec::new();
        }
        let base = self.texts;
        self.texts += texts.len();
        if self.first_text.is_none() {
            self.first_text = texts.first().map(|t| {
                let mut end = t.text.len().min(4096);
                while !t.text.is_char_boundary(end) {
                    end -= 1;
                }
                t.text[..end].to_owned()
            });
        }
        let scanned: Vec<_> = texts
            .iter()
            .enumerate()
            .map(|(i, t)| lr_secret_scanner::ExtractedText {
                label: t.label.clone(),
                text: t.text.clone(),
                message_index: base + i,
            })
            .collect();
        let added = self.findings.add(self.policy.scanner.scan_ranked(&scanned));
        let mut registered = Vec::new();
        for position in added {
            let finding = &self.findings.findings()[position];
            let raw = finding.raw_text.clone();
            let text = &texts[finding.message_index - base];
            let start = text.text.find(&raw).unwrap_or_default();
            let end = start + raw.len();
            let mut before = start.saturating_sub(POPUP_CONTEXT);
            while !text.text.is_char_boundary(before) {
                before += 1;
            }
            let mut after = (end + POPUP_CONTEXT).min(text.text.len());
            while !text.text.is_char_boundary(after) {
                after -= 1;
            }
            let remembered = self.policy.remembered(&self.state, &self.client_id, &raw);
            match remembered {
                Remembered::Redact => self.redaction.add(&raw),
                Remembered::Ignore => self.dismissed += 1,
                Remembered::New => {}
            }
            self.seen.push(Seen {
                label: text.label.clone(),
                message_index: text.message_index,
                excerpt: finding_excerpt(&text.text, &raw),
                line: text.text[..start].bytes().filter(|b| *b == b'\n').count() + 1,
                before: text.text[before..start].to_owned(),
                after: text.text[end..after].to_owned(),
                new: remembered == Remembered::New,
            });
            if remembered == Remembered::New
                && self.policy.action == lr_config::SecretScanAction::Redact
            {
                match &self.context {
                    Some(context) if context.register(&raw).is_ok() => registered.push(raw),
                    _ => secret_scan_warning(
                        &self.state,
                        &self.client_id,
                        Some(&self.session_id),
                        "Secret mapping could not be stored; request forwarded with incomplete coverage",
                    ),
                }
            }
        }
        self.elapsed += started.elapsed();
        registered
    }

    /// The body was forwarded completely: record the aggregated scan and, for
    /// Notify, show the popup (with bounded context and no request JSON,
    /// since the whole request was never held).
    pub fn finish(self, model: &str) {
        let context = self.context.clone();
        lr_types::secret_context::sync_scope(context, || self.finish_scan(model));
    }

    fn finish_scan(self, model: &str) {
        let Self {
            state,
            client_id,
            session_id,
            provider,
            policy,
            findings,
            seen,
            redaction,
            dismissed,
            first_text,
            elapsed,
            ..
        } = self;
        let all = findings.into_findings();
        let mut mask = lr_secret_scanner::redaction::Redaction::default();
        for finding in &all {
            mask.add(&finding.raw_text);
        }
        let kept: Vec<_> = all
            .into_iter()
            .zip(seen)
            .filter(|(_, seen)| seen.new)
            .collect();
        if kept.is_empty()
            || (policy.bypass && policy.action != lr_config::SecretScanAction::Redact)
        {
            if dismissed > 0 {
                tracing::debug!(
                    "Secret scan: {dismissed} finding(s) ignored by client {client_id}'s dismissed-secret list"
                );
            }
            return;
        }
        tracing::info!(
            "Secret scan found {} potential secrets in streamed request from client {}",
            kept.len(),
            client_id
        );
        let client_ctx = ClientAuthContext {
            client_id: client_id.clone(),
        };
        if policy.action == lr_config::SecretScanAction::Notify {
            use lr_mcp::gateway::firewall::{
                SecretFindingContext, SecretFindingSummary, SecretScanApprovalDetails,
            };
            let details = SecretScanApprovalDetails {
                findings: kept
                    .iter()
                    .map(|(f, seen)| SecretFindingSummary {
                        rule_id: f.rule_id.clone(),
                        rule_description: f.rule_description.clone(),
                        category: f.category.clone(),
                        matched_text: f.matched_text.clone(),
                        raw_text: f.raw_text.clone(),
                        entropy: f.entropy,
                        request_match: None,
                        context: Some(SecretFindingContext {
                            source: match seen.message_index {
                                Some(i) => format!("Message {} · {}", i + 1, seen.label),
                                None => seen.label.clone(),
                            },
                            line: seen.line,
                            before: mask.apply_text(&seen.before),
                            after: mask.apply_text(&seen.after),
                        }),
                    })
                    .collect(),
                scan_duration_ms: elapsed.as_millis() as u64,
                request_json: None,
                llm_source: Some(secret_scan_llm_source(
                    model,
                    provider.as_deref(),
                    &state.provider_registry.list_providers(),
                    &[],
                )),
            };
            let client_name = state
                .client_manager
                .get_client(&client_id)
                .map(|c| c.name)
                .unwrap_or_else(|| client_id.clone());
            let preview = secret_scan_popup_preview(&details);
            state.mcp_gateway.firewall_manager.notify_secret_scan(
                client_id.clone(),
                client_name,
                model.to_owned(),
                details,
                preview,
            );
        }
        let outcome = SecretScanOutcome::Allow;
        let mut findings_json = Value::Array(
            kept.iter()
                .map(|(finding, seen)| {
                    monitor_finding_entry(
                        finding,
                        Some((seen.label.as_str(), seen.excerpt.clone())),
                        &outcome,
                    )
                })
                .collect(),
        );
        redaction.apply(&mut findings_json);
        let monitor_preview = findings_json
            .get(0)
            .and_then(|f| f["context_excerpt"].as_str())
            .map(str::to_owned)
            .or(first_text)
            .unwrap_or_default();
        super::super::monitor_helpers::record_secret_scan(
            &state,
            Some(&client_ctx),
            Some(&session_id),
            &monitor_preview,
            policy.scanner.rule_metadata().len(),
            kept.len(),
            findings_json,
            &format!("{:?}", policy.action).to_lowercase(),
            elapsed.as_millis() as u64,
        );
    }
}
