//! Shedding the bulk of an old event's payload so the event stays listed.
//!
//! The store's memory budget is mostly spent on request and response bodies.
//! Dropping whole events to stay inside it empties the monitor long before the
//! event-count limit is reached when traffic carries long contexts, so the
//! store first shortens the payloads of its oldest events and keeps their
//! metadata, usage and list previews. Only when that is not enough are events
//! evicted.

use serde_json::Value;

use crate::types::MonitorEvent;

/// Longest string kept in a trimmed payload, in bytes.
const KEEP_STRING_BYTES: usize = 256;

/// Array elements kept from each end of a long array in a trimmed payload.
const KEEP_ARRAY_ENDS: usize = 8;

/// Appended to a string that was shortened.
const TRIMMED_SUFFIX: &str = "… [trimmed to save memory]";

/// Shorten the event's payload in place: long strings keep their start and
/// long arrays keep their first and last elements, so message histories still
/// show how a conversation began and where it ended.
///
/// Returns `false`, leaving the event untouched, when the shortened payload no
/// longer fits the event's typed shape.
pub(crate) fn trim_payload(event: &mut MonitorEvent) -> bool {
    let Ok(mut value) = serde_json::to_value(&event.data) else {
        return false;
    };
    if !shrink(&mut value) {
        return false;
    }
    match serde_json::from_value(value) {
        Ok(data) => {
            event.data = data;
            event.payload_trimmed = true;
            true
        }
        Err(_) => false,
    }
}

/// Returns whether anything was shortened.
fn shrink(value: &mut Value) -> bool {
    match value {
        Value::String(text) => {
            if text.len() <= KEEP_STRING_BYTES + TRIMMED_SUFFIX.len() {
                return false;
            }
            let mut end = KEEP_STRING_BYTES;
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            text.truncate(end);
            text.push_str(TRIMMED_SUFFIX);
            true
        }
        Value::Array(items) => {
            let mut changed = false;
            if items.len() > 2 * KEEP_ARRAY_ENDS {
                items.drain(KEEP_ARRAY_ENDS..items.len() - KEEP_ARRAY_ENDS);
                changed = true;
            }
            for item in items.iter_mut() {
                changed |= shrink(item);
            }
            changed
        }
        Value::Object(map) => {
            let mut changed = false;
            for item in map.values_mut() {
                changed |= shrink(item);
            }
            changed
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn long_strings_keep_their_start_on_a_char_boundary() {
        let mut value = json!("é".repeat(1000));
        assert!(shrink(&mut value));
        let text = value.as_str().unwrap();
        assert!(text.ends_with(TRIMMED_SUFFIX));
        assert!(text.len() <= KEEP_STRING_BYTES + TRIMMED_SUFFIX.len());
        assert!(text.starts_with("éé"));
    }

    #[test]
    fn long_arrays_keep_both_ends() {
        let mut value = json!((0..100).collect::<Vec<_>>());
        assert!(shrink(&mut value));
        let items: Vec<i64> = serde_json::from_value(value).unwrap();
        assert_eq!(items.len(), 2 * KEEP_ARRAY_ENDS);
        assert_eq!(items[0], 0);
        assert_eq!(*items.last().unwrap(), 99);
    }

    #[test]
    fn small_values_are_untouched() {
        let original = json!({"model": "gpt", "messages": [{"role": "user", "content": "hi"}]});
        let mut value = original.clone();
        assert!(!shrink(&mut value));
        assert_eq!(value, original);
    }
}
