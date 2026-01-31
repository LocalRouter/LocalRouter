//! OpenAPI specification generation module
//!
//! Generates OpenAPI 3.1 specification from code annotations using utoipa.

pub mod extensions;

use utoipa::OpenApi;

/// OpenAPI documentation builder
///
/// This struct uses utoipa's derive macro to automatically generate
/// an OpenAPI 3.1 specification from the annotated route handlers and types.
#[derive(OpenApi)]
#[openapi(
    info(
        title = "LocalRouter API",
        version = "0.1.0",
        description = "OpenAI-compatible API gateway with intelligent routing, multi-provider support, and advanced features"
    ),
    servers(
        (url = "http://localhost:3625", description = "Local development server"),
        (url = "http://127.0.0.1:3625", description = "Local development server (IPv4)")
    ),
    paths(
        // Chat endpoints
        crate::server::routes::chat::chat_completions,

        // Completions endpoints
        crate::server::routes::completions::completions,

        // Embeddings endpoints
        crate::server::routes::embeddings::embeddings,

        // Models endpoints
        crate::server::routes::models::list_models,
        crate::server::routes::models::get_model,
        crate::server::routes::models::get_model_pricing,

        // Generation tracking
        crate::server::routes::generation::get_generation,

        // MCP endpoints
        crate::server::routes::mcp::mcp_gateway_get_handler,
        crate::server::routes::mcp::mcp_gateway_handler,
        crate::server::routes::mcp::mcp_server_handler,
        crate::server::routes::mcp::mcp_server_sse_handler,
        crate::server::routes::mcp::mcp_server_streaming_handler,
        crate::server::routes::mcp::elicitation_response_handler,

        // OAuth endpoints
        crate::server::routes::oauth::token_endpoint,

        // System endpoints
        crate::server::health_check,
        crate::server::serve_openapi_json,
        crate::server::serve_openapi_yaml
    ),
    components(
        schemas(
            // Request types
            crate::server::types::ChatCompletionRequest,
            crate::server::types::CompletionRequest,
            crate::server::types::EmbeddingRequest,

            // Response types
            crate::server::types::ChatCompletionResponse,
            crate::server::types::ChatCompletionChunk,
            crate::server::types::CompletionResponse,
            crate::server::types::EmbeddingResponse,
            crate::server::types::ModelsResponse,
            crate::server::types::ModelData,
            crate::server::types::ModelPricing,
            crate::server::types::CatalogInfo,
            crate::server::types::PricingSource,
            crate::server::types::GenerationDetailsResponse,
            crate::server::types::ErrorResponse,
            crate::server::types::ApiError,

            // Message types
            crate::server::types::ChatMessage,
            crate::server::types::MessageContent,
            crate::server::types::ContentPart,
            crate::server::types::ImageUrl,

            // Tool types (request)
            crate::server::types::Tool,
            crate::server::types::ToolChoice,
            crate::server::types::FunctionDefinition,
            crate::server::types::FunctionName,

            // Tool types (response)
            crate::server::types::ToolCall,
            crate::server::types::FunctionCall,
            crate::server::types::ToolCallDelta,
            crate::server::types::FunctionCallDelta,

            // Token usage types
            crate::server::types::TokenUsage,
            crate::providers::PromptTokensDetails,
            crate::providers::CompletionTokensDetails,

            // Configuration types
            crate::server::types::ResponseFormat,

            // Choice types
            crate::server::types::ChatCompletionChoice,
            crate::server::types::CompletionChoice,
            crate::server::types::ChatCompletionChunkChoice,
            crate::server::types::CompletionChunkChoice,
            crate::server::types::ChunkDelta,

            // Embedding types
            crate::server::types::EmbeddingData,
            crate::server::types::EmbeddingVector,
            crate::server::types::EmbeddingUsage,

            // Provider types (for model capabilities and metrics)
            crate::providers::ModelCapabilities,
            crate::providers::PerformanceMetrics,

            // MCP protocol types
            crate::mcp::protocol::JsonRpcRequest,
            crate::mcp::protocol::JsonRpcResponse,
            crate::mcp::protocol::ElicitationResponse,
            crate::server::types::MessageResponse,

            // OAuth types
            crate::server::routes::oauth::TokenRequest,
            crate::server::routes::oauth::TokenResponse,
            crate::server::routes::oauth::TokenErrorResponse,

            // Generation tracking
            crate::server::routes::generation::GenerationQuery,

            // Feature adapter extension schemas
            crate::server::openapi::extensions::ExtendedThinkingParams,
            crate::server::openapi::extensions::ReasoningTokensParams,
            crate::server::openapi::extensions::ThinkingLevelParams,
            crate::server::openapi::extensions::StructuredOutputsParams,
            crate::server::openapi::extensions::PromptCachingParams,
            crate::server::openapi::extensions::CacheControl,
            crate::server::openapi::extensions::LogprobsParams,
            crate::server::openapi::extensions::JsonModeParams,
            crate::server::openapi::extensions::FeatureAdapterExtensions,
        )
    ),
    tags(
        (name = "chat", description = "Chat completion endpoints"),
        (name = "completions", description = "Text completion endpoints"),
        (name = "embeddings", description = "Embeddings endpoints"),
        (name = "models", description = "Model management and information"),
        (name = "monitoring", description = "Usage tracking and monitoring"),
        (name = "mcp", description = "MCP server proxy endpoints"),
        (name = "oauth", description = "OAuth 2.0 authentication endpoints"),
        (name = "system", description = "System health and information")
    ),
    modifiers(&SecurityAddon)
)]
pub struct ApiDoc;

/// Security scheme addon
///
/// Adds bearer_auth and oauth2 security schemes to the OpenAPI spec.
struct SecurityAddon;

impl utoipa::Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        if let Some(components) = openapi.components.as_mut() {
            // Add bearer authentication for API key-based endpoints
            components.add_security_scheme(
                "bearer_auth",
                utoipa::openapi::security::SecurityScheme::Http(
                    utoipa::openapi::security::HttpBuilder::new()
                        .scheme(utoipa::openapi::security::HttpAuthScheme::Bearer)
                        .bearer_format("API Key")
                        .description(Some("API key authentication. Include your API key in the Authorization header as 'Bearer <your-api-key>'"))
                        .build()
                ),
            );

            // Add OAuth2 authentication for MCP proxy endpoints
            components.add_security_scheme(
                "oauth2",
                utoipa::openapi::security::SecurityScheme::OAuth2(
                    utoipa::openapi::security::OAuth2::new([
                        utoipa::openapi::security::Flow::ClientCredentials(
                            utoipa::openapi::security::ClientCredentials::new(
                                "/oauth/token",
                                utoipa::openapi::security::Scopes::new(),
                            ),
                        ),
                    ]),
                ),
            );
        }
    }
}

/// Get the OpenAPI specification as JSON
///
/// Returns the complete OpenAPI 3.1 specification in JSON format.
pub fn get_openapi_json() -> Result<String, serde_json::Error> {
    serde_json::to_string_pretty(&ApiDoc::openapi())
}

/// Get the OpenAPI specification as YAML
///
/// Returns the complete OpenAPI 3.1 specification in YAML format.
pub fn get_openapi_yaml() -> Result<String, serde_yaml::Error> {
    serde_yaml::to_string(&ApiDoc::openapi())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_openapi_spec_validity() {
        let spec = ApiDoc::openapi();

        // Note: OpenAPI version is validated by utoipa at compile time
        // We trust utoipa to use the correct version (3.1.0)

        // Validate info
        assert_eq!(spec.info.title, "LocalRouter API");
        assert_eq!(spec.info.version, "0.1.0");

        // Validate paths exist
        assert!(spec.paths.paths.contains_key("/v1/chat/completions"));
        assert!(spec.paths.paths.contains_key("/v1/completions"));
        assert!(spec.paths.paths.contains_key("/v1/embeddings"));
        assert!(spec.paths.paths.contains_key("/v1/models"));
        assert!(spec.paths.paths.contains_key("/v1/models/{id}"));
        assert!(spec
            .paths
            .paths
            .contains_key("/v1/models/{provider}/{model}/pricing"));
        assert!(spec.paths.paths.contains_key("/v1/generation"));
        assert!(spec.paths.paths.contains_key("/mcp/{server_id}"));
        assert!(spec.paths.paths.contains_key("/mcp/{server_id}/stream"));
        assert!(spec.paths.paths.contains_key("/health"));
        assert!(spec.paths.paths.contains_key("/"));

        // Validate security schemes exist
        let components = spec.components.as_ref().unwrap();
        assert!(components.security_schemes.contains_key("bearer_auth"));
        assert!(components.security_schemes.contains_key("oauth2"));
    }

    #[test]
    fn test_json_generation() {
        let json = get_openapi_json();
        assert!(json.is_ok());

        let json_str = json.unwrap();
        assert!(json_str.contains("\"openapi\": \"3.1.0\""));
        assert!(json_str.contains("LocalRouter API"));
    }

    #[test]
    fn test_yaml_generation() {
        let yaml = get_openapi_yaml();
        assert!(yaml.is_ok());

        let yaml_str = yaml.unwrap();
        assert!(yaml_str.contains("openapi: 3.1.0"));
        assert!(yaml_str.contains("LocalRouter API"));
    }
}
