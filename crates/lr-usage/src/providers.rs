//! What each remote provider family reports about usage, and where its API
//! lives.
//!
//! One row per family. The family id is the LocalRouter provider type of
//! that provider (so `ProviderIcon` and Settings line up), and the account
//! id prefix (`groq:api`). Findings and doc links per provider are in
//! `plan/2026-10-11-USAGE_LIMITS_ALL_PROVIDERS.md`.

use crate::types::{AccountKind, AccountRef};

/// How a family's rate-limit response headers are read. Every response is
/// read the generic way (any `x-ratelimit-*` / `anthropic-ratelimit-*`
/// triple); the scheme says what the family's names mean.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeaderScheme {
    /// No rate-limit headers documented (any that arrive are still read
    /// generically, without a known period).
    None,
    /// `x-ratelimit-{limit,remaining,reset}-<what>` named by what they
    /// count (OpenAI; Cerebras' names carry the period: `-requests-day`,
    /// `-tokens-minute`), and Anthropic's `anthropic-ratelimit-*`.
    Generic,
    /// OpenAI-style names, but `-requests` is per day and `-tokens` per
    /// minute (Groq).
    GroqDailyRequests,
    /// Per-second limits: `x-ratelimit-{limit,remaining,reset}` for
    /// requests and `x-tokenlimit-{limit,remaining}` for tokens (Together).
    TogetherPerSecond,
    /// `x-ratelimit-*-{requests,tokens-per-minute,tokens-per-day}` with
    /// resets in epoch seconds, `0` meaning none pending (DigitalOcean).
    DigitalOcean,
    /// IETF `RateLimit-Policy` / `RateLimit` structured fields (LLM Gateway).
    IetfStructured,
}

/// Which request paths on a family's hosts are its API.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathRule {
    Any,
    Prefix(&'static str),
    /// A path segment sequence anywhere in the path (Cloudflare's
    /// `/client/v4/accounts/<id>/ai/`).
    Contains(&'static str),
}

/// A remote provider family.
#[derive(Debug, Clone, Copy)]
pub struct ProviderSpec {
    /// Family id: the LocalRouter provider type.
    pub family: &'static str,
    pub label: &'static str,
    /// API hosts whose traffic bills to the family's API account.
    pub hosts: &'static [&'static str],
    /// Which paths on those hosts are the API.
    pub path: PathRule,
    /// API (pay per use) for all but flat-rate plans sold as an API.
    pub kind: AccountKind,
    pub headers: HeaderScheme,
    /// LocalRouter can ask a documented usage / credits endpoint with the
    /// provider's own API key (when the user allows it).
    pub usage_endpoint: bool,
}

impl ProviderSpec {
    /// Whether a request path on one of the family's hosts is its API.
    pub fn matches_path(&self, path: &str) -> bool {
        match self.path {
            PathRule::Any => true,
            PathRule::Prefix(p) => path.starts_with(p),
            PathRule::Contains(p) => path.contains(p),
        }
    }

    /// The family's account (login unknown).
    pub fn account(&self) -> AccountRef {
        AccountRef::new(self.family, self.kind)
    }

    /// Whether the family reports any limits (headers or an endpoint).
    pub fn reports_limits(&self) -> bool {
        self.headers != HeaderScheme::None || self.usage_endpoint
    }
}

const fn spec(
    family: &'static str,
    label: &'static str,
    hosts: &'static [&'static str],
    headers: HeaderScheme,
    usage_endpoint: bool,
) -> ProviderSpec {
    ProviderSpec {
        family,
        label,
        hosts,
        path: PathRule::Any,
        kind: AccountKind::Api,
        headers,
        usage_endpoint,
    }
}

const fn on_path(spec: ProviderSpec, path: PathRule) -> ProviderSpec {
    ProviderSpec { path, ..spec }
}

const fn subscription(spec: ProviderSpec) -> ProviderSpec {
    ProviderSpec {
        kind: AccountKind::Subscription,
        ..spec
    }
}

use HeaderScheme as H;

/// Remote providers whose usage LocalRouter attributes to an account.
/// Order matters where hosts are shared: the first matching row wins
/// (OpenCode Go's `/zen/go/` before Zen's `/zen/`).
pub const PROVIDERS: &[ProviderSpec] = &[
    spec("openai", "OpenAI", &["api.openai.com"], H::Generic, false),
    spec(
        "anthropic",
        "Anthropic",
        &["api.anthropic.com"],
        H::Generic,
        false,
    ),
    spec(
        "openrouter",
        "OpenRouter",
        &["openrouter.ai"],
        H::Generic,
        true,
    ),
    spec(
        "groq",
        "Groq",
        &["api.groq.com"],
        H::GroqDailyRequests,
        false,
    ),
    spec(
        "cerebras",
        "Cerebras",
        &["api.cerebras.ai"],
        H::Generic,
        false,
    ),
    spec(
        "mistral",
        "Mistral",
        &["api.mistral.ai", "codestral.mistral.ai"],
        H::Generic,
        false,
    ),
    spec("xai", "xAI", &["api.x.ai"], H::None, false),
    spec(
        "togetherai",
        "Together AI",
        &["api.together.xyz", "api.together.ai"],
        H::TogetherPerSecond,
        false,
    ),
    spec(
        "deepinfra",
        "DeepInfra",
        &["api.deepinfra.com"],
        H::None,
        false,
    ),
    spec(
        "gemini",
        "Gemini",
        &["generativelanguage.googleapis.com"],
        H::None,
        false,
    ),
    spec(
        "perplexity",
        "Perplexity",
        &["api.perplexity.ai"],
        H::None,
        false,
    ),
    spec(
        "cohere",
        "Cohere",
        &["api.cohere.com", "api.cohere.ai"],
        H::None,
        false,
    ),
    spec(
        "digitalocean",
        "DigitalOcean",
        &["inference.do-ai.run"],
        H::DigitalOcean,
        false,
    ),
    spec(
        "llmgateway",
        "LLM Gateway",
        &["api.llmgateway.io"],
        H::IetfStructured,
        true,
    ),
    spec(
        "vercel_ai_gateway",
        "Vercel AI Gateway",
        &["ai-gateway.vercel.sh"],
        H::None,
        true,
    ),
    spec(
        "nvidia_nim",
        "NVIDIA NIM",
        &["integrate.api.nvidia.com"],
        H::None,
        false,
    ),
    on_path(
        spec(
            "cloudflare_ai",
            "Cloudflare Workers AI",
            &["api.cloudflare.com"],
            H::None,
            false,
        ),
        PathRule::Contains("/ai/"),
    ),
    spec(
        "huggingface",
        "Hugging Face",
        &["router.huggingface.co"],
        H::None,
        false,
    ),
    spec("llm7", "LLM7", &["api.llm7.io"], H::None, false),
    spec(
        "kluster_ai",
        "kluster.ai",
        &["api.kluster.ai"],
        H::None,
        false,
    ),
    spec("zhipu", "Zhipu AI", &["open.bigmodel.cn"], H::None, false),
    spec(
        "github_models",
        "GitHub Models",
        &["models.github.ai", "models.inference.ai.azure.com"],
        H::None,
        false,
    ),
    subscription(on_path(
        spec(
            "opencode_go",
            "OpenCode Go",
            &["opencode.ai"],
            H::None,
            false,
        ),
        PathRule::Prefix("/zen/go/"),
    )),
    on_path(
        spec(
            "opencode_zen",
            "OpenCode Zen",
            &["opencode.ai"],
            H::None,
            false,
        ),
        PathRule::Prefix("/zen/"),
    ),
    spec("typesafe", "TypeSafe", &["api.typesafe.ai"], H::None, false),
    spec(
        "deepseek",
        "DeepSeek",
        &["api.deepseek.com"],
        H::None,
        false,
    ),
    spec("moonshot", "Moonshot", &["api.moonshot.ai"], H::None, false),
];

/// The spec of family `family`.
pub fn spec_for(family: &str) -> Option<&'static ProviderSpec> {
    PROVIDERS.iter().find(|s| s.family == family)
}

/// The family whose API a request to `host` (lowercase, no port) + `path`
/// is.
pub fn spec_for_request(host: &str, path: &str) -> Option<&'static ProviderSpec> {
    PROVIDERS
        .iter()
        .find(|s| s.hosts.contains(&host) && s.matches_path(path))
}

/// The usage account of the provider an API base URL points at, for
/// providers that can point anywhere (the generic OpenAI-compatible
/// provider).
pub fn account_for_base_url(base_url: &str) -> Option<AccountRef> {
    let url = reqwest::Url::parse(base_url.trim()).ok()?;
    let host = normalize_host(url.host_str()?);
    let path = format!("{}/", url.path().trim_end_matches('/'));
    spec_for_request(&host, &path).map(ProviderSpec::account)
}

/// Lowercase, without a trailing dot or a port.
pub(crate) fn normalize_host(host: &str) -> String {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    host.split(':').next().unwrap_or(&host).to_string()
}

/// API provider types with a documented usage / credits endpoint that
/// takes their own key.
pub fn polled_api_provider_types() -> Vec<&'static str> {
    PROVIDERS
        .iter()
        .filter(|s| s.usage_endpoint)
        .map(|s| s.family)
        .collect()
}

/// Provider types whose usage endpoint LocalRouter can ask (Settings →
/// Usage lists a switch for each of their instances): the subscriptions
/// LocalRouter signs in to, and [`polled_api_provider_types`].
pub fn polled_provider_types() -> Vec<&'static str> {
    let mut types = vec!["openai-chatgpt-plus", "github-copilot"];
    types.extend(polled_api_provider_types());
    types
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn families_are_unique_and_hosts_lowercase() {
        let mut families: Vec<_> = PROVIDERS.iter().map(|s| s.family).collect();
        families.sort_unstable();
        let n = families.len();
        families.dedup();
        assert_eq!(families.len(), n, "duplicate family");
        for s in PROVIDERS {
            assert!(!s.hosts.is_empty(), "{}", s.family);
            for h in s.hosts {
                assert_eq!(*h, h.to_ascii_lowercase(), "{}", s.family);
            }
        }
    }

    #[test]
    fn requests_find_their_family_by_host_and_path() {
        let family = |host: &str, path: &str| spec_for_request(host, path).map(|s| s.family);
        assert_eq!(family("api.groq.com", "/openai/v1/chat"), Some("groq"));
        assert_eq!(family("codestral.mistral.ai", "/v1/fim"), Some("mistral"));
        assert_eq!(
            family("opencode.ai", "/zen/go/v1/chat"),
            Some("opencode_go")
        );
        assert_eq!(family("opencode.ai", "/zen/v1/chat"), Some("opencode_zen"));
        assert_eq!(family("opencode.ai", "/docs"), None);
        assert_eq!(
            family("api.cloudflare.com", "/client/v4/accounts/abc/ai/v1/chat"),
            Some("cloudflare_ai")
        );
        assert_eq!(family("api.cloudflare.com", "/client/v4/zones"), None);
        assert_eq!(family("localhost", "/v1"), None);
    }

    #[test]
    fn base_urls_point_at_accounts() {
        let account = |url: &str| account_for_base_url(url).map(|a| a.id());
        assert_eq!(
            account("https://api.groq.com/openai/v1").as_deref(),
            Some("groq:api")
        );
        assert_eq!(
            account("https://API.Together.xyz:443/v1/").as_deref(),
            Some("togetherai:api")
        );
        assert_eq!(
            account("https://opencode.ai/zen/go/v1").as_deref(),
            Some("opencode_go:subscription")
        );
        assert_eq!(
            account("https://opencode.ai/zen/v1").as_deref(),
            Some("opencode_zen:api")
        );
        assert_eq!(account("http://localhost:11434/v1"), None);
        assert_eq!(account("not a url"), None);
    }

    #[test]
    fn what_each_family_reports() {
        let reports = |f: &str| spec_for(f).unwrap().reports_limits();
        assert!(reports("groq"));
        assert!(reports("openrouter"));
        assert!(reports("vercel_ai_gateway"), "credits endpoint");
        assert!(!reports("gemini"));
        assert!(!reports("perplexity"));
        assert_eq!(
            polled_api_provider_types(),
            vec!["openrouter", "llmgateway", "vercel_ai_gateway"]
        );
        let all = polled_provider_types();
        assert!(all.contains(&"github-copilot") && all.contains(&"openrouter"));
    }
}
