use serde_json::json;

use super::types::*;

/// Relevance thresholds for search activation
const HIGH_RELEVANCE_THRESHOLD: f32 = 0.7;
const LOW_RELEVANCE_THRESHOLD: f32 = 0.3;
const MIN_ACTIVATIONS: usize = 3;

/// Create the virtual search tool for deferred loading
pub fn create_search_tool() -> NamespacedTool {
    NamespacedTool {
        name: "search".to_string(),
        original_name: "search".to_string(),
        server_id: "_gateway".to_string(),
        description: Some(
            "Search for tools, resources, or prompts across all connected MCP servers. \
             Use this to discover and activate capabilities before using them. \
             Activated items will remain available for the rest of the session."
                .to_string(),
        ),
        input_schema: json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Search query (keywords or natural language)"
                },
                "type": {
                    "type": "string",
                    "enum": ["tools", "resources", "prompts", "all"],
                    "default": "all",
                    "description": "What to search for"
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

/// Search tools and return matches with relevance scores
pub fn search_tools(
    query: &str,
    catalog: &[NamespacedTool],
    limit: usize,
) -> Vec<(NamespacedTool, f32)> {
    search_items(
        query,
        catalog,
        limit,
        |tool| &tool.name,
        |tool| tool.description.as_deref().unwrap_or(""),
    )
}

/// Search resources and return matches with relevance scores
pub fn search_resources(
    query: &str,
    catalog: &[NamespacedResource],
    limit: usize,
) -> Vec<(NamespacedResource, f32)> {
    search_items(
        query,
        catalog,
        limit,
        |resource| &resource.name,
        |resource| resource.description.as_deref().unwrap_or(""),
    )
}

/// Search prompts and return matches with relevance scores
pub fn search_prompts(
    query: &str,
    catalog: &[NamespacedPrompt],
    limit: usize,
) -> Vec<(NamespacedPrompt, f32)> {
    search_items(
        query,
        catalog,
        limit,
        |prompt| &prompt.name,
        |prompt| prompt.description.as_deref().unwrap_or(""),
    )
}

/// Generic search function with activation logic
fn search_items<T, F1, F2>(
    query: &str,
    catalog: &[T],
    limit: usize,
    name_fn: F1,
    description_fn: F2,
) -> Vec<(T, f32)>
where
    T: Clone,
    F1: Fn(&T) -> &str,
    F2: Fn(&T) -> &str,
{
    // Score all items
    let mut scored: Vec<(T, f32)> = catalog
        .iter()
        .map(|item| {
            let score = calculate_relevance_score(query, name_fn(item), description_fn(item));
            (item.clone(), score)
        })
        .filter(|(_, score)| *score > 0.0)
        .collect();

    // Sort by score descending
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    // Activation logic:
    // 1. Include all items above HIGH threshold
    // 2. If fewer than MIN_ACTIVATIONS, include more until we have MIN_ACTIVATIONS (above LOW threshold)
    let mut to_activate = Vec::new();

    for (item, score) in &scored {
        if *score >= HIGH_RELEVANCE_THRESHOLD {
            to_activate.push((item.clone(), *score));
        }
    }

    if to_activate.len() < MIN_ACTIVATIONS {
        for (item, score) in &scored {
            if *score >= LOW_RELEVANCE_THRESHOLD && to_activate.len() < MIN_ACTIVATIONS {
                // Check if not already added
                if !to_activate
                    .iter()
                    .any(|(existing, _)| name_fn(existing) == name_fn(item))
                {
                    to_activate.push((item.clone(), *score));
                }
            }
        }
    }

    // Apply limit
    to_activate.truncate(limit);

    to_activate
}

/// Calculate relevance score for a search query
fn calculate_relevance_score(query: &str, name: &str, description: &str) -> f32 {
    let query_lower = query.to_lowercase();
    let keywords: Vec<&str> = query_lower.split_whitespace().collect();
    if keywords.is_empty() {
        return 0.0;
    }

    let name_lower = name.to_lowercase();
    let description_lower = description.to_lowercase();

    let mut score = 0.0;

    for keyword in &keywords {
        // Exact match in name (highest weight)
        if name_lower == *keyword {
            score += 5.0;
        }
        // Partial match in name (high weight)
        else if name_lower.contains(keyword) {
            score += 3.0;
        }
        // Match in description (medium weight)
        else if description_lower.contains(keyword) {
            score += 1.0;
        }
    }

    // Normalize by query length
    score / keywords.len() as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_search_tool() {
        let tool = create_search_tool();
        assert_eq!(tool.name, "search");
        assert_eq!(tool.server_id, "_gateway");
        assert!(tool.description.is_some());
    }

    #[test]
    fn test_calculate_relevance_score() {
        // Exact name match should score highest
        let score = calculate_relevance_score("read", "read", "some description");
        assert!(score > 3.0);

        // Partial name match
        let score = calculate_relevance_score("read", "read_file", "some description");
        assert!(score > 1.0);

        // Description match
        let score = calculate_relevance_score("read", "something", "read this file");
        assert!(score > 0.0);

        // No match
        let score = calculate_relevance_score("read", "write", "create something");
        assert_eq!(score, 0.0);
    }

    #[test]
    fn test_search_tools_activation_logic() {
        let catalog = vec![
            NamespacedTool {
                name: "filesystem__read_file".to_string(),
                original_name: "read_file".to_string(),
                server_id: "filesystem".to_string(),
                description: Some("Read a file".to_string()),
                input_schema: json!({}),
            },
            NamespacedTool {
                name: "filesystem__write_file".to_string(),
                original_name: "write_file".to_string(),
                server_id: "filesystem".to_string(),
                description: Some("Write a file".to_string()),
                input_schema: json!({}),
            },
            NamespacedTool {
                name: "github__read_issue".to_string(),
                original_name: "read_issue".to_string(),
                server_id: "github".to_string(),
                description: Some("Read an issue".to_string()),
                input_schema: json!({}),
            },
        ];

        let results = search_tools("read", &catalog, 10);

        // Should return tools with "read" in name or description
        assert!(!results.is_empty());
        assert!(results.iter().any(|(tool, _)| tool.name.contains("read")));
    }

    #[test]
    fn test_search_tools_limit() {
        let catalog = vec![
            NamespacedTool {
                name: "tool1".to_string(),
                original_name: "tool1".to_string(),
                server_id: "server".to_string(),
                description: Some("test tool".to_string()),
                input_schema: json!({}),
            },
            NamespacedTool {
                name: "tool2".to_string(),
                original_name: "tool2".to_string(),
                server_id: "server".to_string(),
                description: Some("test tool".to_string()),
                input_schema: json!({}),
            },
            NamespacedTool {
                name: "tool3".to_string(),
                original_name: "tool3".to_string(),
                server_id: "server".to_string(),
                description: Some("test tool".to_string()),
                input_schema: json!({}),
            },
        ];

        let results = search_tools("test", &catalog, 2);
        assert!(results.len() <= 2);
    }

    #[test]
    fn test_minimum_activations() {
        // Even with low scores, should activate at least MIN_ACTIVATIONS (3)
        let catalog = vec![
            NamespacedTool {
                name: "tool1".to_string(),
                original_name: "tool1".to_string(),
                server_id: "server".to_string(),
                description: Some("something related".to_string()),
                input_schema: json!({}),
            },
            NamespacedTool {
                name: "tool2".to_string(),
                original_name: "tool2".to_string(),
                server_id: "server".to_string(),
                description: Some("also related".to_string()),
                input_schema: json!({}),
            },
            NamespacedTool {
                name: "tool3".to_string(),
                original_name: "tool3".to_string(),
                server_id: "server".to_string(),
                description: Some("related too".to_string()),
                input_schema: json!({}),
            },
        ];

        let results = search_tools("related", &catalog, 10);
        // Should activate at least 3 if available and above LOW threshold
        assert!(results.len() >= MIN_ACTIVATIONS || results.len() == catalog.len());
    }
}
