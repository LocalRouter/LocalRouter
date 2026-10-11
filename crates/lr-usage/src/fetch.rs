//! Usage endpoint clients. Each takes a credential and returns a parsed
//! report; callers decide when (and whether) to ask.

use std::time::Duration;

use serde_json::Value;

use crate::bodies;
use crate::types::UsageReport;

const CLAUDE_USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
const CLAUDE_PROFILE_URL: &str = "https://api.anthropic.com/api/oauth/profile";
const CODEX_USAGE_URL: &str = "https://chatgpt.com/backend-api/wham/usage";
const COPILOT_USER_URL: &str = "https://api.github.com/copilot_internal/user";

#[derive(Debug, Clone, PartialEq)]
pub enum FetchError {
    /// The credential was rejected (expired, revoked, or lacks the scope).
    Unauthorized,
    /// The endpoint throttled us; retry after this many seconds if given.
    RateLimited(Option<u64>),
    Http(u16),
    Network(String),
    /// The body was not a usage response.
    Unrecognized,
}

impl std::fmt::Display for FetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FetchError::Unauthorized => write!(f, "login rejected"),
            FetchError::RateLimited(Some(s)) => write!(f, "rate limited (retry in {s}s)"),
            FetchError::RateLimited(None) => write!(f, "rate limited"),
            FetchError::Http(code) => write!(f, "HTTP {code}"),
            FetchError::Network(e) => write!(f, "network error: {e}"),
            FetchError::Unrecognized => write!(f, "unrecognized response"),
        }
    }
}

/// A plain client for usage polls (no provider middleware, so polls are not
/// mistaken for model traffic).
///
/// Never proxied: the process may inherit `HTTPS_PROXY` pointing at a
/// LocalRouter HTTPS proxy, and polls must go straight to the provider.
pub fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .no_proxy()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(20))
        .user_agent(concat!("LocalRouter/", env!("CARGO_PKG_VERSION")))
        .build()
        .unwrap_or_default()
}

async fn get_json(req: reqwest::RequestBuilder) -> Result<Value, FetchError> {
    let resp = req
        .send()
        .await
        .map_err(|e| FetchError::Network(e.to_string()))?;
    json_body(resp).await
}

/// A usage endpoint's response as JSON, or the error its status means.
/// Shared with providers that ask their usage endpoint through their own
/// HTTP client.
pub async fn json_body(resp: reqwest::Response) -> Result<Value, FetchError> {
    check_status(resp.status().as_u16(), resp.headers())?;
    resp.json::<Value>()
        .await
        .map_err(|_| FetchError::Unrecognized)
}

/// The error an unsuccessful usage-endpoint status means.
pub fn check_status(status: u16, headers: &http::HeaderMap) -> Result<(), FetchError> {
    match status {
        200..=299 => Ok(()),
        401 | 403 => Err(FetchError::Unauthorized),
        429 => {
            let retry = headers
                .get(http::header::RETRY_AFTER)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| parse_retry_after(v, chrono::Utc::now().timestamp()));
            Err(FetchError::RateLimited(retry))
        }
        other => Err(FetchError::Http(other)),
    }
}

/// `Retry-After` as delay-seconds or an HTTP-date, in seconds from `now`.
pub fn parse_retry_after(value: &str, now: i64) -> Option<u64> {
    let v = value.trim();
    if let Ok(secs) = v.parse::<u64>() {
        return Some(secs);
    }
    let at = chrono::DateTime::parse_from_rfc2822(v).ok()?.timestamp();
    Some((at - now).max(0) as u64)
}

/// Claude subscription usage (`/api/oauth/usage`) with a Claude OAuth token.
pub async fn fetch_claude_usage(
    client: &reqwest::Client,
    token: &str,
) -> Result<UsageReport, FetchError> {
    let body = get_json(
        client
            .get(CLAUDE_USAGE_URL)
            .bearer_auth(token)
            .header("anthropic-beta", "oauth-2025-04-20")
            .header("Accept", "application/json"),
    )
    .await?;
    bodies::parse_claude_usage(&body).ok_or(FetchError::Unrecognized)
}

/// Claude plan tier (`/api/oauth/profile`).
pub async fn fetch_claude_plan(
    client: &reqwest::Client,
    token: &str,
) -> Result<Option<String>, FetchError> {
    let body = get_json(
        client
            .get(CLAUDE_PROFILE_URL)
            .bearer_auth(token)
            .header("anthropic-beta", "oauth-2025-04-20")
            .header("Accept", "application/json"),
    )
    .await?;
    Ok(bodies::parse_claude_profile(&body))
}

/// ChatGPT subscription usage (`/backend-api/wham/usage`).
pub async fn fetch_codex_usage(
    client: &reqwest::Client,
    token: &str,
    account_id: Option<&str>,
) -> Result<UsageReport, FetchError> {
    let mut req = client
        .get(CODEX_USAGE_URL)
        .bearer_auth(token)
        .header("Accept", "application/json");
    if let Some(id) = account_id.filter(|s| !s.is_empty()) {
        req = req.header("ChatGPT-Account-Id", id);
    }
    let body = get_json(req).await?;
    bodies::parse_codex_usage(&body, crate::tracker::now_secs()).ok_or(FetchError::Unrecognized)
}

/// GitHub Copilot quotas (`copilot_internal/user`) with the GitHub OAuth token.
pub async fn fetch_copilot_usage(
    client: &reqwest::Client,
    github_token: &str,
) -> Result<UsageReport, FetchError> {
    let body = get_json(
        client
            .get(COPILOT_USER_URL)
            .header("Authorization", format!("token {github_token}"))
            .header("Accept", "application/json")
            .header("X-Github-Api-Version", "2025-04-01"),
    )
    .await?;
    bodies::parse_copilot_user(&body).ok_or(FetchError::Unrecognized)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_after_seconds_and_http_date() {
        assert_eq!(parse_retry_after(" 120 ", 0), Some(120));
        let now = chrono::DateTime::parse_from_rfc2822("Sat, 10 Oct 2026 12:00:00 GMT")
            .unwrap()
            .timestamp();
        assert_eq!(
            parse_retry_after("Sat, 10 Oct 2026 12:05:00 GMT", now),
            Some(300)
        );
        assert_eq!(
            parse_retry_after("Sat, 10 Oct 2026 11:00:00 GMT", now),
            Some(0)
        );
        assert_eq!(parse_retry_after("soon", now), None);
    }

    #[test]
    fn status_errors() {
        let mut h = http::HeaderMap::new();
        assert_eq!(check_status(200, &h), Ok(()));
        assert_eq!(check_status(401, &h), Err(FetchError::Unauthorized));
        assert_eq!(check_status(403, &h), Err(FetchError::Unauthorized));
        assert_eq!(check_status(500, &h), Err(FetchError::Http(500)));
        assert_eq!(check_status(429, &h), Err(FetchError::RateLimited(None)));
        h.insert("retry-after", "30".parse().unwrap());
        assert_eq!(
            check_status(429, &h),
            Err(FetchError::RateLimited(Some(30)))
        );
    }
}
