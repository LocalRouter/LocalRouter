//! The firewall side of a streamed proxy request: detection, remembered
//! redactions, vault masking, attachments, the restoration contract and the
//! `extensions` cleanup, applied one unit at a time with the functions the
//! buffered path ([`super::proxy::AppFirewall`]'s `evaluate`) applies to the
//! whole request, in the same order.

use std::sync::Arc;

use lr_proxy::request_stream::{Answer, Question, StreamAbort, StreamSession, TailUnit, UnitScope};
use lr_secret_scanner::placeholders::{StreamProtect, StreamUnitScope, Vault};
use lr_server::routes::pipeline::stream_scan::StreamSecretScan;
use lr_types::secret_context::SecretContext;
use serde_json::Value;

/// Bytes of tool and schema definitions kept for the restoration contract.
const CONTRACT_BUDGET: usize = 8 * 1024 * 1024;

/// The subtrees [`Vault::set_request_contract`] reads, captured from the
/// forwarded units.
#[derive(Default)]
struct ContractCapture {
    members: serde_json::Map<String, Value>,
    additional_tools: Vec<Value>,
    bytes: usize,
    overflow: bool,
}

impl ContractCapture {
    fn keep(&mut self, value: &Value) -> Option<Value> {
        if self.overflow {
            return None;
        }
        self.bytes += lr_monitor::serialized_len(value);
        if self.bytes > CONTRACT_BUDGET {
            self.overflow = true;
            self.members.clear();
            self.additional_tools.clear();
            return None;
        }
        Some(value.clone())
    }

    fn observe(&mut self, scope: &UnitScope, value: &Value) {
        match scope {
            UnitScope::RootMember { key }
                if matches!(key.as_str(), "tools" | "response_format" | "text") =>
            {
                if let Some(value) = self.keep(value) {
                    self.members.insert(key.clone(), value);
                }
            }
            UnitScope::Element { key, .. }
                if key == "input" && value["type"] == "additional_tools" =>
            {
                if let Some(value) = self.keep(value) {
                    self.additional_tools.push(value);
                }
            }
            _ => {}
        }
    }

    fn request(self) -> Option<Value> {
        if self.overflow {
            return None;
        }
        let mut request = self.members;
        if !self.additional_tools.is_empty() {
            request.insert("input".into(), Value::Array(self.additional_tools));
        }
        Some(Value::Object(request))
    }
}

/// A decision deferred until the whole body was judged (the model approval
/// popup when GuardRails also judge the request, so the popups come in the
/// buffered path's order: Secret Scan, GuardRails, model).
pub(crate) type Deferred = Box<
    dyn FnOnce() -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<(), StreamAbort>> + Send>,
        > + Send,
>;

/// GuardRails over a streamed body: the text projection is built from the
/// units as they are released (masked, as the buffered path judges the
/// masked request), the driver holds the whole output, and at the end the
/// safety models judge the projection before anything is sent.
pub(crate) struct GuardrailsHold {
    projection: lr_guardrails::text_extractor::RequestTextProjection,
    /// The request's `model`, once the body ended: the verdict is due.
    due: Option<String>,
    /// Runs after an Allow verdict.
    then: Option<Deferred>,
    /// The verdict was asked.
    asked: bool,
}

impl GuardrailsHold {
    pub(crate) fn new(then: Option<Deferred>) -> Self {
        Self {
            projection: Default::default(),
            due: None,
            then,
            asked: false,
        }
    }

    /// Add a released unit (its forwarded value) to the projection. Text
    /// attachments are judged decoded, as on the buffered path.
    fn observe(&mut self, scope: &UnitScope, value: &Value) {
        match scope {
            UnitScope::RootMember { key } => self.projection.push_member(key, value),
            UnitScope::Element { key, index } => {
                let Ok(index) = usize::try_from(*index) else {
                    return;
                };
                let file_parts =
                    value
                        .get("content")
                        .and_then(Value::as_array)
                        .is_some_and(|parts| {
                            parts.iter().any(|part| {
                                matches!(
                                    part.get("type").and_then(Value::as_str),
                                    Some("file" | "input_file")
                                )
                            })
                        });
                if !file_parts {
                    self.projection.push_element(key, index, value);
                    return;
                }
                let request = serde_json::json!({ key: [value] });
                let projected = guardrail_projection(&request);
                self.projection
                    .push_element(key, index, &projected[key.as_str()][0]);
            }
        }
    }

    /// Bytes of text the projection holds.
    pub(crate) fn text_bytes(&self) -> usize {
        self.projection.text_bytes()
    }
}

/// What GuardRails judge for a request: its text attachments decoded (the
/// native server's projection). Computed outside any request scope, so an
/// undecodable attachment is reported once, by the Secret Scan.
pub(crate) fn guardrail_projection(body: &Value) -> std::borrow::Cow<'_, Value> {
    if !lr_server::documents::has_file_parts(body) {
        return std::borrow::Cow::Borrowed(body);
    }
    lr_types::secret_context::sync_scope(None, || {
        lr_server::documents::scan_projection(body)
            .map(|(projection, _)| std::borrow::Cow::Owned(projection))
            .unwrap_or(std::borrow::Cow::Borrowed(body))
    })
}

/// The answer of the GuardRails verdict question.
struct Verdict(Result<(), StreamAbort>);

/// A streamed request ended with the refusal the buffered path answers
/// (`RequestAction::reject_json`), so the client and the monitor see the same
/// status and body.
pub(crate) fn refusal(status: u16, message: &str) -> StreamAbort {
    match lr_proxy::interceptor::RequestAction::reject_json(status, message) {
        lr_proxy::interceptor::RequestAction::Reject { body, .. } => {
            StreamAbort::with_json(status, message, body)
        }
        _ => StreamAbort::new(status, message),
    }
}

/// One streamed request of one client.
pub(crate) struct AppStreamSession {
    pub(crate) state: lr_server::state::AppState,
    pub(crate) client_id: String,
    pub(crate) session_id: Option<String>,
    pub(crate) scan: Option<StreamSecretScan>,
    /// The request's vault: masks (Redact) or only restores (Off/Notify with
    /// references from earlier turns).
    pub(crate) vault: Option<Arc<Vault>>,
    /// Redact and Ask: mask known originals.
    pub(crate) masking: bool,
    protect: StreamProtect,
    contract: ContractCapture,
    guardrails: Option<GuardrailsHold>,
}

impl AppStreamSession {
    /// Judge the whole body with GuardRails before any of it is sent.
    pub(crate) fn with_guardrails(mut self, hold: GuardrailsHold) -> Self {
        self.guardrails = Some(hold);
        self
    }

    /// Bytes of text the GuardRails projection holds.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn projection_bytes(&self) -> Option<usize> {
        self.guardrails.as_ref().map(GuardrailsHold::text_bytes)
    }

    pub(crate) fn new(
        state: lr_server::state::AppState,
        client_id: String,
        session_id: Option<String>,
        scan: Option<StreamSecretScan>,
        vault: Option<Arc<Vault>>,
        masking: bool,
    ) -> Self {
        Self {
            state,
            client_id,
            session_id,
            scan,
            vault,
            masking,
            protect: StreamProtect::default(),
            contract: ContractCapture::default(),
            guardrails: None,
        }
    }

    fn context(&self) -> Option<Arc<dyn SecretContext>> {
        self.vault
            .clone()
            .map(|vault| vault as Arc<dyn SecretContext>)
    }

    /// The GuardRails verdict, once the whole body was released: the same
    /// scan, monitor events, popup and refusal the buffered path runs on the
    /// masked request (inside the request's masking scope, as there), then
    /// the deferred model approval.
    fn verdict(&mut self) -> Option<Question> {
        let hold = self.guardrails.as_mut()?;
        if hold.asked {
            return None;
        }
        let model = hold.due.clone()?;
        hold.asked = true;
        let texts = std::mem::take(&mut hold.projection).finish();
        let then = hold.then.take();
        let state = self.state.clone();
        let client_id = self.client_id.clone();
        let session_id = self.session_id.clone();
        let context = self.context().filter(|_| self.masking);
        Some(Question(Box::pin(async move {
            use lr_server::routes::pipeline::{
                scan_for_guardrails, GuardrailInput, GuardrailScanOutcome,
            };
            let outcome = lr_types::secret_context::scope(
                context,
                scan_for_guardrails(
                    &state,
                    &client_id,
                    &model,
                    GuardrailInput::Texts(&texts),
                    session_id.as_deref(),
                ),
            )
            .await;
            drop(texts);
            let verdict = match outcome {
                GuardrailScanOutcome::Deny(message) => Err(refusal(403, &message)),
                GuardrailScanOutcome::Allow => match then {
                    Some(then) => then().await,
                    None => Ok(()),
                },
            };
            Answer(Box::new(Verdict(verdict)))
        })))
    }

    fn warn(&self, message: &str) {
        lr_server::routes::pipeline::secret_scan_warning(
            &self.state,
            &self.client_id,
            self.session_id.as_deref(),
            message,
        );
    }

    fn mask_units(&mut self, units: &mut [TailUnit]) -> Result<(), StreamAbort> {
        // Remembered redactions, attachments first (as the buffered path).
        if let Some(plan) = self.scan.as_ref().map(|scan| scan.redaction().clone()) {
            if !plan.is_empty() {
                for unit in units.iter_mut() {
                    let attachments = in_conversation(unit, |request| {
                        lr_server::documents::redact_text_files(request, &plan)
                    });
                    if attachments.is_err() {
                        self.warn(
                            "Attachment redaction unavailable; original attachment forwarded",
                        );
                    }
                    plan.apply(&mut unit.value);
                }
            }
        }
        if self.masking {
            let Some(vault) = self.vault.clone() else {
                return Ok(());
            };
            for unit in units.iter_mut() {
                let _ = in_conversation(unit, |request| {
                    lr_server::documents::protect_text_files(request, vault.as_ref())
                });
            }
            let mut scoped: Vec<(StreamUnitScope, &mut Value)> = units
                .iter_mut()
                .map(|unit| {
                    let scope = match &unit.scope {
                        UnitScope::RootMember { key } => StreamUnitScope::RootMember(key.clone()),
                        UnitScope::Element { .. } => StreamUnitScope::Element,
                    };
                    (scope, &mut unit.value)
                })
                .collect();
            if vault
                .protect_stream_units(&mut self.protect, &mut scoped)
                .is_err()
            {
                // The buffered path would forward the original here; a
                // streamed request is ended instead, so no original leaves.
                vault.protection_diagnostic(
                    "Secret masking could not safely complete; the streamed request was stopped",
                );
                return Err(StreamAbort::new(
                    502,
                    "Secret masking could not safely complete; request stopped",
                ));
            }
            for unit in units.iter_mut() {
                if !matches!(&unit.scope, UnitScope::RootMember { key } if key == "extensions") {
                    continue;
                }
                if let Some(extensions) = unit.value.as_object_mut() {
                    extensions.remove("localrouter.session");
                    if extensions.is_empty() {
                        unit.drop = true;
                    }
                }
            }
        }
        if self.vault.is_some() {
            for unit in units.iter() {
                if !unit.drop {
                    self.contract.observe(&unit.scope, &unit.value);
                }
            }
        }
        Ok(())
    }
}

/// Run a whole-request attachment transform on one element of the
/// conversation: those transforms only visit `messages[]`/`input[]`.
fn in_conversation(
    unit: &mut TailUnit,
    transform: impl FnOnce(&mut Value) -> lr_types::AppResult<()>,
) -> lr_types::AppResult<()> {
    let UnitScope::Element { key, .. } = &unit.scope else {
        return Ok(());
    };
    let mut request = serde_json::Map::new();
    request.insert(
        key.clone(),
        Value::Array(vec![std::mem::take(&mut unit.value)]),
    );
    let mut request = Value::Object(request);
    let result = transform(&mut request);
    unit.value = request[key.as_str()][0].take();
    result
}

impl StreamSession for AppStreamSession {
    fn inspect(&mut self, scope: &UnitScope, value: &Value) -> Result<Vec<String>, StreamAbort> {
        let Some(scan) = self.scan.as_mut() else {
            return Ok(Vec::new());
        };
        Ok(match scope {
            UnitScope::RootMember { key } => scan.inspect(key, None, value),
            UnitScope::Element { key, index } => scan.inspect(key, Some(*index), value),
        })
    }

    fn mask(&mut self, units: &mut [TailUnit]) -> Result<(), StreamAbort> {
        let context = self.context();
        lr_types::secret_context::sync_scope(context, || self.mask_units(units))?;
        if let Some(hold) = &mut self.guardrails {
            // What will be forwarded is what GuardRails judge.
            for unit in units.iter().filter(|unit| !unit.drop) {
                hold.observe(&unit.scope, &unit.value);
            }
        }
        Ok(())
    }

    fn holds_until_verdict(&self) -> bool {
        self.guardrails.is_some()
    }

    fn pending(&self) -> bool {
        self.scan.as_ref().is_some_and(StreamSecretScan::pending)
    }

    fn question(&mut self) -> Option<Question> {
        if let Some(asked) = self.scan.as_mut().and_then(StreamSecretScan::question) {
            return Some(Question(Box::pin(
                async move { Answer(Box::new(asked.await)) },
            )));
        }
        self.verdict()
    }

    fn answer(&mut self, answer: Answer) -> Result<Vec<String>, StreamAbort> {
        use lr_server::routes::pipeline::stream_scan::AskOutcome;
        let answer = match answer.0.downcast::<Verdict>() {
            Ok(verdict) => return verdict.0.map(|()| Vec::new()),
            Err(other) => other,
        };
        let Ok(outcome) = answer.downcast::<AskOutcome>() else {
            return Err(StreamAbort::new(500, "Unexpected Secret Scan answer"));
        };
        let context = self.context();
        let Some(scan) = self.scan.as_mut() else {
            return Ok(Vec::new());
        };
        match lr_types::secret_context::sync_scope(context, || scan.answer(*outcome)) {
            Ok(registered) => Ok(registered),
            Err(denied) => {
                let lr_proxy::interceptor::RequestAction::Reject { body, .. } =
                    lr_proxy::interceptor::RequestAction::secret_scan_denied()
                else {
                    unreachable!("secret_scan_denied is a Reject");
                };
                Err(StreamAbort::with_json(403, denied.0, body))
            }
        }
    }

    fn diagnostic(&mut self, message: &str) {
        match &self.vault {
            Some(vault) => vault.protection_diagnostic(message),
            None => self.warn(message),
        }
    }

    fn finish(&mut self, metadata: &lr_secret_scanner::request_stream::RequestMetadata) {
        let model = metadata.model.value().cloned().unwrap_or_default();
        if let Some(hold) = &mut self.guardrails {
            hold.due = Some(model.clone());
        }
        if let Some(scan) = self.scan.take() {
            scan.finish(&model);
        }
        let Some(vault) = self.vault.clone() else {
            return;
        };
        if self.masking {
            vault.finish_stream_protect(std::mem::take(&mut self.protect));
        }
        match std::mem::take(&mut self.contract).request() {
            Some(request) => {
                let _ = vault.set_request_contract(&request);
            }
            None => {
                let _ = vault.set_request_contract(&Value::Object(Default::default()));
                vault.protection_diagnostic("Restoration contract unavailable; request forwarded");
            }
        }
    }
}
