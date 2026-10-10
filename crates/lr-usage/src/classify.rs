//! Which usage account a request belongs to.
//!
//! Decided from the request alone — host, path and *how* it authenticates —
//! so the same rules apply to proxied traffic and LocalRouter's own calls.
//! Tokens are only inspected for their shape (and, for ChatGPT, the plan
//! claim in the JWT); they are never stored.

use base64::Engine;
use http::HeaderMap;

use crate::types::AccountRef;

/// A classified request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Classified {
    pub account: AccountRef,
    /// Plan read off the credential, when it carries one.
    pub plan_hint: Option<String>,
}

/// Usage endpoints whose responses are parsed when seen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsageEndpoint {
    ClaudeUsage,
    ClaudeProfile,
    CodexUsage,
    CopilotUser,
}

/// Remote API hosts → provider family. Local providers are not tracked.
const API_HOSTS: &[(&str, &str)] = &[
    ("api.openai.com", "openai"),
    ("openrouter.ai", "openrouter"),
    ("api.groq.com", "groq"),
    ("api.cerebras.ai", "cerebras"),
    ("api.mistral.ai", "mistral"),
    ("api.x.ai", "xai"),
    ("api.together.xyz", "togetherai"),
    ("api.together.ai", "togetherai"),
    ("api.deepinfra.com", "deepinfra"),
    ("generativelanguage.googleapis.com", "gemini"),
    ("api.perplexity.ai", "perplexity"),
    ("api.cohere.com", "cohere"),
    ("api.cohere.ai", "cohere"),
    ("api.deepseek.com", "deepseek"),
    ("api.moonshot.ai", "moonshot"),
];

fn bearer(headers: &HeaderMap) -> Option<&str> {
    let v = headers.get(http::header::AUTHORIZATION)?.to_str().ok()?;
    let (scheme, token) = v.split_once(' ')?;
    scheme.eq_ignore_ascii_case("bearer").then(|| token.trim())
}

/// Classify a request to `host` + `path` by its headers.
pub fn classify(host: &str, path: &str, headers: &HeaderMap) -> Option<Classified> {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    let host = host.split(':').next().unwrap_or(&host);
    if host == "api.anthropic.com" {
        let oauth = bearer(headers).is_some_and(|t| t.starts_with("sk-ant-oat"))
            || headers
                .get("anthropic-beta")
                .and_then(|v| v.to_str().ok())
                .is_some_and(|v| v.contains("oauth-"));
        let account = if oauth {
            AccountRef::subscription("anthropic")
        } else if headers.contains_key("x-api-key") || bearer(headers).is_some() {
            AccountRef::api("anthropic")
        } else {
            return None;
        };
        return Some(Classified {
            account,
            plan_hint: None,
        });
    }
    if host == "chatgpt.com" || host.ends_with(".chatgpt.com") {
        if !path.starts_with("/backend-api") {
            return None;
        }
        return Some(Classified {
            account: AccountRef::subscription("openai"),
            plan_hint: bearer(headers).and_then(chatgpt_plan_from_jwt),
        });
    }
    if host.ends_with("githubcopilot.com")
        || (host == "api.github.com" && path.starts_with("/copilot_internal"))
    {
        return Some(Classified {
            account: AccountRef::subscription("github-copilot"),
            plan_hint: None,
        });
    }
    API_HOSTS
        .iter()
        .find(|(h, _)| *h == host)
        .map(|(_, provider)| Classified {
            account: AccountRef::api(*provider),
            plan_hint: None,
        })
}

/// Which usage endpoint, if any, a request path is.
pub fn usage_endpoint(host: &str, path: &str) -> Option<UsageEndpoint> {
    let path = path.split('?').next().unwrap_or(path);
    match host.to_ascii_lowercase().as_str() {
        "api.anthropic.com" => match path {
            "/api/oauth/usage" => Some(UsageEndpoint::ClaudeUsage),
            "/api/oauth/profile" => Some(UsageEndpoint::ClaudeProfile),
            _ => None,
        },
        "chatgpt.com" if path == "/backend-api/wham/usage" => Some(UsageEndpoint::CodexUsage),
        "api.github.com" if path == "/copilot_internal/user" => Some(UsageEndpoint::CopilotUser),
        _ => None,
    }
}

/// A string claim under `https://api.openai.com/auth` in a ChatGPT access
/// token. The signature is not checked: claims only label the account.
fn chatgpt_auth_claim(token: &str, claim: &str) -> Option<String> {
    let payload = token.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload.trim_end_matches('='))
        .ok()?;
    let claims: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    claims
        .pointer(&format!("/https:~1~1api.openai.com~1auth/{claim}"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// The ChatGPT plan (`chatgpt_plan_type`) of a ChatGPT access token.
pub fn chatgpt_plan_from_jwt(token: &str) -> Option<String> {
    chatgpt_auth_claim(token, "chatgpt_plan_type")
}

/// The ChatGPT account id (`chatgpt_account_id`) of a ChatGPT access token.
pub fn chatgpt_account_from_jwt(token: &str) -> Option<String> {
    chatgpt_auth_claim(token, "chatgpt_account_id")
}

/// The account a LocalRouter provider type bills to, for remote providers.
pub fn account_for_provider_type(provider_type: &str) -> Option<AccountRef> {
    match provider_type {
        "openai-chatgpt-plus" => Some(AccountRef::subscription("openai")),
        "github-copilot" => Some(AccountRef::subscription("github-copilot")),
        "anthropic" | "openai" | "openrouter" | "groq" | "cerebras" | "mistral" | "xai"
        | "togetherai" | "deepinfra" | "gemini" | "perplexity" | "cohere" => {
            Some(AccountRef::api(provider_type))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::AccountKind;

    fn headers(pairs: &[(&'static str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.insert(*k, v.parse().unwrap());
        }
        h
    }

    #[test]
    fn anthropic_oauth_vs_api_key() {
        let sub = classify(
            "api.anthropic.com",
            "/v1/messages",
            &headers(&[("authorization", "Bearer sk-ant-oat01-abc")]),
        )
        .unwrap();
        assert_eq!(sub.account, AccountRef::subscription("anthropic"));
        let api = classify(
            "api.anthropic.com",
            "/v1/messages",
            &headers(&[("x-api-key", "sk-ant-api03-abc")]),
        )
        .unwrap();
        assert_eq!(api.account.kind, AccountKind::Api);
        assert!(classify("api.anthropic.com", "/v1/messages", &HeaderMap::new()).is_none());
    }

    #[test]
    fn chatgpt_backend_reads_plan_claim() {
        let claims = serde_json::json!({
            "https://api.openai.com/auth": {"chatgpt_plan_type": "pro", "chatgpt_account_id": "acc-1"}
        });
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(serde_json::to_vec(&claims).unwrap());
        let token = format!("Bearer eyJhbGciOiJSUzI1NiJ9.{payload}.sig");
        let c = classify(
            "chatgpt.com",
            "/backend-api/codex/responses",
            &headers(&[("authorization", &token)]),
        )
        .unwrap();
        assert_eq!(c.account, AccountRef::subscription("openai"));
        assert_eq!(c.plan_hint.as_deref(), Some("pro"));
        let raw = token.trim_start_matches("Bearer ");
        assert_eq!(chatgpt_account_from_jwt(raw).as_deref(), Some("acc-1"));
        assert!(classify("chatgpt.com", "/c/123", &HeaderMap::new()).is_none());
        assert_eq!(chatgpt_plan_from_jwt("not-a-jwt"), None);
    }

    #[test]
    fn api_hosts_and_local_hosts() {
        let c = classify(
            "api.openai.com:443",
            "/v1/chat/completions",
            &HeaderMap::new(),
        )
        .unwrap();
        assert_eq!(c.account, AccountRef::api("openai"));
        assert!(classify("localhost", "/v1/chat/completions", &HeaderMap::new()).is_none());
        assert_eq!(
            classify(
                "api.individual.githubcopilot.com",
                "/chat",
                &HeaderMap::new()
            )
            .unwrap()
            .account,
            AccountRef::subscription("github-copilot")
        );
    }

    #[test]
    fn usage_endpoints() {
        assert_eq!(
            usage_endpoint("api.anthropic.com", "/api/oauth/usage?x=1"),
            Some(UsageEndpoint::ClaudeUsage)
        );
        assert_eq!(
            usage_endpoint("chatgpt.com", "/backend-api/wham/usage"),
            Some(UsageEndpoint::CodexUsage)
        );
        assert_eq!(usage_endpoint("api.anthropic.com", "/v1/messages"), None);
    }

    #[test]
    fn provider_types() {
        assert_eq!(
            account_for_provider_type("openai-chatgpt-plus"),
            Some(AccountRef::subscription("openai"))
        );
        assert_eq!(account_for_provider_type("ollama"), None);
    }
}
