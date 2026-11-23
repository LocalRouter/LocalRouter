//! `n > 1` for providers that return fewer choices than asked for.
//!
//! The request's `n` is passed to the provider. Providers whose API takes
//! `n` answer with that many choices; the others drop it and answer with
//! one. The router then asks for the missing choices one at a time and
//! merges them into the first response, so every provider answers `n`
//! choices. Which providers honour `n` is read from the choices they
//! return, not from a list.

use lr_providers::{CompletionRequest, CompletionResponse, TokenUsage};

/// How many more choices `response` needs to answer `requested`.
pub(crate) fn missing(requested: Option<u32>, response: &CompletionResponse) -> usize {
    let requested = requested.unwrap_or(1) as usize;
    requested.saturating_sub(response.choices.len())
}

/// The request sent for each missing choice: the same request for one
/// choice.
pub(crate) fn single_choice_request(request: &CompletionRequest) -> CompletionRequest {
    let mut single = request.clone();
    single.n = None;
    single
}

/// Append `extra`'s choices to `into` with the next indexes, and add its
/// usage (and its gateway-reported cost) to `into`'s.
pub(crate) fn merge(into: &mut CompletionResponse, extra: CompletionResponse) {
    let into_cost = lr_providers::gateway::reported_cost(into.extensions.as_ref());
    let extra_cost = lr_providers::gateway::reported_cost(extra.extensions.as_ref());
    for mut choice in extra.choices {
        choice.index = into.choices.len() as u32;
        into.choices.push(choice);
    }
    add_usage(&mut into.usage, &extra.usage);
    set_reported_cost(into, into_cost.zip(extra_cost).map(|(a, b)| a + b));
}

/// Keep a gateway-reported cost only while every merged response reported
/// one; otherwise the cost is computed from the summed usage instead.
fn set_reported_cost(response: &mut CompletionResponse, cost: Option<f64>) {
    let Some(gateway) = response
        .extensions
        .as_mut()
        .and_then(|extensions| extensions.get_mut(lr_providers::gateway::EXTENSION))
        .and_then(serde_json::Value::as_object_mut)
    else {
        return;
    };
    match cost {
        Some(cost) => {
            gateway.insert("cost_usd".into(), serde_json::json!(cost));
        }
        None => {
            gateway.remove("cost_usd");
        }
    }
}

fn add(a: Option<u32>, b: Option<u32>) -> Option<u32> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.saturating_add(b)),
        (a, None) => a,
        (None, b) => b,
    }
}

fn add_usage(into: &mut TokenUsage, extra: &TokenUsage) {
    into.prompt_tokens = into.prompt_tokens.saturating_add(extra.prompt_tokens);
    into.completion_tokens = into
        .completion_tokens
        .saturating_add(extra.completion_tokens);
    into.total_tokens = into.total_tokens.saturating_add(extra.total_tokens);
    match (
        &mut into.prompt_tokens_details,
        &extra.prompt_tokens_details,
    ) {
        (Some(a), Some(b)) => {
            a.anthropic_cache_creation_1h_tokens = add(
                a.anthropic_cache_creation_1h_tokens,
                b.anthropic_cache_creation_1h_tokens,
            );
            a.cached_tokens = add(a.cached_tokens, b.cached_tokens);
            a.cache_creation_tokens = add(a.cache_creation_tokens, b.cache_creation_tokens);
            a.cache_read_tokens = add(a.cache_read_tokens, b.cache_read_tokens);
        }
        (None, Some(b)) => into.prompt_tokens_details = Some(b.clone()),
        _ => {}
    }
    match (
        &mut into.completion_tokens_details,
        &extra.completion_tokens_details,
    ) {
        (Some(a), Some(b)) => {
            a.reasoning_tokens = add(a.reasoning_tokens, b.reasoning_tokens);
            a.thinking_tokens = add(a.thinking_tokens, b.thinking_tokens);
            a.audio_tokens = add(a.audio_tokens, b.audio_tokens);
        }
        (None, Some(b)) => into.completion_tokens_details = Some(b.clone()),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lr_providers::{
        ChatMessage, ChatMessageContent, CompletionChoice, CompletionTokensDetails,
        PromptTokensDetails,
    };
    use serde_json::json;

    fn response(texts: &[&str], prompt: u32, completion: u32) -> CompletionResponse {
        CompletionResponse {
            id: "r".into(),
            object: "chat.completion".into(),
            created: 0,
            model: "m".into(),
            provider: "p".into(),
            choices: texts
                .iter()
                .enumerate()
                .map(|(i, text)| CompletionChoice {
                    index: i as u32,
                    message: ChatMessage {
                        role: "assistant".into(),
                        content: ChatMessageContent::Text((*text).into()),
                        tool_calls: None,
                        tool_call_id: None,
                        name: None,
                        reasoning_content: None,
                        annotations: None,
                    },
                    finish_reason: Some("stop".into()),
                    logprobs: None,
                })
                .collect(),
            usage: TokenUsage {
                prompt_tokens: prompt,
                completion_tokens: completion,
                total_tokens: prompt + completion,
                prompt_tokens_details: None,
                completion_tokens_details: None,
            },
            system_fingerprint: None,
            service_tier: None,
            extensions: None,
            routellm_win_rate: None,
            request_usage_entries: None,
        }
    }

    fn with_cost(mut response: CompletionResponse, cost: f64) -> CompletionResponse {
        response.extensions = Some(
            [(
                lr_providers::gateway::EXTENSION.to_string(),
                json!({ "cost_usd": cost, "provider": "x" }),
            )]
            .into(),
        );
        response
    }

    #[test]
    fn missing_counts_what_the_provider_did_not_return() {
        assert_eq!(missing(None, &response(&["a"], 1, 1)), 0);
        assert_eq!(missing(Some(1), &response(&["a"], 1, 1)), 0);
        assert_eq!(missing(Some(3), &response(&["a"], 1, 1)), 2);
        // A provider that honours n needs nothing more.
        assert_eq!(missing(Some(2), &response(&["a", "b"], 1, 1)), 0);
    }

    #[test]
    fn merged_choices_get_consecutive_indexes_and_summed_usage() {
        let mut first = response(&["a"], 10, 5);
        first.usage.prompt_tokens_details = Some(PromptTokensDetails {
            anthropic_cache_creation_1h_tokens: None,
            cached_tokens: Some(4),
            cache_creation_tokens: None,
            cache_read_tokens: None,
        });
        let mut second = response(&["b"], 10, 7);
        second.usage.prompt_tokens_details = Some(PromptTokensDetails {
            anthropic_cache_creation_1h_tokens: None,
            cached_tokens: Some(6),
            cache_creation_tokens: Some(1),
            cache_read_tokens: None,
        });
        second.usage.completion_tokens_details = Some(CompletionTokensDetails {
            reasoning_tokens: Some(3),
            thinking_tokens: None,
            audio_tokens: None,
        });

        merge(&mut first, second);
        merge(&mut first, response(&["c"], 10, 1));

        let indexes: Vec<u32> = first.choices.iter().map(|c| c.index).collect();
        assert_eq!(indexes, vec![0, 1, 2]);
        let texts: Vec<String> = first
            .choices
            .iter()
            .map(|c| c.message.content.as_text())
            .collect();
        assert_eq!(texts, vec!["a", "b", "c"]);
        assert_eq!(first.usage.prompt_tokens, 30);
        assert_eq!(first.usage.completion_tokens, 13);
        assert_eq!(first.usage.total_tokens, 43);
        let prompt = first.usage.prompt_tokens_details.unwrap();
        assert_eq!(prompt.cached_tokens, Some(10));
        assert_eq!(prompt.cache_creation_tokens, Some(1));
        assert_eq!(
            first
                .usage
                .completion_tokens_details
                .unwrap()
                .reasoning_tokens,
            Some(3)
        );
    }

    #[test]
    fn reported_costs_add_up_only_when_every_response_has_one() {
        let mut first = with_cost(response(&["a"], 1, 1), 0.25);
        merge(&mut first, with_cost(response(&["b"], 1, 1), 0.5));
        assert_eq!(
            lr_providers::gateway::reported_cost(first.extensions.as_ref()),
            Some(0.75)
        );

        // One response without a reported cost: the total is unknown, so
        // it is computed from the summed usage instead of under-reported.
        merge(&mut first, response(&["c"], 1, 1));
        assert_eq!(
            lr_providers::gateway::reported_cost(first.extensions.as_ref()),
            None
        );
        // The rest of the gateway extension is kept.
        assert_eq!(
            first.extensions.unwrap()[lr_providers::gateway::EXTENSION]["provider"],
            "x"
        );
    }

    #[test]
    fn the_single_choice_request_drops_n_only() {
        let request: CompletionRequest = serde_json::from_value(json!({
            "model": "m",
            "messages": [{"role": "user", "content": "hi"}],
            "n": 3,
            "temperature": 0.5,
        }))
        .unwrap();
        let single = single_choice_request(&request);
        assert_eq!(single.n, None);
        assert_eq!(single.temperature, Some(0.5));
        assert_eq!(single.messages.len(), 1);
    }
}
