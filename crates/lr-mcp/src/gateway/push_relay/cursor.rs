//! LocalRouter-issued event cursors.
//!
//! A cursor is opaque to clients and HMAC-signed with a key that lives only
//! as long as this process. It records the process epoch, the client
//! journal position and a digest of its scope (event names and canonical
//! filter arguments), and is bound to the client it was issued to. A cursor
//! from another epoch (a previous LocalRouter process) cannot be verified and
//! resets to "now" with `truncated: true`.

use base64::Engine;
use ring::hmac;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

const PREFIX: &str = "lr1.";

/// The canonical scope a cursor is valid for.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CursorScope {
    canonical: String,
}

impl CursorScope {
    /// Scope of a read over `names` (order-insensitive) with `filter`.
    pub fn new(names: &[String], filter: &Map<String, Value>) -> Self {
        let mut names: Vec<&str> = names.iter().map(String::as_str).collect();
        names.sort_unstable();
        names.dedup();
        Self {
            canonical: format!(
                "{}\n{}",
                names.join("\n"),
                canonical_json(&Value::Object(filter.clone()))
            ),
        }
    }

    fn digest(&self) -> String {
        let digest = Sha256::digest(self.canonical.as_bytes());
        digest[..8].iter().map(|b| format!("{b:02x}")).collect()
    }
}

/// Where a decoded cursor points.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CursorPosition {
    /// A journal position issued by this process.
    At(u64),
    /// Issued by another process: restart at "now" and report truncation.
    Reset,
}

/// Why a cursor was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CursorError {
    /// Not a LocalRouter cursor, tampered with, or issued to another client.
    Malformed,
    /// Issued for a different set of events or filter.
    WrongScope,
}

impl CursorError {
    pub fn message(&self) -> &'static str {
        match self {
            Self::Malformed => "Invalid cursor",
            Self::WrongScope => "Cursor was issued for different events or arguments",
        }
    }
}

/// Signing key and epoch of this process.
pub struct CursorKey {
    key: hmac::Key,
    epoch: String,
}

impl Default for CursorKey {
    fn default() -> Self {
        Self::new()
    }
}

impl CursorKey {
    pub fn new() -> Self {
        let mut secret = [0u8; 32];
        secret[..16].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
        secret[16..].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
        let epoch = uuid::Uuid::new_v4().simple().to_string()[..12].to_string();
        Self {
            key: hmac::Key::new(hmac::HMAC_SHA256, &secret),
            epoch,
        }
    }

    pub fn epoch(&self) -> &str {
        &self.epoch
    }

    fn mac(&self, message: &str) -> String {
        let tag = hmac::sign(&self.key, message.as_bytes());
        tag.as_ref().iter().map(|b| format!("{b:02x}")).collect()
    }

    /// Sign arbitrary text (used for watch tokens).
    pub fn sign(&self, domain: &str, message: &str) -> String {
        self.mac(&format!("{domain}\u{1f}{message}"))
    }

    /// Constant-time check of [`Self::sign`].
    pub fn verify(&self, domain: &str, message: &str, signature: &str) -> bool {
        let Some(bytes) = decode_hex(signature) else {
            return false;
        };
        hmac::verify(
            &self.key,
            format!("{domain}\u{1f}{message}").as_bytes(),
            &bytes,
        )
        .is_ok()
    }

    /// Issue a cursor for `client_id` at journal position `seq`.
    pub fn issue(&self, client_id: &str, scope: &CursorScope, seq: u64) -> String {
        let body = format!("{}.{}.{}", self.epoch, seq, scope.digest());
        let mac = self.mac(&format!("{body}.{client_id}"));
        format!(
            "{PREFIX}{}",
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(format!("{body}.{mac}"))
        )
    }

    /// Decode a cursor presented by `client_id` for `scope`.
    pub fn decode(
        &self,
        client_id: &str,
        scope: &CursorScope,
        cursor: &str,
    ) -> Result<CursorPosition, CursorError> {
        let encoded = cursor.strip_prefix(PREFIX).ok_or(CursorError::Malformed)?;
        let raw = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(encoded)
            .map_err(|_| CursorError::Malformed)?;
        let raw = String::from_utf8(raw).map_err(|_| CursorError::Malformed)?;
        let parts: Vec<&str> = raw.split('.').collect();
        let [epoch, seq, digest, mac] = parts.as_slice() else {
            return Err(CursorError::Malformed);
        };
        let seq: u64 = seq.parse().map_err(|_| CursorError::Malformed)?;
        if *epoch != self.epoch {
            return Ok(CursorPosition::Reset);
        }
        let body = format!("{epoch}.{seq}.{digest}.{client_id}");
        let Some(mac) = decode_hex(mac) else {
            return Err(CursorError::Malformed);
        };
        hmac::verify(&self.key, body.as_bytes(), &mac).map_err(|_| CursorError::Malformed)?;
        if *digest != scope.digest() {
            return Err(CursorError::WrongScope);
        }
        Ok(CursorPosition::At(seq))
    }
}

fn decode_hex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(text.get(i..i + 2)?, 16).ok())
        .collect()
}

/// JSON with object keys sorted at every level, so equal values produce the
/// same text whatever order they were written in.
pub fn canonical_json(value: &Value) -> String {
    fn sorted(value: &Value) -> Value {
        match value {
            Value::Object(map) => {
                let mut keys: Vec<&String> = map.keys().collect();
                keys.sort();
                let mut out = Map::new();
                for key in keys {
                    out.insert(key.clone(), sorted(&map[key]));
                }
                Value::Object(out)
            }
            Value::Array(items) => Value::Array(items.iter().map(sorted).collect()),
            other => other.clone(),
        }
    }
    // serde_json's map preserves insertion order only with `preserve_order`;
    // build the text by hand so the ordering holds either way.
    fn write(value: &Value, out: &mut String) {
        match value {
            Value::Object(map) => {
                out.push('{');
                let mut keys: Vec<&String> = map.keys().collect();
                keys.sort();
                for (i, key) in keys.into_iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    out.push_str(&Value::String(key.clone()).to_string());
                    out.push(':');
                    write(&map[key], out);
                }
                out.push('}');
            }
            Value::Array(items) => {
                out.push('[');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    write(item, out);
                }
                out.push(']');
            }
            other => out.push_str(&other.to_string()),
        }
    }
    let mut out = String::new();
    write(&sorted(value), &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn scope(names: &[&str], filter: Value) -> CursorScope {
        let names: Vec<String> = names.iter().map(|s| s.to_string()).collect();
        CursorScope::new(&names, filter.as_object().unwrap())
    }

    #[test]
    fn round_trips_for_its_client_and_scope() {
        let key = CursorKey::new();
        let s = scope(&["b", "a"], json!({"x": 1, "y": {"b": 2, "a": 1}}));
        let cursor = key.issue("client-1", &s, 42);
        assert!(cursor.starts_with(PREFIX));
        // Same scope written differently
        let same = scope(&["a", "b", "a"], json!({"y": {"a": 1, "b": 2}, "x": 1}));
        assert_eq!(
            key.decode("client-1", &same, &cursor),
            Ok(CursorPosition::At(42))
        );
    }

    #[test]
    fn refuses_other_clients_scopes_and_tampering() {
        let key = CursorKey::new();
        let s = scope(&["a"], json!({}));
        let cursor = key.issue("client-1", &s, 7);
        assert_eq!(
            key.decode("client-2", &s, &cursor),
            Err(CursorError::Malformed)
        );
        assert_eq!(
            key.decode("client-1", &scope(&["b"], json!({})), &cursor),
            Err(CursorError::WrongScope)
        );
        assert_eq!(
            key.decode("client-1", &s, "garbage"),
            Err(CursorError::Malformed)
        );
        // Change the position without re-signing
        let raw = String::from_utf8(
            base64::engine::general_purpose::URL_SAFE_NO_PAD
                .decode(cursor.strip_prefix(PREFIX).unwrap())
                .unwrap(),
        )
        .unwrap();
        let forged = raw.replacen(".7.", ".9.", 1);
        let forged = format!(
            "{PREFIX}{}",
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(forged)
        );
        assert_eq!(
            key.decode("client-1", &s, &forged),
            Err(CursorError::Malformed)
        );
    }

    #[test]
    fn foreign_epoch_resets() {
        let old = CursorKey::new();
        let new = CursorKey::new();
        let s = scope(&["a"], json!({}));
        let cursor = old.issue("client-1", &s, 3);
        assert_eq!(
            new.decode("client-1", &s, &cursor),
            Ok(CursorPosition::Reset)
        );
    }

    #[test]
    fn signatures_verify_only_their_message() {
        let key = CursorKey::new();
        let sig = key.sign("watch", "client-1|123");
        assert!(key.verify("watch", "client-1|123", &sig));
        assert!(!key.verify("watch", "client-2|123", &sig));
        assert!(!key.verify("other", "client-1|123", &sig));
        assert!(!key.verify("watch", "client-1|123", "zz"));
    }

    #[test]
    fn canonical_json_sorts_keys() {
        assert_eq!(
            canonical_json(&json!({"b": [ {"d": 1, "c": 2} ], "a": null})),
            r#"{"a":null,"b":[{"c":2,"d":1}]}"#
        );
    }
}
