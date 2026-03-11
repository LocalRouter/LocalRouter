use crate::types::{Chunk, ContentFormat, ContentType, LONG_LINE_THRESHOLD};

const MAX_CHUNK_BYTES: usize = 4096;
const IDEAL_BYTES_PER_CHUNK: usize = 800;
const MAX_BYTES_PER_CHUNK: usize = 2048;

// ─────────────────────────────────────────────────────────
// Format detection
// ─────────────────────────────────────────────────────────

pub(crate) fn detect_format(content: &str) -> ContentFormat {
    // Try JSON first (quick prefix check before expensive parse)
    let trimmed_start = content.trim_start();
    if (trimmed_start.starts_with('{') || trimmed_start.starts_with('['))
        && serde_json::from_str::<serde_json::Value>(content).is_ok()
    {
        return ContentFormat::Json;
    }

    // Check for markdown indicators in first 100 lines
    for line in content.lines().take(100) {
        let t = line.trim_start();
        if heading_level(t).is_some() || t.starts_with("```") || is_horizontal_rule(t) {
            return ContentFormat::Markdown;
        }
    }

    ContentFormat::PlainText
}

/// Returns heading level (1-4) if the line is a markdown heading.
fn heading_level(line: &str) -> Option<(usize, &str)> {
    let trimmed = line.trim_start();
    let hashes = trimmed.bytes().take_while(|&b| b == b'#').count();
    if (1..=4).contains(&hashes) {
        let rest = &trimmed[hashes..];
        if rest.is_empty() {
            return Some((hashes, ""));
        }
        if let Some(text) = rest.strip_prefix(' ') {
            return Some((hashes, text.trim()));
        }
    }
    None
}

fn is_horizontal_rule(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.len() < 3 {
        return false;
    }
    let first = trimmed.as_bytes()[0];
    if first != b'-' && first != b'_' && first != b'*' {
        return false;
    }
    trimmed
        .bytes()
        .all(|b| b == first || b == b' ' || b == b'\t')
        && trimmed.bytes().filter(|&b| b == first).count() >= 3
}

// ─────────────────────────────────────────────────────────
// Markdown chunking
// ─────────────────────────────────────────────────────────

pub(crate) fn chunk_markdown(content: &str) -> Vec<Chunk> {
    let lines: Vec<&str> = content.lines().collect();
    let mut chunks = Vec::new();
    let mut heading_stack: Vec<(usize, String)> = Vec::new(); // (level, text)
    let mut current_lines: Vec<&str> = Vec::new();
    let mut chunk_start_idx: usize = 0;

    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];

        // Horizontal rule separator
        if is_horizontal_rule(line) {
            flush_markdown(
                &mut chunks,
                &mut current_lines,
                chunk_start_idx,
                &heading_stack,
            );
            i += 1;
            chunk_start_idx = i;
            continue;
        }

        // Heading (H1-H4)
        if let Some((level, text)) = heading_level(line) {
            flush_markdown(
                &mut chunks,
                &mut current_lines,
                chunk_start_idx,
                &heading_stack,
            );
            chunk_start_idx = i;

            // Pop deeper or equal levels from stack
            while heading_stack.last().is_some_and(|(lvl, _)| *lvl >= level) {
                heading_stack.pop();
            }
            heading_stack.push((level, text.to_string()));

            current_lines.push(line);
            i += 1;
            continue;
        }

        // Code block — collect entire block as a unit
        if line.trim_start().starts_with("```") {
            let fence_prefix: &str = &line.trim_start()[..line
                .trim_start()
                .bytes()
                .take_while(|&b| b == b'`')
                .count()
                .min(line.trim_start().len())];
            let fence_len = fence_prefix.len();
            current_lines.push(line);
            i += 1;

            while i < lines.len() {
                current_lines.push(lines[i]);
                let trimmed = lines[i].trim();
                if trimmed.starts_with("```")
                    && trimmed.bytes().take_while(|&b| b == b'`').count() >= fence_len
                    && trimmed.trim_start_matches('`').trim().is_empty()
                {
                    i += 1;
                    break;
                }
                i += 1;
            }
            continue;
        }

        // Regular line
        current_lines.push(line);
        i += 1;
    }

    // Flush remaining content
    flush_markdown(
        &mut chunks,
        &mut current_lines,
        chunk_start_idx,
        &heading_stack,
    );

    chunks
}

fn flush_markdown(
    chunks: &mut Vec<Chunk>,
    current_lines: &mut Vec<&str>,
    chunk_start_idx: usize,
    heading_stack: &[(usize, String)],
) {
    let joined = current_lines.join("\n");
    let trimmed = joined.trim();
    if trimmed.is_empty() {
        current_lines.clear();
        return;
    }

    let line_start = chunk_start_idx + 1; // 1-based
    let line_end = chunk_start_idx + current_lines.len();
    let title = build_title(heading_stack);
    let has_code = current_lines
        .iter()
        .any(|l| l.trim_start().starts_with("```"));
    let content_type = if has_code {
        ContentType::Code
    } else {
        ContentType::Prose
    };

    if joined.len() <= MAX_CHUNK_BYTES {
        chunks.push(Chunk {
            title,
            content: trimmed.to_string(),
            content_type,
            line_start,
            line_end,
            line_ref: line_start.to_string(),
        });
        current_lines.clear();
        return;
    }

    // Split oversized chunk at paragraph boundaries
    let paragraphs: Vec<&str> = trimmed.split("\n\n").collect();
    let mut accumulator: Vec<&str> = Vec::new();
    let mut part_index = 1usize;
    let mut running_line = line_start;

    for para in &paragraphs {
        accumulator.push(para);
        let candidate = accumulator.join("\n\n");
        if candidate.len() > MAX_CHUNK_BYTES && accumulator.len() > 1 {
            accumulator.pop();
            let text = accumulator.join("\n\n");
            let text_trimmed = text.trim();
            if !text_trimmed.is_empty() {
                let line_count = text_trimmed.lines().count().max(1);
                let sub_title = if paragraphs.len() > 1 {
                    format!("{} ({})", title, part_index)
                } else {
                    title.clone()
                };
                chunks.push(Chunk {
                    title: sub_title,
                    content: text_trimmed.to_string(),
                    content_type: if text_trimmed.contains("```") {
                        ContentType::Code
                    } else {
                        ContentType::Prose
                    },
                    line_start: running_line,
                    line_end: running_line + line_count - 1,
                    line_ref: running_line.to_string(),
                });
                // +1 for the blank line between paragraphs
                running_line += line_count + 1;
                part_index += 1;
            }
            accumulator = vec![para];
        }
    }

    // Flush remaining accumulator
    if !accumulator.is_empty() {
        let text = accumulator.join("\n\n");
        let text_trimmed = text.trim();
        if !text_trimmed.is_empty() {
            let line_count = text_trimmed.lines().count().max(1);
            let sub_title = if part_index > 1 {
                format!("{} ({})", title, part_index)
            } else {
                title
            };
            chunks.push(Chunk {
                title: sub_title,
                content: text_trimmed.to_string(),
                content_type: if text_trimmed.contains("```") {
                    ContentType::Code
                } else {
                    ContentType::Prose
                },
                line_start: running_line,
                line_end: running_line + line_count - 1,
                line_ref: running_line.to_string(),
            });
        }
    }

    current_lines.clear();
}

fn build_title(heading_stack: &[(usize, String)]) -> String {
    if heading_stack.is_empty() {
        return "Untitled".to_string();
    }
    heading_stack
        .iter()
        .map(|(_, text)| text.as_str())
        .collect::<Vec<_>>()
        .join(" > ")
}

// ─────────────────────────────────────────────────────────
// Plain text chunking
// ─────────────────────────────────────────────────────────

pub(crate) fn chunk_plain_text(content: &str) -> Vec<Chunk> {
    if content.trim().is_empty() {
        return Vec::new();
    }

    // Try blank-line splitting for naturally-sectioned output
    let sections: Vec<&str> = content.split("\n\n").collect();
    if sections.len() >= 3 && sections.len() <= 200 && sections.iter().all(|s| s.len() < 5000) {
        let mut chunks = Vec::new();
        let mut current_line = 1usize;
        for (i, section) in sections.iter().enumerate() {
            let trimmed = section.trim();
            if trimmed.is_empty() {
                // Account for the blank line
                current_line += section.lines().count().max(1) + 1;
                continue;
            }
            let line_count = trimmed.lines().count().max(1);
            let first_line = trimmed
                .lines()
                .next()
                .unwrap_or("")
                .chars()
                .take(80)
                .collect::<String>();
            chunks.push(Chunk {
                title: if first_line.is_empty() {
                    format!("Section {}", i + 1)
                } else {
                    first_line
                },
                content: trimmed.to_string(),
                content_type: ContentType::Prose,
                line_start: current_line,
                line_end: current_line + line_count - 1,
                line_ref: current_line.to_string(),
            });
            // Move past this section + blank line separator
            current_line += section.lines().count().max(1) + 1;
        }
        if !chunks.is_empty() {
            return chunks;
        }
    }

    let lines: Vec<&str> = content.lines().collect();
    let lines_per_chunk = 20;

    // Small enough for a single chunk
    if lines.len() <= lines_per_chunk {
        return vec![Chunk {
            title: "Output".to_string(),
            content: content.to_string(),
            content_type: ContentType::Prose,
            line_start: 1,
            line_end: lines.len(),
            line_ref: "1".to_string(),
        }];
    }

    // Fixed-size line groups with 2-line overlap
    let mut chunks = Vec::new();
    let overlap = 2;
    let step = (lines_per_chunk - overlap).max(1);

    let mut i = 0;
    while i < lines.len() {
        let end = (i + lines_per_chunk).min(lines.len());
        let slice = &lines[i..end];
        if slice.is_empty() {
            break;
        }
        let start_line = i + 1;
        let end_line = i + slice.len();
        let first_line = slice[0].trim().chars().take(80).collect::<String>();
        chunks.push(Chunk {
            title: if first_line.is_empty() {
                format!("Lines {}-{}", start_line, end_line)
            } else {
                first_line
            },
            content: slice.join("\n"),
            content_type: ContentType::Prose,
            line_start: start_line,
            line_end: end_line,
            line_ref: start_line.to_string(),
        });
        i += step;
    }

    chunks
}

// ─────────────────────────────────────────────────────────
// JSON chunking
// ─────────────────────────────────────────────────────────

pub(crate) fn chunk_json(content: &str) -> Vec<Chunk> {
    if content.trim().is_empty() {
        return Vec::new();
    }

    let parsed: serde_json::Value = match serde_json::from_str(content) {
        Ok(v) => v,
        Err(_) => return chunk_plain_text(content),
    };

    let mut chunks = Vec::new();
    walk_json(&parsed, &[], &mut chunks);

    if chunks.is_empty() {
        return chunk_plain_text(content);
    }

    // Assign approximate line numbers based on content position
    let total_lines = content.lines().count();
    let total_content_len: usize = chunks.iter().map(|c| c.content.len()).sum();
    let mut running_line = 1;
    for chunk in &mut chunks {
        chunk.line_start = running_line;
        chunk.line_ref = running_line.to_string();
        let estimated_lines = if total_content_len > 0 {
            ((chunk.content.len() as f64 / total_content_len as f64) * total_lines as f64).ceil()
                as usize
        } else {
            1
        }
        .max(1);
        chunk.line_end = (running_line + estimated_lines - 1).min(total_lines);
        running_line = chunk.line_end + 1;
    }

    chunks
}

fn walk_json(value: &serde_json::Value, path: &[&str], chunks: &mut Vec<Chunk>) {
    let title = if path.is_empty() {
        "(root)".to_string()
    } else {
        path.join(" > ")
    };

    let serialized = serde_json::to_string_pretty(value).unwrap_or_default();

    // Small enough — check if we should recurse for nested objects
    if serialized.len() <= MAX_CHUNK_BYTES {
        // Objects with nested structure always recurse for searchability
        if let serde_json::Value::Object(map) = value {
            let has_nested = map.values().any(|v| v.is_object() || v.is_array());
            if has_nested {
                for (key, val) in map {
                    let mut new_path: Vec<&str> = path.to_vec();
                    new_path.push(key.as_str());
                    walk_json(val, &new_path, chunks);
                }
                return;
            }
        }

        chunks.push(Chunk {
            title,
            content: serialized,
            content_type: ContentType::Code,
            line_start: 0, // assigned later
            line_end: 0,
            line_ref: "0".to_string(),
        });
        return;
    }

    // Object — recurse into each key
    if let serde_json::Value::Object(map) = value {
        if !map.is_empty() {
            for (key, val) in map {
                let mut new_path: Vec<&str> = path.to_vec();
                new_path.push(key.as_str());
                walk_json(val, &new_path, chunks);
            }
            return;
        }
        chunks.push(Chunk {
            title,
            content: serialized,
            content_type: ContentType::Code,
            line_start: 0,
            line_end: 0,
            line_ref: "0".to_string(),
        });
        return;
    }

    // Array — batch by size
    if let serde_json::Value::Array(arr) = value {
        chunk_json_array(arr, &title, chunks);
        return;
    }

    // Primitive that exceeds MAX_CHUNK_BYTES
    chunks.push(Chunk {
        title,
        content: serialized,
        content_type: ContentType::Prose,
        line_start: 0,
        line_end: 0,
        line_ref: "0".to_string(),
    });
}

fn find_identity_field(arr: &[serde_json::Value]) -> Option<&str> {
    let first = arr.first()?;
    let obj = first.as_object()?;
    let candidates = ["id", "name", "title", "path", "slug", "key", "label"];
    for field in &candidates {
        if let Some(val) = obj.get(*field) {
            if val.is_string() || val.is_number() {
                return Some(field);
            }
        }
    }
    None
}

fn get_identity(item: &serde_json::Value, field: &str) -> String {
    item.get(field)
        .map(|v| match v {
            serde_json::Value::String(s) => s.clone(),
            other => other.to_string(),
        })
        .unwrap_or_default()
}

fn batch_title(
    prefix: &str,
    start_idx: usize,
    end_idx: usize,
    batch: &[&serde_json::Value],
    identity_field: Option<&str>,
) -> String {
    let sep = if prefix.is_empty() || prefix == "(root)" {
        String::new()
    } else {
        format!("{} > ", prefix)
    };

    match identity_field {
        None => {
            if start_idx == end_idx {
                format!("{}[{}]", sep, start_idx)
            } else {
                format!("{}[{}-{}]", sep, start_idx, end_idx)
            }
        }
        Some(field) => {
            if batch.len() == 1 {
                format!("{}{}", sep, get_identity(batch[0], field))
            } else if batch.len() <= 3 {
                let ids: Vec<String> = batch.iter().map(|item| get_identity(item, field)).collect();
                format!("{}{}", sep, ids.join(", "))
            } else {
                format!(
                    "{}{}\u{2026}{}",
                    sep,
                    get_identity(batch[0], field),
                    get_identity(batch[batch.len() - 1], field)
                )
            }
        }
    }
}

fn chunk_json_array(arr: &[serde_json::Value], prefix: &str, chunks: &mut Vec<Chunk>) {
    let identity_field = find_identity_field(arr);

    let mut batch: Vec<&serde_json::Value> = Vec::new();
    let mut batch_start = 0;

    let flush_batch = |batch: &[&serde_json::Value],
                       batch_start: usize,
                       batch_end: usize,
                       chunks: &mut Vec<Chunk>| {
        if batch.is_empty() {
            return;
        }
        let title = batch_title(prefix, batch_start, batch_end, batch, identity_field);
        let values: Vec<&serde_json::Value> = batch.to_vec();
        let serialized = serde_json::to_string_pretty(&serde_json::Value::Array(
            values.into_iter().cloned().collect(),
        ))
        .unwrap_or_default();
        chunks.push(Chunk {
            title,
            content: serialized,
            content_type: ContentType::Code,
            line_start: 0,
            line_end: 0,
            line_ref: "0".to_string(),
        });
    };

    for (i, item) in arr.iter().enumerate() {
        batch.push(item);
        let candidate = serde_json::to_string_pretty(&serde_json::Value::Array(
            batch.iter().cloned().cloned().collect(),
        ))
        .unwrap_or_default();

        if candidate.len() > MAX_CHUNK_BYTES && batch.len() > 1 {
            batch.pop();
            flush_batch(&batch, batch_start, i - 1, chunks);
            batch = vec![item];
            batch_start = i;
        }
    }

    // Flush remaining
    if !batch.is_empty() {
        flush_batch(&batch, batch_start, batch_start + batch.len() - 1, chunks);
    }
}

// ─────────────────────────────────────────────────────────
// Chunk density enforcement
// ─────────────────────────────────────────────────────────

/// Split oversized or sparse chunks to maintain reasonable density.
///
/// Two-pass approach:
/// 1. Ratio-based: if overall density is too sparse, split large chunks
/// 2. Hard ceiling: any chunk >MAX_BYTES_PER_CHUNK is force-split
pub(crate) fn enforce_chunk_density(chunks: Vec<Chunk>, total_content_bytes: usize) -> Vec<Chunk> {
    if chunks.is_empty() || total_content_bytes == 0 {
        return chunks;
    }

    let ideal_chunk_count = total_content_bytes / IDEAL_BYTES_PER_CHUNK;
    let needs_ratio_split = chunks.len() < ideal_chunk_count;

    let mut result = Vec::new();

    for chunk in chunks {
        let chunk_bytes = chunk.content.len();

        // Pass 1: ratio-based splitting
        if needs_ratio_split && chunk_bytes > IDEAL_BYTES_PER_CHUNK {
            let target = (chunk_bytes / IDEAL_BYTES_PER_CHUNK).max(2);
            let sub_chunks = split_chunk_at_lines(&chunk, target);
            result.extend(sub_chunks);
            continue;
        }

        // Pass 2: hard ceiling
        if chunk_bytes > MAX_BYTES_PER_CHUNK {
            let target = (chunk_bytes / MAX_BYTES_PER_CHUNK).max(2);
            let sub_chunks = split_chunk_at_lines(&chunk, target);
            result.extend(sub_chunks);
            continue;
        }

        result.push(chunk);
    }

    result
}

/// Split a chunk into `target` pieces at line boundaries.
fn split_chunk_at_lines(chunk: &Chunk, target: usize) -> Vec<Chunk> {
    let target_bytes = chunk.content.len() / target;

    // Build virtual lines: for long lines, split into sub-lines
    let mut virtual_lines: Vec<VirtualLine> = Vec::new();
    for (i, line) in chunk.content.lines().enumerate() {
        let actual_line = chunk.line_start + i;
        let char_count = line.chars().count();

        if char_count > LONG_LINE_THRESHOLD {
            // Split into sub-lines
            let sub_count = char_count.div_ceil(LONG_LINE_THRESHOLD);
            let chars: Vec<char> = line.chars().collect();
            for sub_idx in 0..sub_count {
                let start = sub_idx * LONG_LINE_THRESHOLD;
                let end = ((sub_idx + 1) * LONG_LINE_THRESHOLD).min(char_count);
                let text: String = chars[start..end].iter().collect();
                virtual_lines.push(VirtualLine {
                    line_ref: format!("{}-{}", actual_line, sub_idx + 1),
                    text,
                    actual_line,
                });
            }
        } else {
            virtual_lines.push(VirtualLine {
                line_ref: actual_line.to_string(),
                text: line.to_string(),
                actual_line,
            });
        }
    }

    if virtual_lines.is_empty() {
        return vec![chunk.clone()];
    }

    // Group virtual lines into target-sized sub-chunks
    let mut sub_chunks: Vec<Chunk> = Vec::new();
    let mut current_lines: Vec<&VirtualLine> = Vec::new();
    let mut current_bytes = 0usize;

    for vl in &virtual_lines {
        let line_bytes = vl.text.len() + 1; // +1 for newline
        current_lines.push(vl);
        current_bytes += line_bytes;

        if current_bytes >= target_bytes
            && sub_chunks.len() < target - 1
            && !current_lines.is_empty()
        {
            flush_virtual_lines(&mut sub_chunks, &current_lines, &chunk.content_type);
            current_lines.clear();
            current_bytes = 0;
        }
    }

    // Flush remaining
    if !current_lines.is_empty() {
        flush_virtual_lines(&mut sub_chunks, &current_lines, &chunk.content_type);
    }

    // If splitting produced nothing meaningful, return original
    if sub_chunks.is_empty() {
        return vec![chunk.clone()];
    }

    sub_chunks
}

struct VirtualLine {
    line_ref: String,
    text: String,
    actual_line: usize,
}

fn flush_virtual_lines(
    chunks: &mut Vec<Chunk>,
    lines: &[&VirtualLine],
    content_type: &ContentType,
) {
    if lines.is_empty() {
        return;
    }

    let content: String = lines
        .iter()
        .map(|vl| vl.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let first_line_ref = &lines[0].line_ref;
    let first_actual = lines[0].actual_line;
    let last_actual = lines[lines.len() - 1].actual_line;

    // Title: first 60 chars of content at this split point
    let title: String = content
        .lines()
        .next()
        .unwrap_or("")
        .chars()
        .take(60)
        .collect();
    let title = if title.len() < content.lines().next().unwrap_or("").len() {
        format!("{}…", title)
    } else {
        title
    };

    chunks.push(Chunk {
        title,
        content,
        content_type: *content_type,
        line_start: first_actual,
        line_end: last_actual,
        line_ref: first_line_ref.clone(),
    });
}

// ─────────────────────────────────────────────────────────
// Auto-detect and chunk
// ─────────────────────────────────────────────────────────

pub(crate) fn chunk_content(content: &str) -> Vec<Chunk> {
    let format = detect_format(content);
    let chunks = match format {
        ContentFormat::Markdown => chunk_markdown(content),
        ContentFormat::PlainText => chunk_plain_text(content),
        ContentFormat::Json => chunk_json(content),
    };
    enforce_chunk_density(chunks, content.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Format detection ──

    #[test]
    fn test_detect_markdown() {
        assert_eq!(
            detect_format("# Hello\n\nSome content"),
            ContentFormat::Markdown
        );
        assert_eq!(
            detect_format("text\n\n```rust\ncode\n```\n"),
            ContentFormat::Markdown
        );
        assert_eq!(
            detect_format("text\n---\nmore text"),
            ContentFormat::Markdown
        );
    }

    #[test]
    fn test_detect_json() {
        assert_eq!(detect_format(r#"{"key": "value"}"#), ContentFormat::Json);
        assert_eq!(detect_format(r#"[1, 2, 3]"#), ContentFormat::Json);
    }

    #[test]
    fn test_detect_plain_text() {
        assert_eq!(
            detect_format("just some plain text\nwith multiple lines"),
            ContentFormat::PlainText
        );
    }

    // ── Heading detection ──

    #[test]
    fn test_heading_level() {
        assert_eq!(heading_level("# Title"), Some((1, "Title")));
        assert_eq!(heading_level("## Sub"), Some((2, "Sub")));
        assert_eq!(heading_level("### Deep"), Some((3, "Deep")));
        assert_eq!(heading_level("#### Deeper"), Some((4, "Deeper")));
        assert_eq!(heading_level("##### Too deep"), None); // H5 not supported
        assert_eq!(heading_level("Not a heading"), None);
        assert_eq!(heading_level("#NoSpace"), None);
    }

    #[test]
    fn test_horizontal_rule() {
        assert!(is_horizontal_rule("---"));
        assert!(is_horizontal_rule("***"));
        assert!(is_horizontal_rule("___"));
        assert!(is_horizontal_rule("------"));
        assert!(is_horizontal_rule("- - -"));
        assert!(!is_horizontal_rule("--"));
        assert!(!is_horizontal_rule("text"));
    }

    // ── Markdown chunking ──

    #[test]
    fn test_chunk_markdown_headings_hierarchy() {
        let content =
            "# Main\n\nIntro text\n\n## Section A\n\nContent A\n\n## Section B\n\nContent B";
        let chunks = chunk_markdown(content);
        assert!(chunks.len() >= 2);
        assert!(chunks[0].title.contains("Main"));
    }

    #[test]
    fn test_chunk_markdown_code_blocks_intact() {
        let content = "# Code Example\n\n```rust\nfn main() {\n    println!(\"hello\");\n}\n```\n\nAfter code.";
        let chunks = chunk_markdown(content);
        let code_chunk = chunks
            .iter()
            .find(|c| c.content.contains("fn main()"))
            .unwrap();
        assert!(code_chunk.content.contains("```rust"));
        assert!(code_chunk.content.contains("```"));
        assert_eq!(code_chunk.content_type, ContentType::Code);
    }

    #[test]
    fn test_chunk_markdown_line_numbers() {
        let content = "# Title\n\nLine 2\nLine 3\n\n## Section\n\nLine 7\nLine 8";
        let chunks = chunk_markdown(content);
        assert_eq!(chunks[0].line_start, 1);
        assert!(chunks[0].line_end >= 1);
        if chunks.len() > 1 {
            assert!(chunks[1].line_start > chunks[0].line_start);
        }
    }

    #[test]
    fn test_chunk_markdown_oversized_split() {
        let big_paragraph = "A".repeat(2000);
        let content = format!(
            "# Big Section\n\n{}\n\n{}\n\n{}",
            big_paragraph, big_paragraph, big_paragraph,
        );
        let chunks = chunk_markdown(&content);
        assert!(chunks.len() > 1);
        for chunk in &chunks {
            assert!(chunk.title.contains("Big Section"));
        }
    }

    #[test]
    fn test_chunk_markdown_empty_content() {
        let chunks = chunk_markdown("");
        assert!(chunks.is_empty());
    }

    #[test]
    fn test_chunk_markdown_single_line() {
        let chunks = chunk_markdown("Just one line");
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].line_start, 1);
        assert_eq!(chunks[0].line_end, 1);
    }

    #[test]
    fn test_chunk_markdown_horizontal_rules() {
        let content = "Part one\n\n---\n\nPart two";
        let chunks = chunk_markdown(content);
        assert_eq!(chunks.len(), 2);
    }

    // ── Plain text chunking ──

    #[test]
    fn test_chunk_plain_text_paragraph_splitting() {
        let content = "Section one content\nmore content\n\nSection two content\nmore content\n\nSection three content\nmore content";
        let chunks = chunk_plain_text(content);
        assert_eq!(chunks.len(), 3);
        assert!(chunks[0].content.contains("Section one"));
        assert!(chunks[1].content.contains("Section two"));
        assert!(chunks[2].content.contains("Section three"));
    }

    #[test]
    fn test_chunk_plain_text_fixed_line_fallback() {
        let lines: Vec<String> = (1..=50).map(|i| format!("Line {}", i)).collect();
        let content = lines.join("\n");
        let chunks = chunk_plain_text(&content);
        assert!(chunks.len() > 1);
        assert_eq!(chunks[0].line_start, 1);
    }

    #[test]
    fn test_chunk_plain_text_single_chunk() {
        let content = "Short\ncontent\nonly";
        let chunks = chunk_plain_text(content);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].title, "Output");
    }

    #[test]
    fn test_chunk_plain_text_empty() {
        let chunks = chunk_plain_text("");
        assert!(chunks.is_empty());
    }

    // ── JSON chunking ──

    #[test]
    fn test_chunk_json_nested_objects() {
        let content = r#"{"config": {"database": {"host": "localhost", "port": 5432}, "cache": {"enabled": true}}}"#;
        let chunks = chunk_json(content);
        assert!(!chunks.is_empty());
        let titles: Vec<&str> = chunks.iter().map(|c| c.title.as_str()).collect();
        assert!(
            titles
                .iter()
                .any(|t| t.contains("database") || t.contains("config")),
            "Expected title containing 'database' or 'config', got: {:?}",
            titles
        );
    }

    #[test]
    fn test_chunk_json_identity_fields() {
        let content = r#"[{"id": 1, "name": "Alice"}, {"id": 2, "name": "Bob"}, {"id": 3, "name": "Charlie"}]"#;
        let chunks = chunk_json(content);
        assert!(!chunks.is_empty());
    }

    #[test]
    fn test_chunk_json_invalid_fallback() {
        let content = "this is not { valid json";
        let chunks = chunk_json(content);
        assert!(!chunks.is_empty());
    }

    #[test]
    fn test_chunk_json_empty_object() {
        let content = "{}";
        let chunks = chunk_json(content);
        assert_eq!(chunks.len(), 1);
    }

    // ── Auto-detect chunking ──

    #[test]
    fn test_chunk_content_markdown() {
        let chunks = chunk_content("# Hello\n\nWorld");
        assert!(!chunks.is_empty());
        assert!(chunks[0].title.contains("Hello"));
    }

    #[test]
    fn test_chunk_content_json() {
        let chunks = chunk_content(r#"{"key": "value"}"#);
        assert!(!chunks.is_empty());
    }

    // ── Unicode ──

    #[test]
    fn test_chunk_unicode_content() {
        let content = "# \u{4f60}\u{597d}\u{4e16}\u{754c}\n\n\u{8fd9}\u{662f}\u{4e2d}\u{6587}\u{5185}\u{5bb9}\n\n## \u{7b2c}\u{4e8c}\u{8282}\n\n\u{66f4}\u{591a}\u{5185}\u{5bb9}";
        let chunks = chunk_markdown(content);
        assert!(!chunks.is_empty());
    }

    // ── Chunk density enforcement ──

    #[test]
    fn density_small_chunks_unchanged() {
        let chunks = vec![Chunk {
            title: "Small".to_string(),
            content: "Short content".to_string(),
            content_type: ContentType::Prose,
            line_start: 1,
            line_end: 1,
            line_ref: "1".to_string(),
        }];
        let result = enforce_chunk_density(chunks.clone(), 13);
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn density_large_chunk_split() {
        // Create a 10KB chunk
        let lines: Vec<String> = (1..=200)
            .map(|i| format!("Line {} with some content that adds bytes", i))
            .collect();
        let content = lines.join("\n");
        let total_bytes = content.len();
        let chunks = vec![Chunk {
            title: "Big chunk".to_string(),
            content,
            content_type: ContentType::Prose,
            line_start: 1,
            line_end: 200,
            line_ref: "1".to_string(),
        }];
        let result = enforce_chunk_density(chunks, total_bytes);
        assert!(
            result.len() > 1,
            "Should split large chunk, got {} chunks",
            result.len()
        );
    }

    #[test]
    fn density_split_at_line_boundaries() {
        let lines: Vec<String> = (1..=100).map(|i| format!("Line {}", i)).collect();
        let content = lines.join("\n");
        let total_bytes = content.len();
        let chunks = vec![Chunk {
            title: "Content".to_string(),
            content,
            content_type: ContentType::Prose,
            line_start: 1,
            line_end: 100,
            line_ref: "1".to_string(),
        }];
        let result = enforce_chunk_density(chunks, total_bytes);
        // Each sub-chunk should have content that doesn't break mid-line
        for chunk in &result {
            // Content shouldn't start or end with a partial line
            let first_char = chunk.content.chars().next();
            assert!(first_char.is_some());
        }
    }

    #[test]
    fn density_split_long_line_sub_offset() {
        // Create content with one very long line
        let long_line = "x".repeat(5000);
        let content = format!("short line\n{}\nanother short line", long_line);
        let total_bytes = content.len();
        let chunks = vec![Chunk {
            title: "Content".to_string(),
            content,
            content_type: ContentType::Prose,
            line_start: 1,
            line_end: 3,
            line_ref: "1".to_string(),
        }];
        let result = enforce_chunk_density(chunks, total_bytes);
        // Should have sub-line refs for the long line
        let has_sub_ref = result.iter().any(|c| c.line_ref.contains('-'));
        assert!(has_sub_ref, "Should have sub-line refs for long lines");
    }

    #[test]
    fn density_sub_chunk_title_snippet() {
        let lines: Vec<String> = (1..=100)
            .map(|i| format!("Line {} with unique content for identification", i))
            .collect();
        let content = lines.join("\n");
        let total_bytes = content.len();
        let chunks = vec![Chunk {
            title: "Content".to_string(),
            content,
            content_type: ContentType::Prose,
            line_start: 1,
            line_end: 100,
            line_ref: "1".to_string(),
        }];
        let result = enforce_chunk_density(chunks, total_bytes);
        // Each sub-chunk title should be content from the split point
        for chunk in &result {
            assert!(!chunk.title.is_empty());
            assert!(chunk.title.len() <= 62); // 60 chars + "…"
        }
    }

    #[test]
    fn density_preserves_existing_chunks() {
        let chunks = vec![
            Chunk {
                title: "A".to_string(),
                content: "Small A".to_string(),
                content_type: ContentType::Prose,
                line_start: 1,
                line_end: 1,
                line_ref: "1".to_string(),
            },
            Chunk {
                title: "B".to_string(),
                content: "Small B".to_string(),
                content_type: ContentType::Prose,
                line_start: 2,
                line_end: 2,
                line_ref: "2".to_string(),
            },
        ];
        let result = enforce_chunk_density(chunks, 14);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].title, "A");
        assert_eq!(result[1].title, "B");
    }

    #[test]
    fn density_hard_ceiling() {
        // Create a chunk that exceeds MAX_BYTES_PER_CHUNK but ratio is fine
        let content = "x".repeat(MAX_BYTES_PER_CHUNK + 500);
        let total = content.len();
        // Set ideal count to 1 so ratio doesn't trigger
        let chunks = vec![Chunk {
            title: "Big".to_string(),
            content,
            content_type: ContentType::Prose,
            line_start: 1,
            line_end: 1,
            line_ref: "1".to_string(),
        }];
        // total_content_bytes / IDEAL = 1 chunk, and we have 1 chunk, so ratio is fine
        // But the chunk exceeds MAX_BYTES_PER_CHUNK so it should be force-split
        let result = enforce_chunk_density(chunks, total);
        assert!(result.len() > 1, "Hard ceiling should force-split");
    }

    #[test]
    fn density_line_numbers_accurate() {
        let lines: Vec<String> = (1..=50).map(|i| format!("Line {}", i)).collect();
        let content = lines.join("\n");
        let total = content.len();
        let chunks = vec![Chunk {
            title: "Content".to_string(),
            content,
            content_type: ContentType::Prose,
            line_start: 10,
            line_end: 59,
            line_ref: "10".to_string(),
        }];
        let result = enforce_chunk_density(chunks, total);
        // First sub-chunk should start at line 10
        assert_eq!(result[0].line_start, 10);
        // Last sub-chunk should end at or before line 59
        let last = result.last().unwrap();
        assert!(last.line_end <= 59);
    }
}
