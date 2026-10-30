//! Webhook egress: LocalRouter signs and POSTs events to a client's callback
//! URL (SEP-3415 webhook delivery, Standard Webhooks signatures). Outbound
//! only; LocalRouter never receives webhooks.

use std::time::Duration;

use base64::Engine;
use ring::hmac;
use serde_json::{json, Value};

/// Header carrying the subscription id on every delivery.
pub const SUBSCRIPTION_ID_HEADER: &str = "x-mcp-subscription-id";
const SECRET_PREFIX: &str = "whsec_";
const MAX_URL_LEN: usize = 4096;
const MAX_CHALLENGE_RESPONSE: usize = 64 * 1024;

/// Why a callback URL or secret was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebhookInputError(pub String);

fn is_loopback_host(url: &reqwest::Url) -> bool {
    let Some(host) = url.host_str() else {
        return false;
    };
    let host = host.trim_start_matches('[').trim_end_matches(']');
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

/// Validate a callback URL: HTTPS, or `http://` to a loopback address; no
/// credentials, no fragment.
pub fn validate_callback_url(url: &str) -> Result<reqwest::Url, WebhookInputError> {
    let refuse = || {
        WebhookInputError(
            "delivery.url must be HTTPS (or http:// to a loopback address) without credentials or a fragment".into(),
        )
    };
    if url.len() > MAX_URL_LEN {
        return Err(refuse());
    }
    let parsed = reqwest::Url::parse(url).map_err(|_| refuse())?;
    let scheme_ok = match parsed.scheme() {
        "https" => true,
        "http" => is_loopback_host(&parsed),
        _ => false,
    };
    if !scheme_ok
        || parsed.host().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.fragment().is_some()
    {
        return Err(refuse());
    }
    Ok(parsed)
}

/// Decode a `whsec_…` signing secret.
pub fn decode_secret(secret: &str) -> Result<Vec<u8>, WebhookInputError> {
    let encoded = secret
        .strip_prefix(SECRET_PREFIX)
        .ok_or_else(|| WebhookInputError("delivery.secret must start with whsec_".into()))?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|_| WebhookInputError("delivery.secret is not valid base64".into()))?;
    if bytes.len() < 16 {
        return Err(WebhookInputError(
            "delivery.secret must hold at least 16 bytes".into(),
        ));
    }
    Ok(bytes)
}

/// Standard Webhooks headers for `body`: `webhook-id`, `webhook-timestamp`,
/// `webhook-signature` (`v1,<base64 HMAC-SHA256 of "id.timestamp.body">`),
/// plus the subscription id.
pub fn signed_headers(
    body: &[u8],
    webhook_id: &str,
    subscription_id: &str,
    secret: &[u8],
    timestamp: i64,
) -> Vec<(&'static str, String)> {
    let key = hmac::Key::new(hmac::HMAC_SHA256, secret);
    let mut context = hmac::Context::with_key(&key);
    context.update(format!("{webhook_id}.{timestamp}.").as_bytes());
    context.update(body);
    let signature = base64::engine::general_purpose::STANDARD.encode(context.sign().as_ref());
    vec![
        ("content-type", "application/json".to_string()),
        ("webhook-id", webhook_id.to_string()),
        ("webhook-timestamp", timestamp.to_string()),
        ("webhook-signature", format!("v1,{signature}")),
        (SUBSCRIPTION_ID_HEADER, subscription_id.to_string()),
    ]
}

/// HTTP client for deliveries: no redirects, per-attempt timeout, no proxy
/// for loopback targets.
pub fn http_client(timeout: Duration) -> reqwest::Client {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(timeout)
        .build()
        .unwrap_or_default()
}

async fn post(
    client: &reqwest::Client,
    url: &str,
    body: Vec<u8>,
    webhook_id: &str,
    subscription_id: &str,
    secret: &[u8],
) -> Result<reqwest::Response, String> {
    let mut request = client.post(url);
    for (name, value) in signed_headers(
        &body,
        webhook_id,
        subscription_id,
        secret,
        chrono::Utc::now().timestamp(),
    ) {
        request = request.header(name, value);
    }
    let response = request.body(body).send().await.map_err(|e| {
        if e.is_timeout() {
            "timeout".to_string()
        } else {
            "connection_failed".to_string()
        }
    })?;
    if response.status().is_success() {
        Ok(response)
    } else {
        Err(format!("http_{}", response.status().as_u16()))
    }
}

/// The verification challenge sent before the first delivery: the receiver
/// must answer 2xx with `{"challenge": <the same value>}`.
pub async fn verify_endpoint(
    client: &reqwest::Client,
    url: &str,
    subscription_id: &str,
    secret: &[u8],
) -> Result<(), String> {
    let challenge = uuid::Uuid::new_v4().simple().to_string();
    let body = serde_json::to_vec(&json!({"type": "verification", "challenge": challenge}))
        .map_err(|_| "encode_failed".to_string())?;
    let webhook_id = format!("verification_{}", uuid::Uuid::new_v4().simple());
    let response = post(client, url, body, &webhook_id, subscription_id, secret).await?;
    let bytes = response
        .bytes()
        .await
        .map_err(|_| "challenge_failed".to_string())?;
    if bytes.len() > MAX_CHALLENGE_RESPONSE {
        return Err("challenge_failed".into());
    }
    let echoed: Value =
        serde_json::from_slice(&bytes).map_err(|_| "challenge_failed".to_string())?;
    if echoed.get("challenge").and_then(Value::as_str) == Some(challenge.as_str()) {
        Ok(())
    } else {
        Err("challenge_failed".into())
    }
}

/// POST one event. `webhook-id` is the event id, so receivers deduplicate
/// retries.
pub async fn deliver_event(
    client: &reqwest::Client,
    url: &str,
    subscription_id: &str,
    secret: &[u8],
    event_id: &str,
    body: &Value,
) -> Result<(), String> {
    let body = serde_json::to_vec(body).map_err(|_| "encode_failed".to_string())?;
    post(client, url, body, event_id, subscription_id, secret)
        .await
        .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn callback_urls_follow_the_rules() {
        assert!(validate_callback_url("https://example.com/hook").is_ok());
        assert!(validate_callback_url("http://127.0.0.1:9000/hook").is_ok());
        assert!(validate_callback_url("http://localhost/hook").is_ok());
        assert!(validate_callback_url("http://[::1]:1/hook").is_ok());
        assert!(validate_callback_url("http://example.com/hook").is_err());
        assert!(validate_callback_url("https://user:pw@example.com/hook").is_err());
        assert!(validate_callback_url("https://example.com/hook#frag").is_err());
        assert!(validate_callback_url("ftp://example.com/hook").is_err());
        assert!(validate_callback_url("not a url").is_err());
        let long = format!("https://example.com/{}", "a".repeat(MAX_URL_LEN));
        assert!(validate_callback_url(&long).is_err());
    }

    #[test]
    fn secrets_need_prefix_and_length() {
        let good = format!(
            "whsec_{}",
            base64::engine::general_purpose::STANDARD.encode([7u8; 24])
        );
        assert_eq!(decode_secret(&good).unwrap(), vec![7u8; 24]);
        assert!(decode_secret("abc").is_err());
        assert!(decode_secret("whsec_!!!").is_err());
        let short = format!(
            "whsec_{}",
            base64::engine::general_purpose::STANDARD.encode([1u8; 4])
        );
        assert!(decode_secret(&short).is_err());
    }

    /// Matches the Standard Webhooks reference vector.
    #[test]
    fn signature_matches_standard_webhooks_vector() {
        let secret = decode_secret("whsec_MfKQ9r8GKYqrTwjUPD8ILPZIo2LaLaSw").unwrap();
        let headers = signed_headers(
            br#"{"test": 2432232314}"#,
            "msg_p5jXN8AQM9LWM0D4loKWxJek",
            "sub",
            &secret,
            1614265330,
        );
        let signature = headers
            .iter()
            .find(|(n, _)| *n == "webhook-signature")
            .unwrap()
            .1
            .clone();
        assert_eq!(signature, "v1,g0hM9SsE+OTPJTGt/tmIKtSyZlE3uFJELVlNIOLJ1OE=");
        assert!(headers
            .iter()
            .any(|(n, v)| *n == SUBSCRIPTION_ID_HEADER && v == "sub"));
    }
}
