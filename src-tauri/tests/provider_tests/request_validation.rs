//! Request validation utilities for provider tests
//!
//! Helpers for validating that providers send correct HTTP requests to APIs

use serde_json::Value;
use std::collections::HashMap;
use wiremock::Request;

/// Extract and validate JSON body from request
pub fn extract_json_body(request: &Request) -> Value {
    let body_bytes = &request.body;
    serde_json::from_slice(body_bytes).expect("Request body should be valid JSON")
}

/// Validate that request has expected header with exact value
pub fn assert_header_equals(request: &Request, header: &str, expected: &str) {
    let headers = request
        .headers
        .get(header)
        .unwrap_or_else(|| panic!("Request should have '{}' header", header));

    assert_eq!(
        headers.to_str().unwrap(),
        expected,
        "Header '{}' should equal '{}'",
        header,
        expected
    );
}

/// Validate that request has header with value containing substring
pub fn assert_header_contains(request: &Request, header: &str, substring: &str) {
    let headers = request
        .headers
        .get(header)
        .unwrap_or_else(|| panic!("Request should have '{}' header", header));

    let value = headers.to_str().unwrap();
    assert!(
        value.contains(substring),
        "Header '{}' value '{}' should contain '{}'",
        header,
        value,
        substring
    );
}

/// Validate Authorization header format
pub fn assert_bearer_token(request: &Request, expected_prefix: &str) {
    assert_header_contains(request, "authorization", "Bearer ");

    let auth = request.headers.get("authorization").unwrap();
    let token = auth.to_str().unwrap().strip_prefix("Bearer ").unwrap();

    if !expected_prefix.is_empty() {
        assert!(
            token.starts_with(expected_prefix),
            "Token should start with '{}'",
            expected_prefix
        );
    }
}

/// Validate that request has specific query parameter
pub fn assert_query_param(request: &Request, param: &str, expected: &str) {
    let url = &request.url;
    let query_pairs: HashMap<String, String> = url.query_pairs().into_owned().collect();

    assert!(
        query_pairs.contains_key(param),
        "Request should have query parameter '{}'",
        param
    );

    assert_eq!(
        query_pairs.get(param).unwrap(),
        expected,
        "Query parameter '{}' should equal '{}'",
        param,
        expected
    );
}

/// Validate Content-Type header
pub fn assert_content_type_json(request: &Request) {
    assert_header_contains(request, "content-type", "application/json");
}

/// Validate that JSON body has specific field with expected value
pub fn assert_json_field(body: &Value, field: &str, expected: &Value) {
    let actual = body.get(field).unwrap_or_else(|| {
        panic!(
            "JSON body should have field '{}'. Body: {}",
            field,
            serde_json::to_string_pretty(body).unwrap()
        )
    });

    assert_eq!(
        actual, expected,
        "Field '{}' should equal {:?}",
        field, expected
    );
}

/// Validate that JSON body has specific string field
pub fn assert_json_string_field(body: &Value, field: &str, expected: &str) {
    assert_json_field(body, field, &Value::String(expected.to_string()));
}

/// Validate that JSON body has specific boolean field
pub fn assert_json_bool_field(body: &Value, field: &str, expected: bool) {
    assert_json_field(body, field, &Value::Bool(expected));
}

/// Validate that JSON body has array field with specific length
pub fn assert_json_array_length(body: &Value, field: &str, expected_length: usize) {
    let array = body.get(field).and_then(|v| v.as_array()).unwrap_or_else(|| {
        panic!("JSON body should have array field '{}'", field)
    });

    assert_eq!(
        array.len(),
        expected_length,
        "Array '{}' should have {} elements",
        field,
        expected_length
    );
}

/// Validate that messages array has correct structure
pub fn assert_messages_format(body: &Value, expected_count: usize) {
    assert_json_array_length(body, "messages", expected_count);

    let messages = body.get("messages").unwrap().as_array().unwrap();
    for (i, msg) in messages.iter().enumerate() {
        assert!(
            msg.is_object(),
            "Message {} should be an object",
            i
        );
        assert!(
            msg.get("role").is_some(),
            "Message {} should have 'role' field",
            i
        );
        assert!(
            msg.get("content").is_some(),
            "Message {} should have 'content' field",
            i
        );
    }
}

/// Validate request method
pub fn assert_method(request: &Request, expected: &str) {
    assert_eq!(
        request.method.as_str(),
        expected,
        "Request method should be {}",
        expected
    );
}

/// Validate request path
pub fn assert_path(request: &Request, expected: &str) {
    assert_eq!(
        request.url.path(),
        expected,
        "Request path should be {}",
        expected
    );
}

/// Validate that request path matches regex pattern
pub fn assert_path_matches(request: &Request, pattern: &str) {
    let path = request.url.path();
    let re = regex::Regex::new(pattern).unwrap();
    assert!(
        re.is_match(path),
        "Request path '{}' should match pattern '{}'",
        path,
        pattern
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_json_field_validation() {
        let body = json!({
            "model": "test-model",
            "temperature": 0.7,
            "stream": false
        });

        assert_json_string_field(&body, "model", "test-model");
        assert_json_bool_field(&body, "stream", false);
    }

    #[test]
    fn test_array_validation() {
        let body = json!({
            "messages": [
                {"role": "user", "content": "Hello"},
                {"role": "assistant", "content": "Hi"}
            ]
        });

        assert_json_array_length(&body, "messages", 2);
        assert_messages_format(&body, 2);
    }
}
