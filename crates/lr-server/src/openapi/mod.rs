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
        crate::routes::chat::chat_completions,

        // Completions endpoints
        crate::routes::completions::completions,

        // Embeddings endpoints
        crate::routes::embeddings::embeddings,

        // Models endpoints
        crate::routes::models::list_models,
        crate::routes::models::get_model,
        crate::routes::models::get_model_pricing,

        // Generation tracking
        crate::routes::generation::get_generation,

        // MCP endpoints
        crate::routes::mcp::mcp_gateway_get_handler,
        crate::routes::mcp::mcp_gateway_handler,
        crate::routes::mcp::mcp_server_handler,
        crate::routes::mcp::mcp_server_sse_handler,
        crate::routes::mcp::mcp_server_streaming_handler,
        crate::routes::mcp::elicitation_response_handler,

        // OAuth endpoints
        crate::routes::oauth::token_endpoint,

        // System endpoints
        crate::health_check,
        crate::serve_openapi_json,
        crate::serve_openapi_yaml
    ),
    components(
        schemas(
            // Request types
            crate::types::ChatCompletionRequest,
            crate::types::CompletionRequest,
            crate::types::EmbeddingRequest,

            // Response types
            crate::types::ChatCompletionResponse,
            crate::types::ChatCompletionChunk,
            crate::types::CompletionResponse,
            crate::types::EmbeddingResponse,
            crate::types::ModelsResponse,
            crate::types::ModelData,
            crate::types::ModelPricing,
            crate::types::CatalogInfo,
            crate::types::PricingSource,
            crate::types::GenerationDetailsResponse,
            crate::types::ErrorResponse,
            crate::types::ApiError,

            // Message types
            crate::types::ChatMessage,
            crate::types::MessageContent,
            crate::types::ContentPart,
            crate::types::ImageUrl,

            // Tool types (request)
            crate::types::Tool,
            crate::types::ToolChoice,
            crate::types::FunctionDefinition,
            crate::types::FunctionName,

            // Tool types (response)
            crate::types::ToolCall,
            crate::types::FunctionCall,
            crate::types::ToolCallDelta,
            crate::types::FunctionCallDelta,

            // Token usage types
            crate::types::TokenUsage,
            lr_providers::PromptTokensDetails,
            lr_providers::CompletionTokensDetails,

            // Configuration types
            crate::types::ResponseFormat,

            // Choice types
            crate::types::ChatCompletionChoice,
            crate::types::CompletionChoice,
            crate::types::ChatCompletionChunkChoice,
            crate::types::CompletionChunkChoice,
            crate::types::ChunkDelta,

            // Embedding types
            crate::types::EmbeddingData,
            crate::types::EmbeddingVector,
            crate::types::EmbeddingUsage,

            // Provider types (for model capabilities and metrics)
            lr_providers::ModelCapabilities,
            lr_providers::PerformanceMetrics,

            // MCP protocol types
            lr_mcp::protocol::JsonRpcRequest,
            lr_mcp::protocol::JsonRpcResponse,
            lr_mcp::protocol::ElicitationResponse,
            crate::types::MessageResponse,

            // OAuth types
            crate::routes::oauth::TokenRequest,
            crate::routes::oauth::TokenResponse,
            crate::routes::oauth::TokenErrorResponse,

            // Generation tracking
            crate::routes::generation::GenerationQuery,

            // Feature adapter extension schemas
            crate::openapi::extensions::ExtendedThinkingParams,
            crate::openapi::extensions::ReasoningTokensParams,
            crate::openapi::extensions::ThinkingLevelParams,
            crate::openapi::extensions::StructuredOutputsParams,
            crate::openapi::extensions::PromptCachingParams,
            crate::openapi::extensions::CacheControl,
            crate::openapi::extensions::LogprobsParams,
            crate::openapi::extensions::JsonModeParams,
            crate::openapi::extensions::FeatureAdapterExtensions,
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
