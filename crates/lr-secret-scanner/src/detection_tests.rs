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
    let text = format!("first {GITHUB_PAT} then aws_secret_access_key={RANDOM_40}");
    let findings = scan(&[&text]);
    let ids: Vec<&str> = findings.iter().map(|f| f.rule_id.as_str()).collect();
    assert_eq!(ids, ["github-pat", "aws-secret-access-key"]);
}

// AWS key IDs never authenticate by themselves, including when a generic
// assignment would otherwise identify the field as sensitive.
#[test]
fn aws_identifiers_are_not_secrets() {
    for prefix in ["AKIA", "ASIA", "ABIA", "ACCA"] {
        let id = format!("{prefix}Z7Q3VJ2M4XK9PLR6");
        for text in [
            id.clone(),
            format!("AWS_ACCESS_KEY_ID={id}"),
            format!(r#"api_key="{id}""#),
            format!(r#"token="{id}""#),
            format!("Authorization: Bearer {id}"),
        ] {
            assert_clean(&text);
        }
    }
}

#[test]
fn aws_secrets_work_across_config_spellings_and_encodings() {
    for key in [
        "AWS_SECRET_ACCESS_KEY",
        "aws-secret-access-key",
        "awsSecretAccessKey",
        "SecretAccessKey",
        "secret_access_key",
        "AWS_SECRET_KEY",
        "AWS Secret Access Key",
    ] {
        for text in [
            format!("export {key}={RANDOM_40}"),
            format!("{key}: '{RANDOM_40}'"),
            serde_json::json!({key: RANDOM_40}).to_string(),
            serde_json::json!({"arguments": serde_json::json!({key: RANDOM_40}).to_string()})
                .to_string(),
        ] {
            assert_single(&text, "aws-secret-access-key", RANDOM_40);
        }
    }
    // The key ID is left intact when a paired secret is redacted, everywhere.
    let text = format!(
        "AWS_ACCESS_KEY_ID=AKIAZ7Q3VJ2M4XK9PLR6\nAWS_SECRET_ACCESS_KEY={RANDOM_40}\n{RANDOM_40}"
    );
    let finding = assert_single(&text, "aws-secret-access-key", RANDOM_40);
    let mut redaction = crate::redaction::Redaction::default();
    redaction.add(&finding.raw_text);
    let output = redaction.apply_text(&text);
    assert!(output.contains("AKIAZ7Q3VJ2M4XK9PLR6"));
    assert!(!output.contains(RANDOM_40));
    assert_eq!(
        output.matches(crate::redaction::REDACTION_MESSAGE).count(),
        2
    );
    assert_clean(&format!("aws_secret_access_key={}x", RANDOM_40));
    assert_clean(RANDOM_40); // no contextual evidence
    assert_clean("aws_secret_access_key=wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY");
}

#[test]
fn aws_session_tokens_are_complete_and_variable_length() {
    // STS encodings contain long repeated runs; they are not random API keys.
    let token = format!("{RANDOM_40}//////////AAAAAA{RANDOM_SIG}==");
    for key in [
        "AWS_SESSION_TOKEN",
        "aws_security_token",
        "SessionToken",
        "sessionToken",
        "X-Amz-Security-Token",
    ] {
        assert_single(&format!("{key}={token}"), "aws-session-token", &token);
        assert_single(
            &serde_json::json!({key: token}).to_string(),
            "aws-session-token",
            &token,
        );
    }
    let long = token.repeat(12);
    assert_single(
        &format!("AWS_SESSION_TOKEN={long}"),
        "aws-session-token",
        &long,
    );
}

#[test]
fn unquoted_generic_credentials_are_detected_without_capturing_delimiters() {
    for (key, value, rule) in [
        ("API_KEY", RANDOM_24, "generic-api-key-unquoted"),
        ("apiSecret", RANDOM_24, "generic-api-key-unquoted"),
        ("apiToken", RANDOM_24, "generic-api-key-unquoted"),
        ("CLIENT_SECRET", RANDOM_24, "generic-secret-unquoted"),
        (
            "DB_PASSWORD",
            "Xk9!mP2vQ7?zR4tL",
            "generic-password-unquoted",
        ),
    ] {
        for suffix in ["", "\n", " ./run", ";", ",", "}", "&next=1"] {
            assert_single(&format!("export {key}={value}{suffix}"), rule, value);
        }
    }
    for key in [
        "nextPageToken",
        "token_id",
        "public_api_key",
        "publishableApiKey",
        "password_hash",
        "password_file",
    ] {
        assert_clean(&format!("{key}={RANDOM_40}"));
        assert_clean(&format!(r#"{key}="{RANDOM_40}""#));
    }
    for value in [
        "$API_KEY",
        "${API_KEY}",
        "<API_KEY>",
        "your_api_key_here",
        "sk-demo-key-1234567890",
    ] {
        assert_clean(&format!("API_KEY={value}"));
    }
}

#[test]
fn common_words_inside_random_credentials_do_not_suppress_them() {
    for word in ["test", "here", "your", "insert", "demo"] {
        let value = format!("{RANDOM_24}{word}{RANDOM_24}");
        assert_single(
            &format!(r#"api_key="{value}""#),
            "generic-api-key-assignment",
            &value,
        );
        let pat = format!("ghp_{value}");
        assert_single(&pat, "github-pat", &pat);
    }
}

#[test]
fn adjacent_assignments_do_not_lose_shared_delimiters() {
    let text = format!("client_secret={RANDOM_24} client_secret={RANDOM_40}");
    let findings = scan(&[&text]);
    assert_eq!(findings.len(), 2);
    assert_eq!(findings[0].raw_text, RANDOM_24);
    assert_eq!(findings[1].raw_text, RANDOM_40);
}

#[test]
fn short_passwords_are_masked_in_serialization_and_debug() {
    let secret = "K7!pQ2zR";
    let finding = assert_single(
        &format!("password='{secret}'"),
        "generic-password-assignment",
        secret,
    );
    assert!(!finding.matched_text.contains(secret));
    assert!(finding.matched_text.chars().all(|c| c == '*'));
    assert!(!serde_json::to_string(&finding).unwrap().contains(secret));
    assert!(!format!("{finding:?}").contains(secret));
}

#[test]
fn basic_auth_is_checked_after_decoding_and_redacts_the_encoded_credential() {
    use base64::Engine as _;
    let encoded = base64::engine::general_purpose::STANDARD.encode("deploy:Xk9!mP2vQ7?zR4tL");
    assert_single(
        &format!("Authorization: Basic {encoded}"),
        "basic-auth-header",
        &encoded,
    );
    assert_single(
        &format!(r#"{{"Authorization":"Basic {encoded}"}}"#),
        "basic-auth-header",
        &encoded,
    );
    for credential in [
        "user:password",
        "deploy:${PASSWORD}",
        "not-a-credential",
        "user:",
    ] {
        let encoded = base64::engine::general_purpose::STANDARD.encode(credential);
        assert_clean(&format!("Authorization: Basic {encoded}"));
    }
    assert_clean("Authorization: Basic ////////////////");
}

#[test]
fn long_bearer_tokens_are_never_truncated() {
    let token = RANDOM_40.repeat(40);
    assert_single(
        &format!("Authorization: Bearer {token}"),
        "bearer-token-header",
        &token,
    );
}

/// Each vendor rule needs a working positive example, not just a regex that
/// compiles. Shared prefixes must still choose the most specific rule.
#[test]
fn vendor_rule_detection_matrix() {
    for (rule, token) in [
        ("gcp-api-key", format!("AIza{}", &RANDOM_40[..35])),
        (
            "azure-storage-key",
            format!("AccountKey={}==", &RANDOM_40.repeat(3)[..86]),
        ),
        (
            "openai-api-key",
            format!("sk-{RANDOM_40}T3BlbkFJ{RANDOM_40}"),
        ),
        ("openai-api-key-v2", format!("sk-proj-{RANDOM_40}")),
        (
            "anthropic-api-key",
            format!("sk-ant-api03-{}", RANDOM_40.repeat(3)),
        ),
        ("anthropic-api-key-v2", format!("sk-ant-{RANDOM_40}")),
        ("groq-api-key", format!("gsk_{}", RANDOM_40.repeat(2))),
        ("cohere-api-key", format!("COHERE_API_KEY={RANDOM_40}")),
        ("huggingface-token", format!("hf_{RANDOM_40}")),
        ("github-pat", GITHUB_PAT.to_string()),
        ("github-oauth", format!("gho_{RANDOM_40}")),
        ("github-app-token", format!("ghs_{RANDOM_40}")),
        (
            "github-fine-grained-pat",
            format!("github_pat_{}", RANDOM_40.repeat(3)),
        ),
        ("gitlab-pat", format!("glpat-{RANDOM_40}")),
        ("stripe-secret-key", format!("sk_live_{RANDOM_24}")),
        ("stripe-restricted-key", format!("rk_live_{RANDOM_24}")),
        (
            "slack-bot-token",
            format!("xoxb-7392048176-9372840165-{RANDOM_24}"),
        ),
        (
            "slack-user-token",
            format!("xoxp-7392048176-9372840165-8371920465-{}", &RANDOM_40[..32]),
        ),
        (
            "slack-webhook",
            format!("https://hooks.slack.com/services/T39LQ8N/B92JH7M/{RANDOM_24}"),
        ),
    ] {
        let findings = scan(&[&token]);
        assert_eq!(findings.len(), 1, "{rule}: {findings:?}");
        assert_eq!(findings[0].rule_id, rule);
        assert!(token.contains(&findings[0].raw_text));
    }
}
