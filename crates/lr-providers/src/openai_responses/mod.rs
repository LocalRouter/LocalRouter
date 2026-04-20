//! Translator between our chat-completions-shaped `CompletionRequest`
//! and the OpenAI Responses API (`POST /responses`).
//!
//! Two consumers:
//!
//! 1. **Outbound** (phase 1) — `OpenAIProvider` in ChatGPT-backend mode
//!    branches `complete` / `stream_complete` through this module so
//!    subscription-token users can actually chat.
//! 2. **Inbound** (phase 2) — the `/v1/responses` server route uses
//!    the reverse mapping (`response_to_chat_request`) so any of our
//!    chat-completions providers looks like a Responses-API upstream
//!    to the client.
//!
//! See `/Users/matus/.claude/plans/glittery-whistling-rabbit.md` for
//! the full design and the mapping cheat-sheet.

pub mod http;
pub mod request;
pub mod response;
pub mod stream;
pub mod types;

pub use http::create_response;
pub use http::stream_response;
pub use request::translate_completion_request;
pub use response::response_to_completion;
pub use stream::responses_to_completion_chunks;
