//! Post-match filters that separate real secrets from look-alikes.
//!
//! A regex match only says "this has the shape of a secret". Most of what
//! coding agents send that has that shape is not one: documentation examples
//! (`ghp_your_token_here`), test fixtures (`password = "secret-123"`), template
//! references (`${POSTGRES_PASSWORD}`), or prose that a loose pattern ran into.
//! These filters run on the captured secret value only — never on the
//! surrounding text — and reject the obvious non-secrets.
//!
//! Two strengths, chosen per rule:
//!
//! - [`PlaceholderPolicy::Strict`] for rules anchored on a vendor prefix
//!   (`ghp_`, `sk-ant-`). The prefix already makes a match high
//!   confidence, so only values that are unmistakably fake are dropped.
//! - [`PlaceholderPolicy::Generic`] for keyword and URI rules, where the value
//!   itself is the only evidence. Adds checks for human-readable words,
//!   whitespace, environment-variable names and short counting sequences.

use base64::Engine as _;

/// How aggressively a rule discards placeholder-looking values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaceholderPolicy {
    /// No placeholder heuristics. For structured material (PEM bodies) where
    /// long runs such as `AAAAA` are normal; the rule's validator and entropy
    /// gate do the work instead.
    Off,
    /// Contextual opaque credentials (e.g. STS tokens) can contain repeated
    /// encoded runs. Reject explicit stand-ins, not their internal structure.
    Contextual,
    /// Drop only values that are unmistakably fake.
    Strict,
    /// Also drop values that read as words, identifiers, or test fixtures.
    Generic,
}

/// Words that never appear in a real credential with a vendor prefix but are
/// the stock filler of documentation and examples.
const STRICT_WORDS: &[&str] = &[
    "example",
    "sample",
    "placeholder",
    "your",
    "dummy",
    "redacted",
    "changeme",
    "replaceme",
    "replace_me",
    "insert",
];

/// Filler typical of test fixtures and docs. Only checked for keyword/URI
/// rules, where nothing but the value distinguishes a secret from prose.
const GENERIC_WORDS: &[&str] = &[
    "password",
    "passwd",
    "secret",
    "test",
    "demo",
    "fake",
    "mock",
    "default",
    "admin",
    "letmein",
    "qwerty",
    "hunter2",
    "foobar",
    "lorem",
    "token",
    "apikey",
    "api_key",
    "api-key",
    "xxx",
    "todo",
    "here",
    "null",
    "none",
    "undefined",
    "process.env",
    "environ",
    "getenv",
];

/// AWS access key IDs identify a credential but do not authenticate requests.
/// Keep them out of generic rules as well as the built-in rule list.
pub fn is_aws_access_key_id(value: &str) -> bool {
    value.len() == 20
        && ["AKIA", "ASIA", "ABIA", "ACCA"]
            .iter()
            .any(|p| value.starts_with(p))
        && value[4..]
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
}

/// Whether `value` is a placeholder rather than a real secret.
pub fn is_placeholder(value: &str, policy: PlaceholderPolicy) -> bool {
    match policy {
        PlaceholderPolicy::Off => false,
        PlaceholderPolicy::Contextual => is_explicit_placeholder(value),
        PlaceholderPolicy::Strict => is_strict_placeholder(value),
        PlaceholderPolicy::Generic => is_strict_placeholder(value) || is_generic_placeholder(value),
    }
}

// A random credential can contain "test", "here", or "your" by chance.
// Match whole components instead of silently allowing such credentials.
fn contains_placeholder_word(value: &str, words: &[&str]) -> bool {
    words.iter().any(|word| {
        value.match_indices(word).any(|(start, matched)| {
            let end = start + matched.len();
            (start == 0 || !value.as_bytes()[start - 1].is_ascii_alphanumeric())
                && (end == value.len() || !value.as_bytes()[end].is_ascii_alphanumeric())
        })
    })
}

fn is_explicit_placeholder(value: &str) -> bool {
    // Template and variable references: ${VAR}, {{var}}, $VAR, <password>, %s.
    if value.starts_with('$')
        || value.contains("${")
        || value.contains("{{")
        || value.contains("%s")
        || (value.starts_with('<') && value.ends_with('>'))
    {
        return true;
    }
    // Masked or elided values.
    if value.contains("***") || value.contains("...") || value.contains('\u{2022}') {
        return true;
    }
    let lower = value.to_ascii_lowercase();
    if matches!(
        lower.as_str(),
        "akiaiosfodnn7example" | "wjalrxutnfemi/k7mdeng/bpxrficyexamplekey"
    ) || contains_placeholder_word(&lower, STRICT_WORDS)
    {
        return true;
    }
    false
}

fn is_strict_placeholder(value: &str) -> bool {
    if is_explicit_placeholder(value) {
        return true;
    }
    // xxxxx / 00000 / AAAAA filler.
    if longest_repeat_run(value) >= 5 {
        return true;
    }
    // abcdef / 123456, or a whole value built from counting sequences such as
    // A1b2C3d4E5f6 (the letters and the digits each count upward).
    longest_sequential_run(value) >= 6 || is_counting_pattern(value)
}

fn is_generic_placeholder(value: &str) -> bool {
    // Credentials don't contain whitespace; descriptions and prose do.
    if value.chars().any(char::is_whitespace) {
        return true;
    }
    let lower = value.to_ascii_lowercase();
    if contains_placeholder_word(&lower, GENERIC_WORDS) {
        return true;
    }
    // URLs and filesystem paths, not secrets.
    if value.contains("://") || value.starts_with('/') || value.starts_with("~/") {
        return true;
    }
    // Only digits: ids, timestamps, counters.
    if value.chars().all(|c| c.is_ascii_digit()) {
        return true;
    }
    // Only lowercase words (warehouse, jazyk_ci, local-dev-research) or only an
    // upper-case identifier (OPENAI_API_KEY): names, not random material. A
    // random token of this length essentially never lacks digits and mixed case.
    let wordy = |upper: bool| {
        value.chars().all(|c| {
            (if upper {
                c.is_ascii_uppercase()
            } else {
                c.is_ascii_lowercase()
            }) || matches!(c, '-' | '_' | '.')
        })
    };
    if wordy(false) || wordy(true) {
        return true;
    }
    // References to an environment variable: sk-...-API_KEY, MY_SERVICE_TOKEN.
    if ends_with_env_suffix(value) || has_upper_snake_identifier(value) {
        return true;
    }
    // Counting sequences: short human-chosen values (secret-123, student123,
    // shots-123456) or long values with an obvious run.
    let digit_run = longest_digit_sequence(value);
    if value.chars().count() <= 16 {
        digit_run >= 3
    } else {
        digit_run >= 5 || longest_sequential_run(value) >= 5
    }
}

/// Longest run of one repeated character.
fn longest_repeat_run(value: &str) -> usize {
    let mut best = 0;
    let mut run = 0;
    let mut prev = None;
    for c in value.chars() {
        if Some(c) == prev {
            run += 1;
        } else {
            run = 1;
            prev = Some(c);
        }
        best = best.max(run);
    }
    best
}

/// Longest run of adjacent ascending characters within one class
/// (`abcdef`, `ABCD`, `12345`).
fn longest_sequential_run(value: &str) -> usize {
    let bytes = value.as_bytes();
    let mut best = usize::from(!bytes.is_empty());
    let mut run = 1;
    for pair in bytes.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        let same_class = (a.is_ascii_digit() && b.is_ascii_digit())
            || (a.is_ascii_lowercase() && b.is_ascii_lowercase())
            || (a.is_ascii_uppercase() && b.is_ascii_uppercase());
        if same_class && b == a + 1 {
            run += 1;
            best = best.max(run);
        } else {
            run = 1;
        }
    }
    best
}

/// Longest run of adjacent digits counting up or down by one (`123`, `9876`).
fn longest_digit_sequence(value: &str) -> usize {
    let bytes = value.as_bytes();
    let mut best = 0;
    let mut run = 0;
    let mut dir: i16 = 0;
    for (i, &b) in bytes.iter().enumerate() {
        if !b.is_ascii_digit() {
            run = 0;
            dir = 0;
            continue;
        }
        let prev = i.checked_sub(1).map(|j| bytes[j]);
        match prev {
            Some(p) if p.is_ascii_digit() => {
                let step = i16::from(b) - i16::from(p);
                if (step == 1 || step == -1) && (dir == 0 || dir == step) {
                    run += 1;
                    dir = step;
                } else if step == 1 || step == -1 {
                    run = 2;
                    dir = step;
                } else {
                    run = 1;
                    dir = 0;
                }
            }
            _ => {
                run = 1;
                dir = 0;
            }
        }
        best = best.max(run);
    }
    best
}

/// Whether the letters and the digits of `value` each mostly count upward,
/// e.g. `ghp_A1b2C3d4E5f6G7h8I9j0K1l2M3n4O5p6Q7r8`. Random tokens advance by
/// exactly one only ~1/26 (letters) or ~1/10 (digits) of the time.
fn is_counting_pattern(value: &str) -> bool {
    let letters: Vec<u8> = value
        .bytes()
        .filter(u8::is_ascii_alphabetic)
        .map(|b| b.to_ascii_lowercase())
        .collect();
    let digits: Vec<u8> = value.bytes().filter(u8::is_ascii_digit).collect();

    let fraction = |seq: &[u8], wraps: bool| -> f32 {
        if seq.len() < 2 {
            return 0.0;
        }
        let steps = seq
            .windows(2)
            .filter(|w| w[1] == w[0] + 1 || (wraps && w[0] == b'9' && w[1] == b'0'))
            .count();
        steps as f32 / (seq.len() - 1) as f32
    };

    (letters.len() >= 8 && fraction(&letters, false) >= 0.6)
        || (digits.len() >= 6 && fraction(&digits, true) >= 0.6)
}

/// `..._KEY`, `..._TOKEN`, … — the value names a variable instead of holding one.
fn ends_with_env_suffix(value: &str) -> bool {
    ["_KEY", "_TOKEN", "_SECRET", "_PASSWORD", "_PASS", "_PWD"]
        .iter()
        .any(|s| value.ends_with(s))
}

/// Three or more upper-case words joined by underscores (`YOUR_API_KEY`).
fn has_upper_snake_identifier(value: &str) -> bool {
    value
        .split(|c: char| !(c.is_ascii_uppercase() || c == '_'))
        .any(|segment| segment.split('_').filter(|word| word.len() >= 2).count() >= 3)
}

/// Passwords in a connection URI that are defaults or mirror another part of
/// the URI (`postgres://app:app@…`, `…/warehouse` with password `warehouse`).
const URI_DEFAULT_PASSWORDS: &[&str] = &[
    "postgres",
    "postgresql",
    "mysql",
    "mariadb",
    "mongo",
    "mongodb",
    "redis",
    "rabbitmq",
    "root",
    "guest",
    "pass",
    "user",
    "username",
    "pwd",
];

/// Validator for URI rules. Capture groups: 1 = user, 2 = password, 3 = host,
/// 4 = path (database name), when present.
pub fn uri_password_is_real(caps: &regex::Captures<'_>) -> bool {
    let Some(password) = caps.get(2).map(|m| m.as_str()) else {
        return false;
    };
    let user = caps.get(1).map_or("", |m| m.as_str());
    let host = caps.get(3).map_or("", |m| m.as_str());
    let database = caps.get(4).map_or("", |m| m.as_str());

    let lower = password.to_ascii_lowercase();
    if URI_DEFAULT_PASSWORDS.contains(&lower.as_str()) {
        return false;
    }
    if lower == user.to_ascii_lowercase() || (!database.is_empty() && password == database) {
        return false;
    }
    // Documentation hosts.
    let host = host.to_ascii_lowercase();
    !(host.contains("example") || host.contains("your") || host == "host" || host == "hostname")
}

/// RFC 7617 encodes user:password, not encrypted material. Check the decoded
/// password so base64-encoded documentation examples do not become findings.
pub fn basic_auth_has_secret_password(caps: &regex::Captures<'_>) -> bool {
    let Some(encoded) = caps.get(1).map(|m| m.as_str()) else {
        return false;
    };
    let Ok(bytes) = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .or_else(|_| base64::engine::general_purpose::STANDARD_NO_PAD.decode(encoded))
    else {
        return false;
    };
    let Ok(decoded) = std::str::from_utf8(&bytes) else {
        return false;
    };
    decoded.split_once(':').is_some_and(|(_, password)| {
        password.chars().count() >= 8
            && crate::entropy::shannon_entropy(password) >= 3.0
            && !is_placeholder(password, PlaceholderPolicy::Generic)
    })
}

/// Validator for the JWT rule: the header segment must decode to a JSON
/// object naming an algorithm. Rejects `eyJ`-prefixed strings that are not
/// tokens. Capture group 1 = header segment.
pub fn jwt_header_is_valid(caps: &regex::Captures<'_>) -> bool {
    let Some(header) = caps.get(1).map(|m| m.as_str()) else {
        return false;
    };
    let Ok(decoded) =
        base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(header.trim_end_matches('='))
    else {
        return false;
    };
    serde_json::from_slice::<serde_json::Value>(&decoded).is_ok_and(|v| v.get("alg").is_some())
}

/// Validator for PEM private keys: the body must hold a real amount of key
/// material, not just a header quoted in documentation. Capture group 1 = the
/// key text (body, optionally with its header; 64+ base64 characters needed).
pub fn pem_body_has_key_material(caps: &regex::Captures<'_>) -> bool {
    let Some(body) = caps.get(1).map(|m| m.as_str()) else {
        return false;
    };
    let body = body.replace("\\n", "").replace("\\r", "");
    body.chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '='))
        .count()
        >= 64
}

/// Words in an assignment key that mark its value as opaque but not secret:
/// pagination cursors, CSRF nonces, counters, names and references.
const NON_SECRET_KEY_WORDS: &[&str] = &[
    "page",
    "next",
    "cursor",
    "continuation",
    "csrf",
    "xsrf",
    "count",
    "limit",
    "max",
    "type",
    "name",
    "path",
    "file",
    "url",
    "uri",
    "id",
    "arn",
    "ref",
    "hash",
    "length",
    "expiry",
    "expires",
    "expiration",
    "ttl",
    "public",
    "publishable",
];

/// Split an identifier into lowercase words on `_`, `-`, `.` and camelCase
/// boundaries: `nextPageToken` -> next, page, token.
fn key_words(key: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut prev_lower = false;
    for c in key.chars() {
        if !c.is_ascii_alphanumeric() {
            if !current.is_empty() {
                words.push(std::mem::take(&mut current));
            }
            prev_lower = false;
            continue;
        }
        if c.is_ascii_uppercase() && prev_lower && !current.is_empty() {
            words.push(std::mem::take(&mut current));
        }
        prev_lower = c.is_ascii_lowercase() || c.is_ascii_digit();
        current.push(c.to_ascii_lowercase());
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
}

/// Validator for keyword assignment rules whose key is capture group 1.
pub fn assignment_key_is_secret(caps: &regex::Captures<'_>) -> bool {
    let Some(key) = caps.get(1) else {
        return true;
    };
    !key_words(key.as_str())
        .iter()
        .any(|w| NON_SECRET_KEY_WORDS.contains(&w.as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strict_rejects_unmistakable_placeholders() {
        for value in [
            "AKIAIOSFODNN7EXAMPLE",
            "ghp_your_github_token_here_0000000000",
            "ghp_A1b2C3d4E5f6G7h8I9j0K1l2M3n4O5p6Q7r8",
            "ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghij",
            // Built at run time so the fixture is not a Stripe-shaped literal.
            format!("sk_{}_{}", "live", "x".repeat(24)).as_str(),
            "${POSTGRES_PASSWORD}",
            "$PGPASSWORD",
            "<password>",
            "sk-proj-****************",
            "{{ api_key }}",
        ] {
            assert!(
                is_placeholder(value, PlaceholderPolicy::Strict),
                "{value} should be a placeholder"
            );
        }
    }

    #[test]
    fn strict_keeps_random_tokens() {
        for value in [
            "ghp_q8Zr2LmX7vKp4NwT9sYb3HdJ6fGc1eRa0uVi",
            "AKIAZ7Q3VJ2M4XK9PLR6",
            "sk-ant-api03-Rk9vQ2mZpL7xT4wN8bY3jH6fC1dS0gUeKiOaMnBtVcXz",
            format!(
                "sk_{}_{}",
                "test", "51HqRsT2eZvKYlo2C8Nf3xQpL9mWdJ4bV7rTsYk"
            )
            .as_str(),
        ] {
            assert!(
                !is_placeholder(value, PlaceholderPolicy::Strict),
                "{value} should not be a placeholder"
            );
        }
    }

    #[test]
    fn generic_rejects_fixture_values_from_real_false_positives() {
        for value in [
            "your_mysql_password",
            "secret-123",
            "secret-456",
            "student123",
            "shots-123456",
            "login-password-123",
            "old-password-123",
            "correct horse battery staple",
            "md5 hash or null",
            "warehouse",
            "jazyk_ci",
            "local-token-for-research",
            "llmgtw_your_api_key_here",
            "sk-free-tier-API_KEY",
            "sk-demo-key-1234567890",
            "OPENAI_API_KEY",
            "12345678901234567890",
        ] {
            assert!(
                is_placeholder(value, PlaceholderPolicy::Generic),
                "{value} should be a placeholder"
            );
        }
    }

    #[test]
    fn generic_keeps_random_values() {
        for value in [
            "Xk9#mP2vQ7!zR4tL",
            "wJalrXUtnFEMIK7MDENGbPxRfiCYzQ8vT1aB3cD5",
            "f3a9c1d7e5b2408697a1c3e5f7092b4d",
            "q8Zr2LmX7vKp4NwT9sYb",
        ] {
            assert!(
                !is_placeholder(value, PlaceholderPolicy::Generic),
                "{value} should not be a placeholder"
            );
        }
    }

    #[test]
    fn digit_sequences_count_both_directions() {
        assert_eq!(longest_digit_sequence("a123b"), 3);
        assert_eq!(longest_digit_sequence("x9876"), 4);
        assert_eq!(longest_digit_sequence("2026"), 1);
        assert_eq!(longest_digit_sequence("1210"), 3);
        assert_eq!(longest_digit_sequence(""), 0);
    }

    #[test]
    fn key_words_split_on_separators_and_camel_case() {
        assert_eq!(key_words("nextPageToken"), ["next", "page", "token"]);
        assert_eq!(key_words("csrf_token"), ["csrf", "token"]);
        assert_eq!(key_words("security-token"), ["security", "token"]);
        assert_eq!(key_words("API_KEY"), ["api", "key"]);
    }

    #[test]
    fn placeholder_policy_off_never_rejects() {
        assert!(!is_placeholder("AAAAAAAA-----", PlaceholderPolicy::Off));
    }

    #[test]
    fn jwt_header_must_decode_to_an_algorithm() {
        let re = regex::Regex::new(r"(eyJ[A-Za-z0-9_-]+)\.").unwrap();
        // {"alg":"HS256","typ":"JWT"}
        let caps = re
            .captures("eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.")
            .unwrap();
        assert!(jwt_header_is_valid(&caps));
        // {"foo":"bar"}
        let caps = re.captures("eyJmb28iOiJiYXIifQ.").unwrap();
        assert!(!jwt_header_is_valid(&caps));
    }
}
