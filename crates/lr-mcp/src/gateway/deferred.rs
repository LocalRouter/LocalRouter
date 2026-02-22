use regex::Regex;
use serde_json::json;
use std::collections::HashMap;

use super::types::*;

/// Minimum activations for search results
const MIN_ACTIVATIONS: usize = 3;

/// Search mode for the deferred search tool
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SearchMode {
    /// Regex pattern matching against names, descriptions, and argument info
    Regex,
    /// BM25 ranking (term frequency / inverse document frequency)
    Bm25,
}

impl SearchMode {
    pub fn parse_str(s: &str) -> Self {
        match s {
            "bm25" => Self::Bm25,
            _ => Self::Regex,
        }
    }
}

/// Create the virtual search tool for deferred loading.
///
/// The description and `type` enum adapt based on which item types are deferred.
/// Tools are always deferred when the search tool exists; resources and prompts
/// depend on the client's declared capabilities.
pub fn create_search_tool(resources_deferred: bool, prompts_deferred: bool) -> NamespacedTool {
    // Build the list of searchable types
    let mut type_names: Vec<&str> = vec!["tools"];
    if resources_deferred {
        type_names.push("resources");
    }
    if prompts_deferred {
        type_names.push("prompts");
    }

    // Build description
    let types_label = match type_names.len() {
        1 => type_names[0].to_string(),
        2 => format!("{} and {}", type_names[0], type_names[1]),
        _ => {
            let last = type_names.last().unwrap();
            let rest = &type_names[..type_names.len() - 1];
            format!("{}, and {}", rest.join(", "), last)
        }
    };

    let description = format!(
        "Search for {} across all connected MCP servers. \
         Use this to discover and activate capabilities before using them. \
         Activated items will remain available for the rest of the session.",
        types_label
    );

    // Build type enum values
    let mut type_enum: Vec<&str> = type_names.clone();
    if type_names.len() > 1 {
        type_enum.push("all");
    }
    let default_type = if type_names.len() > 1 { "all" } else { "tools" };

    NamespacedTool {
        name: "search".to_string(),
        original_name: "search".to_string(),
        server_id: "_gateway".to_string(),
        description: Some(description),
        input_schema: json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Search query — regex pattern (in regex mode) or keywords (in bm25 mode)"
                },
                "type": {
                    "type": "string",
                    "enum": type_enum,
                    "default": default_type,
                    "description": "What to search for"
                },
                "mode": {
                    "type": "string",
                    "enum": ["regex", "bm25"],
                    "default": "regex",
                    "description": "Search mode: 'regex' for pattern matching (default), 'bm25' for relevance ranking"
                },
                "limit": {
                    "type": "integer",
                    "default": 10,
                    "minimum": 1,
                    "maximum": 50,
                    "description": "Maximum results to activate"
                }
            },
            "required": ["query"]
        }),
    }
}

// ---------------------------------------------------------------------------
// Searchable text extraction
// ---------------------------------------------------------------------------

/// Extract all searchable text for a tool: name, description, and argument names/descriptions.
fn tool_searchable_text(tool: &NamespacedTool) -> String {
    let mut parts = vec![tool.name.clone()];
    if let Some(desc) = &tool.description {
        parts.push(desc.clone());
    }
    // Extract argument names and descriptions from input_schema
    if let Some(props) = tool
        .input_schema
        .get("properties")
        .and_then(|p| p.as_object())
    {
        for (arg_name, arg_schema) in props {
            parts.push(arg_name.clone());
            if let Some(arg_desc) = arg_schema.get("description").and_then(|d| d.as_str()) {
                parts.push(arg_desc.to_string());
            }
        }
    }
    parts.join(" ")
}

fn resource_searchable_text(resource: &NamespacedResource) -> String {
    let mut parts = vec![resource.name.clone()];
    if let Some(desc) = &resource.description {
        parts.push(desc.clone());
    }
    parts.push(resource.uri.clone());
    parts.join(" ")
}

fn prompt_searchable_text(prompt: &NamespacedPrompt) -> String {
    let mut parts = vec![prompt.name.clone()];
    if let Some(desc) = &prompt.description {
        parts.push(desc.clone());
    }
    // Include argument names and descriptions
    if let Some(args) = &prompt.arguments {
        for arg in args {
            parts.push(arg.name.clone());
            if let Some(arg_desc) = &arg.description {
                parts.push(arg_desc.clone());
            }
        }
    }
    parts.join(" ")
}

// ---------------------------------------------------------------------------
// Regex search
// ---------------------------------------------------------------------------

/// Search items using regex pattern matching.
fn search_regex<T, F>(
    pattern: &str,
    catalog: &[T],
    limit: usize,
    text_fn: F,
    name_fn: fn(&T) -> &str,
) -> Vec<(T, f32)>
where
    T: Clone,
    F: Fn(&T) -> String,
{
    // Build regex, fall back to escaped literal if invalid
    let re = Regex::new(&format!("(?i){}", pattern))
        .unwrap_or_else(|_| Regex::new(&format!("(?i){}", regex::escape(pattern))).unwrap());

    let mut matches: Vec<(T, f32)> = catalog
        .iter()
        .filter_map(|item| {
            let text = text_fn(item);
            let name = name_fn(item);

            // Count matches and compute a score
            let match_count = re.find_iter(&text).count();
            if match_count == 0 {
                return None;
            }

            // Higher score for name matches vs description/argument matches
            let name_lower = name.to_lowercase();
            let name_score = if re.is_match(&name_lower) { 5.0 } else { 0.0 };
            let total_score = name_score + match_count as f32;

            Some((item.clone(), total_score))
        })
        .collect();

    // Sort by score descending
    matches.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    // Ensure minimum activations
    let take = limit.max(MIN_ACTIVATIONS).min(matches.len());
    matches.truncate(take);

    matches
}

// ---------------------------------------------------------------------------
// BM25 search
// ---------------------------------------------------------------------------

/// BM25 parameters
const BM25_K1: f32 = 1.2;
const BM25_B: f32 = 0.75;

/// Tokenize text into lowercase terms.
fn tokenize(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect()
}

/// Search items using BM25 ranking.
fn search_bm25<T, F>(query: &str, catalog: &[T], limit: usize, text_fn: F) -> Vec<(T, f32)>
where
    T: Clone,
    F: Fn(&T) -> String,
{
    if catalog.is_empty() {
        return Vec::new();
    }

    let query_terms = tokenize(query);
    if query_terms.is_empty() {
        return Vec::new();
    }

    let n = catalog.len() as f32;

    // Pre-tokenize all documents
    let docs: Vec<Vec<String>> = catalog
        .iter()
        .map(|item| tokenize(&text_fn(item)))
        .collect();

    // Compute average document length
    let avg_dl: f32 = if docs.is_empty() {
        1.0
    } else {
        docs.iter().map(|d| d.len() as f32).sum::<f32>() / n
    };

    // Compute document frequency for each query term
    let mut df: HashMap<&str, f32> = HashMap::new();
    for term in &query_terms {
        let count = docs.iter().filter(|doc| doc.contains(term)).count() as f32;
        df.insert(term.as_str(), count);
    }

    // Score each document
    let mut scored: Vec<(T, f32)> = catalog
        .iter()
        .zip(docs.iter())
        .filter_map(|(item, doc_tokens)| {
            let dl = doc_tokens.len() as f32;
            let mut score = 0.0f32;

            for term in &query_terms {
                let tf = doc_tokens.iter().filter(|t| *t == term).count() as f32;
                if tf == 0.0 {
                    continue;
                }

                let doc_freq = *df.get(term.as_str()).unwrap_or(&0.0);
                // IDF with smoothing
                let idf = ((n - doc_freq + 0.5) / (doc_freq + 0.5) + 1.0).ln();

                // BM25 term score
                let tf_component =
                    (tf * (BM25_K1 + 1.0)) / (tf + BM25_K1 * (1.0 - BM25_B + BM25_B * dl / avg_dl));

                score += idf * tf_component;
            }

            if score > 0.0 {
                Some((item.clone(), score))
            } else {
                None
            }
        })
        .collect();

    // Sort by score descending
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    // Ensure minimum activations
    let take = limit.max(MIN_ACTIVATIONS).min(scored.len());
    scored.truncate(take);

    scored
}

// ---------------------------------------------------------------------------
// Public search functions
// ---------------------------------------------------------------------------

/// Search tools and return matches with relevance scores
pub fn search_tools(
    query: &str,
    catalog: &[NamespacedTool],
    limit: usize,
    mode: SearchMode,
) -> Vec<(NamespacedTool, f32)> {
    match mode {
        SearchMode::Regex => search_regex(query, catalog, limit, tool_searchable_text, |t| &t.name),
        SearchMode::Bm25 => search_bm25(query, catalog, limit, tool_searchable_text),
    }
}

/// Search resources and return matches with relevance scores
pub fn search_resources(
    query: &str,
    catalog: &[NamespacedResource],
    limit: usize,
    mode: SearchMode,
) -> Vec<(NamespacedResource, f32)> {
    match mode {
        SearchMode::Regex => {
            search_regex(query, catalog, limit, resource_searchable_text, |r| &r.name)
        }
        SearchMode::Bm25 => search_bm25(query, catalog, limit, resource_searchable_text),
    }
}

/// Search prompts and return matches with relevance scores
pub fn search_prompts(
    query: &str,
    catalog: &[NamespacedPrompt],
    limit: usize,
    mode: SearchMode,
) -> Vec<(NamespacedPrompt, f32)> {
    match mode {
        SearchMode::Regex => {
            search_regex(query, catalog, limit, prompt_searchable_text, |p| &p.name)
        }
        SearchMode::Bm25 => search_bm25(query, catalog, limit, prompt_searchable_text),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_tool(name: &str, desc: &str, schema: serde_json::Value) -> NamespacedTool {
        NamespacedTool {
            name: name.to_string(),
            original_name: name.to_string(),
            server_id: "server".to_string(),
            description: Some(desc.to_string()),
            input_schema: schema,
        }
    }

    fn make_resource(name: &str, desc: &str) -> NamespacedResource {
        NamespacedResource {
            name: name.to_string(),
            original_name: name.to_string(),
            server_id: "server".to_string(),
            description: Some(desc.to_string()),
            uri: format!("resource://{}", name),
            mime_type: None,
        }
    }

    #[test]
    fn test_create_search_tool_all_types() {
        let tool = create_search_tool(true, true);
        assert_eq!(tool.name, "search");
        assert_eq!(tool.server_id, "_gateway");
        let desc = tool.description.unwrap();
        assert!(desc.contains("tools"));
        assert!(desc.contains("resources"));
        assert!(desc.contains("prompts"));
        // Type enum should include all + individual types
        let type_enum = &tool.input_schema["properties"]["type"]["enum"];
        assert!(type_enum.as_array().unwrap().iter().any(|v| v == "all"));
        assert!(type_enum
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v == "resources"));
        assert!(type_enum.as_array().unwrap().iter().any(|v| v == "prompts"));
    }

    #[test]
    fn test_create_search_tool_tools_only() {
        let tool = create_search_tool(false, false);
        let desc = tool.description.unwrap();
        assert!(desc.contains("tools"));
        assert!(!desc.contains("resources"));
        assert!(!desc.contains("prompts"));
        // Type enum should only have "tools", no "all"
        let type_enum = &tool.input_schema["properties"]["type"]["enum"];
        let values: Vec<&str> = type_enum
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert_eq!(values, vec!["tools"]);
    }

    #[test]
    fn test_create_search_tool_partial() {
        let tool = create_search_tool(true, false);
        let desc = tool.description.unwrap();
        assert!(desc.contains("tools and resources"));
        assert!(!desc.contains("prompts"));
        let type_enum = &tool.input_schema["properties"]["type"]["enum"];
        let values: Vec<&str> = type_enum
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert_eq!(values, vec!["tools", "resources", "all"]);
    }

    #[test]
    fn test_regex_search_name_match() {
        let catalog = vec![
            make_tool("read_file", "Read a file", json!({})),
            make_tool("write_file", "Write a file", json!({})),
        ];

        let results = search_tools("read", &catalog, 10, SearchMode::Regex);
        assert!(!results.is_empty());
        assert_eq!(results[0].0.name, "read_file");
    }

    #[test]
    fn test_regex_search_argument_match() {
        let catalog = vec![
            make_tool(
                "execute",
                "Run something",
                json!({
                    "type": "object",
                    "properties": {
                        "file_path": {
                            "type": "string",
                            "description": "Path to the Python script to execute"
                        }
                    }
                }),
            ),
            make_tool("list", "List items", json!({})),
        ];

        // Search for "python" which only appears in argument description
        let results = search_tools("python", &catalog, 10, SearchMode::Regex);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0.name, "execute");
    }

    #[test]
    fn test_regex_search_pattern() {
        let catalog = vec![
            make_tool("read_file", "Read a file", json!({})),
            make_tool("read_dir", "Read a directory", json!({})),
            make_tool("write_file", "Write a file", json!({})),
        ];

        let results = search_tools("read_.*", &catalog, 10, SearchMode::Regex);
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_regex_invalid_pattern_falls_back_to_literal() {
        let catalog = vec![make_tool("test[tool", "A tool with brackets", json!({}))];

        // Invalid regex like "[tool" should be escaped and match literally
        let results = search_tools("[tool", &catalog, 10, SearchMode::Regex);
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_bm25_search_basic() {
        let catalog = vec![
            make_tool("read_file", "Read a file from disk", json!({})),
            make_tool("write_file", "Write a file to disk", json!({})),
            make_tool("delete_file", "Delete a file", json!({})),
        ];

        let results = search_tools("read file", &catalog, 10, SearchMode::Bm25);
        assert!(!results.is_empty());
        // read_file should rank highest (matches both terms in name and description)
        assert_eq!(results[0].0.name, "read_file");
    }

    #[test]
    fn test_bm25_search_argument_match() {
        let catalog = vec![
            make_tool(
                "execute",
                "Run a script",
                json!({
                    "type": "object",
                    "properties": {
                        "language": {
                            "type": "string",
                            "description": "Programming language: python, javascript, ruby"
                        }
                    }
                }),
            ),
            make_tool("list", "List items", json!({})),
        ];

        let results = search_tools("python", &catalog, 10, SearchMode::Bm25);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0.name, "execute");
    }

    #[test]
    fn test_search_tools_limit() {
        let catalog = vec![
            make_tool("tool1", "test tool", json!({})),
            make_tool("tool2", "test tool", json!({})),
            make_tool("tool3", "test tool", json!({})),
            make_tool("tool4", "test tool", json!({})),
            make_tool("tool5", "test tool", json!({})),
        ];

        // Limit 2 but MIN_ACTIVATIONS is 3, so should get 3
        let results = search_tools("test", &catalog, 2, SearchMode::Regex);
        assert_eq!(results.len(), 3);

        // Limit 4 should get 4
        let results = search_tools("test", &catalog, 4, SearchMode::Regex);
        assert_eq!(results.len(), 4);
    }

    #[test]
    fn test_search_resources() {
        let catalog = vec![
            make_resource("config", "Application configuration"),
            make_resource("logs", "Application log files"),
        ];

        let results = search_resources("config", &catalog, 10, SearchMode::Regex);
        assert!(!results.is_empty());
        assert_eq!(results[0].0.name, "config");
    }

    #[test]
    fn test_bm25_idf_ranking() {
        // "file" appears in all docs, "read" only in one — "read" should have higher IDF
        let catalog = vec![
            make_tool("read_file", "Read a file", json!({})),
            make_tool("write_file", "Write a file", json!({})),
            make_tool("delete_file", "Delete a file", json!({})),
        ];

        let results = search_tools("read", &catalog, 10, SearchMode::Bm25);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0.name, "read_file");
    }

    #[test]
    fn test_minimum_activations() {
        let catalog = vec![
            make_tool("tool1", "something related", json!({})),
            make_tool("tool2", "also related", json!({})),
            make_tool("tool3", "related too", json!({})),
        ];

        let results = search_tools("related", &catalog, 10, SearchMode::Regex);
        assert!(results.len() >= MIN_ACTIVATIONS || results.len() == catalog.len());
    }
}
