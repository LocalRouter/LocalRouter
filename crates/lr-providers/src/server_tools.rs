//! Provider-executed retrieval tools, separate from client function definitions.
use lr_types::{AppError, AppResult};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;

/// Transitional private ingress envelope; never accepted directly from clients.
pub const INGRESS_EXTENSION: &str = "_localrouter_server_tools";
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServerToolsProtocol {
    OpenRouter,
    LlmGateway,
    OpenaiResponses,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, ToSchema)]
#[serde(default, deny_unknown_fields)]
pub struct SearchParameters {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub engine: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_results: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_total_results: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_uses: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_characters: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub search_context_size: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed_domains: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub excluded_domains: Option<Vec<String>>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, ToSchema)]
#[serde(default, deny_unknown_fields)]
pub struct FetchParameters {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub engine: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_uses: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_content_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed_domains: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocked_domains: Option<Vec<String>>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, ToSchema)]
#[serde(default, deny_unknown_fields)]
pub struct SearchFilters {
    pub allowed_domains: Option<Vec<String>>,
}
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(tag = "type", deny_unknown_fields)]
pub enum ServerToolDefinition {
    #[serde(rename = "openrouter:web_search")]
    OpenRouterWebSearch {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parameters: Option<SearchParameters>,
    },
    #[serde(rename = "openrouter:web_fetch")]
    OpenRouterWebFetch {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parameters: Option<FetchParameters>,
    },
    #[serde(rename = "web_search")]
    WebSearch {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        filters: Option<SearchFilters>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        search_context_size: Option<String>,
    },
}
fn invalid(message: &str) -> AppError {
    AppError::InvalidParams(message.into())
}
fn count(value: Option<u32>, max: u32) -> bool {
    value.is_none_or(|value| (1..=max).contains(&value))
}
fn domains(value: &Option<Vec<String>>) -> AppResult<()> {
    if value.as_ref().is_some_and(|domains| {
        domains.is_empty()
            || domains.len() > 20
            || domains.iter().any(|domain| {
                domain.is_empty()
                    || domain.len() > 253
                    || !domain
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-'))
                    || domain.split('.').any(|label| {
                        label.is_empty()
                            || label.len() > 63
                            || label.starts_with('-')
                            || label.ends_with('-')
                    })
            })
    }) {
        return Err(invalid(
            "Tool domain filters require 1–20 bare host names up to 253 bytes",
        ));
    }
    Ok(())
}
fn context_size(value: &Option<String>) -> bool {
    value
        .as_deref()
        .is_none_or(|value| matches!(value, "low" | "medium" | "high"))
}
impl ServerToolDefinition {
    pub fn family(&self) -> lr_config::UpstreamToolKind {
        match self {
            Self::OpenRouterWebFetch { .. } => lr_config::UpstreamToolKind::WebFetch,
            _ => lr_config::UpstreamToolKind::WebSearch,
        }
    }
    pub fn wire_type(&self) -> &'static str {
        match self {
            Self::OpenRouterWebSearch { .. } => "openrouter:web_search",
            Self::OpenRouterWebFetch { .. } => "openrouter:web_fetch",
            Self::WebSearch { .. } => "web_search",
        }
    }
    pub fn validate(&self) -> AppResult<()> {
        match self {
            Self::OpenRouterWebSearch {
                parameters: Some(parameters),
            } => {
                let engine = parameters.engine.as_deref().unwrap_or("auto");
                if !matches!(
                    engine,
                    "auto" | "native" | "exa" | "parallel" | "perplexity" | "firecrawl"
                ) || !count(
                    parameters.max_results,
                    if engine == "perplexity" { 20 } else { 25 },
                ) || !count(parameters.max_total_results, 1000)
                    || !count(parameters.max_uses, 30)
                    || !count(parameters.max_characters, 100_000)
                    || !context_size(&parameters.search_context_size)
                {
                    return Err(invalid("Invalid search tool engine or bounded options"));
                }
                domains(&parameters.allowed_domains)?;
                domains(&parameters.excluded_domains)?;
                if matches!(engine, "auto" | "native")
                    && (parameters.max_results.is_some()
                        || parameters.max_uses.is_some()
                        || parameters.max_characters.is_some()
                        || parameters.search_context_size.is_some()
                        || parameters.allowed_domains.is_some()
                        || parameters.excluded_domains.is_some()
                        || parameters.mode.is_some())
                {
                    return Err(invalid("Strict search count/domain/content options require a verified non-native engine; native providers may ignore them"));
                }
                if engine == "firecrawl"
                    && (parameters.max_characters.is_some()
                        || parameters.search_context_size.is_some())
                {
                    return Err(invalid(
                        "Firecrawl ignores search content-size options; choose a verified engine",
                    ));
                }
                if parameters.allowed_domains.is_some()
                    && parameters.excluded_domains.is_some()
                    && engine != "exa"
                {
                    return Err(invalid(
                        "Both domain lists are supported together only by the Exa search engine",
                    ));
                }
                if parameters.mode.as_deref().is_some_and(|mode| match engine {
                    "exa" => !matches!(
                        mode,
                        "instant" | "fast" | "auto" | "deep-lite" | "deep" | "deep-reasoning"
                    ),
                    "parallel" => !matches!(mode, "turbo" | "fast" | "basic" | "advanced"),
                    _ => true,
                }) {
                    return Err(invalid("Unsupported engine-specific search mode"));
                }
            }
            Self::OpenRouterWebFetch {
                parameters: Some(parameters),
            } => {
                let engine = parameters.engine.as_deref().unwrap_or("auto");
                if !matches!(
                    engine,
                    "auto" | "native" | "exa" | "openrouter" | "parallel" | "firecrawl"
                ) || !count(parameters.max_uses, 30)
                    || !count(parameters.max_content_tokens, 1_000_000)
                {
                    return Err(invalid("Invalid fetch tool engine or bounded options"));
                }
                domains(&parameters.allowed_domains)?;
                domains(&parameters.blocked_domains)?;
                if engine != "exa"
                    && (parameters.max_uses.is_some()
                        || parameters.max_content_tokens.is_some()
                        || parameters.allowed_domains.is_some()
                        || parameters.blocked_domains.is_some())
                {
                    return Err(invalid("Strict fetch limits and filters currently require the verified Exa engine mapping"));
                }
            }
            Self::WebSearch {
                filters,
                search_context_size,
            } => {
                if let Some(filters) = filters {
                    domains(&filters.allowed_domains)?;
                }
                if !context_size(search_context_size) {
                    return Err(invalid("Invalid native search context size"));
                }
            }
            _ => {}
        }
        Ok(())
    }
    pub fn validate_protocol(&self, protocol: ServerToolsProtocol) -> AppResult<()> {
        self.validate()?;
        match (self,protocol) {
            (Self::OpenRouterWebSearch {..}|Self::OpenRouterWebFetch {..},ServerToolsProtocol::OpenRouter)=>Ok(()),
            (Self::WebSearch {filters:None,search_context_size:None},ServerToolsProtocol::LlmGateway)=>Ok(()),
            (Self::WebSearch {..},ServerToolsProtocol::OpenaiResponses)=>Ok(()),
            _=>Err(invalid("This provider/endpoint has no verified mapping for the requested server tool or its options")),
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PositionedServerTool {
    pub position: usize,
    pub definition: ServerToolDefinition,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServerToolsContext {
    pub tools: Vec<PositionedServerTool>,
    pub original_tool_count: usize,
    pub max_tool_calls: Option<u32>,
    pub forced_type: Option<String>,
    /// Derived from current client authorization, never accepted from the wire.
    #[serde(skip)]
    pub policy_max_tool_calls: Option<u32>,
}
impl ServerToolsContext {
    /// Validate before classifiers or upstream IO; unknown tool fees cannot
    /// satisfy an existing cost ceiling or a free-only strategy.
    pub fn authorize(
        &mut self,
        policy: &lr_config::UpstreamToolsPolicy,
        privacy: &lr_config::ClientPrivacyConfig,
        has_cost_ceiling: bool,
        free_only: bool,
    ) -> AppResult<()> {
        self.validate()?;
        policy.validate().map_err(AppError::InvalidParams)?;
        if privacy.restricted() {
            return Err(invalid(
                "Provider-executed tools have no verified mapping for strict privacy profiles",
            ));
        }
        if self
            .tools
            .iter()
            .any(|tool| !policy.allowed_tools.contains(&tool.definition.family()))
        {
            return Err(invalid(
                "Provider-executed tool is not enabled for this client",
            ));
        }
        if has_cost_ceiling || free_only {
            return Err(invalid("Provider-executed tools have unknown maximum fees and cannot satisfy cost-limited or free-only routing"));
        }
        self.policy_max_tool_calls = policy.max_tool_calls;
        Ok(())
    }
    pub fn validate(&self) -> AppResult<()> {
        if self.tools.is_empty()
            || self.tools.len() > 8
            || !(1..=128).contains(&self.original_tool_count)
            || !count(self.max_tool_calls, 30)
        {
            return Err(invalid("Invalid bounded server tool request"));
        }
        let mut positions = std::collections::HashSet::new();
        let mut kinds = std::collections::HashSet::new();
        for tool in &self.tools {
            if tool.position >= self.original_tool_count
                || !positions.insert(tool.position)
                || !kinds.insert(tool.definition.wire_type())
            {
                return Err(invalid("Duplicate or invalid server tool definitions"));
            }
            tool.definition.validate()?;
        }
        if self
            .forced_type
            .as_deref()
            .is_some_and(|forced| forced != "web_search" || !kinds.contains(forced))
        {
            return Err(invalid("Unsupported forced server tool choice"));
        }
        Ok(())
    }
    pub fn wire_body(
        &self,
        protocol: ServerToolsProtocol,
        body: &mut Value,
        effective_cap: Option<u32>,
    ) -> AppResult<()> {
        self.validate()?;
        let effective_cap = match (effective_cap, self.policy_max_tool_calls) {
            (Some(explicit), Some(policy)) => Some(explicit.min(policy)),
            (explicit, policy) => explicit.or(policy),
        };
        let effective_cap = match (self.max_tool_calls, effective_cap) {
            (Some(request), Some(policy)) => Some(request.min(policy)),
            (request, policy) => request.or(policy),
        };
        if !count(effective_cap, 30) {
            return Err(invalid("Invalid effective server-tool step cap"));
        }
        for tool in &self.tools {
            tool.definition.validate_protocol(protocol)?;
        }
        if protocol == ServerToolsProtocol::LlmGateway && effective_cap.is_some() {
            return Err(invalid("This gateway has no verified global server-tool step-cap mapping; choose provider-managed limits explicitly"));
        }
        let mut tools = body
            .get("tools")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if tools.len() + self.tools.len() != self.original_tool_count {
            return Err(invalid(
                "Function/server tool projection changed after validation",
            ));
        }
        let mut native = self.tools.iter().collect::<Vec<_>>();
        native.sort_by_key(|tool| tool.position);
        for tool in native {
            tools.insert(
                tool.position,
                serde_json::to_value(&tool.definition)
                    .map_err(|_| invalid("Invalid server tool"))?,
            );
        }
        body["tools"] = Value::Array(tools);
        if let Some(cap) = effective_cap {
            body["max_tool_calls"] = Value::from(cap);
        }
        if let Some(kind) = &self.forced_type {
            if protocol != ServerToolsProtocol::LlmGateway
                && protocol != ServerToolsProtocol::OpenaiResponses
            {
                return Err(invalid(
                    "Forced named server tool choice is not verified for this gateway",
                ));
            }
            body["tool_choice"] = serde_json::json!({"type":kind});
        }
        Ok(())
    }
}

/// Responses uses flat URL citations; OpenRouter chat can nest the citation.
/// URLs stay untrusted data: no fetch, HTML interpretation or asset loading.
pub fn response_annotations(annotations: &[Value]) -> Vec<Value> {
    annotations
        .iter()
        .take(128)
        .filter_map(|annotation| {
            if annotation.get("type").and_then(Value::as_str) != Some("url_citation") {
                return None;
            }
            let citation = annotation.get("url_citation").unwrap_or(annotation);
            let url = citation.get("url")?.as_str()?;
            if !(url.starts_with("https://") || url.starts_with("http://")) || url.len() > 8192 {
                return None;
            }
            let mut output = serde_json::json!({"type":"url_citation","url":url});
            for field in ["title", "start_index", "end_index"] {
                if let Some(value) = citation.get(field) {
                    output[field] = value.clone();
                }
            }
            Some(output)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn request() -> ServerToolsContext {
        ServerToolsContext {
            tools: vec![PositionedServerTool {
                position: 1,
                definition: ServerToolDefinition::OpenRouterWebSearch { parameters: None },
            }],
            original_tool_count: 2,
            max_tool_calls: Some(2),
            forced_type: None,
            policy_max_tool_calls: None,
        }
    }
    #[test]
    fn responses_citations_normalize_shapes_without_fetching_or_inventing_sources() {
        let annotations = vec![
            json!({"type":"url_citation","url_citation":{"url":"https://example.com","title":"Source","start_index":0,"end_index":5}}),
            json!({"type":"url_citation","url":"http://example.org","title":"Other"}),
            json!({"type":"url_citation","url":"javascript:alert(1)"}),
            json!({"type":"unknown","url":"https://example.com"}),
        ];
        let normalized = response_annotations(&annotations);
        assert_eq!(normalized.len(), 2);
        assert_eq!(normalized[0]["url"], "https://example.com");
        assert_eq!(normalized[0]["start_index"], 0);
        assert!(normalized[1].get("start_index").is_none());
    }
    #[test]
    fn authorization_is_explicit_current_and_never_assumes_unknown_fees_are_free() {
        let mut request = request();
        let privacy = lr_config::ClientPrivacyConfig::default();
        let mut policy = lr_config::UpstreamToolsPolicy::default();
        assert!(request.authorize(&policy, &privacy, false, false).is_err());
        policy
            .allowed_tools
            .push(lr_config::UpstreamToolKind::WebSearch);
        policy.max_tool_calls = Some(1);
        request.authorize(&policy, &privacy, false, false).unwrap();
        let function = json!({"type":"function","function":{"name":"f"}});
        let mut body = json!({"tools":[function.clone()]});
        request
            .wire_body(ServerToolsProtocol::OpenRouter, &mut body, Some(20))
            .unwrap();
        assert_eq!(body["max_tool_calls"], 1);
        for (cost, free) in [(true, false), (false, true)] {
            assert!(request.authorize(&policy, &privacy, cost, free).is_err());
        }
        for privacy in [
            lr_config::ClientPrivacyConfig {
                local_only: true,
                ..Default::default()
            },
            lr_config::ClientPrivacyConfig {
                no_retention: true,
                ..Default::default()
            },
        ] {
            assert!(request.authorize(&policy, &privacy, false, false).is_err());
        }
        // Derived caps never round-trip through an ingress or capture envelope.
        let decoded: ServerToolsContext =
            serde_json::from_value(serde_json::to_value(&request).unwrap()).unwrap();
        assert_eq!(decoded.policy_max_tool_calls, None);
        policy.allowed_tools.clear();
        assert!(request
            .authorize(
                &policy,
                &lr_config::ClientPrivacyConfig::default(),
                false,
                false
            )
            .is_err());
    }
    #[test]
    fn client_functions_keep_their_order_and_limits_only_narrow() {
        let request = request();
        let function = json!({"type":"function","function":{"name":"client_owned","parameters":{"type":"object"}}});
        let mut body = json!({"tools":[function.clone()]});
        request
            .wire_body(ServerToolsProtocol::OpenRouter, &mut body, Some(8))
            .unwrap();
        assert_eq!(body["max_tool_calls"], 2);
        assert_eq!(body["tools"][0], function);
        assert_eq!(body["tools"][1], json!({"type":"openrouter:web_search"}));
        assert!(body["tools"][1].get("function").is_none());
        assert!(request
            .wire_body(
                ServerToolsProtocol::LlmGateway,
                &mut json!({"tools":[function]}),
                None
            )
            .is_err());
    }
    #[test]
    fn native_mapping_does_not_claim_unsupported_limits_or_filters() {
        let mut request = ServerToolsContext {
            tools: vec![PositionedServerTool {
                position: 0,
                definition: ServerToolDefinition::WebSearch {
                    filters: None,
                    search_context_size: None,
                },
            }],
            original_tool_count: 1,
            max_tool_calls: None,
            forced_type: Some("web_search".into()),
            policy_max_tool_calls: None,
        };
        let mut body = json!({});
        request
            .wire_body(ServerToolsProtocol::LlmGateway, &mut body, None)
            .unwrap();
        assert_eq!(body["tool_choice"], json!({"type":"web_search"}));
        assert!(request
            .wire_body(ServerToolsProtocol::LlmGateway, &mut json!({}), Some(8))
            .is_err());
        request.tools[0].definition = ServerToolDefinition::WebSearch {
            filters: Some(SearchFilters {
                allowed_domains: Some(vec!["example.com".into()]),
            }),
            search_context_size: None,
        };
        assert!(request
            .wire_body(ServerToolsProtocol::LlmGateway, &mut json!({}), None)
            .is_err());
        assert!(request
            .wire_body(
                ServerToolsProtocol::OpenaiResponses,
                &mut json!({}),
                Some(8)
            )
            .is_ok());
    }
    #[test]
    fn ignored_constraints_unknown_fields_and_invalid_projections_fail_closed() {
        let native:ServerToolDefinition=serde_json::from_value(json!({"type":"openrouter:web_search","parameters":{"engine":"native","allowed_domains":["example.com"]}})).unwrap();
        assert!(native.validate().is_err());
        let exa:ServerToolDefinition=serde_json::from_value(json!({"type":"openrouter:web_search","parameters":{"engine":"exa","allowed_domains":["example.com"],"excluded_domains":["other.example.com"],"max_uses":3}})).unwrap();
        exa.validate().unwrap();
        assert!(serde_json::from_value::<ServerToolDefinition>(
            json!({"type":"openrouter:web_fetch","parameters":{"unverified_option":true}})
        )
        .is_err());
        assert!(
            serde_json::from_value::<ServerToolDefinition>(json!({"type":"openrouter:shell"}))
                .is_err()
        );
        let mut request = request();
        request.tools[0].position = 2;
        assert!(request.validate().is_err());
        request.tools[0].position = 1;
        assert!(request
            .wire_body(ServerToolsProtocol::OpenRouter, &mut json!({}), Some(8))
            .is_err());
    }
}
