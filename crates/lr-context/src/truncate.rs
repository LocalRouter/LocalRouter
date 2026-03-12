//! Smart truncation: 60% head + 40% tail, line-aligned.

/// Truncate text to fit within `max_bytes`, keeping 60% head and 40% tail.
/// Line-aligned: never splits mid-line. If text fits, returns as-is.
pub(crate) fn smart_truncate(text: &str, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text.to_string();
    }

    let lines: Vec<&str> = text.lines().collect();
    let total_lines = lines.len();

    // Single line or empty: truncate at char boundary
    if total_lines <= 1 {
        let separator_reserve = 100;
        let usable = max_bytes.saturating_sub(separator_reserve);
        let head_budget = (usable * 6) / 10;
        let tail_budget = (usable * 4) / 10;

        let head_end = char_floor(text, head_budget);
        let tail_len = char_floor_rev(text, tail_budget);
        let tail_start = text.len() - tail_len;

        if head_end >= tail_start || tail_len == 0 {
            let end = char_floor(text, max_bytes);
            return text[..end].to_string();
        }

        let skipped_bytes = tail_start - head_end;
        let skipped_kb = skipped_bytes as f64 / 1024.0;
        return format!(
            "{}\n\u{2026} [{:.1}KB truncated] \u{2026}\n{}",
            &text[..head_end],
            skipped_kb,
            &text[tail_start..]
        );
    }

    // Reserve ~100 bytes for the separator line
    let separator_reserve = 100;
    let usable = max_bytes.saturating_sub(separator_reserve);
    let head_budget = (usable * 6) / 10;
    let tail_budget = (usable * 4) / 10;

    // Accumulate head lines
    let mut head_count = 0;
    let mut head_bytes = 0;
    for line in &lines {
        let needed = line.len() + 1;
        if head_bytes + needed > head_budget && head_count > 0 {
            break;
        }
        head_count += 1;
        head_bytes += needed;
    }

    // Accumulate tail lines (from end)
    let mut tail_count = 0;
    let mut tail_bytes = 0;
    for line in lines.iter().rev() {
        let needed = line.len() + 1;
        if tail_bytes + needed > tail_budget && tail_count > 0 {
            break;
        }
        tail_count += 1;
        tail_bytes += needed;
    }

    // Ensure no overlap
    let tail_start_idx = total_lines.saturating_sub(tail_count);
    if head_count >= tail_start_idx {
        // Lines are individually too large for the budget.
        // Fall back to char-level truncation on the joined text.
        let head_end = char_floor(text, head_budget);
        let tail_len = char_floor_rev(text, tail_budget.saturating_sub(80));
        let tail_start = text.len().saturating_sub(tail_len);

        if head_end >= tail_start || tail_len == 0 {
            let end = char_floor(text, max_bytes);
            return text[..end].to_string();
        }

        let skipped_bytes = tail_start - head_end;
        let skipped_kb = skipped_bytes as f64 / 1024.0;
        return format!(
            "{}\n\u{2026} [{:.1}KB truncated] \u{2026}\n{}",
            &text[..head_end],
            skipped_kb,
            &text[tail_start..]
        );
    }

    let skipped_lines = tail_start_idx - head_count;
    let skipped_bytes: usize = lines[head_count..tail_start_idx]
        .iter()
        .map(|l| l.len() + 1)
        .sum();
    let skipped_kb = skipped_bytes as f64 / 1024.0;

    let head = lines[..head_count].join("\n");
    let tail = lines[tail_start_idx..].join("\n");

    format!(
        "{}\n\u{2026} [{} lines / {:.1}KB truncated \u{2014} showing first {} + last {} lines] \u{2026}\n{}",
        head, skipped_lines, skipped_kb, head_count, tail_count, tail
    )
}

/// Largest byte offset <= max_bytes that's a valid char boundary.
fn char_floor(s: &str, max_bytes: usize) -> usize {
    if max_bytes >= s.len() {
        return s.len();
    }
    let mut i = max_bytes;
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// Largest byte count from the end <= max_bytes at a valid char boundary.
fn char_floor_rev(s: &str, max_bytes: usize) -> usize {
    if max_bytes >= s.len() {
        return s.len();
    }
    let start = s.len() - max_bytes;
    let mut i = start;
    while i < s.len() && !s.is_char_boundary(i) {
        i += 1;
    }
    s.len() - i
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_short_text_unchanged() {
        let text = "Hello, world!\nSecond line.";
        assert_eq!(smart_truncate(text, 1000), text);
    }

    #[test]
    fn truncate_long_text_split() {
        let lines: Vec<String> = (1..=100)
            .map(|i| format!("Line {}: some content here", i))
            .collect();
        let text = lines.join("\n");
        let result = smart_truncate(&text, 500);
        assert!(result.contains("Line 1:"));
        assert!(result.contains("Line 100:"));
        assert!(result.contains("\u{2026}"));
    }

    #[test]
    fn truncate_separator_shows_stats() {
        let lines: Vec<String> = (1..=100).map(|i| format!("Line {}: content", i)).collect();
        let text = lines.join("\n");
        let result = smart_truncate(&text, 500);
        assert!(result.contains("lines"));
        assert!(result.contains("KB truncated"));
        assert!(result.contains("showing first"));
        assert!(result.contains("last"));
    }

    #[test]
    fn truncate_single_long_line() {
        let text = "x".repeat(10000);
        let result = smart_truncate(&text, 500);
        assert!(result.contains("\u{2026}"));
        assert!(result.len() < 1000);
    }

    #[test]
    fn truncate_unicode_safe() {
        // Create text with multi-byte chars that exceeds limit
        let line = "\u{4f60}\u{597d}".repeat(500); // 你好 repeated
        let text = format!("{}\n{}", line, line);
        let result = smart_truncate(&text, 500);
        // Should not panic and result should be valid UTF-8
        // Result may be larger than 500 due to separator + multi-byte chars at boundaries
        assert!(result.len() < text.len());
    }

    #[test]
    fn truncate_exact_boundary() {
        let text = "Hello\nWorld";
        assert_eq!(smart_truncate(text, text.len()), text);
    }
}
