//! Protection layer for quoted text and code blocks during compression.
//!
//! After BERT scores all words, words inside detected quote/code regions are
//! force-kept regardless of score. This preserves exact content within quotes,
//! inline code, and fenced code blocks while still compressing surrounding prose.

use std::collections::HashSet;

/// A quote delimiter pair (open, close, symmetric).
struct QuoteDelimiter {
    open: char,
    close: char,
    symmetric: bool,
}

/// All supported quote delimiter types.
const QUOTE_DELIMITERS: &[QuoteDelimiter] = &[
    // ASCII double quote
    QuoteDelimiter {
        open: '"',
        close: '"',
        symmetric: true,
    },
    // ASCII single quote
    QuoteDelimiter {
        open: '\'',
        close: '\'',
        symmetric: true,
    },
    // Curly double quotes
    QuoteDelimiter {
        open: '\u{201C}',
        close: '\u{201D}',
        symmetric: false,
    },
    // Curly single quotes
    QuoteDelimiter {
        open: '\u{2018}',
        close: '\u{2019}',
        symmetric: false,
    },
    // German double quotes
    QuoteDelimiter {
        open: '\u{201E}',
        close: '\u{201D}',
        symmetric: false,
    },
    // German single quotes
    QuoteDelimiter {
        open: '\u{201A}',
        close: '\u{2019}',
        symmetric: false,
    },
    // Guillemet double
    QuoteDelimiter {
        open: '\u{00AB}',
        close: '\u{00BB}',
        symmetric: false,
    },
    // Guillemet single
    QuoteDelimiter {
        open: '\u{2039}',
        close: '\u{203A}',
        symmetric: false,
    },
    // Heavy double quotes
    QuoteDelimiter {
        open: '\u{275D}',
        close: '\u{275E}',
        symmetric: false,
    },
    // Heavy single quotes
    QuoteDelimiter {
        open: '\u{275B}',
        close: '\u{275C}',
        symmetric: false,
    },
    // Full-width double quote
    QuoteDelimiter {
        open: '\u{FF02}',
        close: '\u{FF02}',
        symmetric: true,
    },
    // Full-width single quote
    QuoteDelimiter {
        open: '\u{FF07}',
        close: '\u{FF07}',
        symmetric: true,
    },
    // CJK corner brackets
    QuoteDelimiter {
        open: '\u{300C}',
        close: '\u{300D}',
        symmetric: false,
    },
    // CJK double corner brackets
    QuoteDelimiter {
        open: '\u{300E}',
        close: '\u{300F}',
        symmetric: false,
    },
];

/// Trailing punctuation characters to strip when checking for closing delimiters.
const TRAILING_PUNCT: &[char] = &['.', ',', ';', ':', '!', '?'];

/// Detect which words in the input should be protected (force-kept) during compression.
///
/// Protected words are those inside:
/// - Fenced code blocks (``` ... ```)
/// - Inline code (`...`)
/// - Quoted strings (all supported delimiter types)
///
/// Returns a boolean mask: `result[i]` is true if `words[i]` should be protected.
pub fn detect_protected_words(words: &[&str]) -> Vec<bool> {
    let len = words.len();
    let mut protected = vec![false; len];

    // Pre-scan: for symmetric delimiters, count boundary appearances.
    // If a symmetric delimiter char appears fewer than 2 times at word boundaries,
    // it can't form a pair, so disable it (e.g., apostrophes in contractions).
    let mut symmetric_enabled = vec![true; QUOTE_DELIMITERS.len()];
    for (delim_idx, delim) in QUOTE_DELIMITERS.iter().enumerate() {
        if !delim.symmetric {
            continue;
        }
        let mut boundary_count = 0u32;
        for word in words {
            if word.starts_with(delim.open) {
                boundary_count += 1;
            }
            if word.len() > 1 || !word.starts_with(delim.open) {
                // Check end (strip trailing punct)
                let stripped = word.trim_end_matches(TRAILING_PUNCT);
                if stripped.ends_with(delim.close) {
                    boundary_count += 1;
                }
            }
        }
        if boundary_count < 2 {
            symmetric_enabled[delim_idx] = false;
        }
    }

    let mut in_fenced = false;
    let mut in_backtick = false;
    let mut active_quotes: HashSet<usize> = HashSet::new();

    for (i, word) in words.iter().enumerate() {
        // 1. Fenced code block check (highest priority)
        if word.contains("```") {
            in_fenced = !in_fenced;
            protected[i] = true;
            continue;
        }

        if in_fenced {
            protected[i] = true;
            continue;
        }

        // 2. Backtick (inline code) check
        let starts_backtick = word.starts_with('`') && !word.starts_with("```");
        let ends_backtick = word.ends_with('`') && !word.ends_with("```");

        if starts_backtick && ends_backtick && word.len() > 1 {
            // Self-contained inline code: `code`
            protected[i] = true;
            // Don't change in_backtick state
        } else if starts_backtick && !in_backtick {
            in_backtick = true;
            protected[i] = true;
        } else if ends_backtick && in_backtick {
            in_backtick = false;
            protected[i] = true;
        }

        if in_backtick {
            protected[i] = true;
        }

        // 3. Quote delimiter check (all types, independently)
        for (delim_idx, delim) in QUOTE_DELIMITERS.iter().enumerate() {
            if delim.symmetric && !symmetric_enabled[delim_idx] {
                continue;
            }

            let word_starts_with_open = word.starts_with(delim.open);
            let stripped = word.trim_end_matches(TRAILING_PUNCT);
            let word_ends_with_close = stripped.ends_with(delim.close);

            // For symmetric single-quote: skip mid-word apostrophes
            if delim.symmetric && delim.open == '\'' && is_apostrophe(word) {
                continue;
            }

            if !active_quotes.contains(&delim_idx) {
                // Not currently in this quote type — check for opening
                if word_starts_with_open {
                    active_quotes.insert(delim_idx);
                    protected[i] = true;

                    // Self-contained: opens and closes on same word
                    if word_ends_with_close && stripped.len() > 1 {
                        // Check it's not just the open char itself
                        let inner = if delim.symmetric {
                            // For symmetric, need at least open...close with something between
                            stripped.len() > 1
                                && stripped.starts_with(delim.open)
                                && stripped.ends_with(delim.close)
                                && stripped.chars().count() > 1
                        } else {
                            true
                        };
                        if inner {
                            active_quotes.remove(&delim_idx);
                        }
                    }
                }
            } else {
                // Currently in this quote type — check for closing
                protected[i] = true;
                if word_ends_with_close {
                    active_quotes.remove(&delim_idx);
                }
            }
        }

        // If any quote region is active, this word is protected
        if !active_quotes.is_empty() {
            protected[i] = true;
        }
    }

    protected
}

/// Check if a single-quote in a word is an apostrophe (mid-word between alphanumerics).
fn is_apostrophe(word: &str) -> bool {
    // Word must contain a single quote that's not at the start or end
    let chars: Vec<char> = word.chars().collect();
    if chars.len() < 3 {
        return false;
    }
    // If word starts with ' it could be a quote opener — not an apostrophe
    if chars[0] == '\'' {
        return false;
    }
    // Check if ' appears between alphanumeric chars
    for i in 1..chars.len() - 1 {
        if chars[i] == '\'' && chars[i - 1].is_alphanumeric() && chars[i + 1].is_alphanumeric() {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn protected_words<'a>(words: &[&'a str]) -> Vec<&'a str> {
        let mask = detect_protected_words(words);
        words
            .iter()
            .zip(mask.iter())
            .filter(|(_, &p)| p)
            .map(|(&w, _)| w)
            .collect()
    }

    fn unprotected_words<'a>(words: &[&'a str]) -> Vec<&'a str> {
        let mask = detect_protected_words(words);
        words
            .iter()
            .zip(mask.iter())
            .filter(|(_, &p)| !p)
            .map(|(&w, _)| w)
            .collect()
    }

    #[test]
    fn test_fenced_code_block() {
        let words: Vec<&str> = "before ``` def foo(): return 1 ``` after".split_whitespace().collect();
        let mask = detect_protected_words(&words);
        // "before" and "after" should not be protected
        assert!(!mask[0]); // before
        assert!(mask[1]); // ```
        assert!(mask[2]); // def
        assert!(mask[3]); // foo():
        assert!(mask[4]); // return
        assert!(mask[5]); // 1
        assert!(mask[6]); // ```
        assert!(!mask[7]); // after
    }

    #[test]
    fn test_inline_code_self_contained() {
        let words: Vec<&str> = "use `bcrypt.checkpw()` instead".split_whitespace().collect();
        let mask = detect_protected_words(&words);
        assert!(!mask[0]); // use
        assert!(mask[1]); // `bcrypt.checkpw()`
        assert!(!mask[2]); // instead
    }

    #[test]
    fn test_inline_code_multiword() {
        let words: Vec<&str> = "use `bcrypt checkpw` instead".split_whitespace().collect();
        let mask = detect_protected_words(&words);
        assert!(!mask[0]); // use
        assert!(mask[1]); // `bcrypt
        assert!(mask[2]); // checkpw`
        assert!(!mask[3]); // instead
    }

    #[test]
    fn test_double_quoted_string() {
        let words: Vec<&str> =
            r#"reported "a persistent connection timeout" when trying"#.split_whitespace().collect();
        let mask = detect_protected_words(&words);
        assert!(!mask[0]); // reported
        assert!(mask[1]); // "a
        assert!(mask[2]); // persistent
        assert!(mask[3]); // connection
        assert!(mask[4]); // timeout"
        assert!(!mask[5]); // when
        assert!(!mask[6]); // trying
    }

    #[test]
    fn test_self_contained_quote() {
        let words: Vec<&str> = r#"the "hello" world"#.split_whitespace().collect();
        let mask = detect_protected_words(&words);
        assert!(!mask[0]); // the
        assert!(mask[1]); // "hello"
        assert!(!mask[2]); // world
    }

    #[test]
    fn test_curly_double_quotes() {
        let words: Vec<&str> = "he said \u{201C}hello world\u{201D} today".split_whitespace().collect();
        let mask = detect_protected_words(&words);
        assert!(!mask[0]); // he
        assert!(!mask[1]); // said
        assert!(mask[2]); // \u{201C}hello
        assert!(mask[3]); // world\u{201D}
        assert!(!mask[4]); // today
    }

    #[test]
    fn test_curly_single_quotes() {
        let words: Vec<&str> = "the \u{2018}quick brown\u{2019} fox".split_whitespace().collect();
        let mask = detect_protected_words(&words);
        assert!(!mask[0]); // the
        assert!(mask[1]); // 'quick
        assert!(mask[2]); // brown'
        assert!(!mask[3]); // fox
    }

    #[test]
    fn test_german_quotes() {
        let words: Vec<&str> = "er sagte \u{201E}hallo welt\u{201D} heute".split_whitespace().collect();
        let mask = detect_protected_words(&words);
        assert!(!mask[0]);
        assert!(!mask[1]);
        assert!(mask[2]); // „hallo
        assert!(mask[3]); // welt"
        assert!(!mask[4]);
    }

    #[test]
    fn test_guillemets() {
        let words: Vec<&str> = "il dit \u{00AB}bonjour monde\u{00BB} ici".split_whitespace().collect();
        let mask = detect_protected_words(&words);
        assert!(!mask[0]);
        assert!(!mask[1]);
        assert!(mask[2]); // «bonjour
        assert!(mask[3]); // monde»
        assert!(!mask[4]);
    }

    #[test]
    fn test_cjk_corner_brackets() {
        let words: Vec<&str> = "text \u{300C}hello world\u{300D} more".split_whitespace().collect();
        let mask = detect_protected_words(&words);
        assert!(!mask[0]);
        assert!(mask[1]);
        assert!(mask[2]);
        assert!(!mask[3]);
    }

    #[test]
    fn test_apostrophe_not_treated_as_quote() {
        let words: Vec<&str> = "it's don't can't won't".split_whitespace().collect();
        let mask = detect_protected_words(&words);
        // Apostrophes should NOT trigger protection
        assert!(mask.iter().all(|&p| !p));
    }

    #[test]
    fn test_closing_with_trailing_punctuation() {
        let words: Vec<&str> = r#"said "hello world," and left"#.split_whitespace().collect();
        let mask = detect_protected_words(&words);
        assert!(!mask[0]); // said
        assert!(mask[1]); // "hello
        assert!(mask[2]); // world,"
        assert!(!mask[3]); // and
        assert!(!mask[4]); // left
    }

    #[test]
    fn test_unclosed_quote_protects_to_end() {
        // With two boundary appearances (opening + some closing), the unclosed one protects to end
        let words: Vec<&str> = r#""first" then "hello world and more"#.split_whitespace().collect();
        let mask = detect_protected_words(&words);
        assert!(mask[0]); // "first"
        assert!(!mask[1]); // then
        assert!(mask[2]); // "hello
        assert!(mask[3]); // world
        assert!(mask[4]); // and
        assert!(mask[5]); // more
    }

    #[test]
    fn test_multiple_quoted_regions() {
        let words: Vec<&str> =
            r#""hello" world "goodbye""#.split_whitespace().collect();
        let mask = detect_protected_words(&words);
        assert!(mask[0]); // "hello"
        assert!(!mask[1]); // world
        assert!(mask[2]); // "goodbye"
    }

    #[test]
    fn test_overlapping_quote_types() {
        // Single and double quotes overlap — union approach protects all
        let words: Vec<&str> = r#"'Hello I "am over' here""#.split_whitespace().collect();
        let prot = protected_words(&words);
        let unprot = unprotected_words(&words);
        // All words should be protected since at least one quote region covers each
        assert!(prot.len() >= 4); // Most words protected
        assert!(unprot.is_empty() || unprot.len() <= 1);
    }

    #[test]
    fn test_empty_input() {
        let words: Vec<&str> = vec![];
        let mask = detect_protected_words(&words);
        assert!(mask.is_empty());
    }

    #[test]
    fn test_no_special_content() {
        let words: Vec<&str> = "hello world this is plain text".split_whitespace().collect();
        let mask = detect_protected_words(&words);
        assert!(mask.iter().all(|&p| !p));
    }

    #[test]
    fn test_mixed_content() {
        // Prose + quoted string + inline code + fenced block
        let words: Vec<&str> = "The error is \"connection timeout\" and `db.verify()` fails before ``` def auth(): pass ``` end"
            .split_whitespace()
            .collect();
        let mask = detect_protected_words(&words);

        // "The", "error", "is" - not protected
        assert!(!mask[0]);
        assert!(!mask[1]);
        assert!(!mask[2]);
        // "\"connection" "timeout\"" - protected
        assert!(mask[3]);
        assert!(mask[4]);
        // "and" - not protected
        assert!(!mask[5]);
        // `db.verify()` - protected (self-contained)
        assert!(mask[6]);
        // "fails", "before" - not protected
        assert!(!mask[7]);
        assert!(!mask[8]);
        // ``` def auth(): pass ``` - all protected
        assert!(mask[9]);
        assert!(mask[10]);
        assert!(mask[11]);
        assert!(mask[12]);
        assert!(mask[13]);
        // "end" - not protected
        assert!(!mask[14]);
    }

    #[test]
    fn test_full_width_quotes() {
        let words: Vec<&str> = "text \u{FF02}hello world\u{FF02} more".split_whitespace().collect();
        let mask = detect_protected_words(&words);
        assert!(!mask[0]);
        assert!(mask[1]);
        assert!(mask[2]);
        assert!(!mask[3]);
    }

    #[test]
    fn test_heavy_quotes() {
        let words: Vec<&str> = "said \u{275D}hello world\u{275E} today".split_whitespace().collect();
        let mask = detect_protected_words(&words);
        assert!(!mask[0]);
        assert!(mask[1]);
        assert!(mask[2]);
        assert!(!mask[3]);
    }

    #[test]
    fn test_guillemet_single() {
        let words: Vec<&str> = "mot \u{2039}bonjour\u{203A} ici".split_whitespace().collect();
        let mask = detect_protected_words(&words);
        assert!(!mask[0]);
        assert!(mask[1]);
        assert!(!mask[2]);
    }

    #[test]
    fn test_cjk_double_corner() {
        let words: Vec<&str> = "text \u{300E}hello world\u{300F} more".split_whitespace().collect();
        let mask = detect_protected_words(&words);
        assert!(!mask[0]);
        assert!(mask[1]);
        assert!(mask[2]);
        assert!(!mask[3]);
    }

    #[test]
    fn test_fenced_block_with_language() {
        let words: Vec<&str> = "before ```python def foo(): pass ``` after".split_whitespace().collect();
        let mask = detect_protected_words(&words);
        assert!(!mask[0]); // before
        assert!(mask[1]); // ```python
        assert!(mask[2]); // def
        assert!(mask[3]); // foo():
        assert!(mask[4]); // pass
        assert!(mask[5]); // ```
        assert!(!mask[6]); // after
    }

    #[test]
    fn test_single_quote_used_as_quote() {
        // When single quotes appear at word boundaries (at least 2), treat as quotes
        let words: Vec<&str> = "he said 'hello world' today".split_whitespace().collect();
        let mask = detect_protected_words(&words);
        assert!(!mask[0]); // he
        assert!(!mask[1]); // said
        assert!(mask[2]); // 'hello
        assert!(mask[3]); // world'
        assert!(!mask[4]); // today
    }
}
