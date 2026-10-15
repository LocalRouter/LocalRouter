//! End-to-end detection quality tests: what the built-in rules must and must
//! not report, including false positives seen in real coding-agent traffic.

use crate::entropy::shannon_entropy;
use crate::regex_engine::RegexEngine;
use crate::types::{ExtractedText, SecretFinding};

/// Random material for fixtures. Counting sequences (`abcdef…`, `A1b2C3…`)
/// and words like "example" are treated as placeholders, so real-looking
/// tests need real-looking randomness.
const RANDOM_24: &str = "snc8ULtipEOSCG1zr0WpV03V";
const RANDOM_40: &str = "6nmCEa00cbNmH0B4zKPSYOPP5z1oYFR26doJldQr";
const RANDOM_SIG: &str = "x1b2pY8oyoZTSgellcxwMVPoksLpGNYP8gFVQLFjssu";
const GITHUB_PAT: &str = "ghp_q8Zr2LmX7vKp4NwT9sYb3HdJ6fGc1eRa0uVi";
/// Base64url of `{"alg":"HS256","typ":"JWT"}` and a small claims object.
const JWT_HEAD_AND_CLAIMS: &str =
    "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiI4ZjNrMiIsImlhdCI6MTc2MDAwMDAwMH0";
const PEM_BODY: &str = "whtWShJK3py2Nh1bWey7TsPWCMmmPRsRIs0cg240wR5nPxMgnPFkNj6Dl0IbSfxxVikEf/GcBB/TEZ5PhCH5a2Px9L8ihbSWxS+T2qPjUrE3Xtt5QSqydTGneV72XJxG";

fn engine() -> RegexEngine {
    // The user's configured global threshold.
    RegexEngine::new(3.5, &[]).unwrap()
}

fn scan(texts: &[&str]) -> Vec<SecretFinding> {
    let texts: Vec<ExtractedText> = texts
        .iter()
        .enumerate()
        .map(|(i, t)| ExtractedText {
            label: format!("user[{i}]"),
            text: t.to_string(),
            message_index: i,
        })
        .collect();
    engine().scan(&texts, None)
}

fn assert_clean(text: &str) {
    let findings = scan(&[text]);
    assert!(
        findings.is_empty(),
        "expected no findings for {text:?}, got {:?}",
        findings
            .iter()
            .map(|f| (&f.rule_id, &f.raw_text, f.entropy))
            .collect::<Vec<_>>()
    );
}

fn assert_single(text: &str, rule_id: &str, secret: &str) -> SecretFinding {
    let findings = scan(&[text]);
    assert_eq!(
        findings.len(),
        1,
        "expected exactly one finding for {text:?}, got {:?}",
        findings
            .iter()
            .map(|f| (&f.rule_id, &f.raw_text))
            .collect::<Vec<_>>()
    );
    let finding = findings.into_iter().next().unwrap();
    assert_eq!(finding.rule_id, rule_id, "wrong rule for {text:?}");
    assert_eq!(finding.raw_text, secret, "wrong secret for {text:?}");
    finding
}

// === Connection URIs ===

/// The reported false positive: a psql command line with a password-less URI.
/// The old pattern ran from the URI's `user@host:port` across the SQL to the
/// next `@` (inside an email literal) and reported 400+ chars as the password.
#[test]
fn psql_command_without_password_is_not_flagged() {
    let command = r#"psql postgres://lms@127.0.0.1:55432/app_checkout -X -c \"SELECT p.id, p.status, p.enrollment_state, p.amount, p.currency, p.raw->>'livemode' AS live_mode, (p.raw->>'signInUsedAt') IS NOT NULL AS sign_in_claimed, (SELECT count(*) FROM enrollments e WHERE e.user_id = p.user_id) AS enrollments FROM payments p JOIN users u ON u.id=p.user_id WHERE u.email='october.oneclick.2.20261014@test.invalid'\""#;
    assert_clean(command);
    // Also as the JSON-encoded tool-call arguments an agent actually sends.
    let arguments = serde_json::json!({ "cmd": command.replace("\\\"", "\"") }).to_string();
    assert_clean(&arguments);
}

/// With a real password, only the password is reported — not the command.
#[test]
fn psql_command_with_password_reports_only_the_password() {
    let password = "Vq7!pL2x9Rk4zT";
    let command = format!(
        r#"psql postgres://lms:{password}@127.0.0.1:55432/app_checkout -X -c \"SELECT * FROM users u WHERE u.email='a.b@test.invalid'\""#
    );
    let finding = assert_single(&command, "postgres-uri", password);
    assert!(
        finding.raw_text.len() < 32,
        "secret must be the password only"
    );
    assert!((finding.entropy - shannon_entropy(password)).abs() < f32::EPSILON);

    // Quoted URI inside JSON-encoded arguments.
    let arguments = serde_json::json!({
        "cmd": format!("psql \"postgresql://lms:{password}@db.internal/app\" -c \"SELECT 1\"")
    })
    .to_string();
    assert_single(&arguments, "postgres-uri", password);
}

#[test]
fn uri_rules_flag_real_passwords_in_every_scheme() {
    let password = "Hq3vT8mZ2xLp9wKr";
    for (uri, rule) in [
        (
            format!("postgres://app:{password}@db:5432/main"),
            "postgres-uri",
        ),
        (
            format!("postgresql://app:{password}@db/main"),
            "postgres-uri",
        ),
        (format!("mysql://app:{password}@db:3306/main"), "mysql-uri"),
        (
            format!("mysql+pymysql://app:{password}@db/main"),
            "mysql-uri",
        ),
        (
            format!("mongodb+srv://app:{password}@cluster0.abcd.mongodb.net/main"),
            "mongodb-uri",
        ),
        (format!("redis://:{password}@cache:6379/0"), "redis-uri"),
        (
            format!("rediss://default:{password}@cache:6380"),
            "redis-uri",
        ),
        (format!("amqp://app:{password}@mq:5672/vhost"), "amqp-uri"),
        (
            format!("ftp://deploy:{password}@files.internal/pub"),
            "uri-credentials",
        ),
    ] {
        assert_single(&format!("DATABASE_URL={uri} ./run"), rule, password);
    }
}

/// A password containing `@` is taken up to the last `@` before the host.
#[test]
fn uri_password_may_contain_at_sign() {
    assert_single(
        "postgres://app:Hq3v@T8mZ2xLp9@db/main",
        "postgres-uri",
        "Hq3v@T8mZ2xLp9",
    );
}

#[test]
fn uri_placeholder_and_default_passwords_are_not_flagged() {
    for uri in [
        "postgres://postgres:postgres@localhost:5432/postgres",
        "postgres://user:password@localhost/db",
        "postgresql://app:app@localhost/app",
        "postgres://jazyk_ci:jazyk_ci@localhost/jazyk_ci",
        "postgres://etl:warehouse@localhost/warehouse",
        "postgres://user:${POSTGRES_PASSWORD}@db/app",
        "postgres://user:$PGPASSWORD@db/app",
        "postgres://user:<password>@db/app",
        "postgres://user:***@db/app",
        "postgres://user:xxxxxxxx@db/app",
        "postgres://user:changeme@db/app",
        "postgres://user:Hq3vT8mZ2xLp9wKr@example.com/app",
        "mysql://root:secret@localhost/app",
        "mysql://root:pass@localhost/app",
        "redis://:<password>@localhost:6379",
        "redis://localhost:6379/0",
        "amqp://guest:guest@localhost:5672/",
        "mongodb://localhost:27017/app",
        "https://github.com/org/repo.git",
        "postgres://127.0.0.1:55432/app -c \"select 'x@y'\"",
    ] {
        assert_clean(uri);
    }
}

// === Entropy gating ===

/// Entropy is measured on the secret only, and keyword rules require it.
#[test]
fn keyword_rules_are_gated_by_the_secrets_entropy() {
    assert_clean(r#"client_secret = "q8q8q8q8q8q8q8q8q8q8q8q8""#);
    let finding = assert_single(
        &format!(r#"client_secret = "{RANDOM_24}""#),
        "generic-secret-assignment",
        RANDOM_24,
    );
    assert!((finding.entropy - shannon_entropy(RANDOM_24)).abs() < f32::EPSILON);
}

/// Vendor-prefixed tokens keep working at the rule's own threshold, even when
/// the global threshold is set very high.
#[test]
fn structured_prefix_rules_use_their_own_threshold() {
    let engine = RegexEngine::new(6.0, &[]).unwrap();
    let findings = engine.scan(
        &[ExtractedText {
            label: "user".into(),
            text: format!("export GITHUB_TOKEN={GITHUB_PAT}"),
            message_index: 0,
        }],
        None,
    );
    assert_eq!(findings.len(), 1, "{findings:?}");
    assert_eq!(findings[0].rule_id, "github-pat");
}

#[test]
fn jwt_entropy_is_measured_on_the_signature() {
    let real = format!("{JWT_HEAD_AND_CLAIMS}.{RANDOM_SIG}");
    let finding = assert_single(&format!("cookie: {real}"), "jwt-token", &real);
    assert!((finding.entropy - shannon_entropy(RANDOM_SIG)).abs() < f32::EPSILON);

    // Documentation stand-in signatures.
    assert_clean(&format!("{JWT_HEAD_AND_CLAIMS}.signature"));
    assert_clean(&format!("{JWT_HEAD_AND_CLAIMS}.aaaaaaaaaaaaaaaaaaaaaa"));
    // `eyJ…` strings whose header is not a JWT header.
    assert_clean(&format!(
        "eyJmb28iOiJiYXIiLCJiYXoiOjF9.eyJzdWIiOiI4ZjNrMiJ9.{RANDOM_SIG}"
    ));
}

#[test]
fn pem_header_alone_is_not_a_key() {
    assert_clean("Look for a line starting with -----BEGIN RSA PRIVATE KEY----- in the file");
    assert_clean(r#""-----BEGIN PRIVATE KEY-----\n...\n-----END PRIVATE KEY-----\n""#);

    let raw = format!("-----BEGIN PRIVATE KEY-----\n{PEM_BODY}\n-----END PRIVATE KEY-----");
    let findings = scan(&[&raw]);
    assert_eq!(findings.len(), 1, "{findings:?}");
    assert_eq!(findings[0].rule_id, "private-key-pem");

    let escaped = format!(
        r#"{{"private_key": "-----BEGIN PRIVATE KEY-----\n{PEM_BODY}\n-----END PRIVATE KEY-----\n"}}"#
    );
    let findings = scan(&[&escaped]);
    assert_eq!(findings.len(), 1, "{findings:?}");
    assert_eq!(findings[0].rule_id, "gcp-service-account");
}

// === False positives the user permanently dismissed ===

#[test]
fn dismissed_false_positives_are_not_flagged() {
    for text in [
        // generic-password-assignment: fixtures, prose, placeholders
        r#"DB_PASSWORD="your_mysql_password""#,
        r#"password: "secret-123""#,
        r#"password = "secret-456""#,
        r#"{"password": "student123"}"#,
        r#"password: 'shots-123456'"#,
        r#"old_password = "old-password-123""#,
        r#"login_password = "login-password-123""#,
        r#"password: "correct horse battery staple""#,
        r#"password: "md5 hash or null""#,
        // generic-secret-assignment / github-pat: documentation tokens
        r#"token: "ghp_your_github_token""#,
        r#"GITHUB_TOKEN="ghp_your_personal_access_token""#,
        "ghp_A1b2C3d4E5f6G7h8I9j0K1l2M3n4O5p6Q7r8",
        "ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghij",
        // bearer-token-header: placeholders and variable names
        "Authorization: Bearer llmgtw_your_api_key_here",
        "-H 'Authorization: Bearer sk-free-tier-API_KEY'",
        "-H \"Authorization: Bearer $OPENAI_API_KEY\"",
        "authorization: Bearer local-token-for-research",
        // generic-api-key-assignment
        r#"api_key = "sk-demo-key-1234567890""#,
        // jwt-token
        "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.signature",
        // AWS documentation key
        "aws_access_key_id = AKIAIOSFODNN7EXAMPLE",
        // pagination cursors are opaque, not secret
        r#"{"nextPageToken": "CiAKGjBpNDd2Nmp2Zml2cXRwYjBpOXA3"}"#,
    ] {
        assert_clean(text);
    }
}

#[test]
fn real_generic_secrets_are_still_flagged() {
    assert_single(
        r#"{"api_key": "Zx8Kq2Lw9Rt4mN7vB3pY"}"#,
        "generic-api-key-assignment",
        "Zx8Kq2Lw9Rt4mN7vB3pY",
    );
    assert_single(
        r#"export DB_PASSWORD="Xk9!mP2vQ7?zR4tL""#,
        "generic-password-assignment",
        "Xk9!mP2vQ7?zR4tL",
    );
    assert_single(
        &format!("curl -H \"Authorization: Bearer {RANDOM_24}\" https://api.internal"),
        "bearer-token-header",
        RANDOM_24,
    );
    // JSON-escaped inside tool-call arguments.
    assert_single(
        &format!(r#"{{"content":"{{\"client_secret\": \"{RANDOM_24}\"}}"}}"#),
        "generic-secret-assignment",
        RANDOM_24,
    );
}

// === Deduplication ===

/// One value, many occurrences (a tool call, its output, the transcript the
/// agent resends every turn) is one finding, at its first location.
#[test]
fn the_same_secret_is_reported_once() {
    let line = format!("export GITHUB_TOKEN={GITHUB_PAT}");
    let findings = scan(&[&line, &format!("{line}\n{line}"), &line]);
    assert_eq!(findings.len(), 1, "{findings:?}");
    assert_eq!(findings[0].rule_id, "github-pat");
    assert_eq!(findings[0].message_index, 0);
}

/// A vendor token inside a generic assignment is reported under the vendor
/// rule only, not once per matching rule.
#[test]
fn overlapping_rules_report_one_finding() {
    assert_single(
        &format!(r#"token = "{GITHUB_PAT}""#),
        "github-pat",
        GITHUB_PAT,
    );
    let jwt = format!("{JWT_HEAD_AND_CLAIMS}.{RANDOM_SIG}");
    assert_single(&format!("Authorization: Bearer {jwt}"), "jwt-token", &jwt);
    assert_single(
        &format!("https://x-access-token:{GITHUB_PAT}@github.com/org/repo.git"),
        "github-pat",
        GITHUB_PAT,
    );
}

/// When a value first matches a generic rule and later a specific one, the
/// single finding carries the specific rule but keeps the first location.
#[test]
fn dedupe_prefers_the_more_specific_rule() {
    let generic = format!(r#"client_secret = "{RANDOM_40}""#);
    let specific = format!("aws_secret_access_key = {RANDOM_40}");
    let findings = scan(&["nothing here", &generic, &specific]);
    assert_eq!(findings.len(), 1, "{findings:?}");
    assert_eq!(findings[0].rule_id, "aws-secret-access-key");
    assert_eq!(findings[0].message_index, 1);
}

/// Distinct secrets are all reported, in text order.
#[test]
fn distinct_secrets_are_all_reported_in_order() {
    let text = format!("first {GITHUB_PAT} then AKIAZ7Q3VJ2M4XK9PLR6");
    let findings = scan(&[&text]);
    let ids: Vec<&str> = findings.iter().map(|f| f.rule_id.as_str()).collect();
    assert_eq!(ids, ["github-pat", "aws-access-key-id"]);
}
