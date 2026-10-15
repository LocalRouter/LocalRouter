use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReplayEndpoint {
    ChatCompletions,
    Completions,
    Responses,
    Decisions,
    SystemOne,
}
impl ReplayEndpoint {
    pub fn path(self) -> &'static str {
        match self {
            Self::ChatCompletions => "/v1/chat/completions",
            Self::Completions => "/v1/completions",
            Self::Responses => "/v1/responses",
            Self::Decisions => "/v1/decisions",
            Self::SystemOne => "/v1/systemone",
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReplayInput {
    pub endpoint: ReplayEndpoint,
    pub body: Value,
    pub source_event_id: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum TaskCheck {
    Contains { text: String, case_sensitive: bool },
    Regex { pattern: String },
    JsonValid,
    JsonPointer { pointer: String, expected: Value },
    ExpectedRoute { route: String },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReplayCase {
    pub id: String,
    pub name: String,
    pub input: ReplayInput,
    #[serde(default)]
    pub checks: Vec<TaskCheck>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReplayDataset {
    pub id: String,
    pub revision: u64,
    pub name: String,
    pub cases: Vec<ReplayCase>,
    pub saved_at: Option<DateTime<Utc>>,
}
impl ReplayDataset {
    pub fn empty() -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            revision: 0,
            name: "New dataset".into(),
            cases: vec![],
            saved_at: None,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ReplayTarget {
    Model {
        model: String,
    },
    Policy {
        name: String,
        policy: lr_config::RoutingPolicy,
    },
}
impl ReplayTarget {
    pub fn label(&self) -> String {
        match self {
            Self::Model { model } => model.clone(),
            Self::Policy { name, .. } => name.clone(),
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReplayRunRequest {
    pub client_id: String,
    pub dataset_id: Option<String>,
    pub cases: Vec<ReplayCase>,
    pub targets: Vec<ReplayTarget>,
    pub max_output_tokens: u32,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Running,
    Completed,
    Cancelled,
    Interrupted,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ResultStatus {
    Pending,
    Running,
    Success,
    Error,
    Cancelled,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CheckResult {
    pub kind: String,
    pub passed: bool,
    pub detail: Option<String>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ReplayCost {
    /// Partial sums; unknown calls may add further spend.
    pub reported_usd: Option<f64>,
    pub estimated_usd: Option<f64>,
    pub unknown_calls: usize,
    pub classifier_reported_usd: Option<f64>,
    pub classifier_estimated_usd: Option<f64>,
    pub classifier_cost_unknown: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JudgeGrade {
    pub score: f64,
    pub reasoning: String,
    pub rubric_fingerprint: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReplayResult {
    pub case_id: String,
    pub case_name: String,
    pub target: String,
    pub status: ResultStatus,
    #[serde(default)]
    pub http_request_sent: bool,
    pub http_status: Option<u16>,
    pub latency_ms: Option<u64>,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cost: ReplayCost,
    pub checks: Vec<CheckResult>,
    pub request: Option<Value>,
    pub response: Option<Value>,
    pub judge: Option<JudgeGrade>,
    pub error: Option<String>,
    pub policy_snapshot: Option<Value>,
    pub config_fingerprint: Option<String>,
    pub policy_fingerprint: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReplayRun {
    pub id: String,
    pub revision: u64,
    pub dataset_id: Option<String>,
    pub name: String,
    pub client_id: String,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub stop_reason: Option<String>,
    pub status: RunStatus,
    pub cases: Vec<ReplayCase>,
    pub targets: Vec<ReplayTarget>,
    pub results: Vec<ReplayResult>,
    /// Judge runs are explicitly started, billed and saved separately.
    pub judge_of: Option<String>,
    pub judge_model: Option<String>,
    pub judge_rubric: Option<String>,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactKind {
    Dataset,
    Run,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ArtifactSummary {
    pub id: String,
    pub name: String,
    pub kind: ArtifactKind,
    pub revision: u64,
    pub saved_at: DateTime<Utc>,
    pub case_count: usize,
    pub result_count: usize,
    pub client_id: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ArtifactIssue {
    pub id: String,
    pub kind: ArtifactKind,
    pub error: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ArtifactList {
    pub items: Vec<ArtifactSummary>,
    pub issues: Vec<ArtifactIssue>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SavedArtifact {
    pub schema_version: u32,
    pub summary: ArtifactSummary,
    pub value: Value,
}

/// Payload-free status for polling; full bodies are loaded only when inspected/saved.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReplayResultSummary {
    pub judge: Option<JudgeGrade>,
    pub index: usize,
    pub case_id: String,
    pub case_name: String,
    pub target: String,
    pub status: ResultStatus,
    pub http_request_sent: bool,
    pub http_status: Option<u16>,
    pub latency_ms: Option<u64>,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cost: ReplayCost,
    pub checks: Vec<CheckResult>,
    pub error: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReplayRunSummary {
    pub id: String,
    pub name: String,
    pub client_id: String,
    pub status: RunStatus,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub stop_reason: Option<String>,
    pub revision: u64,
    pub judge_of: Option<String>,
    pub judge_model: Option<String>,
    pub results: Vec<ReplayResultSummary>,
}
impl From<&ReplayRun> for ReplayRunSummary {
    fn from(run: &ReplayRun) -> Self {
        Self {
            id: run.id.clone(),
            name: run.name.clone(),
            client_id: run.client_id.clone(),
            status: run.status.clone(),
            started_at: run.started_at,
            finished_at: run.finished_at,
            stop_reason: run.stop_reason.clone(),
            revision: run.revision,
            judge_of: run.judge_of.clone(),
            judge_model: run.judge_model.clone(),
            results: run
                .results
                .iter()
                .enumerate()
                .map(|(index, r)| ReplayResultSummary {
                    judge: r.judge.clone(),
                    index,
                    case_id: r.case_id.clone(),
                    case_name: r.case_name.clone(),
                    target: r.target.clone(),
                    status: r.status.clone(),
                    http_request_sent: r.http_request_sent,
                    http_status: r.http_status,
                    latency_ms: r.latency_ms,
                    input_tokens: r.input_tokens,
                    output_tokens: r.output_tokens,
                    cost: r.cost.clone(),
                    checks: r.checks.clone(),
                    error: r.error.clone(),
                })
                .collect(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReplayEstimate {
    pub case_id: String,
    pub target: String,
    pub estimated_usd: Option<f64>,
    pub warnings: Vec<String>,
}

impl ReplayRun {
    /// A saved running snapshot cannot prove which later calls were dispatched.
    pub fn mark_interrupted(&mut self) {
        if self.status == RunStatus::Running {
            self.status = RunStatus::Interrupted;
        }
        for result in &mut self.results {
            if matches!(result.status, ResultStatus::Pending | ResultStatus::Running) {
                result.status = ResultStatus::Cancelled;
                result.error = Some(
                    "Interrupted snapshot; later outcomes are unknown. No inference was resumed"
                        .into(),
                );
                if self
                    .targets
                    .iter()
                    .any(|t| matches!(t, ReplayTarget::Policy { .. }) && t.label() == result.target)
                {
                    result.cost.classifier_cost_unknown = true;
                } else {
                    result.cost.unknown_calls = result.cost.unknown_calls.max(1);
                }
            }
        }
    }
}
