//! Built-in secret detection patterns derived from common secret formats
//!
//! These patterns cover the most common API key, token, and credential formats
//! across major cloud providers, AI services, version control platforms, etc.

use crate::types::SecretCategory;

/// A raw pattern definition (before compilation)
pub struct PatternDef {
    pub id: &'static str,
    pub description: &'static str,
    pub regex: &'static str,
    pub secret_group: usize,
    pub entropy: Option<f32>,
    pub keywords: &'static [&'static str],
    pub category: SecretCategory,
}

/// All built-in secret detection patterns
pub fn builtin_patterns() -> Vec<PatternDef> {
    vec![
        // === Cloud Provider Keys ===
        PatternDef {
            id: "aws-access-key-id",
            description: "AWS Access Key ID",
            regex: r"(?:^|[^A-Za-z0-9/+=])(AKIA[0-9A-Z]{16})(?:[^A-Za-z0-9/+=]|$)",
            secret_group: 1,
            entropy: Some(3.0),
            keywords: &["AKIA"],
            category: SecretCategory::CloudProvider,
        },
        PatternDef {
            id: "aws-secret-access-key",
            description: "AWS Secret Access Key",
            regex: r#"(?i)(?:aws_secret_access_key|aws_secret_key|secret_access_key)\s*[=:]\s*['"]?([A-Za-z0-9/+=]{40})['"]?"#,
            secret_group: 1,
            entropy: Some(4.0),
            keywords: &["aws_secret", "secret_access_key"],
            category: SecretCategory::CloudProvider,
        },
        PatternDef {
            id: "gcp-service-account",
            description: "Google Cloud Service Account Key",
            regex: r#""private_key"\s*:\s*"(-----BEGIN (?:RSA )?PRIVATE KEY-----[^"]+)"#,
            secret_group: 1,
            entropy: Some(3.0),
            keywords: &["private_key", "BEGIN"],
            category: SecretCategory::CloudProvider,
        },
        PatternDef {
            id: "gcp-api-key",
            description: "Google Cloud API Key",
            regex: r"AIza[0-9A-Za-z_-]{35}",
            secret_group: 0,
            entropy: Some(3.5),
            keywords: &["AIza"],
            category: SecretCategory::CloudProvider,
        },
        PatternDef {
            id: "azure-storage-key",
            description: "Azure Storage Account Key",
            regex: r#"(?i)(?:AccountKey|storage[_-]?key)\s*[=:]\s*['"]?([A-Za-z0-9+/]{86}==)['"]?"#,
            secret_group: 1,
            entropy: Some(4.0),
            keywords: &["AccountKey", "storage_key", "storage-key"],
            category: SecretCategory::CloudProvider,
        },
        // === AI Service Keys ===
        PatternDef {
            id: "openai-api-key",
            description: "OpenAI API Key",
            regex: r"sk-[A-Za-z0-9]{20}T3BlbkFJ[A-Za-z0-9]{20}",
            secret_group: 0,
            entropy: Some(3.5),
            keywords: &["sk-"],
            category: SecretCategory::AiService,
        },
        PatternDef {
            id: "openai-api-key-v2",
            description: "OpenAI API Key (project-scoped)",
            regex: r"sk-proj-[A-Za-z0-9_-]{40,}",
            secret_group: 0,
            entropy: Some(3.5),
            keywords: &["sk-proj-"],
            category: SecretCategory::AiService,
        },
        PatternDef {
            id: "anthropic-api-key",
            description: "Anthropic API Key",
            regex: r"sk-ant-api03-[A-Za-z0-9_-]{90,}",
            secret_group: 0,
            entropy: Some(3.5),
            keywords: &["sk-ant-api03-"],
            category: SecretCategory::AiService,
        },
        PatternDef {
            id: "anthropic-api-key-v2",
            description: "Anthropic API Key (new format)",
            regex: r"sk-ant-[A-Za-z0-9_-]{40,}",
            secret_group: 0,
            entropy: Some(3.5),
            keywords: &["sk-ant-"],
            category: SecretCategory::AiService,
        },
        PatternDef {
            id: "groq-api-key",
            description: "Groq API Key",
            regex: r"gsk_[A-Za-z0-9]{48,}",
            secret_group: 0,
            entropy: Some(3.5),
            keywords: &["gsk_"],
            category: SecretCategory::AiService,
        },
        PatternDef {
            id: "cohere-api-key",
            description: "Cohere API Key",
            regex: r#"(?i)(?:cohere[_-]?api[_-]?key|co[_-]api[_-]key)\s*[=:]\s*['"]?([A-Za-z0-9]{40})['"]?"#,
            secret_group: 1,
            entropy: Some(3.5),
            keywords: &["cohere", "co_api"],
            category: SecretCategory::AiService,
        },
        PatternDef {
            id: "huggingface-token",
            description: "HuggingFace API Token",
            regex: r"hf_[A-Za-z0-9]{34,}",
            secret_group: 0,
            entropy: Some(3.5),
            keywords: &["hf_"],
            category: SecretCategory::AiService,
        },
        // === Version Control ===
        PatternDef {
            id: "github-pat",
            description: "GitHub Personal Access Token",
            regex: r"ghp_[A-Za-z0-9]{36,}",
            secret_group: 0,
            entropy: Some(3.5),
            keywords: &["ghp_"],
            category: SecretCategory::VersionControl,
        },
        PatternDef {
            id: "github-oauth",
            description: "GitHub OAuth Access Token",
            regex: r"gho_[A-Za-z0-9]{36,}",
            secret_group: 0,
            entropy: Some(3.5),
            keywords: &["gho_"],
            category: SecretCategory::VersionControl,
        },
        PatternDef {
            id: "github-app-token",
            description: "GitHub App Token",
            regex: r"(?:ghu|ghs|ghr)_[A-Za-z0-9]{36,}",
            secret_group: 0,
            entropy: Some(3.5),
            keywords: &["ghu_", "ghs_", "ghr_"],
            category: SecretCategory::VersionControl,
        },
        PatternDef {
            id: "github-fine-grained-pat",
            description: "GitHub Fine-Grained Personal Access Token",
            regex: r"github_pat_[A-Za-z0-9_]{82,}",
            secret_group: 0,
            entropy: Some(3.5),
            keywords: &["github_pat_"],
            category: SecretCategory::VersionControl,
        },
        PatternDef {
            id: "gitlab-pat",
            description: "GitLab Personal Access Token",
            regex: r"glpat-[A-Za-z0-9_-]{20,}",
            secret_group: 0,
            entropy: Some(3.5),
            keywords: &["glpat-"],
            category: SecretCategory::VersionControl,
        },
        // === Database ===
        PatternDef {
            id: "postgres-uri",
            description: "PostgreSQL Connection URI",
            regex: r"postgres(?:ql)?://[^:]+:([^@]+)@[^\s]+",
            secret_group: 1,
            entropy: Some(2.5),
            keywords: &["postgres://", "postgresql://"],
            category: SecretCategory::Database,
        },
        PatternDef {
            id: "mysql-uri",
            description: "MySQL Connection URI",
            regex: r"mysql://[^:]+:([^@]+)@[^\s]+",
            secret_group: 1,
            entropy: Some(2.5),
            keywords: &["mysql://"],
            category: SecretCategory::Database,
        },
        PatternDef {
            id: "mongodb-uri",
            description: "MongoDB Connection URI",
            regex: r"mongodb(?:\+srv)?://[^:]+:([^@]+)@[^\s]+",
            secret_group: 1,
            entropy: Some(2.5),
            keywords: &["mongodb://", "mongodb+srv://"],
            category: SecretCategory::Database,
        },
        PatternDef {
            id: "redis-uri",
            description: "Redis Connection URI with Password",
            regex: r"redis://[^:]*:([^@]+)@[^\s]+",
            secret_group: 1,
            entropy: Some(2.5),
            keywords: &["redis://"],
            category: SecretCategory::Database,
        },
        // === Financial ===
        PatternDef {
            id: "stripe-secret-key",
            description: "Stripe Secret Key",
            regex: r"sk_(?:live|test)_[A-Za-z0-9]{24,}",
            secret_group: 0,
            entropy: Some(3.5),
            keywords: &["sk_live_", "sk_test_"],
            category: SecretCategory::Financial,
        },
        PatternDef {
            id: "stripe-restricted-key",
            description: "Stripe Restricted Key",
            regex: r"rk_(?:live|test)_[A-Za-z0-9]{24,}",
            secret_group: 0,
            entropy: Some(3.5),
            keywords: &["rk_live_", "rk_test_"],
            category: SecretCategory::Financial,
        },
        // === OAuth ===
        PatternDef {
            id: "slack-bot-token",
            description: "Slack Bot Token",
            regex: r"xoxb-[0-9]{10,}-[0-9]{10,}-[A-Za-z0-9]{24,}",
            secret_group: 0,
            entropy: Some(3.0),
            keywords: &["xoxb-"],
            category: SecretCategory::OAuth,
        },
        PatternDef {
            id: "slack-user-token",
            description: "Slack User Token",
            regex: r"xoxp-[0-9]{10,}-[0-9]{10,}-[0-9]{10,}-[A-Za-z0-9]{32}",
            secret_group: 0,
            entropy: Some(3.0),
            keywords: &["xoxp-"],
            category: SecretCategory::OAuth,
        },
        PatternDef {
            id: "slack-webhook",
            description: "Slack Webhook URL",
            regex: r"https://hooks\.slack\.com/services/T[A-Za-z0-9]+/B[A-Za-z0-9]+/[A-Za-z0-9]+",
            secret_group: 0,
            entropy: Some(3.0),
            keywords: &["hooks.slack.com"],
            category: SecretCategory::OAuth,
        },
        // === Generic ===
        PatternDef {
            id: "private-key-pem",
            description: "Private Key (PEM format)",
            regex: r"-----BEGIN (?:RSA |EC |DSA |OPENSSH )?PRIVATE KEY-----",
            secret_group: 0,
            entropy: None,
            keywords: &["BEGIN", "PRIVATE KEY"],
            category: SecretCategory::Generic,
        },
        PatternDef {
            id: "generic-api-key-assignment",
            description: "Generic API Key Assignment",
            regex: r#"(?i)(?:api[_-]?key|apikey|api[_-]?secret|api[_-]?token)\s*[=:]\s*['\"]([A-Za-z0-9_\-/.+]{20,})['\"]"#,
            secret_group: 1,
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
        },
        PatternDef {
            id: "generic-password-assignment",
            description: "Generic Password Assignment",
            regex: r#"(?i)(?:password|passwd|pwd)\s*[=:]\s*['\"]([^'\"]{8,})['\"]"#,
            secret_group: 1,
            entropy: Some(3.0),
            keywords: &["password", "passwd", "pwd"],
            category: SecretCategory::Generic,
        },
        PatternDef {
            id: "generic-secret-assignment",
            description: "Generic Secret Assignment",
            regex: r#"(?i)(?:secret|token|credential)\s*[=:]\s*['\"]([A-Za-z0-9_\-/.+=]{20,})['\"]"#,
            secret_group: 1,
            entropy: Some(3.5),
            keywords: &["secret", "token", "credential"],
            category: SecretCategory::Generic,
        },
        PatternDef {
            id: "jwt-token",
            description: "JSON Web Token",
            regex: r"eyJ[A-Za-z0-9_-]{10,}\.eyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}",
            secret_group: 0,
            entropy: Some(4.0),
            keywords: &["eyJ"],
            category: SecretCategory::Generic,
        },
        PatternDef {
            id: "bearer-token-header",
            description: "Bearer Token in Header",
            regex: r#"(?i)(?:authorization|bearer)\s*[=:]\s*['"]?Bearer\s+([A-Za-z0-9_\-/.+=]{20,})['"]?"#,
            secret_group: 1,
            entropy: Some(3.5),
            keywords: &["Bearer", "authorization"],
            category: SecretCategory::Generic,
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
}
