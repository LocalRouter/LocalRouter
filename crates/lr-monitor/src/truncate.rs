//! Size caps for bodies captured into monitor events.
//!
//! Every capture site has to answer the same question — how much of a body is
//! worth keeping in memory so it can be inspected later — so the cap lives here
//! rather than being re-derived per call site.

use serde_json::Value;

/// Serialized byte length of a JSON value, measured without building the string.
///
/// The common case is a body that fits its cap, and a body that fits should not
/// pay for a full second copy of itself just to be measured.
pub fn serialized_len(value: &Value) -> usize {
    struct Counter(usize);

    impl std::io::Write for Counter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0 += buf.len();
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    let mut counter = Counter(0);
    match serde_json::to_writer(&mut counter, value) {
        Ok(()) => counter.0,
        Err(_) => 0,
    }
}

/// Cap an owned JSON body at `max_bytes` of serialized JSON.
///
/// A body over the cap is replaced by a marker object carrying its real size
/// and a leading slice, so the monitor still shows that a body was captured and
/// how big it was instead of silently holding megabytes of it.
pub fn truncate_json_owned(value: Value, max_bytes: usize) -> Value {
    if serialized_len(&value) <= max_bytes {
        return value;
    }
    marker(&value, max_bytes)
}

/// Borrowing form of [`truncate_json_owned`]; clones when the body fits.
pub fn truncate_json(value: &Value, max_bytes: usize) -> Value {
    if serialized_len(value) <= max_bytes {
        return value.clone();
    }
    marker(value, max_bytes)
}

fn marker(value: &Value, max_bytes: usize) -> Value {
    let serialized = serde_json::to_string(value).unwrap_or_default();
    // `max_bytes` can land inside a multi-byte character, which would panic a
    // plain range slice — bodies routinely carry non-ASCII prose.
    let mut end = max_bytes.min(serialized.len());
    while end > 0 && !serialized.is_char_boundary(end) {
        end -= 1;
    }
    serde_json::json!({
        "_truncated": true,
        "_original_size": serialized.len(),
        "_preview": &serialized[..end],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serialized_len_matches_to_string() {
        for value in [
            serde_json::json!(null),
            serde_json::json!({"a": 1, "b": [1, 2, 3]}),
            serde_json::json!({"text": "héllo wörld ✨"}),
        ] {
            assert_eq!(
                serialized_len(&value),
                serde_json::to_string(&value).unwrap().len()
            );
        }
    }

    #[test]
    fn small_bodies_pass_through_unchanged() {
        let value = serde_json::json!({"model": "gpt-4", "messages": []});
        assert_eq!(truncate_json(&value, 10_000), value);
        assert_eq!(truncate_json_owned(value.clone(), 10_000), value);
    }

    #[test]
    fn large_bodies_become_a_marker() {
        let value = serde_json::json!({"prompt": "x".repeat(50_000)});
        let out = truncate_json(&value, 1_000);

        assert_eq!(out["_truncated"], serde_json::json!(true));
        assert!(out["_original_size"].as_u64().unwrap() > 50_000);
        assert!(out["_preview"].as_str().unwrap().len() <= 1_000);
    }

    #[test]
    fn truncation_never_splits_a_multibyte_character() {
        // A cap that lands mid-character in the serialized form: slicing by
        // bytes alone would panic here.
        let value = serde_json::json!({ "t": "é".repeat(5_000) });
        for cap in 1..64 {
            let out = truncate_json(&value, cap);
            let preview = out["_preview"].as_str().unwrap();
            assert!(preview.len() <= cap);
        }
    }

    #[test]
    fn truncation_shrinks_the_stored_body() {
        let value = serde_json::json!({"prompt": "x".repeat(1_000_000)});
        let out = truncate_json(&value, 4_096);
        assert!(crate::size::json_size(&out) < crate::size::json_size(&value) / 100);
    }
}
