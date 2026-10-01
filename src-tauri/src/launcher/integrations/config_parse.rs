//! Validate existing application settings before merging owned keys.
//!
//! Parse failures must never be treated as empty settings: doing so replaces
//! the user's unrelated configuration during a routine connection change.

use std::path::Path;

pub(super) fn json(data: &str, path: &Path) -> Result<serde_json::Value, String> {
    let value: serde_json::Value = serde_json::from_str(data)
        .map_err(|error| format!("Failed to parse {}: {error}", path.display()))?;
    if !value.is_object() {
        return Err(format!("Expected a JSON object in {}", path.display()));
    }
    Ok(value)
}

pub(super) fn yaml(data: &str, path: &Path) -> Result<serde_yaml::Value, String> {
    let value: serde_yaml::Value = serde_yaml::from_str(data)
        .map_err(|error| format!("Failed to parse {}: {error}", path.display()))?;
    // Empty YAML files are a conventional empty configuration.
    if value.is_null() {
        return Ok(serde_yaml::Value::Mapping(serde_yaml::Mapping::new()));
    }
    if !value.is_mapping() {
        return Err(format!("Expected a YAML mapping in {}", path.display()));
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn malformed_or_non_object_json_cannot_become_empty_settings() {
        let path = Path::new("settings.json");
        for body in ["{", "", "null", "[]", "42", "\"text\""] {
            assert!(json(body, path).is_err(), "accepted {body:?}");
        }
        assert_eq!(json(r#"{"keep": 42}"#, path).unwrap()["keep"], 42);
    }

    #[test]
    fn yaml_requires_a_mapping_but_allows_empty_files() {
        let path = Path::new("settings.yaml");
        for body in ["key: [", "- one\n- two", "42", "a string"] {
            assert!(yaml(body, path).is_err(), "accepted {body:?}");
        }
        assert!(yaml("# empty\n", path).unwrap().is_mapping());
        assert_eq!(yaml("keep: 42", path).unwrap()["keep"].as_i64(), Some(42));
    }
}
