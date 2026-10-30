//! `GET /events/watch`: Push Relay's local event watch for the `EventWait`
//! fallback tool. A plain SSE stream, one `data:` line per event, so a
//! `curl -N … | grep -m 1 '^data:'` pipeline can wait for events.

use std::convert::Infallible;

use axum::{
    extract::{Query, State},
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse, Response,
    },
};
use futures::StreamExt;
use serde::Deserialize;
use utoipa::IntoParams;

use crate::middleware::error::ApiErrorResponse;
use crate::state::AppState;

/// Query of `GET /events/watch`.
#[derive(Debug, Deserialize, IntoParams)]
pub struct EventsWatchQuery {
    /// Watch token minted by the `EventWait` tool (authenticates the client
    /// and fixes the event types and filter).
    pub token: String,
    /// Cursor to continue from (default: now).
    pub cursor: Option<String>,
}

/// Watch pushed events (Push Relay `EventWait`).
///
/// Streams every event the client may read in the token's scope as
/// Server-Sent Events: `data: {"eventId", "name", "timestamp", "data",
/// "cursor"}`, or `{"type": "gap", "truncated": true, "cursor"}` when events
/// were dropped before they could be read. The connection ends after the
/// configured watch lifetime.
#[utoipa::path(
    get,
    path = "/events/watch",
    tag = "mcp",
    params(EventsWatchQuery),
    responses(
        (status = 200, description = "Event stream", content_type = "text/event-stream"),
        (status = 400, description = "Invalid cursor", body = crate::types::ErrorResponse),
        (status = 401, description = "Invalid or expired watch token", body = crate::types::ErrorResponse),
        (status = 403, description = "The events are no longer available to this client", body = crate::types::ErrorResponse),
        (status = 404, description = "Push Relay is not running", body = crate::types::ErrorResponse)
    )
)]
pub async fn events_watch_handler(
    State(state): State<AppState>,
    Query(query): Query<EventsWatchQuery>,
) -> Response {
    use lr_mcp::gateway::push_relay::WatchError;
    let Some(relay) = state.push_relay.clone() else {
        return ApiErrorResponse::new(
            axum::http::StatusCode::NOT_FOUND,
            "not_found_error",
            "Push Relay is not running",
        )
        .into_response();
    };
    let stream = match relay.open_watch(&query.token, query.cursor.as_deref()) {
        Ok(stream) => stream,
        Err(error) => {
            let message = error.message();
            return match error {
                WatchError::Unauthorized | WatchError::Expired => {
                    ApiErrorResponse::unauthorized(message)
                }
                WatchError::Forbidden => ApiErrorResponse::forbidden(message),
                WatchError::BadCursor(_) => ApiErrorResponse::bad_request(message),
            }
            .into_response();
        }
    };
    let events = stream.map(|value| Ok::<_, Infallible>(Event::default().data(value.to_string())));
    Sse::new(events)
        .keep_alive(KeepAlive::default())
        .into_response()
}
