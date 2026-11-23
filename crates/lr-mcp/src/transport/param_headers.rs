//! `Mcp-Param-{Name}` headers for `x-mcp-header`-annotated arguments
//! (MCP 2026-07-28 Streamable HTTP, "Custom Headers from Tool Parameters";
//! applied to MCP Events subscription arguments by SEP-3415).
//!
//! The caller that knows the schema (the Push Relay, for upstream
//! `events/poll` and `events/stream`) computes the headers and attaches them
//! to the request under the reserved `_meta` key [`PARAM_HEADERS_META`].
//! Every transport removes that key before the request is serialized: the
//! HTTP transport sends the headers, stdio and WebSocket drop them (other
//! transports MAY ignore the annotation). The gateway removes it from every
//! downstream client request ([`discard`]), so a client cannot have the
//! HTTP transport send headers the gateway did not compute.

use serde_json::{Map, Value};

use crate::protocol::JsonRpcRequest;

/// Reserved `_meta` key carrying computed `Mcp-Param-*` headers from the
/// gateway to the transport. Never sent upstream.
pub const PARAM_HEADERS_META: &str = "io.localrouter/mcpParamHeaders";

/// The annotation keyword.
const ANNOTATION: &str = "x-mcp-header";

/// JavaScript's safe integer range, which integer header values must fit.
const MAX_SAFE_INTEGER: i64 = (1 << 53) - 1;

/// One annotated property: its path of `properties` keys and header name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeaderAnnotation {
    pub path: Vec<String>,
    pub name: String,
    /// The declared JSON Schema type: `string`, `integer` or `boolean`.
    pub kind: String,
}

fn is_tchar(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b)
}

/// Count every `x-mcp-header` key anywhere in `value`.
fn count_annotations(value: &Value) -> usize {
    match value {
        Value::Object(map) => {
            usize::from(map.contains_key(ANNOTATION))
                + map.values().map(count_annotations).sum::<usize>()
        }
        Value::Array(items) => items.iter().map(count_annotations).sum(),
        _ => 0,
    }
}

fn collect(
    schema: &Map<String, Value>,
    path: &mut Vec<String>,
    out: &mut Vec<HeaderAnnotation>,
) -> Result<(), String> {
    let Some(properties) = schema.get("properties").and_then(Value::as_object) else {
        return Ok(());
    };
    for (key, property) in properties {
        let Some(property) = property.as_object() else {
            continue;
        };
        path.push(key.clone());
        if let Some(annotation) = property.get(ANNOTATION) {
            let name = annotation
                .as_str()
                .ok_or_else(|| format!("{ANNOTATION} on '{}' is not a string", path.join(".")))?;
            if name.is_empty() || !name.bytes().all(is_tchar) {
                return Err(format!(
                    "{ANNOTATION} '{name}' on '{}' is not an HTTP token",
                    path.join(".")
                ));
            }
            let kind = match property.get("type").and_then(Value::as_str) {
                Some(kind @ ("string" | "integer" | "boolean")) => kind.to_string(),
                _ => {
                    return Err(format!(
                        "{ANNOTATION} on '{}' needs type string, integer or boolean",
                        path.join(".")
                    ))
                }
            };
            if out.iter().any(|a| a.name.eq_ignore_ascii_case(name)) {
                return Err(format!("{ANNOTATION} '{name}' is used twice"));
            }
            out.push(HeaderAnnotation {
                path: path.clone(),
                name: name.to_string(),
                kind,
            });
        }
        collect(property, path, out)?;
        path.pop();
    }
    Ok(())
}

/// The annotations of `input_schema`, or why the schema's annotations are
/// invalid (an invalid one makes the whole definition unusable over HTTP).
pub fn annotations(input_schema: &Value) -> Result<Vec<HeaderAnnotation>, String> {
    let Some(schema) = input_schema.as_object() else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    collect(schema, &mut Vec::new(), &mut out)?;
    // Annotations are valid only on statically reachable properties: one
    // under `items`, `oneOf`, `$ref`, ... is not reached above.
    if count_annotations(input_schema) != out.len() {
        return Err(format!(
            "{ANNOTATION} is allowed only on properties reachable through `properties`"
        ));
    }
    Ok(out)
}

/// The header value of one argument, as the transport encodes it; `None`
/// for a value of a type the annotation does not allow.
fn header_value(kind: &str, value: &Value) -> Option<String> {
    let plain = match (kind, value) {
        ("string", Value::String(s)) => s.clone(),
        ("boolean", Value::Bool(b)) => b.to_string(),
        ("integer", Value::Number(n)) => {
            let n = n.as_i64()?;
            if !(-MAX_SAFE_INTEGER..=MAX_SAFE_INTEGER).contains(&n) {
                return None;
            }
            n.to_string()
        }
        _ => return None,
    };
    Some(super::sse::mcp_header_value(&plain))
}

/// `Mcp-Param-{Name}` headers for every annotated property that has a value
/// in `arguments`.
pub fn headers_for(
    annotations: &[HeaderAnnotation],
    arguments: &Map<String, Value>,
) -> Vec<(String, String)> {
    annotations
        .iter()
        .filter_map(|a| {
            let (first, rest) = a.path.split_first()?;
            let mut value = arguments.get(first)?;
            for key in rest {
                value = value.get(key)?;
            }
            Some((
                format!("Mcp-Param-{}", a.name),
                header_value(&a.kind, value)?,
            ))
        })
        .collect()
}

/// Attach computed headers to request `params` (no-op without headers).
pub fn attach(params: &mut Value, headers: &[(String, String)]) {
    if headers.is_empty() {
        return;
    }
    let Some(params) = params.as_object_mut() else {
        return;
    };
    let meta = params
        .entry("_meta".to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    if let Some(meta) = meta.as_object_mut() {
        let map: Map<String, Value> = headers
            .iter()
            .map(|(k, v)| (k.clone(), Value::String(v.clone())))
            .collect();
        meta.insert(PARAM_HEADERS_META.to_string(), Value::Object(map));
    }
}

/// Remove the reserved key from a request that did not come from the
/// gateway (a downstream client's), so only [`attach`] can set it.
pub fn discard(request: &mut JsonRpcRequest) {
    let _ = take(request);
}

/// Remove the attached headers from `request` (and an `_meta` left empty).
pub fn take(request: &mut JsonRpcRequest) -> Vec<(String, String)> {
    let Some(params) = request.params.as_mut().and_then(Value::as_object_mut) else {
        return Vec::new();
    };
    let Some(meta) = params.get_mut("_meta").and_then(Value::as_object_mut) else {
        return Vec::new();
    };
    let Some(headers) = meta.remove(PARAM_HEADERS_META) else {
        return Vec::new();
    };
    if meta.is_empty() {
        params.remove("_meta");
    }
    headers
        .as_object()
        .map(|map| {
            map.iter()
                .filter_map(|(k, v)| {
                    // Only well-formed names: never let a value smuggle a
                    // header the gateway did not compute.
                    let suffix = k.strip_prefix("Mcp-Param-")?;
                    (!suffix.is_empty() && suffix.bytes().all(is_tchar))
                        .then(|| Some((k.clone(), v.as_str()?.to_string())))
                        .flatten()
                })
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn annotations_follow_the_properties_chain() {
        let schema = json!({"type": "object", "properties": {
            "region": {"type": "string", "x-mcp-header": "Region"},
            "severity": {"type": "string"},
            "scope": {"type": "object", "properties": {
                "tenant": {"type": "integer", "x-mcp-header": "Tenant"}
            }}
        }});
        let found = annotations(&schema).unwrap();
        assert_eq!(found.len(), 2);
        assert!(found.contains(&HeaderAnnotation {
            path: vec!["scope".into(), "tenant".into()],
            name: "Tenant".into(),
            kind: "integer".into(),
        }));
        assert_eq!(annotations(&json!({"type": "object"})).unwrap(), vec![]);
        assert_eq!(annotations(&Value::Null).unwrap(), vec![]);
    }

    #[test]
    fn invalid_annotations_are_rejected() {
        for (schema, why) in [
            (
                json!({"properties": {"a": {"type": "number", "x-mcp-header": "A"}}}),
                "number",
            ),
            (
                json!({"properties": {"a": {"type": "string", "x-mcp-header": ""}}}),
                "empty",
            ),
            (
                json!({"properties": {"a": {"type": "string", "x-mcp-header": "A B"}}}),
                "space",
            ),
            (
                json!({"properties": {"a": {"type": "string", "x-mcp-header": "A\r\nX"}}}),
                "CRLF",
            ),
            (
                json!({"properties": {
                    "a": {"type": "string", "x-mcp-header": "Dup"},
                    "b": {"type": "string", "x-mcp-header": "dup"}}}),
                "duplicate",
            ),
            (
                json!({"properties": {"a": {"type": "array",
                    "items": {"type": "string", "x-mcp-header": "Item"}}}}),
                "under items",
            ),
            (
                json!({"oneOf": [{"properties": {"a": {"type": "string", "x-mcp-header": "A"}}}]}),
                "under oneOf",
            ),
        ] {
            assert!(annotations(&schema).is_err(), "{why} must be rejected");
        }
    }

    #[test]
    fn headers_encode_values_and_skip_absent_ones() {
        let schema = json!({"properties": {
            "region": {"type": "string", "x-mcp-header": "Region"},
            "count": {"type": "integer", "x-mcp-header": "Count"},
            "live": {"type": "boolean", "x-mcp-header": "Live"},
            "name": {"type": "string", "x-mcp-header": "Name"},
            "missing": {"type": "string", "x-mcp-header": "Missing"},
            "scope": {"type": "object", "properties": {
                "tenant": {"type": "string", "x-mcp-header": "Tenant"}}}
        }});
        let found = annotations(&schema).unwrap();
        let arguments = json!({"region": "us-west1", "count": -7, "live": true,
            "name": "Zürich", "scope": {"tenant": "=?base64?x?="}});
        let mut headers = headers_for(&found, arguments.as_object().unwrap());
        headers.sort();
        assert_eq!(
            headers,
            vec![
                ("Mcp-Param-Count".to_string(), "-7".to_string()),
                ("Mcp-Param-Live".to_string(), "true".to_string()),
                (
                    "Mcp-Param-Name".to_string(),
                    "=?base64?WsO8cmljaA==?=".to_string()
                ),
                ("Mcp-Param-Region".to_string(), "us-west1".to_string()),
                // A plain value that looks like the sentinel is encoded too
                (
                    "Mcp-Param-Tenant".to_string(),
                    "=?base64?PT9iYXNlNjQ/eD89?=".to_string()
                ),
            ]
        );
        // Out of the safe integer range, or the wrong type: no header
        let arguments = json!({"count": 9007199254740993_i64, "region": 5});
        assert!(headers_for(&found, arguments.as_object().unwrap()).is_empty());
    }

    #[test]
    fn attach_and_take_round_trip_and_leave_no_trace() {
        let headers = vec![("Mcp-Param-Region".to_string(), "eu".to_string())];
        let mut params = json!({"name": "e", "arguments": {"region": "eu"}});
        attach(&mut params, &headers);
        let mut request = JsonRpcRequest::new(Some(json!(1)), "events/poll".into(), Some(params));
        assert_eq!(take(&mut request), headers);
        assert_eq!(
            request.params.unwrap(),
            json!({"name": "e", "arguments": {"region": "eu"}})
        );
        // Other _meta survives
        let mut params = json!({"_meta": {"k": 1}});
        attach(&mut params, &headers);
        let mut request = JsonRpcRequest::new(Some(json!(1)), "events/poll".into(), Some(params));
        take(&mut request);
        assert_eq!(request.params.unwrap(), json!({"_meta": {"k": 1}}));
        // Nothing attached: nothing taken; malformed names are dropped
        let mut request = JsonRpcRequest::new(
            Some(json!(1)),
            "events/poll".into(),
            Some(
                json!({"_meta": {PARAM_HEADERS_META: {"Host": "x", "Mcp-Param-": "y",
                "Mcp-Param-Ok": "z"}}}),
            ),
        );
        assert_eq!(
            take(&mut request),
            vec![("Mcp-Param-Ok".to_string(), "z".to_string())]
        );
        let mut request = JsonRpcRequest::new(Some(json!(1)), "x".into(), None);
        assert!(take(&mut request).is_empty());
    }
}
