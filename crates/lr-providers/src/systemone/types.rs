//! Wire types for the System One decision protocol (`POST /v1/systemone`).
//!
//! The shapes follow TypeSafe's published API reference
//! (<https://docs.typesafe.ai/api>). Laya's `laya-serve`, Kev's `kev.serve`
//! and the other Jev-compatible servers speak the same protocol, sometimes
//! with a few extra fields; those are preserved in the `extra` maps instead
//! of being rejected.
//!
//! Maps are `IndexMap` on purpose: the order of choice options and score
//! levels is meaningful (local decision models show position bias), and the
//! order of questions and answers should round-trip exactly as the client
//! sent it.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use utoipa::ToSchema;

/// Maximum number of options in a Choice question (TypeSafe limit).
pub const MAX_CHOICE_OPTIONS: usize = 255;
/// Minimum number of levels in a Score question.
pub const MIN_SCORE_LEVELS: usize = 2;
/// Maximum number of levels in a Score question.
pub const MAX_SCORE_LEVELS: usize = 10;

/// A System One decision request.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(example = json!({
    "model": "laya/english",
    "state": {"subject": "Refund not received", "body": "I was billed twice for October."},
    "questions": {
        "department": {
            "type": "choice",
            "instructions": "Which team should handle this ticket?",
            "criteria": {"billing": "Payments, invoices, refunds", "support": "Product help"}
        },
        "urgency": {
            "type": "score",
            "instructions": "How urgent is this?",
            "criteria": ["low", "medium", "high"]
        },
        "needs_human": {"type": "noul", "instructions": "Does a human need to review this?"}
    }
}))]
pub struct SystemOneRequest {
    /// The information to evaluate: a string, a JSON object, or an array.
    pub state: Value,
    /// Model to use. `provider/model` targets a specific provider instance,
    /// a bare id (e.g. `jev-latest`) is resolved across providers, and
    /// `localrouter/auto` uses the strategy's prioritized models. When
    /// omitted, LocalRouter picks the only System One provider available to
    /// the client, or falls back to auto-routing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Named, typed questions to answer about the state.
    #[schema(value_type = HashMap<String, SystemOneQuestion>)]
    pub questions: IndexMap<String, SystemOneQuestion>,
    /// Unknown top-level fields sent by the client. Forwarded unchanged to
    /// native System One providers.
    #[serde(flatten)]
    #[schema(value_type = Object)]
    pub extra: Map<String, Value>,
}

/// A typed question.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SystemOneQuestion {
    /// Yes/no question answered with the probability of "yes".
    Noul {
        /// The question: a string, object, or array.
        instructions: Value,
        /// Optional descriptions of what counts as `true` and `false`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[schema(value_type = Option<HashMap<String, Object>>)]
        criteria: Option<IndexMap<String, Value>>,
    },
    /// Pick one option from a set.
    Choice {
        /// The question: a string, object, or array.
        instructions: Value,
        /// Options in order, mapped to a description (string, object, or null).
        #[schema(value_type = HashMap<String, Object>)]
        criteria: IndexMap<String, Value>,
    },
    /// Rate on an ordered rubric of 2 to 10 levels.
    Score {
        /// The question: a string, object, or array.
        instructions: Value,
        /// Ordered level descriptions, lowest first.
        criteria: Vec<Value>,
    },
}

impl SystemOneQuestion {
    /// The question's instructions value.
    pub fn instructions(&self) -> &Value {
        match self {
            SystemOneQuestion::Noul { instructions, .. }
            | SystemOneQuestion::Choice { instructions, .. }
            | SystemOneQuestion::Score { instructions, .. } => instructions,
        }
    }

    /// Wire name of the question type.
    pub fn type_name(&self) -> &'static str {
        match self {
            SystemOneQuestion::Noul { .. } => "noul",
            SystemOneQuestion::Choice { .. } => "choice",
            SystemOneQuestion::Score { .. } => "score",
        }
    }
}

/// A typed answer.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SystemOneAnswer {
    /// Probability (0-1) that the answer is yes/true.
    Noul {
        noul: f64,
        /// Extra fields some backends add (e.g. Kev's `confidence`).
        #[serde(flatten)]
        #[schema(value_type = Object)]
        extra: Map<String, Value>,
    },
    /// The most likely option with the full distribution.
    Choice {
        choice: String,
        /// 0-1, derived from the probability distribution.
        confidence: f64,
        /// Probability per option, in the question's option order.
        #[schema(value_type = HashMap<String, f64>)]
        probabilities: IndexMap<String, f64>,
        #[serde(flatten)]
        #[schema(value_type = Object)]
        extra: Map<String, Value>,
    },
    /// Probability-weighted level index with the full distribution.
    Score {
        /// Expected level, from 0 to levels-1.
        score: f64,
        /// 0-1, derived from the probability distribution.
        confidence: f64,
        /// Level index (as a string) to the level's description.
        #[schema(value_type = HashMap<String, Object>)]
        legend: IndexMap<String, Value>,
        /// Level index (as a string) to probability.
        #[schema(value_type = HashMap<String, f64>)]
        probabilities: IndexMap<String, f64>,
        #[serde(flatten)]
        #[schema(value_type = Object)]
        extra: Map<String, Value>,
    },
}

impl SystemOneAnswer {
    /// Wire name of the answer type.
    pub fn type_name(&self) -> &'static str {
        match self {
            SystemOneAnswer::Noul { .. } => "noul",
            SystemOneAnswer::Choice { .. } => "choice",
            SystemOneAnswer::Score { .. } => "score",
        }
    }
}

/// Token usage for a System One request.
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema, PartialEq)]
pub struct SystemOneUsage {
    #[serde(default)]
    pub input_tokens: Option<u64>,
    #[serde(default)]
    pub output_tokens: Option<u64>,
}

/// How a System One answer was produced.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SystemOneBackend {
    /// A provider that speaks `/v1/systemone` natively.
    #[default]
    Native,
    /// Translated to chat completions; probabilities from the answer letter's
    /// token logprobs.
    LetterLogprobs,
    /// Translated to chat completions; probabilities self-reported by the
    /// model as JSON.
    Json,
}

impl SystemOneBackend {
    /// Value used in the `x-localrouter-systemone-backend` response header.
    pub fn as_str(&self) -> &'static str {
        match self {
            SystemOneBackend::Native => "native",
            SystemOneBackend::LetterLogprobs => "letter_logprobs",
            SystemOneBackend::Json => "json",
        }
    }
}

/// A System One decision response.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SystemOneResponse {
    /// The model that answered, as reported by the backend (e.g. `jev-1.13.0`).
    pub model: String,
    /// One answer per question id, in request order.
    #[serde(default)]
    #[schema(value_type = HashMap<String, SystemOneAnswer>)]
    pub answers: IndexMap<String, SystemOneAnswer>,
    #[serde(default)]
    pub usage: SystemOneUsage,
    /// Extra top-level fields some backends add (e.g. `latency_ms`, `quota`).
    #[serde(flatten)]
    #[schema(value_type = Object)]
    pub extra: Map<String, Value>,
    /// Provider instance that served the request (set by the router).
    #[serde(skip)]
    pub provider: String,
    /// Upstream `x-typesafe-request-id`, when present.
    #[serde(skip)]
    pub request_id: Option<String>,
    /// How the answers were produced.
    #[serde(skip)]
    pub backend: SystemOneBackend,
    /// Cost in USD computed by the router (sum over all upstream calls).
    #[serde(skip)]
    pub cost_usd: Option<f64>,
}

/// Validate a request before routing. Returns a message naming the offending
/// field on failure.
pub fn validate_systemone_request(req: &SystemOneRequest) -> Result<(), String> {
    if req.state.is_null() {
        return Err("'state' is required and must be a string, object, or array".into());
    }
    if req.extra.contains_key("stream") {
        return Err("'stream' is not supported by /v1/systemone".into());
    }
    if let Some(model) = &req.model {
        if model.trim().is_empty() {
            return Err("'model' must not be empty when provided".into());
        }
    }
    if req.questions.is_empty() {
        return Err("'questions' must contain at least one question".into());
    }
    for (id, question) in &req.questions {
        if id.trim().is_empty() {
            return Err("question ids must not be empty".into());
        }
        if question.instructions().is_null() {
            return Err(format!("questions.{id}.instructions is required"));
        }
        match question {
            SystemOneQuestion::Noul { criteria, .. } => {
                if let Some(criteria) = criteria {
                    if let Some(bad) = criteria.keys().find(|k| *k != "true" && *k != "false") {
                        return Err(format!(
                            "questions.{id}.criteria may only contain 'true' and 'false' (found '{bad}')"
                        ));
                    }
                }
            }
            SystemOneQuestion::Choice { criteria, .. } => {
                if criteria.is_empty() {
                    return Err(format!(
                        "questions.{id}.criteria must contain at least one option"
                    ));
                }
                if criteria.len() > MAX_CHOICE_OPTIONS {
                    return Err(format!(
                        "questions.{id}.criteria has {} options; the maximum is {MAX_CHOICE_OPTIONS}",
                        criteria.len()
                    ));
                }
                if criteria.keys().any(|k| k.is_empty()) {
                    return Err(format!(
                        "questions.{id}.criteria option names must not be empty"
                    ));
                }
            }
            SystemOneQuestion::Score { criteria, .. } => {
                if criteria.len() < MIN_SCORE_LEVELS || criteria.len() > MAX_SCORE_LEVELS {
                    return Err(format!(
                        "questions.{id}.criteria must have between {MIN_SCORE_LEVELS} and {MAX_SCORE_LEVELS} levels (found {})",
                        criteria.len()
                    ));
                }
            }
        }
    }
    Ok(())
}

/// Confidence derived from a probability distribution:
/// `(p_max - 1/K) / (1 - 1/K)`, clamped to 0..=1. This is Kev's published
/// formula; TypeSafe's is unpublished. Returns 1.0 for a single option.
pub fn confidence_from_probabilities(probs: &[f64]) -> f64 {
    let k = probs.len();
    if k <= 1 {
        return 1.0;
    }
    let p_max = probs.iter().cloned().fold(0.0_f64, f64::max);
    let uniform = 1.0 / k as f64;
    ((p_max - uniform) / (1.0 - uniform)).clamp(0.0, 1.0)
}

/// Normalize non-negative weights to sum to 1. Negative or non-finite values
/// count as 0. If everything is 0, returns the uniform distribution.
pub fn normalize(weights: &[f64]) -> Vec<f64> {
    let cleaned: Vec<f64> = weights
        .iter()
        .map(|w| if w.is_finite() && *w > 0.0 { *w } else { 0.0 })
        .collect();
    let max = cleaned.iter().copied().fold(0.0_f64, f64::max);
    if max <= 0.0 {
        if cleaned.is_empty() {
            return cleaned;
        }
        let u = 1.0 / cleaned.len() as f64;
        return vec![u; cleaned.len()];
    }
    // Scale first: summing finite large weights can overflow to infinity,
    // which would otherwise turn every normalized probability into zero.
    let sum: f64 = cleaned.iter().map(|w| w / max).sum();
    cleaned.iter().map(|w| (w / max) / sum).collect()
}

/// Build a typed answer for `question` from a probability distribution over
/// its options (choice), levels (score), or `[p_yes, p_no]` (noul). The
/// distribution is normalized first.
pub fn answer_from_distribution(question: &SystemOneQuestion, weights: &[f64]) -> SystemOneAnswer {
    let probs = normalize(weights);
    match question {
        SystemOneQuestion::Noul { .. } => SystemOneAnswer::Noul {
            noul: probs.first().copied().unwrap_or(0.5),
            extra: Map::new(),
        },
        SystemOneQuestion::Choice { criteria, .. } => {
            let mut best_idx = 0;
            for (i, p) in probs.iter().enumerate() {
                if *p > probs[best_idx] {
                    best_idx = i;
                }
            }
            let probabilities: IndexMap<String, f64> = criteria
                .keys()
                .cloned()
                .zip(probs.iter().copied())
                .collect();
            SystemOneAnswer::Choice {
                choice: criteria
                    .get_index(best_idx)
                    .map(|(k, _)| k.clone())
                    .unwrap_or_default(),
                confidence: confidence_from_probabilities(&probs),
                probabilities,
                extra: Map::new(),
            }
        }
        SystemOneQuestion::Score { criteria, .. } => {
            let score = probs.iter().enumerate().map(|(i, p)| i as f64 * p).sum();
            SystemOneAnswer::Score {
                score,
                confidence: confidence_from_probabilities(&probs),
                legend: criteria
                    .iter()
                    .enumerate()
                    .map(|(i, v)| (i.to_string(), v.clone()))
                    .collect(),
                probabilities: probs
                    .iter()
                    .enumerate()
                    .map(|(i, p)| (i.to_string(), *p))
                    .collect(),
                extra: Map::new(),
            }
        }
    }
}

/// Number of answer slots for a question: options (choice), levels (score),
/// or 2 (noul: yes, no).
pub fn option_count(question: &SystemOneQuestion) -> usize {
    match question {
        SystemOneQuestion::Noul { .. } => 2,
        SystemOneQuestion::Choice { criteria, .. } => criteria.len(),
        SystemOneQuestion::Score { criteria, .. } => criteria.len(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn parse_req(v: Value) -> SystemOneRequest {
        serde_json::from_value(v).expect("valid request")
    }

    #[test]
    fn jev_response_round_trips() {
        let body = json!({
            "model": "jev-1.13.0",
            "answers": {
                "department": {"type": "choice", "choice": "technical",
                    "probabilities": {"billing": 0.08, "technical": 0.85, "sales": 0.07},
                    "confidence": 0.82},
                "urgency": {"type": "score", "score": 1.2, "confidence": 0.25,
                    "legend": {"0": "low", "1": "medium", "2": "high"},
                    "probabilities": {"0": 0.15, "1": 0.5, "2": 0.35}},
                "review": {"type": "noul", "noul": 0.75}
            },
            "usage": {"input_tokens": 312, "output_tokens": 48}
        });
        let resp: SystemOneResponse = serde_json::from_value(body.clone()).unwrap();
        assert_eq!(resp.model, "jev-1.13.0");
        assert_eq!(resp.usage.input_tokens, Some(312));
        // Answer order and option order survive.
        let ids: Vec<_> = resp.answers.keys().cloned().collect();
        assert_eq!(ids, vec!["department", "urgency", "review"]);
        match &resp.answers["department"] {
            SystemOneAnswer::Choice { probabilities, .. } => {
                let keys: Vec<_> = probabilities.keys().cloned().collect();
                assert_eq!(keys, vec!["billing", "technical", "sales"]);
            }
            other => panic!("unexpected {other:?}"),
        }
        assert_eq!(serde_json::to_value(&resp).unwrap(), body);
    }

    #[test]
    fn kev_extras_are_preserved() {
        let body = json!({
            "model": "kev-latest",
            "answers": {
                "q": {"type": "noul", "noul": 0.93, "confidence": 0.78, "probabilities": {"true": 0.93, "false": 0.07}}
            },
            "usage": {"input_tokens": 101, "output_tokens": 161},
            "latency_ms": 495
        });
        let resp: SystemOneResponse = serde_json::from_value(body.clone()).unwrap();
        assert_eq!(resp.extra.get("latency_ms"), Some(&json!(495)));
        match &resp.answers["q"] {
            SystemOneAnswer::Noul { noul, extra } => {
                assert_eq!(*noul, 0.93);
                assert_eq!(extra.get("confidence"), Some(&json!(0.78)));
            }
            other => panic!("unexpected {other:?}"),
        }
        assert_eq!(serde_json::to_value(&resp).unwrap(), body);
    }

    #[test]
    fn request_without_model_parses_and_keeps_option_order() {
        let req = parse_req(json!({
            "state": {"body": "billed twice"},
            "questions": {
                "dept": {"type": "choice", "instructions": "which team?",
                    "criteria": {"zeta": "z", "alpha": null, "mid": {"definition": "m"}}}
            }
        }));
        assert!(req.model.is_none());
        match &req.questions["dept"] {
            SystemOneQuestion::Choice { criteria, .. } => {
                let keys: Vec<_> = criteria.keys().cloned().collect();
                assert_eq!(keys, vec!["zeta", "alpha", "mid"]);
            }
            other => panic!("unexpected {other:?}"),
        }
        // Serializing drops the absent model rather than emitting null.
        let v = serde_json::to_value(&req).unwrap();
        assert!(v.get("model").is_none());
    }

    #[test]
    fn unknown_request_fields_are_forwarded() {
        let req = parse_req(json!({
            "state": "x", "model": "m", "vendor_option": 3,
            "questions": {"q": {"type": "noul", "instructions": "?"}}
        }));
        assert_eq!(req.extra.get("vendor_option"), Some(&json!(3)));
        assert_eq!(
            serde_json::to_value(&req).unwrap()["vendor_option"],
            json!(3)
        );
    }

    #[test]
    fn validation_bounds() {
        let ok = parse_req(json!({"state": "s", "questions": {
            "c": {"type": "choice", "instructions": "i", "criteria": {"a": null}},
            "s": {"type": "score", "instructions": "i", "criteria": ["lo", "hi"]},
            "n": {"type": "noul", "instructions": "i", "criteria": {"true": "t", "false": "f"}}
        }}));
        assert!(validate_systemone_request(&ok).is_ok());

        let null_state = parse_req(
            json!({"state": null, "questions": {"n": {"type": "noul", "instructions": "i"}}}),
        );
        assert!(validate_systemone_request(&null_state)
            .unwrap_err()
            .contains("state"));

        let no_questions = parse_req(json!({"state": "s", "questions": {}}));
        assert!(validate_systemone_request(&no_questions)
            .unwrap_err()
            .contains("questions"));

        let empty_choice = parse_req(
            json!({"state": "s", "questions": {"c": {"type": "choice", "instructions": "i", "criteria": {}}}}),
        );
        assert!(validate_systemone_request(&empty_choice).is_err());

        let mut many = serde_json::Map::new();
        for i in 0..256 {
            many.insert(format!("o{i}"), Value::Null);
        }
        let too_many = parse_req(
            json!({"state": "s", "questions": {"c": {"type": "choice", "instructions": "i", "criteria": many}}}),
        );
        assert!(validate_systemone_request(&too_many)
            .unwrap_err()
            .contains("255"));

        let one_level = parse_req(
            json!({"state": "s", "questions": {"s": {"type": "score", "instructions": "i", "criteria": ["only"]}}}),
        );
        assert!(validate_systemone_request(&one_level).is_err());
        let eleven: Vec<String> = (0..11).map(|i| i.to_string()).collect();
        let too_many_levels = parse_req(
            json!({"state": "s", "questions": {"s": {"type": "score", "instructions": "i", "criteria": eleven}}}),
        );
        assert!(validate_systemone_request(&too_many_levels).is_err());

        let bad_noul = parse_req(
            json!({"state": "s", "questions": {"n": {"type": "noul", "instructions": "i", "criteria": {"maybe": "?"}}}}),
        );
        assert!(validate_systemone_request(&bad_noul)
            .unwrap_err()
            .contains("maybe"));

        let streaming = parse_req(
            json!({"state": "s", "stream": true, "questions": {"n": {"type": "noul", "instructions": "i"}}}),
        );
        assert!(validate_systemone_request(&streaming)
            .unwrap_err()
            .contains("stream"));

        let null_instructions = parse_req(
            json!({"state": "s", "questions": {"n": {"type": "noul", "instructions": null}}}),
        );
        assert!(validate_systemone_request(&null_instructions).is_err());
    }

    #[test]
    fn unknown_question_type_is_rejected_by_serde() {
        let r: Result<SystemOneRequest, _> = serde_json::from_value(json!({
            "state": "s", "questions": {"q": {"type": "rank", "instructions": "i"}}
        }));
        assert!(r.is_err());
    }

    #[test]
    fn confidence_formula() {
        assert_eq!(confidence_from_probabilities(&[1.0]), 1.0);
        assert!((confidence_from_probabilities(&[0.5, 0.5]) - 0.0).abs() < 1e-9);
        assert!((confidence_from_probabilities(&[1.0, 0.0]) - 1.0).abs() < 1e-9);
        // (0.9 - 0.5) / 0.5 = 0.8
        assert!((confidence_from_probabilities(&[0.9, 0.1]) - 0.8).abs() < 1e-9);
    }

    #[test]
    fn normalize_handles_degenerate_input() {
        assert_eq!(normalize(&[0.0, 0.0]), vec![0.5, 0.5]);
        assert_eq!(normalize(&[-1.0, f64::NAN, 2.0]), vec![0.0, 0.0, 1.0]);
        assert!(normalize(&[]).is_empty());
        assert_eq!(normalize(&[f64::MAX, f64::MAX]), vec![0.5, 0.5]);
        assert_eq!(
            normalize(&[f64::MIN_POSITIVE, f64::MIN_POSITIVE]),
            vec![0.5, 0.5]
        );
    }

    #[test]
    fn answers_from_distribution() {
        let choice = SystemOneQuestion::Choice {
            instructions: json!("i"),
            criteria: [
                ("a".to_string(), Value::Null),
                ("b".to_string(), Value::Null),
            ]
            .into_iter()
            .collect(),
        };
        match answer_from_distribution(&choice, &[1.0, 3.0]) {
            SystemOneAnswer::Choice {
                choice,
                probabilities,
                confidence,
                ..
            } => {
                assert_eq!(choice, "b");
                assert_eq!(probabilities["a"], 0.25);
                assert!((confidence - 0.5).abs() < 1e-9);
            }
            other => panic!("unexpected {other:?}"),
        }

        let score = SystemOneQuestion::Score {
            instructions: json!("i"),
            criteria: vec![json!("lo"), json!("mid"), json!("hi")],
        };
        match answer_from_distribution(&score, &[0.0, 0.5, 0.5]) {
            SystemOneAnswer::Score {
                score,
                legend,
                probabilities,
                ..
            } => {
                assert!((score - 1.5).abs() < 1e-9);
                assert_eq!(legend["2"], json!("hi"));
                assert_eq!(probabilities["0"], 0.0);
            }
            other => panic!("unexpected {other:?}"),
        }

        let noul = SystemOneQuestion::Noul {
            instructions: json!("i"),
            criteria: None,
        };
        match answer_from_distribution(&noul, &[3.0, 1.0]) {
            SystemOneAnswer::Noul { noul, .. } => assert!((noul - 0.75).abs() < 1e-9),
            other => panic!("unexpected {other:?}"),
        }
    }
}
