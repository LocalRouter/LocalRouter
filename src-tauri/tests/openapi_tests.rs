//! OpenAPI specification integration tests
//!
//! These tests verify that the OpenAPI specification is:
//! 1. Valid according to OpenAPI 3.1 standard
//! 2. Contains all expected endpoints
//! 3. Has proper security schemes configured
//! 4. Can be serialized to both JSON and YAML
//! 5. Includes all necessary components

use localrouterai::server::openapi;
use serde_json::Value;

#[test]
fn test_openapi_spec_validity() {
    // Get the spec as JSON
    let spec_json = openapi::get_openapi_json().expect("Failed to generate OpenAPI JSON");
    let spec: Value = serde_json::from_str(&spec_json).expect("Failed to parse OpenAPI JSON");

    // Validate OpenAPI version
    assert_eq!(
        spec["openapi"].as_str(),
        Some("3.1.0"),
        "OpenAPI version should be 3.1.0"
    );

    // Validate info section
    assert_eq!(spec["info"]["title"].as_str(), Some("LocalRouter AI API"));
    assert_eq!(spec["info"]["version"].as_str(), Some("0.1.0"));
    assert!(
        spec["info"]["description"].as_str().is_some(),
        "API description should be present"
    );

    // Validate servers
    let servers = spec["servers"]
        .as_array()
        .expect("Servers should be an array");
    assert!(servers.len() >= 1, "At least one server should be defined");
}

#[test]
fn test_all_endpoints_present() {
    let spec_json = openapi::get_openapi_json().expect("Failed to generate OpenAPI JSON");
    let spec: Value = serde_json::from_str(&spec_json).expect("Failed to parse OpenAPI JSON");

    let paths = spec["paths"]
        .as_object()
        .expect("Paths should be an object");

    // Core OpenAI-compatible endpoints
    assert!(
        paths.contains_key("/v1/chat/completions"),
        "Missing /v1/chat/completions endpoint"
    );
    assert!(
        paths.contains_key("/v1/completions"),
        "Missing /v1/completions endpoint"
    );
    assert!(
        paths.contains_key("/v1/embeddings"),
        "Missing /v1/embeddings endpoint"
    );
    assert!(
        paths.contains_key("/v1/models"),
        "Missing /v1/models endpoint"
    );
    assert!(
        paths.contains_key("/v1/models/{id}"),
        "Missing /v1/models/:id endpoint"
    );
    assert!(
        paths.contains_key("/v1/models/{provider}/{model}/pricing"),
        "Missing /v1/models/:provider/:model/pricing endpoint"
    );
    assert!(
        paths.contains_key("/v1/generation"),
        "Missing /v1/generation endpoint"
    );

    // MCP endpoints
    assert!(
        paths.contains_key("/mcp/{client_id}/{server_id}"),
        "Missing /mcp/:client_id/:server_id endpoint"
    );
    assert!(
        paths.contains_key("/mcp/health"),
        "Missing /mcp/health endpoint"
    );

    // OAuth endpoints
    assert!(
        paths.contains_key("/oauth/token"),
        "Missing /oauth/token endpoint"
    );

    // System endpoints
    assert!(paths.contains_key("/health"), "Missing /health endpoint");
    assert!(paths.contains_key("/"), "Missing / (root) endpoint");
    assert!(
        paths.contains_key("/openapi.json"),
        "Missing /openapi.json endpoint"
    );
    assert!(
        paths.contains_key("/openapi.yaml"),
        "Missing /openapi.yaml endpoint"
    );
}

#[test]
fn test_security_schemes() {
    let spec_json = openapi::get_openapi_json().expect("Failed to generate OpenAPI JSON");
    let spec: Value = serde_json::from_str(&spec_json).expect("Failed to parse OpenAPI JSON");

    let security_schemes = spec["components"]["securitySchemes"]
        .as_object()
        .expect("Security schemes should be an object");

    // Check bearer_auth for API key authentication
    assert!(
        security_schemes.contains_key("bearer_auth"),
        "Missing bearer_auth security scheme"
    );
    let bearer_auth = &security_schemes["bearer_auth"];
    assert_eq!(bearer_auth["type"].as_str(), Some("http"));
    assert_eq!(bearer_auth["scheme"].as_str(), Some("bearer"));

    // Check oauth2 for MCP endpoints
    assert!(
        security_schemes.contains_key("oauth2"),
        "Missing oauth2 security scheme"
    );
    let oauth2 = &security_schemes["oauth2"];
    assert_eq!(oauth2["type"].as_str(), Some("oauth2"));
}

#[test]
fn test_components_schemas() {
    let spec_json = openapi::get_openapi_json().expect("Failed to generate OpenAPI JSON");
    let spec: Value = serde_json::from_str(&spec_json).expect("Failed to parse OpenAPI JSON");

    let schemas = spec["components"]["schemas"]
        .as_object()
        .expect("Schemas should be an object");

    // Request types
    assert!(
        schemas.contains_key("ChatCompletionRequest"),
        "Missing ChatCompletionRequest schema"
    );
    assert!(
        schemas.contains_key("CompletionRequest"),
        "Missing CompletionRequest schema"
    );
    assert!(
        schemas.contains_key("EmbeddingRequest"),
        "Missing EmbeddingRequest schema"
    );

    // Response types
    assert!(
        schemas.contains_key("ChatCompletionResponse"),
        "Missing ChatCompletionResponse schema"
    );
    assert!(
        schemas.contains_key("CompletionResponse"),
        "Missing CompletionResponse schema"
    );
    assert!(
        schemas.contains_key("EmbeddingResponse"),
        "Missing EmbeddingResponse schema"
    );
    assert!(
        schemas.contains_key("ModelsResponse"),
        "Missing ModelsResponse schema"
    );
    assert!(
        schemas.contains_key("ErrorResponse"),
        "Missing ErrorResponse schema"
    );

    // Feature adapter extensions
    assert!(
        schemas.contains_key("ExtendedThinkingParams"),
        "Missing ExtendedThinkingParams schema"
    );
    assert!(
        schemas.contains_key("PromptCachingParams"),
        "Missing PromptCachingParams schema"
    );
    assert!(
        schemas.contains_key("LogprobsParams"),
        "Missing LogprobsParams schema"
    );
}

#[test]
fn test_tags_present() {
    let spec_json = openapi::get_openapi_json().expect("Failed to generate OpenAPI JSON");
    let spec: Value = serde_json::from_str(&spec_json).expect("Failed to parse OpenAPI JSON");

    let tags = spec["tags"].as_array().expect("Tags should be an array");

    let tag_names: Vec<&str> = tags
        .iter()
        .filter_map(|tag| tag["name"].as_str())
        .collect();

    assert!(tag_names.contains(&"chat"), "Missing 'chat' tag");
    assert!(
        tag_names.contains(&"completions"),
        "Missing 'completions' tag"
    );
    assert!(
        tag_names.contains(&"embeddings"),
        "Missing 'embeddings' tag"
    );
    assert!(tag_names.contains(&"models"), "Missing 'models' tag");
    assert!(
        tag_names.contains(&"monitoring"),
        "Missing 'monitoring' tag"
    );
    assert!(tag_names.contains(&"mcp"), "Missing 'mcp' tag");
    assert!(tag_names.contains(&"oauth"), "Missing 'oauth' tag");
    assert!(tag_names.contains(&"system"), "Missing 'system' tag");
}

#[test]
fn test_json_serialization() {
    let spec_json = openapi::get_openapi_json().expect("Failed to generate OpenAPI JSON");

    // Should be valid JSON
    let _spec: Value =
        serde_json::from_str(&spec_json).expect("OpenAPI JSON should be valid JSON");

    // Should be pretty-printed
    assert!(
        spec_json.contains("  "),
        "JSON should be pretty-printed with indentation"
    );
}

#[test]
fn test_yaml_serialization() {
    let spec_yaml = openapi::get_openapi_yaml().expect("Failed to generate OpenAPI YAML");

    // Should contain YAML indicators
    assert!(
        spec_yaml.contains("openapi: 3.1.0"),
        "YAML should contain OpenAPI version"
    );
    assert!(
        spec_yaml.contains("title: LocalRouter AI API"),
        "YAML should contain API title"
    );

    // Should be valid YAML (parseable)
    let _spec: Value =
        serde_yaml::from_str(&spec_yaml).expect("OpenAPI YAML should be valid YAML");
}

#[test]
fn test_endpoint_methods() {
    let spec_json = openapi::get_openapi_json().expect("Failed to generate OpenAPI JSON");
    let spec: Value = serde_json::from_str(&spec_json).expect("Failed to parse OpenAPI JSON");

    let paths = spec["paths"]
        .as_object()
        .expect("Paths should be an object");

    // Chat completions should have POST
    let chat_completions = &paths["/v1/chat/completions"];
    assert!(
        chat_completions.as_object().unwrap().contains_key("post"),
        "/v1/chat/completions should have POST method"
    );

    // Models should have GET
    let models = &paths["/v1/models"];
    assert!(
        models.as_object().unwrap().contains_key("get"),
        "/v1/models should have GET method"
    );

    // OAuth token should have POST
    let oauth_token = &paths["/oauth/token"];
    assert!(
        oauth_token.as_object().unwrap().contains_key("post"),
        "/oauth/token should have POST method"
    );

    // Health should have GET
    let health = &paths["/health"];
    assert!(
        health.as_object().unwrap().contains_key("get"),
        "/health should have GET method"
    );
}

#[test]
fn test_response_codes() {
    let spec_json = openapi::get_openapi_json().expect("Failed to generate OpenAPI JSON");
    let spec: Value = serde_json::from_str(&spec_json).expect("Failed to parse OpenAPI JSON");

    let paths = spec["paths"]
        .as_object()
        .expect("Paths should be an object");

    // Chat completions should have success and error responses
    let chat_completions_responses = &paths["/v1/chat/completions"]["post"]["responses"];
    let responses = chat_completions_responses
        .as_object()
        .expect("Responses should be an object");

    assert!(
        responses.contains_key("200"),
        "Chat completions should have 200 response"
    );
    assert!(
        responses.contains_key("400"),
        "Chat completions should have 400 response"
    );
    assert!(
        responses.contains_key("401"),
        "Chat completions should have 401 response"
    );
}

#[test]
fn test_feature_adapter_documentation() {
    let spec_json = openapi::get_openapi_json().expect("Failed to generate OpenAPI JSON");
    let spec: Value = serde_json::from_str(&spec_json).expect("Failed to parse OpenAPI JSON");

    let schemas = spec["components"]["schemas"]
        .as_object()
        .expect("Schemas should be an object");

    // All 7 feature adapters should be documented
    let feature_adapters = [
        "ExtendedThinkingParams",
        "ReasoningTokensParams",
        "ThinkingLevelParams",
        "StructuredOutputsParams",
        "PromptCachingParams",
        "LogprobsParams",
        "JsonModeParams",
    ];

    for adapter in feature_adapters {
        assert!(
            schemas.contains_key(adapter),
            "Missing feature adapter schema: {}",
            adapter
        );

        // Each adapter should have a description
        let adapter_schema = &schemas[adapter];
        assert!(
            adapter_schema.get("description").is_some()
                || adapter_schema.get("title").is_some(),
            "Feature adapter {} should have description or title",
            adapter
        );
    }
}

#[test]
fn test_oauth_types_present() {
    let spec_json = openapi::get_openapi_json().expect("Failed to generate OpenAPI JSON");
    let spec: Value = serde_json::from_str(&spec_json).expect("Failed to parse OpenAPI JSON");

    let schemas = spec["components"]["schemas"]
        .as_object()
        .expect("Schemas should be an object");

    assert!(
        schemas.contains_key("TokenRequest"),
        "Missing TokenRequest schema"
    );
    assert!(
        schemas.contains_key("TokenResponse"),
        "Missing TokenResponse schema"
    );
    assert!(
        schemas.contains_key("TokenErrorResponse"),
        "Missing TokenErrorResponse schema"
    );
}

#[test]
fn test_mcp_types_present() {
    let spec_json = openapi::get_openapi_json().expect("Failed to generate OpenAPI JSON");
    let spec: Value = serde_json::from_str(&spec_json).expect("Failed to parse OpenAPI JSON");

    let schemas = spec["components"]["schemas"]
        .as_object()
        .expect("Schemas should be an object");

    assert!(
        schemas.contains_key("JsonRpcRequest"),
        "Missing JsonRpcRequest schema"
    );
    assert!(
        schemas.contains_key("JsonRpcResponse"),
        "Missing JsonRpcResponse schema"
    );
}

#[test]
fn test_examples_present() {
    let spec_json = openapi::get_openapi_json().expect("Failed to generate OpenAPI JSON");
    let spec: Value = serde_json::from_str(&spec_json).expect("Failed to parse OpenAPI JSON");

    let schemas = spec["components"]["schemas"]
        .as_object()
        .expect("Schemas should be an object");

    // Key request types should have examples
    let types_requiring_examples = [
        "ChatCompletionRequest",
        "TokenRequest",
        "ExtendedThinkingParams",
    ];

    for type_name in types_requiring_examples {
        let schema = schemas
            .get(type_name)
            .unwrap_or_else(|| panic!("Schema {} should exist", type_name));

        // Check if example or examples field exists
        let has_example = schema.get("example").is_some() || schema.get("examples").is_some();

        assert!(
            has_example,
            "Schema {} should have example(s)",
            type_name
        );
    }
}
