//! OpenAI Decisions wire contract and lossless adapters to the existing router.
//! Source: https://developers.openai.com/api/reference/resources/decisions/methods/create
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use utoipa::ToSchema;

use super::types::{
    SystemOneAnswer, SystemOneQuestion, SystemOneRequest, SystemOneResponse, SystemOneUsage,
};

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq)]
#[serde(untagged)]
pub enum DecisionValue {
    String(String),
    Bool(bool),
}
impl DecisionValue {
    fn key(&self) -> String {
        serde_json::to_string(self).expect("string/bool serialization")
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq)]
pub struct DecisionChoice {
    pub value: DecisionValue,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq)]
pub struct DecisionLevel {
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum DecisionQuestion {
    Predicate {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        instructions: String,
    },
    Choice {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        instructions: String,
        choices: Vec<DecisionChoice>,
    },
    Score {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        instructions: String,
        levels: Vec<DecisionLevel>,
    },
}
impl DecisionQuestion {
    pub fn name(&self) -> Option<&str> {
        match self {
            Self::Predicate { name, .. } | Self::Choice { name, .. } | Self::Score { name, .. } => {
                name.as_deref()
            }
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum DecisionInputPart {
    InputText {
        text: String,
    },
    InputImage {
        image_url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        detail: Option<DecisionImageDetail>,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum DecisionImageDetail {
    Low,
    High,
    Auto,
    Original,
}
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq)]
#[serde(untagged)]
pub enum DecisionContent {
    Text(String),
    Parts(Vec<DecisionInputPart>),
}
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct DecisionInputMessage {
    pub role: DecisionRole,
    pub content: DecisionContent,
    #[serde(default, rename = "type", skip_serializing_if = "Option::is_none")]
    pub message_type: Option<DecisionMessageType>,
}
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum DecisionRole {
    User,
}
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum DecisionMessageType {
    Message,
}
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq)]
#[serde(untagged)]
pub enum DecisionInput {
    Text(String),
    Messages(Vec<DecisionInputMessage>),
}
impl DecisionInput {
    pub fn has_images(&self) -> bool {
        matches!(self, Self::Messages(messages) if messages.iter().any(|m| matches!(&m.content, DecisionContent::Parts(parts) if parts.iter().any(|p| matches!(p, DecisionInputPart::InputImage { .. })))))
    }
    fn text(&self) -> String {
        match self {
            Self::Text(text) => text.clone(),
            Self::Messages(messages) => messages
                .iter()
                .map(|m| match &m.content {
                    DecisionContent::Text(text) => text.clone(),
                    DecisionContent::Parts(parts) => parts
                        .iter()
                        .filter_map(|p| match p {
                            DecisionInputPart::InputText { text } => Some(text.as_str()),
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                        .join("\n"),
                })
                .collect::<Vec<_>>()
                .join("\n\n"),
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct DecisionRequest {
    pub input: DecisionInput,
    pub model: String,
    pub questions: Vec<DecisionQuestion>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub safety_identifier: Option<String>,
    /// Provider-specific extensions are preserved only for native endpoints.
    #[serde(flatten)]
    #[schema(value_type = Object)]
    pub extra: Map<String, Value>,
}
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq)]
pub struct DecisionProbability {
    pub value: DecisionValue,
    pub probability: f64,
}
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq)]
pub struct DecisionScoreProbability {
    pub value: usize,
    pub label: String,
    pub probability: f64,
}
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DecisionAnswer {
    Predicate {
        name: Option<String>,
        probability: f64,
    },
    Choice {
        name: Option<String>,
        choice: DecisionValue,
        confidence: f64,
        probabilities: Vec<DecisionProbability>,
    },
    Score {
        name: Option<String>,
        score: f64,
        confidence: f64,
        probabilities: Vec<DecisionScoreProbability>,
    },
    Refusal {
        name: Option<String>,
    },
}
impl DecisionAnswer {
    fn name(&self) -> Option<&str> {
        match self {
            Self::Predicate { name, .. }
            | Self::Choice { name, .. }
            | Self::Score { name, .. }
            | Self::Refusal { name } => name.as_deref(),
        }
    }
}
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
pub struct DecisionUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
    #[serde(flatten)]
    #[schema(value_type = Object)]
    pub extra: Map<String, Value>,
}
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct DecisionResponse {
    pub model: String,
    pub answers: Vec<DecisionAnswer>,
    pub usage: DecisionUsage,
    #[serde(flatten)]
    #[schema(value_type = Object)]
    pub extra: Map<String, Value>,
}

impl DecisionRequest {
    pub fn validate(&self) -> Result<(), String> {
        if self.model.trim().is_empty() {
            return Err("'model' must not be empty".into());
        }
        if self.extra.contains_key("stream") {
            return Err("'stream' is not supported by /v1/decisions".into());
        }
        if !(1..=200).contains(&self.questions.len()) {
            return Err("'questions' must contain 1 to 200 questions".into());
        }
        if self
            .safety_identifier
            .as_ref()
            .is_some_and(|s| s.chars().count() > 128)
        {
            return Err("'safety_identifier' must contain at most 128 characters".into());
        }
        let mut names = std::collections::HashSet::new();
        for q in &self.questions {
            if let Some(name) = q.name() {
                if !names.insert(name) {
                    return Err(format!("Duplicate question name '{name}'"));
                }
            }
            match q {
                DecisionQuestion::Choice { choices, .. } => {
                    if !(2..=255).contains(&choices.len()) {
                        return Err("Choice questions require 2 to 255 choices".into());
                    }
                    let mut values = std::collections::HashSet::new();
                    if choices.iter().any(|c| !values.insert(c.value.key())) {
                        return Err("Choice values must be unique".into());
                    }
                }
                DecisionQuestion::Score { levels, .. } if levels.len() < 2 => {
                    return Err("Score questions require at least 2 levels".into());
                }
                _ => {}
            }
        }
        let mut images = 0;
        if let DecisionInput::Messages(messages) = &self.input {
            if messages.is_empty() {
                return Err("'input' messages must not be empty".into());
            }
            for message in messages {
                if let DecisionContent::Parts(parts) = &message.content {
                    if parts.is_empty() {
                        return Err("Input message content must not be empty".into());
                    }
                    for part in parts {
                        if let DecisionInputPart::InputImage { image_url, .. } = part {
                            images += 1;
                            if !image_url.starts_with("data:image/")
                                && !image_url.starts_with("https://")
                                && !image_url.starts_with("http://")
                            {
                                return Err(
                                    "Image input requires a data image or HTTP(S) URL".into()
                                );
                            }
                        }
                    }
                }
            }
        }
        if images > 128 {
            return Err("At most 128 input images are allowed".into());
        }
        Ok(())
    }

    /// Private positional keys prevent unnamed questions and typed choice values colliding.
    pub fn into_systemone(self) -> SystemOneRequest {
        let questions = self
            .questions
            .iter()
            .enumerate()
            .map(|(i, q)| {
                let question = match q {
                    DecisionQuestion::Predicate { instructions, .. } => SystemOneQuestion::Noul {
                        instructions: json!(instructions),
                        criteria: None,
                    },
                    DecisionQuestion::Choice {
                        instructions,
                        choices,
                        ..
                    } => SystemOneQuestion::Choice {
                        instructions: json!(instructions),
                        criteria: choices
                            .iter()
                            .map(|c| (c.value.key(), json!(c.description)))
                            .collect(),
                    },
                    DecisionQuestion::Score {
                        instructions,
                        levels,
                        ..
                    } => SystemOneQuestion::Score {
                        instructions: json!(instructions),
                        criteria: levels
                            .iter()
                            .map(|l| serde_json::to_value(l).expect("level serialization"))
                            .collect(),
                    },
                };
                (format!("decision_{i}"), question)
            })
            .collect();
        SystemOneRequest {
            state: serde_json::to_value(&self.input).expect("input serialization"),
            model: Some(self.model.clone()),
            questions,
            extra: self.extra.clone(),
            decisions: Some(Box::new(self)),
        }
    }

    pub fn from_systemone(request: &SystemOneRequest) -> Result<Self, String> {
        if let Some(original) = &request.decisions {
            let mut updated = (**original).clone();
            updated.model = request
                .model
                .clone()
                .unwrap_or_else(|| original.model.clone());
            updated.extra = request.extra.clone();
            updated.input = serde_json::from_value(request.state.clone())
                .map_err(|e| format!("Invalid decision input: {e}"))?;
            updated.validate()?;
            return Ok(updated);
        }
        let input = DecisionInput::Text(match &request.state {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        });
        let string = |value: &Value| match value {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        let questions = request
            .questions
            .iter()
            .map(|(name, q)| match q {
                SystemOneQuestion::Noul {
                    instructions,
                    criteria,
                } => {
                    let mut instructions = string(instructions);
                    if let Some(criteria) = criteria {
                        instructions
                            .push_str(&format!("\nPredicate criteria: {}", json!(criteria)));
                    }
                    DecisionQuestion::Predicate {
                        name: Some(name.clone()),
                        instructions,
                    }
                }
                SystemOneQuestion::Choice {
                    instructions,
                    criteria,
                } => DecisionQuestion::Choice {
                    name: Some(name.clone()),
                    instructions: string(instructions),
                    choices: criteria
                        .iter()
                        .map(|(value, description)| DecisionChoice {
                            value: DecisionValue::String(value.clone()),
                            description: (!description.is_null()).then(|| string(description)),
                        })
                        .collect(),
                },
                SystemOneQuestion::Score {
                    instructions,
                    criteria,
                } => DecisionQuestion::Score {
                    name: Some(name.clone()),
                    instructions: string(instructions),
                    levels: criteria
                        .iter()
                        .enumerate()
                        .map(|(i, level)| DecisionLevel {
                            label: level
                                .as_str()
                                .map(str::to_string)
                                .unwrap_or_else(|| i.to_string()),
                            description: (!level.is_string()).then(|| string(level)),
                        })
                        .collect(),
                },
            })
            .collect();
        let decision = Self {
            input,
            model: request.model.clone().ok_or("A model is required")?,
            questions,
            safety_identifier: None,
            extra: request.extra.clone(),
        };
        Ok(decision)
    }

    /// OpenRouter alpha uses map questions and image_url state parts.
    pub fn openrouter_request(request: &SystemOneRequest) -> Result<SystemOneRequest, String> {
        let mut adapted = request.clone();
        if request.decisions.is_some() {
            let decision = Self::from_systemone(request)?;
            let mut parts = Vec::new();
            match decision.input {
                DecisionInput::Text(text) => adapted.state = json!(text),
                DecisionInput::Messages(messages) => {
                    for message in messages {
                        match message.content {
                            DecisionContent::Text(text) => parts.push(json!(text)),
                            DecisionContent::Parts(content) => {
                                for part in content {
                                    match part {
                                        DecisionInputPart::InputText { text } => {
                                            parts.push(json!(text))
                                        }
                                        DecisionInputPart::InputImage { image_url, detail } => {
                                            if !image_url.starts_with("data:image/") {
                                                return Err(
                                                    "OpenRouter decision images require data URLs"
                                                        .into(),
                                                );
                                            }
                                            let mut image = json!({"url":image_url});
                                            if let Some(detail) = detail {
                                                image["detail"] = json!(detail);
                                            }
                                            parts.push(
                                                json!({"type":"image_url","image_url":image}),
                                            );
                                        }
                                    }
                                }
                            }
                        }
                    }
                    adapted.state = json!(parts);
                }
            }
            if let Some(identifier) = decision.safety_identifier {
                adapted
                    .extra
                    .insert("safety_identifier".into(), json!(identifier));
            }
        }
        Ok(adapted)
    }

    pub fn validate_vercel(&self) -> Result<(), String> {
        if let DecisionInput::Messages(messages) = &self.input {
            for message in messages {
                if let DecisionContent::Parts(parts) = &message.content {
                    if parts.iter().any(|p| matches!(p, DecisionInputPart::InputImage { image_url, .. } if !image_url.starts_with("data:image/"))) {
                        return Err("Vercel decision images require data URLs".into());
                    }
                }
            }
        }
        for question in &self.questions {
            match question {
                DecisionQuestion::Choice { choices, .. } => {
                    let mut values = std::collections::HashSet::new();
                    for choice in choices {
                        let value = match &choice.value {
                            DecisionValue::String(s) => s.clone(),
                            DecisionValue::Bool(b) => b.to_string(),
                        };
                        if !values.insert(value) {
                            return Err("Vercel cannot distinguish a boolean choice from a string with the same text".into());
                        }
                    }
                }
                DecisionQuestion::Score { levels, .. } if levels.len() > 10 => {
                    return Err("Vercel supports at most 10 score levels".into())
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// Reject image input rather than serializing it as text to a legacy engine.
    pub fn legacy_request(request: &SystemOneRequest) -> Result<SystemOneRequest, String> {
        let decision = Self::from_systemone(request)?;
        if decision.input.has_images() {
            return Err("Image decisions require a native Decisions provider".into());
        }
        let mut legacy = request.clone();
        legacy.state = json!(decision.input.text());
        legacy.decisions = None;
        super::types::validate_systemone_request(&legacy)?;
        Ok(legacy)
    }
}

impl DecisionResponse {
    pub fn into_systemone(self, request: &SystemOneRequest) -> Result<SystemOneResponse, String> {
        let questions = DecisionRequest::from_systemone(request)?;
        if self.answers.len() != questions.questions.len() {
            return Err("Decision provider returned an incorrect answer count".into());
        }
        let mut answers = IndexMap::new();
        for (((id, _), question), answer) in request
            .questions
            .iter()
            .zip(&questions.questions)
            .zip(self.answers)
        {
            if answer.name() != question.name() {
                return Err("Decision provider returned answers with incorrect names/order".into());
            }
            let prob = |p: f64| p.is_finite() && (0.0..=1.0).contains(&p);
            let answer = match (question, answer) {
                (_, DecisionAnswer::Refusal { .. }) => {
                    SystemOneAnswer::Refusal { extra: Map::new() }
                }
                (
                    DecisionQuestion::Predicate { .. },
                    DecisionAnswer::Predicate { probability, .. },
                ) if prob(probability) => SystemOneAnswer::Noul {
                    noul: probability,
                    extra: Map::new(),
                },
                (
                    DecisionQuestion::Choice { choices, .. },
                    DecisionAnswer::Choice {
                        choice,
                        confidence,
                        probabilities,
                        ..
                    },
                ) => {
                    if !prob(confidence)
                        || !choices.iter().any(|c| c.value == choice)
                        || probabilities.len() != choices.len()
                    {
                        return Err("Invalid choice answer".into());
                    }
                    let mut mapped = IndexMap::new();
                    for c in choices {
                        let matches: Vec<_> = probabilities
                            .iter()
                            .filter(|p| p.value == c.value)
                            .collect();
                        if matches.len() != 1 || !prob(matches[0].probability) {
                            return Err("Invalid choice probability distribution".into());
                        }
                        mapped.insert(
                            if request.decisions.is_some() {
                                c.value.key()
                            } else {
                                match &c.value {
                                    DecisionValue::String(s) => s.clone(),
                                    _ => return Err("Legacy choices require strings".into()),
                                }
                            },
                            matches[0].probability,
                        );
                    }
                    if (mapped.values().sum::<f64>() - 1.0).abs() > 0.01 {
                        return Err("Choice probabilities must sum to one".into());
                    }
                    let choice = if request.decisions.is_some() {
                        choice.key()
                    } else {
                        match choice {
                            DecisionValue::String(s) => s,
                            _ => return Err("Legacy choices require strings".into()),
                        }
                    };
                    SystemOneAnswer::Choice {
                        choice,
                        confidence,
                        probabilities: mapped,
                        extra: Map::new(),
                    }
                }
                (
                    DecisionQuestion::Score { levels, .. },
                    DecisionAnswer::Score {
                        score,
                        confidence,
                        probabilities,
                        ..
                    },
                ) => {
                    if !score.is_finite()
                        || !(0.0..=(levels.len() - 1) as f64).contains(&score)
                        || !prob(confidence)
                        || probabilities.len() != levels.len()
                    {
                        return Err("Invalid score answer".into());
                    }
                    let mut mapped = IndexMap::new();
                    for (i, level) in levels.iter().enumerate() {
                        let matches: Vec<_> =
                            probabilities.iter().filter(|p| p.value == i).collect();
                        if matches.len() != 1
                            || matches[0].label != level.label
                            || !prob(matches[0].probability)
                        {
                            return Err("Invalid score probability distribution".into());
                        }
                        mapped.insert(i.to_string(), matches[0].probability);
                    }
                    if (mapped.values().sum::<f64>() - 1.0).abs() > 0.01 {
                        return Err("Score probabilities must sum to one".into());
                    }
                    let legend = match &request.questions[id] {
                        SystemOneQuestion::Score { criteria, .. } => criteria
                            .iter()
                            .enumerate()
                            .map(|(i, c)| (i.to_string(), c.clone()))
                            .collect(),
                        _ => unreachable!(),
                    };
                    SystemOneAnswer::Score {
                        score,
                        confidence,
                        legend,
                        probabilities: mapped,
                        extra: Map::new(),
                    }
                }
                _ => {
                    return Err(
                        "Decision provider returned an invalid answer type or probability".into(),
                    )
                }
            };
            answers.insert(id.clone(), answer);
        }
        Ok(SystemOneResponse {
            model: self.model,
            answers,
            usage: SystemOneUsage {
                input_tokens: Some(self.usage.input_tokens),
                output_tokens: Some(self.usage.output_tokens),
                extra: self.usage.extra,
            },
            extra: self.extra,
            provider: String::new(),
            request_id: None,
            backend: Default::default(),
            cost_usd: None,
        })
    }

    pub fn from_systemone(
        response: &SystemOneResponse,
        request: &SystemOneRequest,
    ) -> Result<Self, String> {
        let decision = DecisionRequest::from_systemone(request)?;
        let mut answers = Vec::new();
        for ((id, _), q) in request.questions.iter().zip(&decision.questions) {
            let name = q.name().map(str::to_string);
            let answer = response
                .answers
                .get(id)
                .ok_or_else(|| format!("Provider omitted answer '{id}'"))?;
            answers.push(match (q, answer) {
                (_, SystemOneAnswer::Refusal { .. }) => DecisionAnswer::Refusal { name },
                (DecisionQuestion::Predicate { .. }, SystemOneAnswer::Noul { noul, .. }) => {
                    DecisionAnswer::Predicate {
                        name,
                        probability: *noul,
                    }
                }
                (
                    DecisionQuestion::Choice { choices, .. },
                    SystemOneAnswer::Choice {
                        choice,
                        confidence,
                        probabilities,
                        ..
                    },
                ) => DecisionAnswer::Choice {
                    name,
                    choice: choices
                        .iter()
                        .find(|c| c.value.key() == *choice)
                        .map(|c| c.value.clone())
                        .ok_or("Provider returned an unknown choice")?,
                    confidence: *confidence,
                    probabilities: choices
                        .iter()
                        .map(|c| {
                            Ok(DecisionProbability {
                                value: c.value.clone(),
                                probability: *probabilities
                                    .get(&c.value.key())
                                    .ok_or("Provider omitted a choice probability")?,
                            })
                        })
                        .collect::<Result<_, String>>()?,
                },
                (
                    DecisionQuestion::Score { levels, .. },
                    SystemOneAnswer::Score {
                        score,
                        confidence,
                        probabilities,
                        ..
                    },
                ) => DecisionAnswer::Score {
                    name,
                    score: *score,
                    confidence: *confidence,
                    probabilities: levels
                        .iter()
                        .enumerate()
                        .map(|(i, l)| {
                            Ok(DecisionScoreProbability {
                                value: i,
                                label: l.label.clone(),
                                probability: *probabilities
                                    .get(&i.to_string())
                                    .ok_or("Provider omitted a score probability")?,
                            })
                        })
                        .collect::<Result<_, String>>()?,
                },
                _ => return Err("Provider returned an incompatible answer type".into()),
            });
        }
        let input_tokens = response.usage.input_tokens.unwrap_or(0);
        let output_tokens = response.usage.output_tokens.unwrap_or(0);
        let wire = Self {
            model: response.model.clone(),
            answers,
            usage: DecisionUsage {
                input_tokens,
                output_tokens,
                total_tokens: input_tokens.saturating_add(output_tokens),
                extra: response.usage.extra.clone(),
            },
            extra: response.extra.clone(),
        };
        // Legacy engines are not trusted to return valid probabilities either.
        wire.clone().into_systemone(request)?;
        Ok(wire)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request() -> DecisionRequest {
        serde_json::from_value(json!({"model":"gateway/decision", "input":"example", "questions":[{"type":"choice", "choices":[{"value":true},{"value":"true"}], "instructions":"pick"},{"type":"predicate", "instructions":"yes?"},{"type":"score", "name":"severity", "instructions":"rate", "levels":[{"label":"low"},{"label":"high"}]}]})).unwrap()
    }
    #[test]
    fn provider_specific_image_and_choice_restrictions_are_explicit() {
        let mut decision = request();
        assert!(decision.validate_vercel().is_err()); // boolean true and string "true"
        decision.questions = vec![DecisionQuestion::Predicate {
            name: None,
            instructions: "?".into(),
        }];
        decision.input = serde_json::from_value(json!([{"role":"user","content":[{"type":"input_text","text":"evidence"},{"type":"input_image","image_url":"data:image/png;base64,AAAA","detail":"high"}]}])).unwrap();
        decision.validate_vercel().unwrap();
        let adapted =
            DecisionRequest::openrouter_request(&decision.clone().into_systemone()).unwrap();
        assert_eq!(
            adapted.state,
            json!(["evidence",{"type":"image_url","image_url":{"url":"data:image/png;base64,AAAA","detail":"high"}}])
        );
        decision.input = serde_json::from_value(json!([{"role":"user","content":[{"type":"input_image","image_url":"https://example.com/image.png"}]}])).unwrap();
        assert!(decision.validate_vercel().is_err());
        assert!(DecisionRequest::openrouter_request(&decision.into_systemone()).is_err());
    }

    #[test]
    fn preserves_unnamed_questions_and_typed_values() {
        let r = request();
        r.validate().unwrap();
        let original = serde_json::to_value(&r).unwrap();
        let canonical = r.into_systemone();
        assert_eq!(canonical.questions.len(), 3);
        assert_eq!(
            serde_json::to_value(DecisionRequest::from_systemone(&canonical).unwrap()).unwrap(),
            original
        );
        let response: DecisionResponse = serde_json::from_value(json!({"model":"d", "answers":[{"type":"choice", "name":null, "choice":true,"confidence":0.8,"probabilities":[{"value":true,"probability":0.9},{"value":"true","probability":0.1}]},{"type":"refusal","name":null},{"type":"score","name":"severity","score":0.8,"confidence":0.6,"probabilities":[{"value":0,"label":"low","probability":0.2},{"value":1,"label":"high","probability":0.8}]}],"usage":{"input_tokens":10,"output_tokens":0,"total_tokens":10}})).unwrap();
        let wire = serde_json::to_value(&response).unwrap();
        let internal = response.into_systemone(&canonical).unwrap();
        assert_eq!(
            serde_json::to_value(DecisionResponse::from_systemone(&internal, &canonical).unwrap())
                .unwrap(),
            wire
        );
    }
    #[test]
    fn native_adapter_uses_sanitized_metadata_from_current_request() {
        let mut original = request();
        original.extra.insert(
            "metadata".into(),
            json!({"localrouter.mode":"plan", "tag":"keep"}),
        );
        let mut canonical = original.into_systemone();
        canonical
            .extra
            .get_mut("metadata")
            .unwrap()
            .as_object_mut()
            .unwrap()
            .remove("localrouter.mode");
        let wire = DecisionRequest::from_systemone(&canonical).unwrap();
        assert_eq!(wire.extra["metadata"], json!({"tag":"keep"}));
    }
    #[test]
    fn validates_duplicates_bounds_and_streaming() {
        let mut r = request();
        r.extra.insert("stream".into(), json!(false));
        assert!(r.validate().is_err());
        r.extra.clear();
        if let DecisionQuestion::Choice { choices, .. } = &mut r.questions[0] {
            choices[1].value = DecisionValue::Bool(true);
        }
        assert!(r.validate().is_err());
        let mut r = request();
        r.questions.clear();
        assert!(r.validate().is_err());
        let mut r = request();
        r.questions[0] = DecisionQuestion::Predicate {
            name: Some("severity".into()),
            instructions: "?".into(),
        };
        assert!(r.validate().is_err());
    }
    #[test]
    fn refuses_image_translation_but_preserves_native_input() {
        let mut r = request();
        r.input = serde_json::from_value(json!([{"role":"user","content":[{"type":"input_image","image_url":"data:image/png;base64,AAAA"}]}])).unwrap();
        r.validate().unwrap();
        let canonical = r.into_systemone();
        assert!(DecisionRequest::from_systemone(&canonical)
            .unwrap()
            .input
            .has_images());
        assert!(DecisionRequest::legacy_request(&canonical).is_err());
        assert!(
            serde_json::from_value::<DecisionInput>(json!([{"role":"system","content":"x"}]))
                .is_err()
        );
    }
    #[test]
    fn invalid_provider_response_is_rejected() {
        let r = request().into_systemone();
        let bad: DecisionResponse = serde_json::from_value(json!({"model":"m","answers":[],"usage":{"input_tokens":0,"output_tokens":0,"total_tokens":0}})).unwrap();
        assert!(bad.into_systemone(&r).is_err());
    }
}
