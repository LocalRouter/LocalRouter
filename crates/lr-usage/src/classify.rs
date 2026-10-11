//! Which usage account a request belongs to.
//!
//! Decided from the request alone — host, path and *how* it authenticates —
//! so the same rules apply to proxied traffic and LocalRouter's own calls.
//! Tokens are only inspected for their shape (and, for ChatGPT, the plan,
//! account and email claims in the JWT); they are never stored.

use base64::Engine;
use http::HeaderMap;

use crate::types::AccountRef;

/// A classified request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Classified {
    pub account: AccountRef,
    /// Plan read off the credential, when it carries one.
    pub plan_hint: Option<String>,
    /// Who the credential belongs to (ChatGPT: the account email).
    pub login_label: Option<String>,
}

/// Usage endpoints whose responses are parsed when seen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsageEndpoint {
    ClaudeUsage,
    ClaudeProfile,
    CodexUsage,
    CopilotUser,
}

fn bearer(headers: &HeaderMap) -> Option<&str> {
    let v = headers.get(http::header::AUTHORIZATION)?.to_str().ok()?;
    let (scheme, token) = v.split_once(' ')?;
    scheme.eq_ignore_ascii_case("bearer").then(|| token.trim())
}

/// Classify a request to `host` + `path` by its headers.
pub fn classify(host: &str, path: &str, headers: &HeaderMap) -> Option<Classified> {
    let host = crate::providers::normalize_host(host);
    let host = host.as_str();
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
            login_label: None,
        });
    }
    if host == "chatgpt.com" || host.ends_with(".chatgpt.com") {
        if !path.starts_with("/backend-api") {
            return None;
        }
        let token = bearer(headers);
        // Codex and LocalRouter send the account id alongside the token too.
        let account_id = token.and_then(chatgpt_account_from_jwt).or_else(|| {
            headers
                .get("chatgpt-account-id")
                .and_then(|v| v.to_str().ok())
                .map(str::to_string)
        });
        return Some(Classified {
            account: AccountRef::chatgpt(account_id.as_deref()),
            plan_hint: token.and_then(chatgpt_plan_from_jwt),
            login_label: token.and_then(chatgpt_email_from_jwt),
        });
    }
    if host.ends_with("githubcopilot.com")
        || (host == "api.github.com" && path.starts_with("/copilot_internal"))
    {
        return Some(Classified {
            account: AccountRef::subscription("github-copilot"),
            plan_hint: None,
            login_label: None,
        });
    }
    crate::providers::spec_for_request(host, path).map(|s| Classified {
        account: s.account(),
        plan_hint: None,
        login_label: None,
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

/// A string claim under `https://api.openai.com/<section>` in a ChatGPT
/// access token. The signature is not checked: claims only label the account.
fn chatgpt_claim(token: &str, section: &str, claim: &str) -> Option<String> {
    let payload = token.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload.trim_end_matches('='))
        .ok()?;
    let claims: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    claims
        .pointer(&format!("/https:~1~1api.openai.com~1{section}/{claim}"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// The ChatGPT plan (`chatgpt_plan_type`) of a ChatGPT access token.
pub fn chatgpt_plan_from_jwt(token: &str) -> Option<String> {
    chatgpt_claim(token, "auth", "chatgpt_plan_type")
}

/// The ChatGPT account id (`chatgpt_account_id`) of a ChatGPT access token.
pub fn chatgpt_account_from_jwt(token: &str) -> Option<String> {
    chatgpt_claim(token, "auth", "chatgpt_account_id")
}

/// The email of the ChatGPT user a ChatGPT access token belongs to.
pub fn chatgpt_email_from_jwt(token: &str) -> Option<String> {
    chatgpt_claim(token, "profile", "email")
}

/// The usage account of a ChatGPT access token: its ChatGPT subscription,
/// told apart from other ChatGPT accounts by the account id claim.
pub fn chatgpt_account_for_token(token: &str) -> AccountRef {
    AccountRef::chatgpt(chatgpt_account_from_jwt(token).as_deref())
}

/// The account a LocalRouter provider type bills to, for remote providers.
/// The login is not known from the type alone (see
/// `ModelProvider::usage_account` for instances that know it).
pub fn account_for_provider_type(provider_type: &str) -> Option<AccountRef> {
    match provider_type {
        "openai-chatgpt-plus" => Some(AccountRef::subscription("openai")),
        "github-copilot" => Some(AccountRef::subscription("github-copilot")),
        other => crate::providers::spec_for(other).map(|s| s.account()),
    }
}

#[cfg(test)]
/// A ChatGPT-style access token (unsigned) with the given claims.
pub(crate) fn chatgpt_jwt(account_id: &str, plan: &str, email: Option<&str>) -> String {
    let mut claims = serde_json::json!({
        "https://api.openai.com/auth": {"chatgpt_plan_type": plan, "chatgpt_account_id": account_id}
    });
    if let Some(email) = email {
        claims["https://api.openai.com/profile"] = serde_json::json!({ "email": email });
    }
    let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(serde_json::to_vec(&claims).unwrap());
    format!("eyJhbGciOiJSUzI1NiJ9.{payload}.sig")
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
    fn chatgpt_backend_reads_plan_account_and_email_claims() {
        let token = chatgpt_jwt("acc-1", "pro", Some("me@example.com"));
        let c = classify(
            "chatgpt.com",
            "/backend-api/codex/responses",
            &headers(&[("authorization", &format!("Bearer {token}"))]),
        )
        .unwrap();
        assert_eq!(c.account, AccountRef::chatgpt(Some("acc-1")));
        assert_eq!(c.account.base_id(), "openai:subscription");
        assert_eq!(c.plan_hint.as_deref(), Some("pro"));
        assert_eq!(c.login_label.as_deref(), Some("me@example.com"));
        assert_eq!(chatgpt_account_from_jwt(&token).as_deref(), Some("acc-1"));
        assert_eq!(
            chatgpt_account_for_token(&token),
            AccountRef::chatgpt(Some("acc-1"))
        );
        assert!(classify("chatgpt.com", "/c/123", &HeaderMap::new()).is_none());
        assert_eq!(chatgpt_plan_from_jwt("not-a-jwt"), None);
        assert_eq!(chatgpt_email_from_jwt("not-a-jwt"), None);
    }

    #[test]
    fn chatgpt_accounts_are_told_apart() {
        let classify_token = |token: &str| {
            classify(
                "chatgpt.com",
                "/backend-api/codex/responses",
                &headers(&[("authorization", &format!("Bearer {token}"))]),
            )
            .unwrap()
            .account
        };
        // The same account through two logins (LocalRouter's and Codex's).
        let a1 = classify_token(&chatgpt_jwt("acc-1", "pro", None));
        let a1_again = classify_token(&chatgpt_jwt("acc-1", "pro", Some("x@y.z")));
        let a2 = classify_token(&chatgpt_jwt("acc-2", "plus", None));
        assert_eq!(a1, a1_again);
        assert_ne!(a1, a2);
        // Without an account claim the `ChatGPT-Account-Id` header decides.
        let by_header = classify(
            "chatgpt.com",
            "/backend-api/wham/usage",
            &headers(&[
                ("authorization", "Bearer opaque"),
                ("chatgpt-account-id", "acc-2"),
            ]),
        )
        .unwrap();
        assert_eq!(by_header.account, a2);
        // Neither: the family's account.
        let unknown = classify(
            "chatgpt.com",
            "/backend-api/codex/models",
            &HeaderMap::new(),
        )
        .unwrap();
        assert_eq!(unknown.account, AccountRef::subscription("openai"));
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
    fn every_api_family_is_classified_by_host() {
        for (host, path, id) in [
            ("api.groq.com", "/openai/v1/chat/completions", "groq:api"),
            ("api.cerebras.ai", "/v1/chat/completions", "cerebras:api"),
            (
                "inference.do-ai.run",
                "/v1/chat/completions",
                "digitalocean:api",
            ),
            (
                "api.llmgateway.io",
                "/v1/chat/completions",
                "llmgateway:api",
            ),
            (
                "ai-gateway.vercel.sh",
                "/v1/chat/completions",
                "vercel_ai_gateway:api",
            ),
            (
                "router.huggingface.co",
                "/v1/chat/completions",
                "huggingface:api",
            ),
            (
                "opencode.ai",
                "/zen/go/v1/chat/completions",
                "opencode_go:subscription",
            ),
        ] {
            let c = classify(host, path, &HeaderMap::new()).unwrap();
            assert_eq!(c.account.id(), id, "{host}{path}");
        }
    }

    #[test]
    fn provider_types() {
        assert_eq!(
            account_for_provider_type("openai-chatgpt-plus"),
            Some(AccountRef::subscription("openai"))
        );
        assert_eq!(account_for_provider_type("ollama"), None);
        assert_eq!(account_for_provider_type("openai_compatible"), None);
        for (t, id) in [
            ("groq", "groq:api"),
            ("digitalocean", "digitalocean:api"),
            ("vercel_ai_gateway", "vercel_ai_gateway:api"),
            ("opencode_go", "opencode_go:subscription"),
            ("typesafe", "typesafe:api"),
        ] {
            assert_eq!(account_for_provider_type(t).unwrap().id(), id);
        }
    }
}
