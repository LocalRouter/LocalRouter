//! OAuth 2.0 endpoints for client authentication
//!
//! Implements the OAuth 2.0 client credentials flow for generating short-lived access tokens.
//! Reference: https://datatracker.ietf.org/doc/html/rfc6749#section-4.4

use axum::{
    extract::{FromRequest, Request, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Form, Json,
};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};
use utoipa::ToSchema;

use lr_clients::{ClientManager, TokenStore};
use lr_monitor;

/// Largest token request body the endpoint reads.
pub const MAX_TOKEN_REQUEST_BYTES: usize = 16 * 1024;

/// Longest `client_id` the endpoint tracks or verifies. Issued client ids are
/// far shorter; anything longer cannot name a client.
const MAX_CLIENT_ID_BYTES: usize = 256;

/// OAuth 2.0 token request (client credentials flow)
///
/// The body is `application/x-www-form-urlencoded` (RFC 6749 §4.4.2) or
/// `application/json`. Client credentials go in the body, or in an
/// `Authorization: Basic base64(client_id:client_secret)` header.
#[derive(Debug, Deserialize, ToSchema)]
#[schema(example = json!({"grant_type": "client_credentials", "client_id": "my-client", "client_secret": "my-secret"}))]
pub struct TokenRequest {
    /// OAuth 2.0 grant type (must be "client_credentials")
    #[schema(example = "client_credentials")]
    pub grant_type: String,

    /// Client ID (optional if using Basic Auth)
    #[serde(default)]
    pub client_id: Option<String>,

    /// Client secret (optional if using Basic Auth)
    #[serde(default)]
    pub client_secret: Option<String>,
}

/// OAuth 2.0 token response
#[derive(Debug, Serialize, ToSchema)]
pub struct TokenResponse {
    /// The access token
    #[schema(example = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9...")]
    pub access_token: String,

    /// Token type (always "Bearer")
    #[schema(example = "Bearer")]
    pub token_type: String,

    /// Token expiration in seconds
    #[schema(example = 3600)]
    pub expires_in: i64,
}

/// OAuth 2.0 error response
#[derive(Debug, Serialize, ToSchema)]
pub struct TokenErrorResponse {
    /// Error code
    #[schema(example = "invalid_client")]
    pub error: String,

    /// Human-readable error description
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_description: Option<String>,
}

impl TokenErrorResponse {
    fn invalid_request(description: &str) -> Self {
        Self {
            error: "invalid_request".to_string(),
            error_description: Some(description.to_string()),
        }
    }

    fn invalid_client() -> Self {
        Self {
            error: "invalid_client".to_string(),
            error_description: Some("Client authentication failed".to_string()),
        }
    }

    fn unsupported_grant_type() -> Self {
        Self {
            error: "unsupported_grant_type".to_string(),
            error_description: Some(
                "Only 'client_credentials' grant type is supported".to_string(),
            ),
        }
    }
}

/// Per-client token request limiter with a bounded number of tracked clients.
///
/// The key is the `client_id` a caller presents before it is authenticated,
/// so the set of keys is attacker-controlled. Tracking is capped: when the
/// table is full, clients whose attempts have all aged out of the window are
/// dropped first, and if none have, the client with the fewest attempts in
/// the window (the oldest among equals) makes room. A client being hammered
/// therefore keeps its count while one-off ids are the ones displaced.
pub struct TokenRateLimiter {
    attempts: Mutex<HashMap<String, VecDeque<Instant>>>,
    max_clients: usize,
}

impl TokenRateLimiter {
    /// Token requests allowed per client within [`Self::WINDOW`].
    pub const MAX_REQUESTS: usize = 10;
    /// Sliding window the request count applies to.
    pub const WINDOW: Duration = Duration::from_secs(60);
    /// Clients tracked at once by [`Self::default`].
    pub const DEFAULT_MAX_CLIENTS: usize = 4096;

    /// A limiter tracking at most `max_clients` clients (at least one).
    pub fn new(max_clients: usize) -> Self {
        Self {
            attempts: Mutex::new(HashMap::new()),
            max_clients: max_clients.max(1),
        }
    }

    /// Record a token request at `now`. Returns `false`, recording nothing,
    /// when the client already used its allowance for the window.
    pub fn try_acquire(&self, client_id: &str, now: Instant) -> bool {
        let live = |attempt: &Instant| now.saturating_duration_since(*attempt) < Self::WINDOW;
        let mut attempts = self.attempts.lock();

        if let Some(recent) = attempts.get_mut(client_id) {
            recent.retain(live);
            if recent.len() >= Self::MAX_REQUESTS {
                return false;
            }
            recent.push_back(now);
            return true;
        }

        if attempts.len() >= self.max_clients {
            attempts.retain(|_, recent| {
                recent.retain(live);
                !recent.is_empty()
            });
        }
        while attempts.len() >= self.max_clients {
            let Some(evicted) = attempts
                .iter()
                .min_by_key(|(_, recent)| (recent.len(), recent.back().copied()))
                .map(|(id, _)| id.clone())
            else {
                break;
            };
            attempts.remove(&evicted);
        }
        attempts.insert(client_id.to_owned(), VecDeque::from([now]));
        true
    }

    /// Number of clients currently tracked.
    pub fn tracked_clients(&self) -> usize {
        self.attempts.lock().len()
    }
}

impl Default for TokenRateLimiter {
    fn default() -> Self {
        Self::new(Self::DEFAULT_MAX_CLIENTS)
    }
}

/// Shared state for OAuth endpoints
#[derive(Clone)]
pub struct OAuthState {
    pub client_manager: Arc<ClientManager>,
    pub token_store: Arc<TokenStore>,
    pub token_rate_limiter: Arc<TokenRateLimiter>,
    pub monitor_store: Arc<lr_monitor::MonitorEventStore>,
}

/// Extract client credentials from Basic Auth header
///
/// Expected format: "Basic base64(client_id:client_secret)"
fn extract_basic_auth(auth_header: Option<&str>) -> Option<(String, String)> {
    let auth_header = auth_header?;

    // Check if it's a Basic auth header
    if !auth_header.starts_with("Basic ") {
        return None;
    }

    // Extract the base64 part
    let encoded = auth_header.strip_prefix("Basic ")?;

    // Decode base64
    let decoded = BASE64.decode(encoded).ok()?;
    let decoded_str = String::from_utf8(decoded).ok()?;

    // Split on first colon. RFC 6749 §2.3.1: both halves are
    // `application/x-www-form-urlencoded` before they are joined, so an id
    // or secret containing `:` arrives as `%3A`. Issued ids and secrets are
    // URL-safe and decode to themselves.
    let (client_id, client_secret) = decoded_str.split_once(':')?;
    Some((form_urldecode(client_id)?, form_urldecode(client_secret)?))
}

/// Decode one `application/x-www-form-urlencoded` value: `+` is a space and
/// `%XX` a byte. A `%` not followed by two hex digits is kept as it is.
/// `None` when the decoded bytes are not UTF-8.
fn form_urldecode(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' => {
                let hex = |b: u8| (b as char).to_digit(16);
                match (
                    bytes.get(i + 1).copied().and_then(hex),
                    bytes.get(i + 2).copied().and_then(hex),
                ) {
                    (Some(high), Some(low)) => {
                        out.push((high * 16 + low) as u8);
                        i += 3;
                        continue;
                    }
                    _ => out.push(b'%'),
                }
            }
            other => out.push(other),
        }
        i += 1;
    }
    String::from_utf8(out).ok()
}

/// Body encodings the token endpoint reads.
#[derive(Debug, PartialEq, Eq)]
enum TokenBodyFormat {
    Form,
    Json,
}

/// Classify the request body from its `Content-Type` media type, ignoring
/// parameters such as `charset`.
fn token_body_format(headers: &HeaderMap) -> Option<TokenBodyFormat> {
    let content_type = headers.get(header::CONTENT_TYPE)?.to_str().ok()?;
    let media_type = content_type.split(';').next()?.trim();
    if media_type.eq_ignore_ascii_case("application/x-www-form-urlencoded") {
        Some(TokenBodyFormat::Form)
    } else if media_type.eq_ignore_ascii_case("application/json") {
        Some(TokenBodyFormat::Json)
    } else {
        None
    }
}

/// Status for a body the extractor refused: an oversized body stays 413,
/// anything else is a malformed request.
fn body_rejection_status(status: StatusCode) -> StatusCode {
    if status == StatusCode::PAYLOAD_TOO_LARGE {
        status
    } else {
        StatusCode::BAD_REQUEST
    }
}

/// Read the token request from a form-encoded or JSON body.
async fn read_token_request(
    mut request: Request,
) -> Result<TokenRequest, (StatusCode, &'static str)> {
    let format = token_body_format(request.headers());
    // Media types are case-insensitive (RFC 9110 §8.3.1), but axum's
    // extractors match the header case-sensitively. Hand them the canonical
    // spelling of the type already classified above; neither reads `charset`.
    let canonical = match format {
        Some(TokenBodyFormat::Form) => Some("application/x-www-form-urlencoded"),
        Some(TokenBodyFormat::Json) => Some("application/json"),
        None => None,
    };
    if let Some(canonical) = canonical {
        request.headers_mut().insert(
            header::CONTENT_TYPE,
            header::HeaderValue::from_static(canonical),
        );
    }
    match format {
        Some(TokenBodyFormat::Form) => Form::<TokenRequest>::from_request(request, &())
            .await
            .map(|Form(body)| body)
            .map_err(|rejection| {
                (
                    body_rejection_status(rejection.status()),
                    "Malformed form-encoded token request",
                )
            }),
        Some(TokenBodyFormat::Json) => Json::<TokenRequest>::from_request(request, &())
            .await
            .map(|Json(body)| body)
            .map_err(|rejection| {
                (
                    body_rejection_status(rejection.status()),
                    "Malformed JSON token request",
                )
            }),
        None => Err((
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "Content-Type must be application/x-www-form-urlencoded or application/json",
        )),
    }
}

fn record_failure(
    state: &OAuthState,
    action: &str,
    client_id: Option<&str>,
    message: String,
    status: StatusCode,
) {
    state.monitor_store.push(
        lr_monitor::MonitorEventType::OAuthEvent,
        client_id.map(str::to_owned),
        None,
        None,
        lr_monitor::MonitorEventData::OAuthEvent {
            action: action.to_string(),
            client_id_hint: client_id.map(str::to_owned),
            message,
            status_code: status.as_u16(),
        },
        lr_monitor::EventStatus::Error,
        None,
    );
}

fn error_response(status: StatusCode, body: TokenErrorResponse) -> Response {
    (status, Json(body)).into_response()
}

/// POST /oauth/token - OAuth 2.0 token endpoint
///
/// Implements the client credentials grant flow:
/// 1. Client sends credentials (either in body or Basic Auth)
/// 2. Server verifies credentials
/// 3. Server generates and returns a short-lived access token (1 hour)
///
/// # Request
/// ```text
/// POST /oauth/token
/// Content-Type: application/x-www-form-urlencoded
///
/// grant_type=client_credentials&client_id=lr-abc123&client_secret=secret123
/// ```
///
/// Or using Basic Authentication:
/// ```text
/// POST /oauth/token
/// Authorization: Basic base64(client_id:client_secret)
/// Content-Type: application/x-www-form-urlencoded
///
/// grant_type=client_credentials
/// ```
///
/// The same fields are also accepted as an `application/json` object.
///
/// # Response
/// ```json
/// {
///   "access_token": "token123",
///   "token_type": "Bearer",
///   "expires_in": 3600
/// }
/// ```
#[utoipa::path(
    post,
    path = "/oauth/token",
    tag = "oauth",
    request_body(
        content(
            (TokenRequest = "application/x-www-form-urlencoded"),
            (TokenRequest = "application/json")
        ),
        description = "Client credentials grant. Send the credentials in the body, or omit them and use an `Authorization: Basic base64(client_id:client_secret)` header (each half form-urlencoded, RFC 6749 §2.3.1)."
    ),
    responses(
        (status = 200, description = "Access token generated successfully", body = TokenResponse),
        (status = 400, description = "Bad request - malformed body, invalid grant type or missing credentials", body = TokenErrorResponse),
        (status = 401, description = "Unauthorized - invalid client credentials", body = TokenErrorResponse),
        (status = 413, description = "Request body too large", body = TokenErrorResponse),
        (status = 415, description = "Content-Type is neither form-encoded nor JSON", body = TokenErrorResponse),
        (status = 429, description = "Too many token requests for this client", body = TokenErrorResponse),
        (status = 500, description = "Internal server error", body = TokenErrorResponse)
    )
)]
pub async fn token_endpoint(State(state): State<OAuthState>, request: Request) -> Response {
    let headers = request.headers().clone();
    let request = match read_token_request(request).await {
        Ok(request) => request,
        Err((status, description)) => {
            record_failure(
                &state,
                "invalid_request_body",
                None,
                description.to_string(),
                status,
            );
            return error_response(status, TokenErrorResponse::invalid_request(description));
        }
    };

    // Verify grant type
    if request.grant_type != "client_credentials" {
        record_failure(
            &state,
            "unsupported_grant_type",
            None,
            format!("Unsupported grant type: {}", request.grant_type),
            StatusCode::BAD_REQUEST,
        );
        return error_response(
            StatusCode::BAD_REQUEST,
            TokenErrorResponse::unsupported_grant_type(),
        );
    }

    // Extract credentials from either body or Basic Auth header
    let (client_id, client_secret) =
        if let (Some(id), Some(secret)) = (request.client_id, request.client_secret) {
            // Credentials in body
            (id, secret)
        } else if let Some(auth) = headers.get(header::AUTHORIZATION) {
            // Try Basic Auth
            match extract_basic_auth(auth.to_str().ok()) {
                Some((id, secret)) => (id, secret),
                None => {
                    record_failure(
                        &state,
                        "invalid_auth_header",
                        None,
                        "Invalid Authorization header format".to_string(),
                        StatusCode::BAD_REQUEST,
                    );
                    return error_response(
                        StatusCode::BAD_REQUEST,
                        TokenErrorResponse::invalid_request("Invalid Authorization header format"),
                    );
                }
            }
        } else {
            record_failure(
                &state,
                "missing_credentials",
                None,
                "Missing client credentials".to_string(),
                StatusCode::BAD_REQUEST,
            );
            return error_response(
                StatusCode::BAD_REQUEST,
                TokenErrorResponse::invalid_request("Missing client credentials"),
            );
        };

    // No issued client id is this long; refuse before the id is tracked,
    // logged or looked up.
    if client_id.len() > MAX_CLIENT_ID_BYTES {
        record_failure(
            &state,
            "credential_validation_failed",
            None,
            "Client authentication failed".to_string(),
            StatusCode::UNAUTHORIZED,
        );
        return error_response(
            StatusCode::UNAUTHORIZED,
            TokenErrorResponse::invalid_client(),
        );
    }

    // Rate limit token requests per client_id
    if !state
        .token_rate_limiter
        .try_acquire(&client_id, Instant::now())
    {
        record_failure(
            &state,
            "rate_limited",
            Some(&client_id),
            "Too many token requests. Try again later.".to_string(),
            StatusCode::TOO_MANY_REQUESTS,
        );
        return error_response(
            StatusCode::TOO_MANY_REQUESTS,
            TokenErrorResponse {
                error: "rate_limit_exceeded".to_string(),
                error_description: Some("Too many token requests. Try again later.".to_string()),
            },
        );
    }

    // Verify client credentials
    let client = match state
        .client_manager
        .verify_credentials(&client_id, &client_secret)
    {
        Ok(Some(client)) => client,
        Ok(None) => {
            record_failure(
                &state,
                "credential_validation_failed",
                Some(&client_id),
                "Client authentication failed".to_string(),
                StatusCode::UNAUTHORIZED,
            );
            return error_response(
                StatusCode::UNAUTHORIZED,
                TokenErrorResponse::invalid_client(),
            );
        }
        Err(e) => {
            tracing::error!("Error verifying client credentials: {}", e);
            record_failure(
                &state,
                "verification_error",
                Some(&client_id),
                format!("Error verifying client credentials: {}", e),
                StatusCode::INTERNAL_SERVER_ERROR,
            );
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                TokenErrorResponse::invalid_request("Internal server error"),
            );
        }
    };

    // Generate access token
    match state
        .client_manager
        .issue_oauth_token(&client.id, &client_secret, &state.token_store)
    {
        Ok(Some((access_token, expires_in))) => {
            tracing::info!("Generated OAuth token for client: {}", client.id);

            (
                StatusCode::OK,
                // RFC 6749 §5.1: responses carrying tokens must not be cached.
                [
                    (header::CACHE_CONTROL, "no-store"),
                    (header::PRAGMA, "no-cache"),
                ],
                Json(TokenResponse {
                    access_token,
                    token_type: "Bearer".to_string(),
                    expires_in,
                }),
            )
                .into_response()
        }
        Ok(None) => error_response(
            StatusCode::UNAUTHORIZED,
            TokenErrorResponse::invalid_client(),
        ),
        Err(e) => {
            tracing::error!("Error generating access token: {}", e);
            record_failure(
                &state,
                "token_generation_failed",
                Some(&client.id),
                format!("Error generating access token: {}", e),
                StatusCode::INTERNAL_SERVER_ERROR,
            );
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                TokenErrorResponse::invalid_request("Failed to generate token"),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, routing::post, Router};
    use tower::ServiceExt;

    #[test]
    fn test_extract_basic_auth() {
        // Valid Basic Auth
        let auth = "Basic bHItYWJjMTIzOnNlY3JldDEyMw=="; // lr-abc123:secret123
        let result = extract_basic_auth(Some(auth));
        assert_eq!(
            result,
            Some(("lr-abc123".to_string(), "secret123".to_string()))
        );

        // Invalid format (not Basic)
        let auth = "Bearer token123";
        let result = extract_basic_auth(Some(auth));
        assert_eq!(result, None);

        // Invalid base64
        let auth = "Basic !!!invalid!!!";
        let result = extract_basic_auth(Some(auth));
        assert_eq!(result, None);

        // Missing colon
        let auth = "Basic bm9jb2xvbg=="; // "nocolon"
        let result = extract_basic_auth(Some(auth));
        assert_eq!(result, None);

        // Empty
        let result = extract_basic_auth(None);
        assert_eq!(result, None);
    }

    #[test]
    fn test_basic_auth_with_colon_in_secret() {
        // client_id:secret:with:colons
        let auth = "Basic bHItYWJjOnNlY3JldDp3aXRoOmNvbG9ucw==";
        let result = extract_basic_auth(Some(auth));
        assert_eq!(
            result,
            Some(("lr-abc".to_string(), "secret:with:colons".to_string()))
        );
    }

    fn basic(id: &str, secret: &str) -> String {
        format!("Basic {}", BASE64.encode(format!("{id}:{secret}")))
    }

    #[test]
    fn basic_auth_credentials_are_form_urldecoded() {
        // RFC 6749 §2.3.1: id and secret are form-encoded before joining.
        assert_eq!(
            extract_basic_auth(Some(&basic("my%3Aclient", "p%40ss+word%2B%25"))),
            Some(("my:client".to_string(), "p@ss word+%".to_string()))
        );
        // Multi-byte UTF-8.
        assert_eq!(
            extract_basic_auth(Some(&basic("caf%C3%A9", "s"))),
            Some(("café".to_string(), "s".to_string()))
        );
        // Issued ids and secrets are URL-safe and decode to themselves.
        let issued = "lr-AbC_d-9";
        assert_eq!(
            extract_basic_auth(Some(&basic(issued, issued))),
            Some((issued.to_string(), issued.to_string()))
        );
        // A stray `%` is kept; bytes that are not UTF-8 are refused.
        assert_eq!(
            extract_basic_auth(Some(&basic("100%", "a%zz%4"))),
            Some(("100%".to_string(), "a%zz%4".to_string()))
        );
        assert_eq!(extract_basic_auth(Some(&basic("%FF", "s"))), None);
    }

    #[tokio::test]
    async fn percent_encoded_basic_credentials_authenticate() {
        let harness = harness();
        let encode_all = |value: &str| {
            value
                .bytes()
                .map(|b| format!("%{b:02X}"))
                .collect::<String>()
        };
        let (status, _, body) = send(
            &harness,
            Some("application/x-www-form-urlencoded"),
            Some(basic(
                &encode_all(&harness.client_id),
                &encode_all(&harness.secret),
            )),
            "grant_type=client_credentials",
        )
        .await;
        assert_token_issued(&harness, status, &body);
    }

    struct Harness {
        app: Router,
        state: OAuthState,
        client_id: String,
        secret: String,
    }

    fn harness() -> Harness {
        let client_manager = Arc::new(ClientManager::with_keychain(
            Vec::new(),
            Arc::new(lr_api_keys::MockKeychain::new()),
        ));
        let (client_id, secret, _) = client_manager
            .create_client("token-test".into(), "default".into())
            .unwrap();
        let state = OAuthState {
            client_manager,
            token_store: Arc::new(TokenStore::new()),
            token_rate_limiter: Arc::new(TokenRateLimiter::default()),
            monitor_store: Arc::new(lr_monitor::MonitorEventStore::new(64)),
        };
        let app = Router::new()
            .route("/oauth/token", post(token_endpoint))
            .layer(axum::extract::DefaultBodyLimit::max(
                MAX_TOKEN_REQUEST_BYTES,
            ))
            .with_state(state.clone());
        Harness {
            app,
            state,
            client_id,
            secret,
        }
    }

    async fn send(
        harness: &Harness,
        content_type: Option<&str>,
        authorization: Option<String>,
        body: impl Into<Body>,
    ) -> (StatusCode, HeaderMap, serde_json::Value) {
        let mut request = axum::http::Request::post("/oauth/token");
        if let Some(content_type) = content_type {
            request = request.header(header::CONTENT_TYPE, content_type);
        }
        if let Some(authorization) = authorization {
            request = request.header(header::AUTHORIZATION, authorization);
        }
        let response = harness
            .app
            .clone()
            .oneshot(request.body(body.into()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
        (status, headers, json)
    }

    fn form_body(harness: &Harness) -> String {
        format!(
            "grant_type=client_credentials&client_id={}&client_secret={}",
            harness.client_id, harness.secret
        )
    }

    fn assert_token_issued(harness: &Harness, status: StatusCode, body: &serde_json::Value) {
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["token_type"], "Bearer");
        let token = body["access_token"].as_str().unwrap();
        assert!(harness.state.token_store.verify_token(token).is_some());
    }

    #[tokio::test]
    async fn form_encoded_request_issues_a_token() {
        let harness = harness();
        let (status, headers, body) = send(
            &harness,
            Some("application/x-www-form-urlencoded"),
            None,
            form_body(&harness),
        )
        .await;
        assert_token_issued(&harness, status, &body);
        assert_eq!(headers[header::CACHE_CONTROL], "no-store");
    }

    #[tokio::test]
    async fn form_content_type_parameters_and_case_are_ignored() {
        let harness = harness();
        let (status, _, body) = send(
            &harness,
            Some("Application/X-WWW-Form-Urlencoded; charset=UTF-8"),
            None,
            form_body(&harness),
        )
        .await;
        assert_token_issued(&harness, status, &body);
    }

    #[tokio::test]
    async fn json_content_type_parameters_and_case_are_ignored() {
        let harness = harness();
        let (status, _, body) = send(
            &harness,
            Some("Application/JSON; charset=utf-8"),
            None,
            serde_json::json!({
                "grant_type": "client_credentials",
                "client_id": harness.client_id,
                "client_secret": harness.secret,
            })
            .to_string(),
        )
        .await;
        assert_token_issued(&harness, status, &body);
    }

    #[tokio::test]
    async fn form_values_are_percent_decoded() {
        let harness = harness();
        let encoded: String = harness
            .secret
            .bytes()
            .map(|byte| format!("%{byte:02X}"))
            .collect();
        let (status, _, body) = send(
            &harness,
            Some("application/x-www-form-urlencoded"),
            None,
            format!(
                "grant_type=client_credentials&client_id={}&client_secret={encoded}",
                harness.client_id
            ),
        )
        .await;
        assert_token_issued(&harness, status, &body);
    }

    #[tokio::test]
    async fn json_request_still_issues_a_token() {
        let harness = harness();
        let (status, _, body) = send(
            &harness,
            Some("application/json"),
            None,
            serde_json::json!({
                "grant_type": "client_credentials",
                "client_id": harness.client_id,
                "client_secret": harness.secret,
            })
            .to_string(),
        )
        .await;
        assert_token_issued(&harness, status, &body);
    }

    #[tokio::test]
    async fn basic_auth_supplies_credentials_for_form_and_json_bodies() {
        let harness = harness();
        let basic = format!(
            "Basic {}",
            BASE64.encode(format!("{}:{}", harness.client_id, harness.secret))
        );
        let (status, _, body) = send(
            &harness,
            Some("application/x-www-form-urlencoded"),
            Some(basic.clone()),
            "grant_type=client_credentials",
        )
        .await;
        assert_token_issued(&harness, status, &body);

        let (status, _, body) = send(
            &harness,
            Some("application/json"),
            Some(basic),
            r#"{"grant_type":"client_credentials"}"#,
        )
        .await;
        assert_token_issued(&harness, status, &body);
    }

    #[tokio::test]
    async fn wrong_secret_is_rejected_in_every_encoding() {
        let harness = harness();
        let (status, _, body) = send(
            &harness,
            Some("application/x-www-form-urlencoded"),
            None,
            format!(
                "grant_type=client_credentials&client_id={}&client_secret=wrong",
                harness.client_id
            ),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(body["error"], "invalid_client");

        let (status, _, body) = send(
            &harness,
            Some("application/json"),
            None,
            serde_json::json!({
                "grant_type": "client_credentials",
                "client_id": harness.client_id,
                "client_secret": "wrong",
            })
            .to_string(),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(body["error"], "invalid_client");
    }

    #[tokio::test]
    async fn unsupported_or_missing_content_type_is_rejected() {
        let harness = harness();
        for content_type in [Some("text/plain"), Some("multipart/form-data"), None] {
            let (status, _, body) = send(&harness, content_type, None, form_body(&harness)).await;
            assert_eq!(
                status,
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "{content_type:?}"
            );
            assert_eq!(body["error"], "invalid_request");
        }
        assert_eq!(harness.state.token_rate_limiter.tracked_clients(), 0);
    }

    #[tokio::test]
    async fn malformed_bodies_are_invalid_requests() {
        let harness = harness();
        for (content_type, body) in [
            ("application/json", "{not json"),
            ("application/json", r#"{"client_id":"a"}"#),
            ("application/x-www-form-urlencoded", "client_id=a"),
            ("application/x-www-form-urlencoded", ""),
        ] {
            let (status, _, json) = send(&harness, Some(content_type), None, body).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{content_type}: {body}");
            assert_eq!(json["error"], "invalid_request");
        }
    }

    #[tokio::test]
    async fn other_grant_types_and_missing_credentials_are_rejected() {
        let harness = harness();
        let (status, _, body) = send(
            &harness,
            Some("application/x-www-form-urlencoded"),
            None,
            "grant_type=password&username=a&password=b",
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"], "unsupported_grant_type");

        let (status, _, body) = send(
            &harness,
            Some("application/x-www-form-urlencoded"),
            None,
            "grant_type=client_credentials",
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"], "invalid_request");
    }

    #[tokio::test]
    async fn oversized_body_is_refused() {
        let harness = harness();
        let (status, _, body) = send(
            &harness,
            Some("application/x-www-form-urlencoded"),
            None,
            format!(
                "grant_type=client_credentials&client_secret={}",
                "a".repeat(MAX_TOKEN_REQUEST_BYTES)
            ),
        )
        .await;
        assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
        assert_eq!(body["error"], "invalid_request");
    }

    #[tokio::test]
    async fn overlong_client_id_is_refused_without_being_tracked() {
        let harness = harness();
        let (status, _, body) = send(
            &harness,
            Some("application/x-www-form-urlencoded"),
            None,
            format!(
                "grant_type=client_credentials&client_id={}&client_secret=s",
                "a".repeat(MAX_CLIENT_ID_BYTES + 1)
            ),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(body["error"], "invalid_client");
        assert_eq!(harness.state.token_rate_limiter.tracked_clients(), 0);
    }

    #[tokio::test]
    async fn eleventh_request_in_a_minute_is_rate_limited() {
        let harness = harness();
        for _ in 0..TokenRateLimiter::MAX_REQUESTS {
            let (status, _, _) = send(
                &harness,
                Some("application/x-www-form-urlencoded"),
                None,
                form_body(&harness),
            )
            .await;
            assert_eq!(status, StatusCode::OK);
        }
        let (status, _, body) = send(
            &harness,
            Some("application/x-www-form-urlencoded"),
            None,
            form_body(&harness),
        )
        .await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(body["error"], "rate_limit_exceeded");
    }

    #[test]
    fn limiter_allows_the_window_allowance_then_recovers() {
        let limiter = TokenRateLimiter::new(8);
        let start = Instant::now();
        for _ in 0..TokenRateLimiter::MAX_REQUESTS {
            assert!(limiter.try_acquire("client", start));
        }
        assert!(!limiter.try_acquire("client", start));
        assert!(!limiter.try_acquire(
            "client",
            start + TokenRateLimiter::WINDOW - Duration::from_millis(1)
        ));
        assert!(limiter.try_acquire("client", start + TokenRateLimiter::WINDOW));
        assert_eq!(limiter.tracked_clients(), 1);
    }

    #[test]
    fn limiter_never_tracks_more_than_its_cap() {
        let limiter = TokenRateLimiter::new(16);
        let start = Instant::now();
        for i in 0..1000 {
            assert!(limiter.try_acquire(&format!("client-{i}"), start));
            assert!(limiter.tracked_clients() <= 16);
        }
        assert_eq!(limiter.tracked_clients(), 16);
    }

    #[test]
    fn limiter_drops_expired_clients_before_live_ones() {
        let limiter = TokenRateLimiter::new(3);
        let start = Instant::now();
        assert!(limiter.try_acquire("stale-a", start));
        assert!(limiter.try_acquire("stale-b", start));
        let later = start + TokenRateLimiter::WINDOW;
        assert!(limiter.try_acquire("live", later - Duration::from_secs(1)));

        assert!(limiter.try_acquire("new", later));
        assert_eq!(limiter.tracked_clients(), 2);
        // "live" kept its attempt: nine more fit, the tenth does not.
        for _ in 0..TokenRateLimiter::MAX_REQUESTS - 1 {
            assert!(limiter.try_acquire("live", later));
        }
        assert!(!limiter.try_acquire("live", later));
    }

    #[test]
    fn limiter_eviction_keeps_a_client_that_is_being_limited() {
        let limiter = TokenRateLimiter::new(4);
        let start = Instant::now();
        for _ in 0..TokenRateLimiter::MAX_REQUESTS {
            assert!(limiter.try_acquire("target", start));
        }
        // A flood of one-off ids, all newer than the target's attempts,
        // must not reset the target's count.
        for i in 0..100 {
            let now = start + Duration::from_millis(i + 1);
            assert!(limiter.try_acquire(&format!("flood-{i}"), now));
        }
        assert_eq!(limiter.tracked_clients(), 4);
        assert!(!limiter.try_acquire("target", start + Duration::from_secs(1)));
    }

    #[test]
    fn limiter_evicts_the_oldest_among_equally_active_clients() {
        let limiter = TokenRateLimiter::new(2);
        let start = Instant::now();
        assert!(limiter.try_acquire("oldest", start));
        assert!(limiter.try_acquire("newer", start + Duration::from_secs(1)));
        assert!(limiter.try_acquire("newest", start + Duration::from_secs(2)));
        assert_eq!(limiter.tracked_clients(), 2);

        // "newer" survived with its one attempt; "oldest" starts over.
        for _ in 0..TokenRateLimiter::MAX_REQUESTS - 1 {
            assert!(limiter.try_acquire("newer", start + Duration::from_secs(3)));
        }
        assert!(!limiter.try_acquire("newer", start + Duration::from_secs(3)));
    }
}
