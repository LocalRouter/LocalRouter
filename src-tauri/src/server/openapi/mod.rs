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
        lr_server::routes::chat::chat_completions,

        // Completions endpoints
        lr_server::routes::completions::completions,

        // Embeddings endpoints
        lr_server::routes::embeddings::embeddings,

        // Models endpoints
        lr_server::routes::models::list_models,
        lr_server::routes::models::get_model,
        lr_server::routes::models::get_model_pricing,

        // Generation tracking
        lr_server::routes::generation::get_generation,

        // MCP endpoints
        lr_server::routes::mcp::mcp_gateway_get_handler,
        lr_server::routes::mcp::mcp_gateway_handler,
        lr_server::routes::mcp::elicitation_response_handler,

        // OAuth endpoints
        lr_server::routes::oauth::token_endpoint,

        // System endpoints
        lr_server::health_check,
        lr_server::serve_openapi_json,
        lr_server::serve_openapi_yaml
    ),
    components(
        schemas(
            // Request types
            lr_server::types::ChatCompletionRequest,
            lr_server::types::CompletionRequest,
            lr_server::types::EmbeddingRequest,

            // Response types
            lr_server::types::ChatCompletionResponse,
            lr_server::types::ChatCompletionChunk,
            lr_server::types::CompletionResponse,
            lr_server::types::EmbeddingResponse,
            lr_server::types::ModelsResponse,
            lr_server::types::ModelData,
            lr_server::types::ModelPricing,
            lr_server::types::CatalogInfo,
            lr_server::types::PricingSource,
            lr_server::types::GenerationDetailsResponse,
            lr_server::types::ErrorResponse,
            lr_server::types::ApiError,

            // Message types
            lr_server::types::ChatMessage,
            lr_server::types::MessageContent,
            lr_server::types::ContentPart,
            lr_server::types::ImageUrl,

            // Tool types (request)
            lr_server::types::Tool,
            lr_server::types::ToolChoice,
            lr_server::types::FunctionDefinition,
            lr_server::types::FunctionName,

            // Tool types (response)
            lr_server::types::ToolCall,
            lr_server::types::FunctionCall,
            lr_server::types::ToolCallDelta,
            lr_server::types::FunctionCallDelta,

            // Token usage types
            lr_server::types::TokenUsage,
            lr_providers::PromptTokensDetails,
            lr_providers::CompletionTokensDetails,

            // Configuration types
            lr_server::types::ResponseFormat,

            // Choice types
            lr_server::types::ChatCompletionChoice,
            lr_server::types::CompletionChoice,
            lr_server::types::ChatCompletionChunkChoice,
            lr_server::types::CompletionChunkChoice,
            lr_server::types::ChunkDelta,

            // Embedding types
            lr_server::types::EmbeddingData,
            lr_server::types::EmbeddingVector,
            lr_server::types::EmbeddingUsage,

            // Provider types (for model capabilities and metrics)
            lr_providers::ModelCapabilities,
            lr_providers::PerformanceMetrics,

            // MCP protocol types
            lr_mcp::protocol::JsonRpcRequest,
            lr_mcp::protocol::JsonRpcResponse,
            lr_mcp::protocol::ElicitationResponse,
            lr_server::types::MessageResponse,

            // OAuth types
            lr_server::routes::oauth::TokenRequest,
            lr_server::routes::oauth::TokenResponse,
            lr_server::routes::oauth::TokenErrorResponse,

            // Generation tracking
            lr_server::routes::generation::GenerationQuery,

            // Feature adapter extension schemas
            lr_server::openapi::extensions::ExtendedThinkingParams,
            lr_server::openapi::extensions::ReasoningTokensParams,
            lr_server::openapi::extensions::ThinkingLevelParams,
            lr_server::openapi::extensions::StructuredOutputsParams,
            lr_server::openapi::extensions::PromptCachingParams,
            lr_server::openapi::extensions::CacheControl,
            lr_server::openapi::extensions::LogprobsParams,
            lr_server::openapi::extensions::JsonModeParams,
            lr_server::openapi::extensions::FeatureAdapterExtensions,
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
    let mut spec = ApiDoc::openapi();
    spec.info.version = env!("CARGO_PKG_VERSION").to_string();
    serde_json::to_string_pretty(&spec)
}

/// Get the OpenAPI specification as YAML
///
/// Returns the complete OpenAPI 3.1 specification in YAML format.
pub fn get_openapi_yaml() -> Result<String, serde_yaml::Error> {
    let mut spec = ApiDoc::openapi();
    spec.info.version = env!("CARGO_PKG_VERSION").to_string();
    serde_yaml::to_string(&spec)
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
        assert_eq!(spec.info.version, env!("CARGO_PKG_VERSION"));

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
        assert!(!spec.paths.paths.contains_key("/mcp/{server_id}"));
        assert!(!spec.paths.paths.contains_key("/mcp/{server_id}/stream"));
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
