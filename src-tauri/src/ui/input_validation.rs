//! Pure input handling shared by Tauri commands.

pub(crate) fn parse_approval_arguments(
    arguments: Option<&str>,
) -> Result<Option<serde_json::Value>, String> {
    arguments
        .map(|text| {
            serde_json::from_str(text).map_err(|error| format!("Invalid edited arguments: {error}"))
        })
        .transpose()
}

pub(crate) fn skill_document(
    name: &str,
    description: Option<&str>,
    content: &str,
) -> Result<String, String> {
    // JSON strings are YAML-compatible scalars, including embedded line breaks.
    let name = serde_json::to_string(name.trim()).map_err(|error| error.to_string())?;
    let mut document = format!("---\nname: {name}\n");
    if let Some(description) = description.filter(|description| !description.trim().is_empty()) {
        let description =
            serde_json::to_string(description.trim()).map_err(|error| error.to_string())?;
        document.push_str(&format!("description: {description}\n"));
    }
    document.push_str("---\n\n");
    document.push_str(content);
    Ok(document)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn malformed_edits_are_rejected_instead_of_discarded() {
        assert!(parse_approval_arguments(Some(r#"{"redacted": "yes""#)).is_err());
        assert!(parse_approval_arguments(Some("")).is_err());
        assert_eq!(parse_approval_arguments(None).unwrap(), None);
        assert_eq!(
            parse_approval_arguments(Some(r#"{"redacted": "yes"}"#)).unwrap(),
            Some(serde_json::json!({ "redacted": "yes" }))
        );
    }

    #[test]
    fn skill_frontmatter_preserves_quoted_multiline_input() {
        let name = "An \"unusual\" skill\\name";
        let description = "First line\n---\ninjected: false\nSecond line";
        let document = skill_document(name, Some(description), "body\n").unwrap();
        let frontmatter = document
            .strip_prefix("---\n")
            .unwrap()
            .split_once("\n---\n\n")
            .unwrap();
        let values: serde_json::Value = serde_yaml::from_str(frontmatter.0).unwrap();
        assert_eq!(values["name"], name);
        assert_eq!(values["description"], description);
        assert!(values.get("injected").is_none());
        assert_eq!(frontmatter.1, "body\n");
    }

    #[test]
    fn blank_skill_descriptions_are_omitted() {
        assert_eq!(
            skill_document(" example ", Some(" \n "), "body").unwrap(),
            "---\nname: \"example\"\n---\n\nbody"
        );
    }
}
