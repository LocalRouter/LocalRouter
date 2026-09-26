//! HTTP plumbing shared by the Hub client, the remote GGUF reader and the
//! downloader.
//!
//! All requests go through a client with automatic redirects disabled;
//! [`get_following`] follows redirects by hand so the bearer token is only
//! ever attached to trusted origins (the configured Hub endpoint and
//! `https://huggingface.co`) and never to CDN hosts.

use std::time::Duration;

use reqwest::header::{HeaderMap, HeaderName, HeaderValue, LOCATION};
use reqwest::{StatusCode, Url};

use crate::hub::HubError;

/// Maximum number of redirects followed for one request.
const MAX_REDIRECTS: usize = 10;

/// Build the redirect-disabled client used throughout the crate.
pub(crate) fn build_client() -> reqwest::Client {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(concat!("LocalRouter/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(30))
        .read_timeout(Duration::from_secs(60))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

/// Same scheme, host and port.
pub(crate) fn same_origin(a: &Url, b: &Url) -> bool {
    a.scheme() == b.scheme()
        && a.host_str().map(str::to_ascii_lowercase) == b.host_str().map(str::to_ascii_lowercase)
        && a.port_or_known_default() == b.port_or_known_default()
}

/// Whether the bearer token may be sent to `url`: only the trusted origin (the
/// configured endpoint) and `https://huggingface.co` itself.
pub(crate) fn token_allowed(url: &Url, trusted: &Url) -> bool {
    same_origin(url, trusted)
        || (url.scheme() == "https"
            && url
                .host_str()
                .is_some_and(|h| h.eq_ignore_ascii_case("huggingface.co"))
            && url.port_or_known_default() == Some(443))
}

/// Metadata the Hub reports on the first hop of a `resolve` request.
#[derive(Debug, Clone, Default)]
pub(crate) struct LinkedMeta {
    /// `x-linked-etag` without quotes (the SHA-256 of LFS files).
    pub etag: Option<String>,
    /// `x-linked-size`.
    pub size: Option<u64>,
    /// `x-repo-commit`.
    pub commit: Option<String>,
}

impl LinkedMeta {
    fn absorb(&mut self, headers: &HeaderMap) {
        let get = |name: &str| {
            headers
                .get(name)
                .and_then(|v| v.to_str().ok())
                .map(|s| {
                    s.trim()
                        .trim_start_matches("W/")
                        .trim_matches('"')
                        .to_string()
                })
                .filter(|s| !s.is_empty())
        };
        if self.etag.is_none() {
            self.etag = get("x-linked-etag");
        }
        if self.size.is_none() {
            self.size = get("x-linked-size").and_then(|s| s.parse().ok());
        }
        if self.commit.is_none() {
            self.commit = get("x-repo-commit");
        }
    }
}

/// Result of [`get_following`].
pub(crate) struct Followed {
    pub response: reqwest::Response,
    pub linked: LinkedMeta,
    pub final_url: Url,
    /// True when the final response came from an origin that is not trusted
    /// with the token (i.e. a CDN).
    pub off_origin: bool,
}

/// `GET url`, following redirects manually. The token is attached only to
/// hops for which [`token_allowed`] holds.
pub(crate) async fn get_following(
    client: &reqwest::Client,
    url: Url,
    trusted: &Url,
    token: Option<&str>,
    extra_headers: &[(HeaderName, HeaderValue)],
    timeout: Option<Duration>,
) -> Result<Followed, HubError> {
    let mut url = url;
    let mut linked = LinkedMeta::default();
    for _ in 0..=MAX_REDIRECTS {
        let mut req = client.get(url.clone());
        for (name, value) in extra_headers {
            req = req.header(name, value);
        }
        if let Some(t) = token {
            if token_allowed(&url, trusted) {
                req = req.bearer_auth(t);
            }
        }
        if let Some(t) = timeout {
            req = req.timeout(t);
        }
        let resp = req.send().await.map_err(network_error)?;
        linked.absorb(resp.headers());
        let status = resp.status();
        if status.is_redirection() && status != StatusCode::NOT_MODIFIED {
            if let Some(loc) = resp.headers().get(LOCATION).and_then(|v| v.to_str().ok()) {
                let next = url
                    .join(loc)
                    .map_err(|e| HubError::Parse(format!("bad redirect location: {e}")))?;
                if next.scheme() != "https" && next.scheme() != "http" {
                    return Err(HubError::Parse("redirect to unsupported scheme".into()));
                }
                url = next;
                continue;
            }
        }
        let off_origin = !token_allowed(&url, trusted);
        return Ok(Followed {
            response: resp,
            linked,
            final_url: url,
            off_origin,
        });
    }
    Err(HubError::Network("too many redirects".into()))
}

/// Convert a reqwest error, dropping the URL (signed CDN URLs are noisy).
pub(crate) fn network_error(e: reqwest::Error) -> HubError {
    HubError::Network(e.without_url().to_string())
}

/// Map a non-success response to a [`HubError`].
pub(crate) async fn error_from_response(resp: reqwest::Response, repo: &str) -> HubError {
    let status = resp.status().as_u16();
    let headers = resp.headers().clone();
    let body = resp.text().await.unwrap_or_default();
    map_error(status, &headers, &body, repo)
}

/// Pure error mapping (see [`HubError`]).
pub(crate) fn map_error(status: u16, headers: &HeaderMap, body: &str, repo: &str) -> HubError {
    let header = |name: &str| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string)
    };
    let code = header("x-error-code");
    let message = header("x-error-message")
        .or_else(|| {
            serde_json::from_str::<serde_json::Value>(body)
                .ok()
                .and_then(|v| v.get("error").and_then(|e| e.as_str()).map(str::to_string))
        })
        .unwrap_or_else(|| {
            let trimmed: String = body.trim().chars().take(300).collect();
            if trimmed.is_empty() {
                StatusCode::from_u16(status)
                    .ok()
                    .and_then(|s| s.canonical_reason())
                    .unwrap_or("request failed")
                    .to_string()
            } else {
                trimmed
            }
        });
    let what = if repo.is_empty() {
        message.clone()
    } else {
        repo.to_string()
    };

    match code.as_deref() {
        Some("GatedRepo") => {
            return HubError::Gated {
                repo: repo.to_string(),
                message,
                requires_login: status == 401,
            }
        }
        Some("EntryNotFound") | Some("RevisionNotFound") => return HubError::NotFound(message),
        Some("RepoNotFound") => {
            return if status == 401 {
                HubError::NotFoundOrPrivate(what)
            } else {
                HubError::NotFound(what)
            }
        }
        _ => {}
    }
    match status {
        429 => HubError::RateLimited {
            retry_after_secs: header("retry-after").and_then(|v| v.trim().parse().ok()),
        },
        401 => HubError::NotFoundOrPrivate(what),
        404 => HubError::NotFound(what),
        _ => HubError::Http { status, message },
    }
}

/// Extract the `rel="next"` URL from a `Link` header.
pub(crate) fn parse_next_link(headers: &HeaderMap) -> Option<String> {
    for value in headers.get_all(reqwest::header::LINK) {
        let Ok(value) = value.to_str() else { continue };
        for part in split_links(value) {
            let mut pieces = part.split(';');
            let Some(target) = pieces.next() else {
                continue;
            };
            let target = target.trim();
            let Some(url) = target.strip_prefix('<').and_then(|t| t.strip_suffix('>')) else {
                continue;
            };
            let is_next = pieces.any(|p| {
                let p = p.trim();
                p.strip_prefix("rel=")
                    .map(|r| r.trim_matches('"').split_whitespace().any(|r| r == "next"))
                    .unwrap_or(false)
            });
            if is_next {
                return Some(url.to_string());
            }
        }
    }
    None
}

/// Split a Link header on commas that are outside `<...>`.
fn split_links(value: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut start = 0;
    for (i, c) in value.char_indices() {
        match c {
            '<' => depth += 1,
            '>' => depth -= 1,
            ',' if depth <= 0 => {
                out.push(&value[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(&value[start..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hm(pairs: &[(&'static str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.append(*k, HeaderValue::from_str(v).unwrap());
        }
        h
    }

    #[test]
    fn token_only_for_trusted_origins() {
        let trusted = Url::parse("http://127.0.0.1:5000").unwrap();
        assert!(token_allowed(
            &Url::parse("http://127.0.0.1:5000/a").unwrap(),
            &trusted
        ));
        assert!(!token_allowed(
            &Url::parse("http://127.0.0.1:5001/a").unwrap(),
            &trusted
        ));
        assert!(token_allowed(
            &Url::parse("https://huggingface.co/api/x").unwrap(),
            &trusted
        ));
        assert!(!token_allowed(
            &Url::parse("http://huggingface.co/api/x").unwrap(),
            &trusted
        ));
        assert!(!token_allowed(
            &Url::parse("https://cdn-lfs.huggingface.co/x").unwrap(),
            &trusted
        ));
        assert!(!token_allowed(
            &Url::parse("https://huggingface.co.evil.com/x").unwrap(),
            &trusted
        ));
    }

    #[test]
    fn link_header_parsing() {
        let h = hm(&[(
            "link",
            "<https://huggingface.co/api/models?cursor=abc,def&limit=2>; rel=\"next\"",
        )]);
        assert_eq!(
            parse_next_link(&h).as_deref(),
            Some("https://huggingface.co/api/models?cursor=abc,def&limit=2")
        );
        let h = hm(&[(
            "link",
            "<https://x/prev>; rel=\"prev\", <https://x/next>; rel=\"next\"",
        )]);
        assert_eq!(parse_next_link(&h).as_deref(), Some("https://x/next"));
        let h = hm(&[("link", "<https://x/prev>; rel=\"prev\"")]);
        assert_eq!(parse_next_link(&h), None);
        assert_eq!(parse_next_link(&HeaderMap::new()), None);
    }

    #[test]
    fn error_mapping() {
        let gated = map_error(
            401,
            &hm(&[
                ("x-error-code", "GatedRepo"),
                ("x-error-message", "Access restricted"),
            ]),
            "",
            "meta-llama/Llama-3",
        );
        assert!(matches!(
            gated,
            HubError::Gated { requires_login: true, ref repo, ref message }
                if repo == "meta-llama/Llama-3" && message == "Access restricted"
        ));
        let gated = map_error(403, &hm(&[("x-error-code", "GatedRepo")]), "", "a/b");
        assert!(matches!(
            gated,
            HubError::Gated {
                requires_login: false,
                ..
            }
        ));
        assert!(matches!(
            map_error(404, &hm(&[("x-error-code", "EntryNotFound")]), "", "a/b"),
            HubError::NotFound(_)
        ));
        assert!(matches!(
            map_error(401, &hm(&[("x-error-code", "RepoNotFound")]), "", "a/b"),
            HubError::NotFoundOrPrivate(_)
        ));
        assert!(matches!(
            map_error(404, &hm(&[("x-error-code", "RepoNotFound")]), "", "a/b"),
            HubError::NotFound(_)
        ));
        assert!(matches!(
            map_error(401, &HeaderMap::new(), "", "a/b"),
            HubError::NotFoundOrPrivate(_)
        ));
        assert!(matches!(
            map_error(429, &hm(&[("retry-after", "42")]), "", ""),
            HubError::RateLimited {
                retry_after_secs: Some(42)
            }
        ));
        match map_error(500, &HeaderMap::new(), "{\"error\":\"boom\"}", "") {
            HubError::Http { status, message } => {
                assert_eq!(status, 500);
                assert_eq!(message, "boom");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn linked_meta_strips_quotes() {
        let mut m = LinkedMeta::default();
        m.absorb(&hm(&[
            ("x-linked-etag", "\"abc123\""),
            ("x-linked-size", "42"),
            ("x-repo-commit", "deadbeef"),
        ]));
        assert_eq!(m.etag.as_deref(), Some("abc123"));
        assert_eq!(m.size, Some(42));
        assert_eq!(m.commit.as_deref(), Some("deadbeef"));
    }
}
