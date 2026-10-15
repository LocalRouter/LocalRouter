//! Native candidate scoring. Results refer to the original request positions.
use lr_types::{AppError, AppResult};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use utoipa::ToSchema;
pub const METADATA: &str = "localrouter.rerank";
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RerankProtocol {
    Cohere,
    LlmGateway,
}
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct RerankRequest {
    pub model: String,
    pub query: String,
    pub documents: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_n: Option<u32>,
    #[serde(default)]
    pub return_documents: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens_per_doc: Option<u32>,
}
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct RankedDocument {
    pub text: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct RerankResult {
    pub index: usize,
    pub relevance_score: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document: Option<RankedDocument>,
}
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct RerankResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub results: Vec<RerankResult>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(value_type=Option<Object>)]
    pub meta: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(value_type=Option<Object>)]
    pub extensions: Option<std::collections::HashMap<String, Value>>,
}
fn invalid(message: &str) -> AppError {
    AppError::InvalidParams(message.into())
}
impl RerankRequest {
    pub fn validate(&self) -> AppResult<()> {
        if self.model.is_empty()
            || self.model.len() > 512
            || self.model.chars().any(char::is_whitespace)
        {
            return Err(invalid("Rerank requires an explicit model up to 512 bytes"));
        }
        if self.query.trim().is_empty() || self.query.len() > 16 * 1024 {
            return Err(invalid("Rerank query requires 1 byte to 16 KiB"));
        }
        if self.documents.is_empty()
            || self.documents.len() > 128
            || self
                .documents
                .iter()
                .any(|d| d.trim().is_empty() || d.len() > 32 * 1024)
            || self.documents.iter().map(String::len).sum::<usize>() > 1024 * 1024
        {
            return Err(invalid(
                "Rerank accepts 1–128 nonempty candidates, 32 KiB each and 1 MiB total",
            ));
        }
        if self
            .top_n
            .is_some_and(|n| n == 0 || n as usize > self.documents.len())
        {
            return Err(invalid(
                "Rerank top_n must be between 1 and the candidate count",
            ));
        }
        if self
            .max_tokens_per_doc
            .is_some_and(|n| !(1..=4096).contains(&n))
        {
            return Err(invalid(
                "Rerank max_tokens_per_doc must be between 1 and 4096",
            ));
        }
        Ok(())
    }
    pub fn wire_body(&self, protocol: RerankProtocol) -> AppResult<Value> {
        self.validate()?;
        if matches!(self.model.as_str(), "auto" | "localrouter/auto") {
            return Err(invalid("Rerank requires an explicit native model"));
        }
        if protocol == RerankProtocol::Cohere
            && (!self.model.starts_with("rerank-") || self.model.contains('/'))
        {
            return Err(invalid(
                "Direct Cohere rerank requires an explicit rerank-* model",
            ));
        }
        if protocol == RerankProtocol::LlmGateway && self.max_tokens_per_doc.is_some() {
            return Err(invalid(
                "Gateway max_tokens_per_doc has no verified native mapping",
            ));
        }
        let mut body = json!({"model":self.model,"query":self.query,"documents":self.documents,"top_n":self.top_n.unwrap_or(self.documents.len() as u32)});
        if let Some(max) = self.max_tokens_per_doc {
            body["max_tokens_per_doc"] = json!(max);
        }
        // return_documents is mapped locally from validated native indices.
        Ok(body)
    }
    pub async fn read_response(&self, response: reqwest::Response) -> AppResult<RerankResponse> {
        if !response.status().is_success() {
            return Err(crate::messages::response_error(response).await);
        }
        let value = crate::messages::read_json_response(response, 4 * 1024 * 1024).await?;
        // Upstream document text is not a source of truth for index mapping.
        let mut value = value;
        if let Some(results) = value.get_mut("results").and_then(Value::as_array_mut) {
            for result in results {
                if let Some(object) = result.as_object_mut() {
                    object.remove("document");
                }
            }
        }
        let mut result: RerankResponse = serde_json::from_value(value).map_err(|_| {
            AppError::Provider("Invalid native rerank response; the dispatch is not retried".into())
        })?;
        let mut indices = std::collections::HashSet::new();
        if result.results.is_empty()
            || result.results.len()
                > self
                    .top_n
                    .map(|n| n as usize)
                    .unwrap_or(self.documents.len())
            || result.results.iter().any(|r| {
                r.index >= self.documents.len()
                    || !indices.insert(r.index)
                    || !r.relevance_score.is_finite()
            })
            || result
                .results
                .windows(2)
                .any(|pair| pair[0].relevance_score < pair[1].relevance_score)
        {
            return Err(AppError::Provider("Native rerank returned invalid indices, ordering or scores; work may already have been billed".into()));
        }
        for entry in &mut result.results {
            entry.document = self.return_documents.then(|| RankedDocument {
                text: self.documents[entry.index].clone(),
            });
        }
        let billed_units = result
            .meta
            .as_ref()
            .and_then(|m| m.get("billed_units"))
            .cloned();
        result.extensions=Some([(METADATA.into(),json!({"cost_usd":null,"cost_source":"unknown","billed_units":billed_units,"candidate_count":self.documents.len(),"source":"native","document_processing":"provider_may_truncate","scores_calibrated":false}))].into_iter().collect());
        Ok(result)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn request() -> RerankRequest {
        serde_json::from_value(json!({"model":"rerank-v4.0-pro","query":"source","documents":["first","second"],"top_n":1,"return_documents":true})).unwrap()
    }
    #[test]
    fn query_candidate_and_native_control_bounds_are_explicit() {
        let mut r = request();
        assert!(r.validate().is_ok());
        r.top_n = Some(0);
        assert!(r.validate().is_err());
        r.top_n = Some(3);
        assert!(r.validate().is_err());
        r = request();
        r.query = " ".into();
        assert!(r.validate().is_err());
        r = request();
        r.documents = vec!["x".repeat(32 * 1024 + 1)];
        assert!(r.validate().is_err());
        r = request();
        r.max_tokens_per_doc = Some(4096);
        assert!(r.wire_body(RerankProtocol::Cohere).is_ok());
        assert!(r.wire_body(RerankProtocol::LlmGateway).is_err());
        let wire = request().wire_body(RerankProtocol::Cohere).unwrap();
        assert!(wire.get("return_documents").is_none());
    }
    #[tokio::test]
    async fn rank_indices_map_to_exact_originals_and_never_trust_upstream_document_text() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET")).respond_with(wiremock::ResponseTemplate::new(200).set_body_json(json!({"id":"native","results":[{"index":1,"relevance_score":0.9,"document":{"text":"rewritten"}}],"meta":{"billed_units":{"search_units":1}}}))).mount(&server).await;
        let result = request()
            .read_response(reqwest::get(server.uri()).await.unwrap())
            .await
            .unwrap();
        assert_eq!(result.results[0].index, 1);
        assert_eq!(result.results[0].document.as_ref().unwrap().text, "second");
        assert_eq!(
            result.meta.as_ref().unwrap()["billed_units"]["search_units"],
            1
        );
        assert!(result.extensions.as_ref().unwrap()[METADATA]["cost_usd"].is_null());
    }
}

#[cfg(test)]
mod response_validation_tests {
    use super::*;
    fn request() -> RerankRequest {
        serde_json::from_value(json!({"model":"qwen3-reranker-8b","query":"source","documents":["first","second"],"top_n":2})).unwrap()
    }
    #[tokio::test]
    async fn invalid_native_indices_duplicates_order_and_scores_fail_without_fabricated_ranking() {
        let server = wiremock::MockServer::start().await;
        for results in [
            json!([]),
            json!([{"index":2,"relevance_score":0.8}]),
            json!([{"index":0,"relevance_score":0.9},{"index":0,"relevance_score":0.8}]),
            json!([{"index":0,"relevance_score":0.2},{"index":1,"relevance_score":0.8}]),
            json!([{"index":0,"relevance_score":null}]),
        ] {
            let _scope = wiremock::Mock::given(wiremock::matchers::method("GET"))
                .respond_with(
                    wiremock::ResponseTemplate::new(200).set_body_json(json!({"results":results})),
                )
                .mount_as_scoped(&server)
                .await;
            assert!(request()
                .read_response(reqwest::get(server.uri()).await.unwrap())
                .await
                .is_err());
        }
        let _scope=wiremock::Mock::given(wiremock::matchers::method("GET")).respond_with(wiremock::ResponseTemplate::new(200).set_body_json(json!({"results":[{"index":1,"relevance_score":0.9,"document":{"text":"not requested"}},{"index":0,"relevance_score":0.8}],"meta":{"billed_units":{"input_tokens":100}}}))).mount_as_scoped(&server).await;
        let result = request()
            .read_response(reqwest::get(server.uri()).await.unwrap())
            .await
            .unwrap();
        assert!(result.results.iter().all(|r| r.document.is_none()));
        assert_eq!(
            result.meta.as_ref().unwrap()["billed_units"]["input_tokens"],
            100
        );
    }
    #[tokio::test]
    async fn direct_cohere_rerank_uses_its_native_endpoint_and_refuses_redirects() {
        use crate::ModelProvider;
        let server = wiremock::MockServer::start().await;
        let target = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .respond_with(wiremock::ResponseTemplate::new(500))
            .expect(0)
            .mount(&target)
            .await;
        wiremock::Mock::given(wiremock::matchers::method("POST")).and(wiremock::matchers::path("/v2/rerank")).and(wiremock::matchers::header("authorization","Bearer fixture")).and(wiremock::matchers::body_partial_json(json!({"model":"rerank-v4.0-pro","query":"source","documents":["first","second"],"top_n":2}))).respond_with(wiremock::ResponseTemplate::new(307).insert_header("location",target.uri())).expect(1).mount(&server).await;
        let provider = crate::cohere::CohereProvider::with_base_url(
            "fixture".into(),
            format!("{}/v2", server.uri()),
        )
        .unwrap();
        let mut request = request();
        request.model = "rerank-v4.0-pro".into();
        assert!(provider.rerank(request).await.is_err());
        server.verify().await;
        target.verify().await;
    }
    #[test]
    fn aggregate_and_candidate_count_limits_are_checked_before_dispatch() {
        let mut r = request();
        r.documents = vec!["x".into(); 129];
        assert!(r.validate().is_err());
        r = request();
        r.documents = vec!["x".repeat(32 * 1024); 33];
        assert!(r.validate().is_err());
        r = request();
        r.documents = vec!["".into()];
        assert!(r.validate().is_err());
        let mut raw = serde_json::to_value(request()).unwrap();
        raw["max_chunks_per_doc"] = json!(4);
        assert!(serde_json::from_value::<RerankRequest>(raw).is_err());
    }
}
