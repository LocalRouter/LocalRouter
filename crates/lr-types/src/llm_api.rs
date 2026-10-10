//! The wire APIs an LLM request can arrive on or be sent upstream with.

use serde::{Deserialize, Serialize};

/// A wire API spoken between a client and LocalRouter, or between
/// LocalRouter and an upstream provider. When the two sides differ for one
/// request, LocalRouter translated it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LlmApi {
    /// OpenAI Chat Completions (`/chat/completions`).
    ChatCompletions,
    /// OpenAI legacy Completions (`/completions`).
    Completions,
    /// OpenAI Responses (`/responses`, including the ChatGPT Codex backend).
    Responses,
    /// Anthropic Messages (`/messages`).
    AnthropicMessages,
    /// Google Gemini `generateContent` / `streamGenerateContent`.
    GeminiGenerateContent,
    /// Cohere v2 Chat (`/v2/chat`).
    CohereChat,
    /// Ollama native chat (`/api/chat`) and generate (`/api/generate`).
    OllamaChat,
    /// System One decisions (`/v1/systemone`).
    SystemOne,
    /// Embeddings (`/embeddings`).
    Embeddings,
    /// Moderations (`/moderations`).
    Moderations,
    /// Image generation and edits (`/images/*`).
    Images,
    /// Speech-to-text, translation and text-to-speech (`/audio/*`).
    Audio,
}

impl LlmApi {
    /// The API a request path belongs to, for paths with or without a `/v1`
    /// (or other) prefix. Unknown paths return `None`.
    pub fn from_path(path: &str) -> Option<Self> {
        let path = path.split('?').next().unwrap_or(path).trim_end_matches('/');
        if path.ends_with("/chat/completions") {
            Some(Self::ChatCompletions)
        } else if path.ends_with("/completions") {
            Some(Self::Completions)
        } else if path.ends_with("/responses") || path.contains("/responses/") {
            Some(Self::Responses)
        } else if path.ends_with("/messages") {
            Some(Self::AnthropicMessages)
        } else if path.contains(":generateContent") || path.contains(":streamGenerateContent") {
            Some(Self::GeminiGenerateContent)
        } else if path.ends_with("/v2/chat") {
            Some(Self::CohereChat)
        } else if path.ends_with("/api/chat") || path.ends_with("/api/generate") {
            Some(Self::OllamaChat)
        } else if path.ends_with("/systemone") {
            Some(Self::SystemOne)
        } else if path.ends_with("/embeddings") {
            Some(Self::Embeddings)
        } else if path.ends_with("/moderations") {
            Some(Self::Moderations)
        } else if path.contains("/images/") {
            Some(Self::Images)
        } else if path.contains("/audio/") {
            Some(Self::Audio)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_map_to_their_api() {
        for (path, api) in [
            ("/v1/chat/completions", LlmApi::ChatCompletions),
            ("/chat/completions", LlmApi::ChatCompletions),
            ("/v1/completions", LlmApi::Completions),
            ("/v1/responses", LlmApi::Responses),
            ("/backend-api/codex/responses", LlmApi::Responses),
            ("/v1/responses/resp_1/input_items", LlmApi::Responses),
            ("/v1/messages?beta=true", LlmApi::AnthropicMessages),
            (
                "/v1beta/models/gemini-2.5-pro:streamGenerateContent",
                LlmApi::GeminiGenerateContent,
            ),
            ("/v2/chat", LlmApi::CohereChat),
            ("/api/chat", LlmApi::OllamaChat),
            ("/api/generate", LlmApi::OllamaChat),
            ("/v1/systemone", LlmApi::SystemOne),
            ("/v1/embeddings", LlmApi::Embeddings),
            ("/v1/moderations", LlmApi::Moderations),
            ("/v1/images/generations", LlmApi::Images),
            ("/v1/audio/speech", LlmApi::Audio),
        ] {
            assert_eq!(LlmApi::from_path(path), Some(api), "{path}");
        }
        assert_eq!(LlmApi::from_path("/v1/models"), None);
    }

    #[test]
    fn serializes_snake_case() {
        assert_eq!(
            serde_json::to_string(&LlmApi::AnthropicMessages).unwrap(),
            "\"anthropic_messages\""
        );
    }
}
