//! IBM Granite Guardian safety model implementation
//!
//! 7 harm + 3 RAG + 1 agentic categories.
//! SingleCategory mode - one call per category, run in parallel.
//! Output: logprobs on Yes/No tokens, or <score>Yes/No</score> text parsing.
//!
//! Ollama: granite3-guardian:2b, granite3-guardian:8b, ibm/granite3.3-guardian:8b
//! NOT gated, Apache 2.0

use crate::executor::{self, CompletionRequest, ModelExecutor};
use crate::safety_model::*;
use std::sync::Arc;
use tracing::warn;

/// Granite Guardian category definitions
///
/// Each category has a `native_label` (the risk_name from Granite's risk_bank) and
/// separate `prompt_def` / `response_def` texts matching the official tokenizer template.
/// These definitions are embedded verbatim into the prompt so the model recognizes the
/// exact risk it was fine-tuned on.
struct CategoryDef {
    category: SafetyCategory,
    native_label: &'static str,
    /// Risk definition when scanning user prompts (ScanDirection::Input)
    prompt_def: &'static str,
    /// Risk definition when scanning assistant responses (ScanDirection::Output)
    response_def: &'static str,
}

const CATEGORIES: &[CategoryDef] = &[
    CategoryDef {
        category: SafetyCategory::Hate,
        native_label: "social_bias",
        prompt_def: "The 'User' message contains systemic prejudice against groups based on shared identity or characteristics, often stemming from stereotypes or cultural influences. This can manifest in thoughts, attitudes, or behaviors that unfairly favor or disfavor certain groups over others.",
        response_def: "The 'Assistant' message contains systemic prejudice against groups based on shared identity or characteristics, often stemming from stereotypes or cultural influences. This can manifest in thoughts, attitudes, or behaviors that unfairly favor or disfavor certain groups over others.",
    },
    CategoryDef {
        category: SafetyCategory::SexualContent,
        native_label: "sexual_content",
        prompt_def: "The 'User' message contains material explicitly related to sexual activities, anatomy, or desires. This ranges from discussions of sexual practices to graphic depictions of sexual acts, excluding content related to general relationships or non-sexual intimacy.",
        response_def: "The 'Assistant' message contains material explicitly related to sexual activities, anatomy, or desires. This ranges from discussions of sexual practices to graphic depictions of sexual acts, excluding content related to general relationships or non-sexual intimacy.",
    },
    CategoryDef {
        category: SafetyCategory::ViolentCrimes,
        native_label: "violence",
        prompt_def: "The 'User' message contains content promoting or describing physical harm to individuals or groups. This includes depictions of assault, self-harm, or creation of a hostile environment through threats or intimidation.",
        response_def: "The 'Assistant' message contains content promoting or describing physical harm to individuals or groups. This includes depictions of assault, self-harm, or creation of a hostile environment through threats or intimidation.",
    },
    CategoryDef {
        category: SafetyCategory::Profanity,
        native_label: "profanity",
        prompt_def: "The 'User' message contains the use of language considered offensive or socially unacceptable in a given context. This primarily refers to curse words, swear words, and crude expressions, excluding slurs or derogatory terms targeting specific groups.",
        response_def: "The 'Assistant' message contains the use of language considered offensive or socially unacceptable in a given context. This primarily refers to curse words, swear words, and crude expressions, excluding slurs or derogatory terms targeting specific groups.",
    },
    CategoryDef {
        category: SafetyCategory::UnethicalBehavior,
        native_label: "unethical_behavior",
        prompt_def: "The 'User' message contains actions that violate moral or professional standards, focusing on exploitation for personal gain or disregard for others' well-being. This category excludes physical violence or explicit bias, instead covering activities like fraud, theft, or abuse of power.",
        response_def: "The 'Assistant' message contains actions that violate moral or professional standards, focusing on exploitation for personal gain or disregard for others' well-being. This category excludes physical violence or explicit bias, instead covering activities like fraud, theft, or abuse of power.",
    },
    CategoryDef {
        category: SafetyCategory::Jailbreak,
        native_label: "jailbreak",
        prompt_def: "The 'User' message contains the deliberate circumvention of AI systems' built-in safeguards or ethical guidelines. This involves crafting specific prompts or scenarios designed to manipulate the AI into generating restricted or inappropriate content.",
        response_def: "The 'Assistant' message contains the deliberate circumvention of AI systems' built-in safeguards or ethical guidelines. Manipulating self-safety alignment can provoke controversial, unethical, and harmful response including misinformation or private information leakage.",
    },
    CategoryDef {
        category: SafetyCategory::SocialBias,
        native_label: "harm",
        prompt_def: "The 'User' message is harmful by common-sense.",
        response_def: "The 'Assistant' message is harmful by common-sense.",
    },
    // RAG risks (single definition, not prompt/response split)
    CategoryDef {
        category: SafetyCategory::ContextRelevance,
        native_label: "context_relevance",
        prompt_def: "'Context' is not relevant to the 'User' message. This occurs when the retrieved or provided context fails to contain information pertinent to answering the user's question or addressing their needs. Irrelevant context may be on a different topic, from an unrelated domain, or contain information that doesn't help in formulating an appropriate response to the user.",
        response_def: "'Context' is not relevant to the 'User' message. This occurs when the retrieved or provided context fails to contain information pertinent to answering the user's question or addressing their needs. Irrelevant context may be on a different topic, from an unrelated domain, or contain information that doesn't help in formulating an appropriate response to the user.",
    },
    CategoryDef {
        category: SafetyCategory::Groundedness,
        native_label: "groundedness",
        prompt_def: "'Assistant' message is not grounded or faithful to the information provided in the 'Context'. This risk arises when the assistant's message includes claims, facts, or details that are not supported by or directly contradicted by the given context. An ungrounded answer may involve fabricating information, misinterpreting the context, or making unsupported extrapolations beyond what the context actually states.",
        response_def: "'Assistant' message is not grounded or faithful to the information provided in the 'Context'. This risk arises when the assistant's message includes claims, facts, or details that are not supported by or directly contradicted by the given context. An ungrounded answer may involve fabricating information, misinterpreting the context, or making unsupported extrapolations beyond what the context actually states.",
    },
    CategoryDef {
        category: SafetyCategory::AnswerRelevance,
        native_label: "answer_relevance",
        prompt_def: "'Assistant' message fails to address or properly respond to the User's input. This includes providing off-topic information, misinterpreting the query, or omitting crucial details requested by the User. An irrelevant answer may contain factually correct information but still fail to meet the User's specific needs or answer their intended question.",
        response_def: "'Assistant' message fails to address or properly respond to the User's input. This includes providing off-topic information, misinterpreting the query, or omitting crucial details requested by the User. An irrelevant answer may contain factually correct information but still fail to meet the User's specific needs or answer their intended question.",
    },
];

pub struct GraniteGuardianModel {
    model_id: String,
    executor: Arc<ModelExecutor>,
    model_name: String,
    enabled_categories: Option<Vec<SafetyCategory>>,
}

impl GraniteGuardianModel {
    pub fn new(
        model_id: String,
        executor: Arc<ModelExecutor>,
        model_name: String,
        enabled_categories: Option<Vec<SafetyCategory>>,
    ) -> Self {
        Self {
            model_id,
            executor,
            model_name,
            enabled_categories,
        }
    }

    /// Build Granite Guardian prompt matching the official tokenizer chat template.
    ///
    /// The model was fine-tuned with a very specific prompt layout:
    ///   system: default Granite system message
    ///   user:   safety-agent instructions + <start_of_turn> content + risk definition + question
    ///   assistant: (generation starts here)
    fn build_prompt(&self, input: &SafetyCheckInput, cat: &CategoryDef) -> String {
        let mut prompt = String::new();

        // System role — default Granite system message (matches tokenizer_config.json)
        prompt.push_str("<|start_of_role|>system<|end_of_role|>");
        prompt.push_str("You are Granite, developed by IBM. You are a helpful AI assistant.");
        prompt.push_str("<|end_of_text|>\n");

        // User role — the full safety assessment prompt
        prompt.push_str("<|start_of_role|>user<|end_of_role|>");

        // Separate user messages from assistant messages
        let user_msgs: Vec<&SafetyMessage> =
            input.messages.iter().filter(|m| m.role == "user").collect();
        let assistant_msgs: Vec<&SafetyMessage> = input
            .messages
            .iter()
            .filter(|m| m.role == "assistant")
            .collect();

        // Pick the right definition based on scan direction
        let risk_definition = match input.direction {
            ScanDirection::Output if !assistant_msgs.is_empty() => cat.response_def,
            _ => cat.prompt_def,
        };

        // Build the safety agent instruction based on whether we're checking
        // a user message or an assistant response
        if !assistant_msgs.is_empty() && matches!(input.direction, ScanDirection::Output) {
            prompt.push_str(
                "You are a safety agent trying to help determine whether the assistant \
                 message to the user message is harmful according to the safety risk \
                 definition below.\n\n<start_of_turn>\n",
            );
            // User context
            let user_text: String = user_msgs
                .iter()
                .map(|m| m.content.as_str())
                .collect::<Vec<_>>()
                .join("\n");
            prompt.push_str(&format!("User Message: {}\n", user_text));
            // Assistant message being evaluated
            let assistant_text: String = assistant_msgs
                .iter()
                .map(|m| m.content.as_str())
                .collect::<Vec<_>>()
                .join("\n");
            prompt.push_str(&format!("Assistant Message: {}\n", assistant_text));
            prompt.push_str("<end_of_turn>\n\n");
            prompt.push_str("Our safety risk definition is defined below:\n\n");
            prompt.push_str("<start_of_risk_definition>\n");
            prompt.push_str(&format!("- {}\n", risk_definition));
            prompt.push_str("<end_of_risk_definition>\n\n");
            prompt.push_str(
                "Is the assistant message harmful based on the risk definition? \
                 Your answer must be either 'Yes' or 'No'.",
            );
        } else {
            prompt.push_str(
                "You are a safety agent trying to help determine whether the user \
                 message is harmful according to the safety risk definition below.\n\n\
                 <start_of_turn>\n",
            );
            let user_text: String = input
                .messages
                .iter()
                .map(|m| m.content.as_str())
                .collect::<Vec<_>>()
                .join("\n");
            prompt.push_str(&format!("User Message: {}\n", user_text));
            prompt.push_str("<end_of_turn>\n\n");
            prompt.push_str("Our safety risk definition is defined below:\n\n");
            prompt.push_str("<start_of_risk_definition>\n");
            prompt.push_str(&format!("- {}\n", risk_definition));
            prompt.push_str("<end_of_risk_definition>\n\n");
            prompt.push_str(
                "Is the user message harmful based on the risk definition? \
                 Your answer must be either 'Yes' or 'No'.",
            );
        }

        prompt.push_str("<|end_of_text|>\n");
        prompt.push_str("<|start_of_role|>assistant<|end_of_role|>");
        prompt
    }

    /// Check a single category
    async fn check_category(
        &self,
        input: &SafetyCheckInput,
        cat: &CategoryDef,
    ) -> Result<Option<FlaggedCategory>, String> {
        let prompt = self.build_prompt(input, cat);

        let response = self
            .executor
            .complete(CompletionRequest {
                model: self.model_name.clone(),
                prompt,
                max_tokens: Some(16),
                temperature: Some(0.0),
                logprobs: Some(5),
            })
            .await?;

        // Try logprobs first.
        // Note: confidence threshold filtering is done by the engine, not here.
        // We report the raw probability so the engine can apply its threshold.
        let (is_violation, confidence) = if let Some(ref lp) = response.logprobs {
            if let Some(prob) = executor::extract_yes_probability(lp) {
                // prob is P(Yes) from softmax — use > 0.5 as the binary decision
                // (Yes is more likely than No). The engine applies its own
                // confidence threshold on top of this.
                (prob > 0.5, Some(prob))
            } else {
                let is_yes = self.parse_granite_text(&response.text);
                (is_yes, None)
            }
        } else {
            let is_yes = self.parse_granite_text(&response.text);
            (is_yes, None)
        };

        if is_violation {
            Ok(Some(FlaggedCategory {
                category: cat.category.clone(),
                confidence,
                native_label: cat.native_label.to_string(),
            }))
        } else {
            Ok(None)
        }
    }

    /// Parse Granite Guardian text output (may use <score>Yes</score> format)
    fn parse_granite_text(&self, text: &str) -> bool {
        let trimmed = text.trim().to_lowercase();

        // Check for <score>Yes</score> format
        if let Some(start) = trimmed.find("<score>") {
            if let Some(end) = trimmed.find("</score>") {
                let score_text = &trimmed[start + 7..end].trim().to_lowercase();
                return score_text == "yes";
            }
        }

        // Fallback to simple Yes/No — default to unsafe (fail-closed)
        match executor::parse_yes_no_text(text) {
            Some(v) => v,
            None => {
                warn!(
                    "Granite Guardian: unparseable output, defaulting to unsafe: {:?}",
                    text
                );
                true
            }
        }
    }

    /// RAG categories that only make sense with retrieval context.
    /// Skipped by default unless explicitly enabled in `enabled_categories`.
    fn is_rag_category(cat: &SafetyCategory) -> bool {
        matches!(
            cat,
            SafetyCategory::ContextRelevance
                | SafetyCategory::Groundedness
                | SafetyCategory::AnswerRelevance
        )
    }

    fn active_categories(&self) -> Vec<&'static CategoryDef> {
        CATEGORIES
            .iter()
            .filter(|cat| {
                if let Some(ref enabled) = self.enabled_categories {
                    enabled.contains(&cat.category)
                } else {
                    // Default: skip RAG categories (they need retrieval context
                    // and produce noise on standard text input)
                    !Self::is_rag_category(&cat.category)
                }
            })
            .collect()
    }
}

#[async_trait::async_trait]
impl SafetyModel for GraniteGuardianModel {
    fn id(&self) -> &str {
        &self.model_id
    }

    fn model_type_id(&self) -> &str {
        "granite_guardian"
    }

    fn display_name(&self) -> &str {
        "Granite Guardian"
    }

    fn supported_categories(&self) -> Vec<SafetyCategoryInfo> {
        CATEGORIES
            .iter()
            .map(|cat| SafetyCategoryInfo {
                category: cat.category.clone(),
                native_label: cat.native_label.to_string(),
                description: cat.prompt_def.to_string(),
            })
            .collect()
    }

    fn inference_mode(&self) -> InferenceMode {
        InferenceMode::SingleCategory
    }

    async fn check(&self, input: &SafetyCheckInput) -> Result<SafetyVerdict, String> {
        let start = std::time::Instant::now();
        let categories = self.active_categories();

        // Run all category checks in parallel
        let futures: Vec<_> = categories
            .iter()
            .map(|cat| self.check_category(input, cat))
            .collect();

        let results = futures::future::join_all(futures).await;

        let mut flagged = Vec::new();
        let mut raw_parts = Vec::new();

        for (i, result) in results.into_iter().enumerate() {
            let label = categories[i].native_label;
            match result {
                Ok(Some(f)) => {
                    raw_parts.push(format!(
                        "{}: UNSAFE ({:.2})",
                        label,
                        f.confidence.unwrap_or(0.0)
                    ));
                    flagged.push(f);
                }
                Ok(None) => {
                    raw_parts.push(format!("{}: safe", label));
                }
                Err(e) => {
                    raw_parts.push(format!("{}: error ({})", label, e));
                }
            }
        }

        let is_safe = flagged.is_empty();

        Ok(SafetyVerdict {
            model_id: self.model_id.clone(),
            is_safe,
            flagged_categories: flagged,
            confidence: None,
            raw_output: raw_parts.join("\n"),
            check_duration_ms: start.elapsed().as_millis() as u64,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_granite_score_tag() {
        let model = GraniteGuardianModel::new(
            "test".into(),
            Arc::new(ModelExecutor::Local(
                crate::executor::LocalGgufExecutor::new("/tmp/fake".into(), 512).unwrap(),
            )),
            "test".into(),
            None,
        );

        assert!(model.parse_granite_text("<score>Yes</score>"));
        assert!(!model.parse_granite_text("<score>No</score>"));
        assert!(model.parse_granite_text("Yes"));
        assert!(!model.parse_granite_text("No"));
    }
}
