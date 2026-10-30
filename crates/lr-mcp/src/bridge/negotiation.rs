//! Protocol negotiation toward the bridge's stdio client.
//!
//! The bridge deliberately does **not** support MCP revision 2026-07-28 toward
//! its stdio client. Claude Code registers a stdio server as a channel
//! (`notifications/claude/channel`) only when it connects on the legacy
//! `initialize` handshake; a server that negotiates 2026-07-28 is not
//! registered as a channel. Declining the newer revision here keeps
//! LocalRouter on the legacy handshake without the client-wide
//! `MCP_PROTOCOL_NEGOTIATION=legacy` switch.
//!
//! How the decline is shaped (evidence, checked 2026-10-30):
//!
//! - The 2026-07-28 stdio transport ("Backward Compatibility") has a dual-era
//!   client probe with `server/discover` before anything else. A
//!   `DiscoverResult` or a *recognized modern error* such as
//!   `UnsupportedProtocolVersionError` (-32022) marks the server as modern
//!   and the client must **not** fall back. "Any other error, or no response"
//!   marks it legacy, and the client falls back to `initialize`.
//! - TypeScript SDK v2 (`packages/client/src/client/probeClassifier.ts`,
//!   `classifyRpcError`) maps -32601 and every unrecognized code to
//!   `legacy`. A -32022 whose `supported` list names a modern version is a
//!   hard error with no fallback.
//!
//! So `server/discover` gets a plain JSON-RPC -32601 "Method not found" (no
//! `data`): exactly what a pre-2026 server answers. -32022 is never sent. The
//! spec reserves -32020..-32099, so no other code from that range is used
//! either. Any other request that declares a version newer than
//! [`BRIDGE_MAX_PROTOCOL_VERSION`] in `_meta` before `initialize` gets
//! -32600 "Server not initialized" and is not forwarded.

use crate::protocol::{meta_keys, INVALID_REQUEST, MCP_PROTOCOL_VERSION, METHOD_NOT_FOUND};
use serde_json::{json, Value};

/// Newest MCP revision the bridge negotiates with its stdio client.
pub const BRIDGE_MAX_PROTOCOL_VERSION: &str = MCP_PROTOCOL_VERSION;

/// Whether `version` is a usable revision string the bridge can agree to: an
/// ISO date (`YYYY-MM-DD`, which orders chronologically as a string) no newer
/// than [`BRIDGE_MAX_PROTOCOL_VERSION`].
fn is_acceptable_version(version: &str) -> bool {
    let b = version.as_bytes();
    let is_date = b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && b.iter()
            .enumerate()
            .all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit());
    is_date && version <= BRIDGE_MAX_PROTOCOL_VERSION
}

/// The revision a message declares in `params._meta` (2026-07-28 style).
fn meta_protocol_version(message: &Value) -> Option<&str> {
    message
        .get("params")?
        .get("_meta")?
        .get(meta_keys::PROTOCOL_VERSION)?
        .as_str()
}

fn error_response(id: &Value, code: i32, message: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message }
    })
}

/// Answer requests the bridge must decline itself instead of forwarding.
///
/// Returns the response to write to stdout, or `None` to forward the request.
pub fn intercept(message: &Value, id: &Value, method: &str, initialized: bool) -> Option<Value> {
    if method == "server/discover" {
        return Some(error_response(id, METHOD_NOT_FOUND, "Method not found"));
    }
    if !initialized && method != "initialize" {
        if let Some(version) = meta_protocol_version(message) {
            if !is_acceptable_version(version) {
                return Some(error_response(
                    id,
                    INVALID_REQUEST,
                    "Server not initialized",
                ));
            }
        }
    }
    None
}

/// Prepare an `initialize` request for the gateway.
///
/// Returns the request to forward and the version negotiated with the client:
/// the requested version when the bridge accepts it, otherwise
/// [`BRIDGE_MAX_PROTOCOL_VERSION`]. The forwarded request asks for the
/// negotiated version, and a 2026-07-28 `_meta` declaration is removed so the
/// gateway serves it as a session (it treats modern `_meta` as a stateless
/// peer, which gets no `Mcp-Session-Id`).
pub fn prepare_initialize(mut message: Value) -> (Value, String) {
    let requested = message
        .get("params")
        .and_then(|p| p.get("protocolVersion"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let negotiated = match requested {
        Some(ref v) if is_acceptable_version(v) => v.clone(),
        _ => BRIDGE_MAX_PROTOCOL_VERSION.to_string(),
    };

    if let Some(params) = message.get_mut("params").and_then(Value::as_object_mut) {
        params.insert("protocolVersion".into(), json!(negotiated));
        if let Some(meta) = params.get_mut("_meta").and_then(Value::as_object_mut) {
            let modern = meta
                .get(meta_keys::PROTOCOL_VERSION)
                .and_then(Value::as_str)
                .is_some_and(|v| !is_acceptable_version(v));
            if modern {
                meta.remove(meta_keys::PROTOCOL_VERSION);
            }
        }
    }
    (message, negotiated)
}

/// Rewrite an `initialize` result whose `protocolVersion` is newer than the
/// version negotiated with the client, so the client only ever sees a revision
/// it asked for (or older).
pub fn rewrite_initialize_result(message: &mut Value, negotiated: &str) {
    if let Some(result) = message.get_mut("result").and_then(Value::as_object_mut) {
        let newer = match result.get("protocolVersion").and_then(Value::as_str) {
            Some(v) => v > negotiated,
            None => false,
        };
        if newer {
            result.insert("protocolVersion".into(), json!(negotiated));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discover_is_method_not_found_without_data() {
        let msg = json!({"jsonrpc":"2.0","id":"discover-1","method":"server/discover",
            "params":{"_meta":{meta_keys::PROTOCOL_VERSION:"2026-07-28"}}});
        let resp = intercept(&msg, &json!("discover-1"), "server/discover", false).unwrap();
        assert_eq!(resp["id"], "discover-1");
        assert_eq!(resp["error"]["code"], -32601);
        assert!(resp["error"].get("data").is_none());
        // Declined after initialize too: the bridge never speaks 2026-07-28.
        assert!(intercept(&msg, &json!(1), "server/discover", true).is_some());
    }

    #[test]
    fn modern_meta_before_initialize_is_declined_not_forwarded() {
        let msg = json!({"jsonrpc":"2.0","id":7,"method":"tools/list",
            "params":{"_meta":{meta_keys::PROTOCOL_VERSION:"2026-07-28"}}});
        let resp = intercept(&msg, &json!(7), "tools/list", false).unwrap();
        assert_eq!(resp["error"]["code"], -32600);
        assert_ne!(resp["error"]["code"], -32022);
        assert!(intercept(&msg, &json!(7), "tools/list", true).is_none());
        let legacy = json!({"jsonrpc":"2.0","id":8,"method":"ping"});
        assert!(intercept(&legacy, &json!(8), "ping", false).is_none());
    }

    #[test]
    fn initialize_negotiates_down_and_strips_modern_meta() {
        let msg = json!({"jsonrpc":"2.0","id":0,"method":"initialize","params":{
            "protocolVersion":"2026-07-28","capabilities":{},
            "_meta":{meta_keys::PROTOCOL_VERSION:"2026-07-28","other":1}}});
        let (fwd, negotiated) = prepare_initialize(msg);
        assert_eq!(negotiated, "2025-11-25");
        assert_eq!(fwd["params"]["protocolVersion"], "2025-11-25");
        assert!(fwd["params"]["_meta"]
            .get(meta_keys::PROTOCOL_VERSION)
            .is_none());
        assert_eq!(fwd["params"]["_meta"]["other"], 1);
    }

    #[test]
    fn initialize_keeps_older_versions_and_rejects_garbage() {
        let (_, v) = prepare_initialize(
            json!({"id":0,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}),
        );
        assert_eq!(v, "2025-06-18");
        let (_, v) = prepare_initialize(
            json!({"id":0,"method":"initialize","params":{"protocolVersion":"latest"}}),
        );
        assert_eq!(v, BRIDGE_MAX_PROTOCOL_VERSION);
    }

    #[test]
    fn result_rewritten_only_when_newer() {
        let mut m = json!({"id":0,"result":{"protocolVersion":"2026-07-28"}});
        rewrite_initialize_result(&mut m, "2025-11-25");
        assert_eq!(m["result"]["protocolVersion"], "2025-11-25");
        let mut m = json!({"id":0,"result":{"protocolVersion":"2025-03-26"}});
        rewrite_initialize_result(&mut m, "2025-11-25");
        assert_eq!(m["result"]["protocolVersion"], "2025-03-26");
    }
}
