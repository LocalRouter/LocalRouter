use jsonrepair::{Options, StreamRepairer};
use tracing::debug;

/// Streaming JSON syntax repairer that wraps jsonrepair's StreamRepairer.
///
/// Feed content chunks in via `push_content()`, get repaired chunks out.
/// Call `finish()` at the end to flush any remaining buffered content.
///
/// The jsonrepair crate handles internal buffering for ambiguous tokens
/// (e.g., holding back a trailing comma until it sees the next token).
pub struct StreamingSyntaxRepairer {
    repairer: StreamRepairer,
}

impl StreamingSyntaxRepairer {
    pub fn new() -> Self {
        Self {
            repairer: StreamRepairer::new(Options::default()),
        }
    }

    /// Push a content chunk through the repairer.
    /// Returns the repaired output for this chunk (may be empty if buffering).
    pub fn push_content(&mut self, chunk: &str) -> String {
        match self.repairer.push(chunk) {
            Ok(Some(output)) => output,
            Ok(None) => String::new(),
            Err(e) => {
                debug!("Streaming repair push error: {}", e);
                // On error, pass through the chunk as-is
                chunk.to_string()
            }
        }
    }

    /// Flush any remaining buffered content.
    /// Call this when the stream is complete to get the final output.
    pub fn finish(&mut self) -> String {
        match self.repairer.flush() {
            Ok(Some(output)) => output,
            Ok(None) => String::new(),
            Err(e) => {
                debug!("Streaming repair flush error: {}", e);
                String::new()
            }
        }
    }
}

impl Default for StreamingSyntaxRepairer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_streaming_basic() {
        let mut repairer = StreamingSyntaxRepairer::new();
        let mut result = String::new();
        result.push_str(&repairer.push_content(r#"{"name": "#));
        result.push_str(&repairer.push_content(r#""John"}"#));
        result.push_str(&repairer.finish());
        assert!(serde_json::from_str::<serde_json::Value>(&result).is_ok());
    }

    #[test]
    fn test_streaming_trailing_comma() {
        let mut repairer = StreamingSyntaxRepairer::new();
        let mut result = String::new();
        result.push_str(&repairer.push_content(r#"{"name": "John","#));
        result.push_str(&repairer.push_content(r#"}"#));
        result.push_str(&repairer.finish());
        assert!(
            serde_json::from_str::<serde_json::Value>(&result).is_ok(),
            "Failed to parse: {}",
            result
        );
    }

    #[test]
    fn test_streaming_missing_closing() {
        let mut repairer = StreamingSyntaxRepairer::new();
        let mut result = String::new();
        result.push_str(&repairer.push_content(r#"{"name": "John""#));
        // Don't send closing brace - finish should add it
        result.push_str(&repairer.finish());
        assert!(
            serde_json::from_str::<serde_json::Value>(&result).is_ok(),
            "Failed to parse: {}",
            result
        );
    }

    #[test]
    fn test_streaming_character_by_character() {
        let input = r#"{"key": "value"}"#;
        let mut repairer = StreamingSyntaxRepairer::new();
        let mut result = String::new();
        for c in input.chars() {
            result.push_str(&repairer.push_content(&c.to_string()));
        }
        result.push_str(&repairer.finish());
        assert!(
            serde_json::from_str::<serde_json::Value>(&result).is_ok(),
            "Failed to parse: {}",
            result
        );
    }

    #[test]
    fn test_streaming_multiple_chunks() {
        let mut repairer = StreamingSyntaxRepairer::new();
        let mut result = String::new();
        result.push_str(&repairer.push_content(r#"{"items": "#));
        result.push_str(&repairer.push_content(r#"[1, 2"#));
        result.push_str(&repairer.push_content(r#", 3,]"#));
        result.push_str(&repairer.push_content(r#"}"#));
        result.push_str(&repairer.finish());
        assert!(
            serde_json::from_str::<serde_json::Value>(&result).is_ok(),
            "Failed to parse: {}",
            result
        );
    }
}
