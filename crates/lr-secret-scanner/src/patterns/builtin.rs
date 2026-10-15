//! Built-in secret detection patterns derived from common secret formats
//!
//! These patterns cover the most common API key, token, and credential formats
//! across major cloud providers, AI services, version control platforms, etc.
//!
//! Order matters: rules are evaluated in list order and the first rule to
//! match a span claims it, so vendor-specific rules come before the generic
//! keyword and URI rules that would otherwise report the same value again.

use crate::filters::{self, PlaceholderPolicy};
use crate::types::SecretCategory;

/// Extra check on a regex match, run after the entropy and placeholder
/// filters. Returning false drops the match.
pub type Validator = fn(&regex::Captures<'_>) -> bool;

/// A raw pattern definition (before compilation)
pub struct PatternDef {
    pub id: &'static str,
    pub description: &'static str,
    pub regex: &'static str,
    /// Capture group holding the secret that is reported, hashed for
    /// dismissal, and (by default) measured for entropy. 0 = whole match.
    pub secret_group: usize,
    /// Capture group whose entropy gates the match, when it is not the
    /// secret itself (e.g. the random tail of a webhook URL, or a JWT's
    /// signature). None = `secret_group`.
    pub entropy_group: Option<usize>,
    /// Minimum Shannon entropy (bits/char) of the entropy group. None = the
    /// global threshold.
    pub entropy: Option<f32>,
    pub keywords: &'static [&'static str],
    pub category: SecretCategory,
    pub placeholder: PlaceholderPolicy,
    pub validate: Option<Validator>,
}

/// Shared defaults: whole match is the secret, vendor-prefix confidence.
const BASE: PatternDef = PatternDef {
    id: "",
    description: "",
    regex: "",
    secret_group: 0,
    entropy_group: None,
    entropy: None,
    keywords: &[],
    category: SecretCategory::Generic,
    placeholder: PlaceholderPolicy::Strict,
    validate: None,
};

/// Connection-URI rule body for one scheme alternation.
///
/// Matches a single URI token with a real `user:password@host` component:
/// - user: no `:`/`@`/`/`, quotes, backslashes, whitespace or brackets;
/// - password (group 2): may contain `:` and `@` (greedy to the last `@`
///   before the host) but nothing that ends a token — whitespace, quotes,
///   backslashes (JSON escapes), `<>{}()`, `/?#`. So `${VAR}` and
///   `<password>` never match, and a URI without a password cannot run on
///   into the following shell command looking for an `@`;
/// - host (group 3), optional port, optional database path (group 4).
macro_rules! uri_regex {
    ($scheme:literal) => {
        concat!(
            r"(?i)\b(?:",
            $scheme,
            r#")://([^\s:@/'"\\`<>{}()\[\]]*):([^\s'"\\`<>{}()/?#\[\]]+)@([A-Za-z0-9._-]+|\[[0-9A-Fa-f:.]+\])(?::\d{1,5})?(?:/([A-Za-z0-9._-]*))?"#
        )
    };
}

/// All built-in secret detection patterns
pub fn builtin_patterns() -> Vec<PatternDef> {
    vec![
        // === Cloud Provider Keys ===
        PatternDef {
            id: "aws-access-key-id",
            description: "AWS Access Key ID",
            regex: r"(?:^|[^A-Za-z0-9/+=])((?:AKIA|ASIA|ABIA|ACCA)[0-9A-Z]{16})(?:[^A-Za-z0-9/+=]|$)",
            secret_group: 1,
            entropy: Some(3.0),
            keywords: &["AKIA", "ASIA", "ABIA", "ACCA"],
            category: SecretCategory::CloudProvider,
            ..BASE
        },
        PatternDef {
            id: "aws-secret-access-key",
            description: "AWS Secret Access Key",
            regex: r#"(?i)(?:aws_secret_access_key|aws_secret_key|secret_access_key|secretaccesskey)\\?['"]?\s*[=:]\s*\\?['"]?([A-Za-z0-9/+=]{40})(?:[^A-Za-z0-9/+=]|$)"#,
            secret_group: 1,
            entropy: Some(4.0),
            keywords: &["aws_secret", "secret_access_key", "secretaccesskey"],
            category: SecretCategory::CloudProvider,
            ..BASE
        },
        PatternDef {
            id: "gcp-service-account",
            description: "Google Cloud Service Account Key",
            regex: r#""private_key\\?"\s*:\s*\\?"(-----BEGIN (?:RSA )?PRIVATE KEY-----[^"]+?)\\?""#,
            secret_group: 1,
            entropy: Some(4.0),
            keywords: &["private_key"],
            category: SecretCategory::CloudProvider,
            // Real keys contain runs like `AAAAA`; the material check below
            // rejects `-----BEGIN PRIVATE KEY-----\n...` stand-ins instead.
            placeholder: PlaceholderPolicy::Off,
            validate: Some(filters::pem_body_has_key_material),
            ..BASE
        },
        PatternDef {
            id: "gcp-api-key",
            description: "Google Cloud API Key",
            regex: r"\bAIza[0-9A-Za-z_-]{35}",
            entropy: Some(3.5),
            keywords: &["AIza"],
            category: SecretCategory::CloudProvider,
            ..BASE
        },
        PatternDef {
            id: "azure-storage-key",
            description: "Azure Storage Account Key",
            regex: r#"(?i)(?:AccountKey|storage[_-]?key)\\?['"]?\s*[=:]\s*\\?['"]?([A-Za-z0-9+/]{86}==)"#,
            secret_group: 1,
            entropy: Some(4.0),
            keywords: &["AccountKey", "storage_key", "storage-key", "storagekey"],
            category: SecretCategory::CloudProvider,
            ..BASE
        },
        // === AI Service Keys ===
        PatternDef {
            id: "openai-api-key",
            description: "OpenAI API Key",
            regex: r"sk-(?:(?:proj|svcacct|admin)-)?[A-Za-z0-9_-]{20,}T3BlbkFJ[A-Za-z0-9_-]{20,}",
            entropy: Some(3.5),
            keywords: &["T3BlbkFJ"],
            category: SecretCategory::AiService,
            ..BASE
        },
        PatternDef {
            id: "openai-api-key-v2",
            description: "OpenAI API Key (project-scoped)",
            regex: r"sk-(?:proj|svcacct|admin)-[A-Za-z0-9_-]{40,}",
            entropy: Some(3.5),
            keywords: &["sk-proj-", "sk-svcacct-", "sk-admin-"],
            category: SecretCategory::AiService,
            ..BASE
        },
        PatternDef {
            id: "anthropic-api-key",
            description: "Anthropic API Key",
            regex: r"sk-ant-api03-[A-Za-z0-9_-]{90,}",
            entropy: Some(3.5),
            keywords: &["sk-ant-api03-"],
            category: SecretCategory::AiService,
            ..BASE
        },
        PatternDef {
            id: "anthropic-api-key-v2",
            description: "Anthropic API Key (new format)",
            regex: r"sk-ant-[A-Za-z0-9_-]{40,}",
            entropy: Some(3.5),
            keywords: &["sk-ant-"],
            category: SecretCategory::AiService,
            ..BASE
        },
        PatternDef {
            id: "groq-api-key",
            description: "Groq API Key",
            regex: r"\bgsk_[A-Za-z0-9]{48,}",
            entropy: Some(3.5),
            keywords: &["gsk_"],
            category: SecretCategory::AiService,
            ..BASE
        },
        PatternDef {
            id: "cohere-api-key",
            description: "Cohere API Key",
            regex: r#"(?i)(?:cohere[_-]?api[_-]?key|co[_-]api[_-]key)\\?['"]?\s*[=:]\s*\\?['"]?([A-Za-z0-9]{40})(?:[^A-Za-z0-9]|$)"#,
            secret_group: 1,
            entropy: Some(3.5),
            keywords: &["cohere", "co_api", "co-api"],
            category: SecretCategory::AiService,
            placeholder: PlaceholderPolicy::Generic,
            ..BASE
        },
        PatternDef {
            id: "huggingface-token",
            description: "HuggingFace API Token",
            regex: r"\bhf_[A-Za-z0-9]{34,}",
            entropy: Some(3.5),
            keywords: &["hf_"],
            category: SecretCategory::AiService,
            ..BASE
        },
        // === Version Control ===
        PatternDef {
            id: "github-pat",
            description: "GitHub Personal Access Token",
            regex: r"\bghp_[A-Za-z0-9]{36,}",
            entropy: Some(3.5),
            keywords: &["ghp_"],
            category: SecretCategory::VersionControl,
            ..BASE
        },
        PatternDef {
            id: "github-oauth",
            description: "GitHub OAuth Access Token",
            regex: r"\bgho_[A-Za-z0-9]{36,}",
            entropy: Some(3.5),
            keywords: &["gho_"],
            category: SecretCategory::VersionControl,
            ..BASE
        },
        PatternDef {
            id: "github-app-token",
            description: "GitHub App Token",
            regex: r"\b(?:ghu|ghs|ghr)_[A-Za-z0-9]{36,}",
            entropy: Some(3.5),
            keywords: &["ghu_", "ghs_", "ghr_"],
            category: SecretCategory::VersionControl,
            ..BASE
        },
        PatternDef {
            id: "github-fine-grained-pat",
            description: "GitHub Fine-Grained Personal Access Token",
            regex: r"\bgithub_pat_[A-Za-z0-9_]{82,}",
            entropy: Some(3.5),
            keywords: &["github_pat_"],
            category: SecretCategory::VersionControl,
            ..BASE
        },
        PatternDef {
            id: "gitlab-pat",
            description: "GitLab Personal Access Token",
            regex: r"\bglpat-[A-Za-z0-9_-]{20,}",
            entropy: Some(3.5),
            keywords: &["glpat-"],
            category: SecretCategory::VersionControl,
            ..BASE
        },
        // === Database ===
        //
        // The secret is the password component only; its entropy is what
        // gates the match. URIs without a password, or with a default,
        // templated, or user/database-mirroring one, are not reported.
        PatternDef {
            id: "postgres-uri",
            description: "PostgreSQL Connection URI",
            regex: uri_regex!("postgres(?:ql)?"),
            secret_group: 2,
            entropy: Some(3.0),
            keywords: &["postgres://", "postgresql://"],
            category: SecretCategory::Database,
            placeholder: PlaceholderPolicy::Generic,
            validate: Some(filters::uri_password_is_real),
            ..BASE
        },
        PatternDef {
            id: "mysql-uri",
            description: "MySQL Connection URI",
            regex: uri_regex!(r"mysql(?:\+[a-z0-9]+)?|mariadb"),
            secret_group: 2,
            entropy: Some(3.0),
            keywords: &["mysql://", "mysql+", "mariadb://"],
            category: SecretCategory::Database,
            placeholder: PlaceholderPolicy::Generic,
            validate: Some(filters::uri_password_is_real),
            ..BASE
        },
        PatternDef {
            id: "mongodb-uri",
            description: "MongoDB Connection URI",
            regex: uri_regex!(r"mongodb(?:\+srv)?"),
            secret_group: 2,
            entropy: Some(3.0),
            keywords: &["mongodb://", "mongodb+srv://"],
            category: SecretCategory::Database,
            placeholder: PlaceholderPolicy::Generic,
            validate: Some(filters::uri_password_is_real),
            ..BASE
        },
        PatternDef {
            id: "redis-uri",
            description: "Redis Connection URI with Password",
            regex: uri_regex!("rediss?"),
            secret_group: 2,
            entropy: Some(3.0),
            keywords: &["redis://", "rediss://"],
            category: SecretCategory::Database,
            placeholder: PlaceholderPolicy::Generic,
            validate: Some(filters::uri_password_is_real),
            ..BASE
        },
        PatternDef {
            id: "amqp-uri",
            description: "AMQP (RabbitMQ) Connection URI",
            regex: uri_regex!("amqps?"),
            secret_group: 2,
            entropy: Some(3.0),
            keywords: &["amqp://", "amqps://"],
            category: SecretCategory::Database,
            placeholder: PlaceholderPolicy::Generic,
            validate: Some(filters::uri_password_is_real),
            ..BASE
        },
        // === Financial ===
        PatternDef {
            id: "stripe-secret-key",
            description: "Stripe Secret Key",
            regex: r"\bsk_(?:live|test)_[A-Za-z0-9]{24,}",
            entropy: Some(3.5),
            keywords: &["sk_live_", "sk_test_"],
            category: SecretCategory::Financial,
            ..BASE
        },
        PatternDef {
            id: "stripe-restricted-key",
            description: "Stripe Restricted Key",
            regex: r"\brk_(?:live|test)_[A-Za-z0-9]{24,}",
            entropy: Some(3.5),
            keywords: &["rk_live_", "rk_test_"],
            category: SecretCategory::Financial,
            ..BASE
        },
        // === OAuth ===
        PatternDef {
            id: "slack-bot-token",
            description: "Slack Bot Token",
            regex: r"\bxoxb-[0-9]{10,}-[0-9]{10,}-[A-Za-z0-9]{24,}",
            entropy: Some(3.0),
            keywords: &["xoxb-"],
            category: SecretCategory::OAuth,
            ..BASE
        },
        PatternDef {
            id: "slack-user-token",
            description: "Slack User Token",
            regex: r"\bxoxp-[0-9]{10,}-[0-9]{10,}-[0-9]{10,}-[A-Za-z0-9]{32}",
            entropy: Some(3.0),
            keywords: &["xoxp-"],
            category: SecretCategory::OAuth,
            ..BASE
        },
        PatternDef {
            id: "slack-webhook",
            description: "Slack Webhook URL",
            regex: r"https://hooks\.slack\.com/services/T[A-Za-z0-9]+/B[A-Za-z0-9]+/([A-Za-z0-9]{20,})",
            // The whole URL is the credential, but only its last segment is
            // random; the rest is fixed or a low-entropy workspace id.
            entropy_group: Some(1),
            entropy: Some(3.5),
            keywords: &["hooks.slack.com"],
            category: SecretCategory::OAuth,
            ..BASE
        },
        // === Generic ===
        PatternDef {
            id: "private-key-pem",
            description: "Private Key (PEM format)",
            // Header plus the key material that follows it (raw or with
            // JSON-escaped newlines). A header quoted on its own in docs is
            // not a key.
            regex: r"-----BEGIN (?:[A-Z0-9]+ )*PRIVATE KEY(?: BLOCK)?-----((?:\\[nr]|[A-Za-z0-9+/=\s])+)",
            entropy_group: Some(1),
            entropy: Some(4.0),
            keywords: &["PRIVATE KEY"],
            category: SecretCategory::Generic,
            placeholder: PlaceholderPolicy::Off,
            validate: Some(filters::pem_body_has_key_material),
            ..BASE
        },
        PatternDef {
            id: "jwt-token",
            description: "JSON Web Token",
            regex: r"\b(eyJ[A-Za-z0-9_-]{10,})\.eyJ[A-Za-z0-9_-]{10,}\.([A-Za-z0-9_-]{16,})",
            // A real signature is random; docs use `.signature` or similar.
            entropy_group: Some(2),
            entropy: Some(3.5),
            keywords: &["eyJ"],
            category: SecretCategory::Generic,
            validate: Some(filters::jwt_header_is_valid),
            ..BASE
        },
        PatternDef {
            id: "generic-api-key-assignment",
            description: "Generic API Key Assignment",
            regex: r#"(?i)([A-Za-z0-9_.-]*(?:api[_-]?key|apikey|api[_-]?secret|api[_-]?token))\\?['"]?\s*[=:]\s*\\?['"]([A-Za-z0-9_\-/.+=~]{20,256})\\?['"]"#,
            secret_group: 2,
            entropy: Some(3.5),
            keywords: &[
                "api_key",
                "apikey",
                "api-key",
                "api_secret",
                "api_token",
                "api-token",
                "api-secret",
            ],
            category: SecretCategory::Generic,
            placeholder: PlaceholderPolicy::Generic,
            validate: Some(filters::assignment_key_is_secret),
            ..BASE
        },
        PatternDef {
            id: "generic-password-assignment",
            description: "Generic Password Assignment",
            regex: r#"(?i)([A-Za-z0-9_.-]*(?:password|passwd|passphrase|pwd))\\?['"]?\s*[=:]\s*\\?['"]([^\s'"\\`]{8,64})\\?['"]"#,
            secret_group: 2,
            entropy: Some(3.2),
            keywords: &["password", "passwd", "passphrase", "pwd"],
            category: SecretCategory::Generic,
            placeholder: PlaceholderPolicy::Generic,
            ..BASE
        },
        PatternDef {
            id: "generic-secret-assignment",
            description: "Generic Secret Assignment",
            regex: r#"(?i)([A-Za-z0-9_.-]*(?:secret|token|credential))\\?['"]?\s*[=:]\s*\\?['"]([A-Za-z0-9_\-/.+=~]{20,256})\\?['"]"#,
            secret_group: 2,
            entropy: Some(3.5),
            keywords: &["secret", "token", "credential"],
            category: SecretCategory::Generic,
            placeholder: PlaceholderPolicy::Generic,
            validate: Some(filters::assignment_key_is_secret),
            ..BASE
        },
        PatternDef {
            id: "bearer-token-header",
            description: "Bearer Token in Header",
            regex: r#"(?i)authorization\\?['"]?\s*[=:]\s*\\?['"]?bearer\s+([A-Za-z0-9_\-/.+=~]{20,512})"#,
            secret_group: 1,
            entropy: Some(3.5),
            keywords: &["authorization"],
            category: SecretCategory::Generic,
            placeholder: PlaceholderPolicy::Generic,
            ..BASE
        },
        // Any other scheme with inline credentials (https://user:token@host,
        // ftp://, smtp://, ...). Last, so the specific URI and token rules
        // above claim their matches first.
        PatternDef {
            id: "uri-credentials",
            description: "Credentials in URI",
            regex: uri_regex!(r"[a-z][a-z0-9+.-]{1,30}"),
            secret_group: 2,
            entropy: Some(3.0),
            keywords: &["://"],
            category: SecretCategory::Generic,
            placeholder: PlaceholderPolicy::Generic,
            validate: Some(filters::uri_password_is_real),
            ..BASE
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_all_patterns_compile() {
        for pattern in builtin_patterns() {
            regex::Regex::new(pattern.regex)
                .unwrap_or_else(|e| panic!("Pattern '{}' failed to compile: {}", pattern.id, e));
        }
    }

    #[test]
    fn test_pattern_ids_unique() {
        let patterns = builtin_patterns();
        let mut ids: Vec<&str> = patterns.iter().map(|p| p.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), patterns.len(), "Duplicate pattern IDs found");
    }

    /// Every configured group must exist in its regex, or the rule silently
    /// never fires.
    #[test]
    fn test_capture_groups_exist() {
        for pattern in builtin_patterns() {
            let re = regex::Regex::new(pattern.regex).unwrap();
            let groups = re.captures_len();
            assert!(
                pattern.secret_group < groups,
                "{}: secret_group",
                pattern.id
            );
            if let Some(g) = pattern.entropy_group {
                assert!(g < groups, "{}: entropy_group", pattern.id);
            }
        }
    }
}
