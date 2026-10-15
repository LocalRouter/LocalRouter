//! Ranked retrieval is a request-based API, separate from generated answers.
use lr_types::{AppError, AppResult};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchProtocol {
    Perplexity,
    LlmGateway,
}
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(untagged)]
pub enum SearchQuery {
    Text(String),
    Multiple(Vec<String>),
}
impl SearchQuery {
    pub fn queries(&self) -> Vec<&str> {
        match self {
            Self::Text(query) => vec![query],
            Self::Multiple(queries) => queries.iter().map(String::as_str).collect(),
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SearchType {
    #[default]
    Web,
    Fast,
    People,
}
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SearchRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub query: SearchQuery,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search_type: Option<SearchType>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_results: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens_per_page: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search_context_size: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub country: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search_language_filter: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search_domain_filter: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search_recency_filter: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search_after_date_filter: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search_before_date_filter: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_updated_after_filter: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_updated_before_filter: Option<String>,
}
fn invalid(message: &str) -> AppError {
    AppError::InvalidParams(message.into())
}
impl SearchRequest {
    pub fn validate(&self) -> AppResult<()> {
        let queries = self.query.queries();
        if queries.is_empty()
            || queries.len() > 5
            || queries
                .iter()
                .any(|query| query.trim().is_empty() || query.len() > 4096)
        {
            return Err(invalid(
                "Search requires 1–5 nonempty queries, each at most 4096 bytes",
            ));
        }
        if self
            .max_results
            .is_some_and(|count| !(1..=50).contains(&count))
            || [self.max_tokens, self.max_tokens_per_page]
                .iter()
                .any(|count| count.is_some_and(|count| !(1..=1_000_000).contains(&count)))
            || self
                .search_context_size
                .as_deref()
                .is_some_and(|size| !matches!(size, "low" | "medium" | "high"))
            || self
                .search_recency_filter
                .as_deref()
                .is_some_and(|recency| {
                    !matches!(recency, "hour" | "day" | "week" | "month" | "year")
                })
        {
            return Err(invalid("Invalid search result, content or recency bounds"));
        }
        let code =
            |code: &str| code.len() == 2 && code.bytes().all(|byte| byte.is_ascii_alphabetic());
        if self
            .country
            .as_deref()
            .is_some_and(|country| !code(country))
            || self
                .search_language_filter
                .as_ref()
                .is_some_and(|languages| {
                    languages.is_empty()
                        || languages.len() > 20
                        || languages.iter().any(|language| !code(language))
                })
        {
            return Err(invalid(
                "Search country/language filters require two-letter codes (at most 20 languages)",
            ));
        }
        if let Some(domains) = &self.search_domain_filter {
            if domains.is_empty()
                || domains.len() > 20
                || domains.iter().any(|domain| {
                    if domain.len() > 253 {
                        return true;
                    }
                    let domain = domain.strip_prefix('-').unwrap_or(domain);
                    domain.len() > 253
                        || domain.split('.').any(|label| {
                            label.is_empty()
                                || label.len() > 63
                                || label.starts_with('-')
                                || label.ends_with('-')
                                || !label
                                    .bytes()
                                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
                        })
                })
                || domains.iter().any(|domain| domain.starts_with('-'))
                    && domains.iter().any(|domain| !domain.starts_with('-'))
            {
                return Err(invalid("Search domain filters require 1–20 bare hosts, either all included or all excluded"));
            }
        }
        for (after, before) in [
            (
                &self.search_after_date_filter,
                &self.search_before_date_filter,
            ),
            (
                &self.last_updated_after_filter,
                &self.last_updated_before_filter,
            ),
        ] {
            let parse = |date: &Option<String>| -> AppResult<Option<chrono::NaiveDate>> {
                date.as_ref()
                    .map(|date| {
                        let parts = date.split('/').collect::<Vec<_>>();
                        if date.len() > 10
                            || parts.len() != 3
                            || parts[2].len() != 4
                            || parts.iter().any(|part| {
                                part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit())
                            })
                        {
                            return Err(invalid("Search dates must use MM/DD/YYYY"));
                        }
                        chrono::NaiveDate::parse_from_str(date, "%m/%d/%Y")
                            .map_err(|_| invalid("Search dates must be valid MM/DD/YYYY dates"))
                    })
                    .transpose()
            };
            if parse(after)?
                .zip(parse(before)?)
                .is_some_and(|(after, before)| after > before)
            {
                return Err(invalid("Search date ranges are reversed"));
            }
        }
        Ok(())
    }
    pub fn wire_body(&self, protocol: SearchProtocol) -> AppResult<serde_json::Value> {
        self.validate()?;
        let model = self
            .model
            .as_deref()
            .ok_or_else(|| invalid("Search requires a resolved provider/model"))?;
        let kind = self.search_type.unwrap_or_else(|| {
            if protocol == SearchProtocol::LlmGateway
                && model == "perplexity/perplexity-search-fast"
            {
                SearchType::Fast
            } else {
                SearchType::Web
            }
        });
        match protocol {
            SearchProtocol::Perplexity => {
                // A local capability alias, never an invented upstream model.
                if self.search_context_size.is_some() {
                    return Err(invalid("Direct Perplexity search has no verified context-size mapping; use content token bounds"));
                }
                if model != "search" {
                    return Err(invalid(
                        "Direct Perplexity ranked search uses the local provider/search route",
                    ));
                }
                if kind != SearchType::People && self.max_results.is_some_and(|count| count > 20) {
                    return Err(invalid(
                        "Direct Perplexity web/fast search supports at most 20 results",
                    ));
                }
            }
            SearchProtocol::LlmGateway => {
                if kind == SearchType::People {
                    return Err(invalid("LLM Gateway does not support people search"));
                }
                let expected = if kind == SearchType::Fast {
                    "perplexity/perplexity-search-fast"
                } else {
                    "perplexity/perplexity-search"
                };
                if model != expected {
                    return Err(invalid(
                        "Search model must match the explicit web/fast search type",
                    ));
                }
            }
        }
        let mut body = serde_json::to_value(self)?;
        if protocol == SearchProtocol::Perplexity {
            body.as_object_mut().unwrap().remove("model");
        }
        Ok(body)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct SearchResult {
    pub title: String,
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snippet: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub date: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_updated: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct SearchResponse {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub results: Vec<SearchResult>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_time: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extensions: Option<std::collections::HashMap<String, serde_json::Value>>,
}
/// Bounded reads prevent an upstream extraction from becoming an unbounded local
/// allocation. A rejected response may still have incurred a provider charge.
pub async fn read_response(
    response: reqwest::Response,
    max_results: u32,
) -> AppResult<SearchResponse> {
    use futures::StreamExt;
    let status = response.status();
    let delay = crate::http_client::retry_after(response.headers(), std::time::SystemTime::now());
    let max_bytes = if status.is_success() {
        4 * 1024 * 1024
    } else {
        64 * 1024
    };
    let oversized = || {
        AppError::Provider(if status.is_success() {
            "Search response exceeds 4 MiB".into()
        } else {
            "Search error response exceeds 64 KiB".into()
        })
    };
    if response
        .content_length()
        .is_some_and(|length| length > max_bytes as u64)
    {
        return Err(oversized());
    }
    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| {
            AppError::Provider("Search response read failed; dispatch is not retried".into())
        })?;
        if bytes.len() + chunk.len() > max_bytes {
            return Err(oversized());
        }
        bytes.extend_from_slice(&chunk);
    }
    if !status.is_success() {
        return Err(crate::http_client::with_retry_after(
            crate::http_client::classify_openai_error(status, &String::from_utf8_lossy(&bytes)),
            delay,
        ));
    }
    let mut response: SearchResponse = serde_json::from_slice(&bytes)
        .map_err(|_| AppError::Provider("Invalid ranked search response".into()))?;
    if response.id.is_empty()
        || response.results.len() > max_results as usize
        || response.results.iter().any(|result| {
            !(result.url.starts_with("https://") || result.url.starts_with("http://"))
        })
    {
        return Err(AppError::Provider(
            "Search response violates result bounds or URL provenance".into(),
        ));
    }
    // There is no verified response charge in these contracts. Never call a
    // missing request fee a zero-cost token estimate or trust opaque extensions.
    response.extensions = Some(std::collections::HashMap::from([(
        "localrouter.search".into(),
        serde_json::json!({"billing_unit":"request","upstream_requests":1,"cost_usd":null,"cost_source":"unknown"}),
    )]));
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn request() -> SearchRequest {
        serde_json::from_value(json!({"model":"search","query":["first","second"],"max_results":5}))
            .unwrap()
    }
    #[test]
    fn provider_contracts_do_not_mix_chat_models_or_ignored_constraints() {
        let mut request = request();
        let wire = request.wire_body(SearchProtocol::Perplexity).unwrap();
        assert!(wire.get("model").is_none());
        assert_eq!(wire["query"], json!(["first", "second"]));
        request.max_results = Some(21);
        assert!(request.wire_body(SearchProtocol::Perplexity).is_err());
        request.search_type = Some(SearchType::People);
        request.wire_body(SearchProtocol::Perplexity).unwrap();
        request.model = Some("perplexity/perplexity-search".into());
        assert!(request.wire_body(SearchProtocol::LlmGateway).is_err());
        request.search_type = Some(SearchType::Fast);
        assert!(request.wire_body(SearchProtocol::LlmGateway).is_err());
        request.model = Some("perplexity/perplexity-search-fast".into());
        request.wire_body(SearchProtocol::LlmGateway).unwrap();
        request.search_type = None;
        request.wire_body(SearchProtocol::LlmGateway).unwrap();
        request.model = Some("sonar".into());
        assert!(request.wire_body(SearchProtocol::Perplexity).is_err());
    }
    #[test]
    fn malformed_oversized_filters_and_opaque_controls_fail_before_dispatch() {
        for extra in [
            json!({"query":[]}),
            json!({"query":" "}),
            json!({"query":vec!["q";6]}),
            json!({"max_results":0}),
            json!({"max_tokens":1_000_001}),
            json!({"country":"USA"}),
            json!({"search_domain_filter":["https://example.com"]}),
            json!({"search_domain_filter":["example.com","-other.com"]}),
            json!({"search_after_date_filter":"02/30/2026"}),
            json!({"search_after_date_filter":"10/15/2026","search_before_date_filter":"10/01/2026"}),
            json!({"search_language_filter":["english"]}),
            json!({"search_recency_filter":"decade"}),
        ] {
            let mut value = json!({"model":"search","query":"q"});
            value
                .as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            let parsed = serde_json::from_value::<SearchRequest>(value);
            assert!(
                parsed.is_err() || parsed.unwrap().validate().is_err(),
                "{extra}"
            );
        }
        assert!(
            serde_json::from_value::<SearchRequest>(json!({"query":"q","web_search":true}))
                .is_err()
        );
        let mut request = request();
        request.query = SearchQuery::Text("x".repeat(4097));
        assert!(request.validate().is_err());
    }
    #[tokio::test]
    async fn bounded_reads_reject_oversized_error_bodies_invalid_provenance_and_preserve_retry_status(
    ) {
        use wiremock::matchers::path;
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let mock = MockServer::start().await;
        let cases=[
            ("large",200,"x".repeat(4*1024*1024+1),"4 MiB"),
            ("error",503,"x".repeat(64*1024+1),"64 KiB"),
            ("invalid",200,"not JSON".into(),"Invalid ranked"),
            ("url",200,json!({"id":"x","results":[{"title":"x","url":"javascript:alert(1)"}]}).to_string(),"provenance"),
            ("count",200,json!({"id":"x","results":[{"title":"a","url":"https://example.com"},{"title":"b","url":"https://example.org"}]}).to_string(),"bounds"),
        ];
        let client = reqwest::Client::new();
        for (name, status, body, expected) in cases {
            Mock::given(path(format!("/{name}")))
                .respond_with(ResponseTemplate::new(status).set_body_string(body))
                .expect(1)
                .mount(&mock)
                .await;
            let response = client
                .get(format!("{}/{name}", mock.uri()))
                .send()
                .await
                .unwrap();
            let error = read_response(response, 1).await.unwrap_err();
            assert!(error.to_string().contains(expected), "{error}");
        }
        Mock::given(path("/rate"))
            .respond_with(
                ResponseTemplate::new(429)
                    .insert_header("retry-after", "7")
                    .set_body_string("rate limited"),
            )
            .expect(1)
            .mount(&mock)
            .await;
        let response = client
            .get(format!("{}/rate", mock.uri()))
            .send()
            .await
            .unwrap();
        assert!(matches!(
            read_response(response, 1).await,
            Err(AppError::RateLimitWithRetryAfter {
                retry_after_secs: 7
            })
        ));
        mock.verify().await;
    }
}
