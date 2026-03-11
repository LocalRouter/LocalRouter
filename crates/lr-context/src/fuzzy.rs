/// Levenshtein edit distance — Unicode-aware, single-row optimization.
pub(crate) fn levenshtein(a: &str, b: &str) -> usize {
    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();

    if a_chars.is_empty() {
        return b_chars.len();
    }
    if b_chars.is_empty() {
        return a_chars.len();
    }

    let mut prev: Vec<usize> = (0..=b_chars.len()).collect();

    for i in 1..=a_chars.len() {
        let mut curr = vec![0; b_chars.len() + 1];
        curr[0] = i;
        for j in 1..=b_chars.len() {
            let cost = if a_chars[i - 1] == b_chars[j - 1] {
                0
            } else {
                1
            };
            curr[j] = (prev[j] + 1).min(curr[j - 1] + 1).min(prev[j - 1] + cost);
        }
        prev = curr;
    }

    prev[b_chars.len()]
}

/// Maximum edit distance allowed for fuzzy correction, based on word length.
pub(crate) fn max_edit_distance(char_count: usize) -> usize {
    if char_count <= 4 {
        1
    } else if char_count <= 12 {
        2
    } else {
        3
    }
}

/// Find the best fuzzy correction for `word` from `candidates`.
/// Returns `None` if an exact match is found or no candidate is close enough.
pub(crate) fn find_best_correction(word: &str, candidates: &[String]) -> Option<String> {
    let max_dist = max_edit_distance(word.chars().count());
    let mut best_word: Option<String> = None;
    let mut best_dist = max_dist + 1;

    for candidate in candidates {
        if candidate == word {
            return None; // exact match — no correction needed
        }
        let dist = levenshtein(word, candidate);
        if dist < best_dist {
            best_dist = dist;
            best_word = Some(candidate.clone());
        }
    }

    if best_dist <= max_dist {
        best_word
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_levenshtein_identical() {
        assert_eq!(levenshtein("hello", "hello"), 0);
        assert_eq!(levenshtein("", ""), 0);
    }

    #[test]
    fn test_levenshtein_one_edit() {
        assert_eq!(levenshtein("cat", "bat"), 1); // substitution
        assert_eq!(levenshtein("cat", "cats"), 1); // insertion
        assert_eq!(levenshtein("cats", "cat"), 1); // deletion
    }

    #[test]
    fn test_levenshtein_multiple_edits() {
        assert_eq!(levenshtein("kitten", "sitting"), 3);
        assert_eq!(levenshtein("saturday", "sunday"), 3);
    }

    #[test]
    fn test_levenshtein_empty_strings() {
        assert_eq!(levenshtein("", "abc"), 3);
        assert_eq!(levenshtein("abc", ""), 3);
    }

    #[test]
    fn test_levenshtein_unicode() {
        assert_eq!(levenshtein("café", "cafe"), 1);
        assert_eq!(levenshtein("naïve", "naive"), 1);
        // CJK characters
        assert_eq!(levenshtein("hello", "hello"), 0);
    }

    #[test]
    fn test_max_edit_distance_boundaries() {
        assert_eq!(max_edit_distance(1), 1);
        assert_eq!(max_edit_distance(3), 1);
        assert_eq!(max_edit_distance(4), 1);
        assert_eq!(max_edit_distance(5), 2);
        assert_eq!(max_edit_distance(8), 2);
        assert_eq!(max_edit_distance(12), 2);
        assert_eq!(max_edit_distance(13), 3);
        assert_eq!(max_edit_distance(20), 3);
    }

    #[test]
    fn test_find_best_correction_exact_match() {
        let candidates = vec!["kubernetes".to_string(), "kubelet".to_string()];
        // Exact match returns None (no correction needed)
        assert_eq!(find_best_correction("kubernetes", &candidates), None);
    }

    #[test]
    fn test_find_best_correction_typo() {
        let candidates = vec!["kubernetes".to_string(), "kubelet".to_string()];
        assert_eq!(
            find_best_correction("kuberntes", &candidates),
            Some("kubernetes".to_string())
        );
    }

    #[test]
    fn test_find_best_correction_no_close_match() {
        let candidates = vec!["apple".to_string(), "banana".to_string()];
        assert_eq!(find_best_correction("xyz", &candidates), None);
    }

    #[test]
    fn test_find_best_correction_empty_candidates() {
        let candidates: Vec<String> = vec![];
        assert_eq!(find_best_correction("test", &candidates), None);
    }

    #[test]
    fn test_find_best_correction_picks_closest() {
        let candidates = vec![
            "configuration".to_string(),
            "confguration".to_string(), // 1 edit from "confguration"
            "computation".to_string(),
        ];
        // "configuraton" is 1 edit from "configuration"
        assert_eq!(
            find_best_correction("configuraton", &candidates),
            Some("configuration".to_string())
        );
    }
}
