//! Shared fuzzy matching for name resolution.
//!
//! Provides a layered matching strategy: exact → case-insensitive →
//! normalized → Levenshtein fuzzy. Used across the codebase for resolving
//! misspelled or differently-cased names (skills, prompts, resources, etc.).

/// How a name was matched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchKind {
    Exact,
    CaseInsensitive,
    Normalized,
    Fuzzy,
}

/// Levenshtein edit distance — Unicode-aware, single-row optimization.
pub fn levenshtein(a: &str, b: &str) -> usize {
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
pub fn max_edit_distance(char_count: usize) -> usize {
    if char_count <= 4 {
        1
    } else if char_count <= 12 {
        2
    } else {
        3
    }
}

/// Normalize a name for comparison: lowercase, replace `_` and ` ` with `-`,
/// trim leading/trailing `-`.
pub fn normalize_name(name: &str) -> String {
    name.to_lowercase()
        .replace(['_', ' '], "-")
        .trim_matches('-')
        .to_string()
}

/// Find the best match for `query` among `candidates`.
///
/// `candidates` is a slice of `(index, name)` pairs. Returns the index of the
/// best match and the kind of match, or `None` if no match is close enough.
///
/// Tries layers in order: exact → case-insensitive → normalized → fuzzy.
pub fn find_best_match(query: &str, candidates: &[(usize, &str)]) -> Option<(usize, MatchKind)> {
    if candidates.is_empty() {
        return None;
    }

    // Layer 1: Exact match
    for &(idx, name) in candidates {
        if name == query {
            return Some((idx, MatchKind::Exact));
        }
    }

    // Layer 2: Case-insensitive
    let query_lower = query.to_lowercase();
    for &(idx, name) in candidates {
        if name.to_lowercase() == query_lower {
            return Some((idx, MatchKind::CaseInsensitive));
        }
    }

    // Layer 3: Normalized (lowercase + separator normalization)
    let query_norm = normalize_name(query);
    for &(idx, name) in candidates {
        if normalize_name(name) == query_norm {
            return Some((idx, MatchKind::Normalized));
        }
    }

    // Layer 4: Fuzzy (Levenshtein distance)
    let max_dist = max_edit_distance(query_lower.chars().count());
    let mut best_idx = None;
    let mut best_dist = max_dist + 1;
    let mut best_name: Option<&str> = None;

    for &(idx, name) in candidates {
        let dist = levenshtein(&query_lower, &name.to_lowercase());
        if dist < best_dist || (dist == best_dist && best_name.is_none_or(|prev| name < prev)) {
            best_dist = dist;
            best_idx = Some(idx);
            best_name = Some(name);
        }
    }

    if best_dist <= max_dist {
        best_idx.map(|idx| (idx, MatchKind::Fuzzy))
    } else {
        None
    }
}

/// Find the best fuzzy correction for `word` from `candidates`.
///
/// Returns `None` if an exact match is found (no correction needed) or no
/// candidate is close enough. This is a convenience wrapper around
/// [`find_best_match`] for use cases that only need the corrected string.
pub fn find_best_correction(word: &str, candidates: &[String]) -> Option<String> {
    let indexed: Vec<(usize, &str)> = candidates.iter().enumerate().map(|(i, s)| (i, s.as_str())).collect();

    match find_best_match(word, &indexed) {
        Some((_, MatchKind::Exact)) => None, // exact match — no correction needed
        Some((idx, _)) => Some(candidates[idx].clone()),
        None => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- levenshtein ---

    #[test]
    fn test_levenshtein_identical() {
        assert_eq!(levenshtein("hello", "hello"), 0);
        assert_eq!(levenshtein("", ""), 0);
    }

    #[test]
    fn test_levenshtein_one_edit() {
        assert_eq!(levenshtein("cat", "bat"), 1);
        assert_eq!(levenshtein("cat", "cats"), 1);
        assert_eq!(levenshtein("cats", "cat"), 1);
    }

    #[test]
    fn test_levenshtein_multiple_edits() {
        assert_eq!(levenshtein("kitten", "sitting"), 3);
        assert_eq!(levenshtein("saturday", "sunday"), 3);
    }

    #[test]
    fn test_levenshtein_empty() {
        assert_eq!(levenshtein("", "abc"), 3);
        assert_eq!(levenshtein("abc", ""), 3);
    }

    #[test]
    fn test_levenshtein_unicode() {
        assert_eq!(levenshtein("café", "cafe"), 1);
        assert_eq!(levenshtein("naïve", "naive"), 1);
    }

    // --- max_edit_distance ---

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

    // --- normalize_name ---

    #[test]
    fn test_normalize_name() {
        assert_eq!(normalize_name("Deploy-App"), "deploy-app");
        assert_eq!(normalize_name("deploy_app"), "deploy-app");
        assert_eq!(normalize_name("deploy app"), "deploy-app");
        assert_eq!(normalize_name("--deploy-app--"), "deploy-app");
        assert_eq!(normalize_name("DEPLOY_APP"), "deploy-app");
    }

    // --- find_best_match ---

    #[test]
    fn test_exact_match() {
        let candidates = vec![(0, "deploy-app"), (1, "build-tool")];
        let result = find_best_match("deploy-app", &candidates);
        assert_eq!(result, Some((0, MatchKind::Exact)));
    }

    #[test]
    fn test_case_insensitive_match() {
        let candidates = vec![(0, "deploy-app"), (1, "build-tool")];
        let result = find_best_match("Deploy-App", &candidates);
        assert_eq!(result, Some((0, MatchKind::CaseInsensitive)));
    }

    #[test]
    fn test_normalized_match() {
        let candidates = vec![(0, "deploy-app"), (1, "build-tool")];
        let result = find_best_match("deploy_app", &candidates);
        assert_eq!(result, Some((0, MatchKind::Normalized)));
    }

    #[test]
    fn test_fuzzy_match() {
        let candidates = vec![(0, "deploy-app"), (1, "build-tool")];
        let result = find_best_match("deplpy-app", &candidates);
        assert_eq!(result, Some((0, MatchKind::Fuzzy)));
    }

    #[test]
    fn test_no_match() {
        let candidates = vec![(0, "deploy-app"), (1, "build-tool")];
        let result = find_best_match("completely-different-name", &candidates);
        assert_eq!(result, None);
    }

    #[test]
    fn test_empty_candidates() {
        let candidates: Vec<(usize, &str)> = vec![];
        assert_eq!(find_best_match("anything", &candidates), None);
    }

    #[test]
    fn test_preference_ordering() {
        let candidates = vec![(0, "Test"), (1, "test")];
        let result = find_best_match("test", &candidates);
        assert_eq!(result, Some((1, MatchKind::Exact)));
    }

    #[test]
    fn test_fuzzy_picks_closest() {
        let candidates = vec![
            (0, "configuration"),
            (1, "computation"),
            (2, "confguration"),
        ];
        let result = find_best_match("configuraton", &candidates);
        assert_eq!(result, Some((0, MatchKind::Fuzzy)));
    }

    #[test]
    fn test_fuzzy_tie_breaks_alphabetically() {
        let candidates = vec![(0, "bbb"), (1, "aaa")];
        let result = find_best_match("aab", &candidates);
        assert_eq!(result, Some((1, MatchKind::Fuzzy)));
    }

    #[test]
    fn test_case_insensitive_before_normalized() {
        let candidates = vec![(0, "deploy-app")];
        let result = find_best_match("Deploy_App", &candidates);
        assert_eq!(result, Some((0, MatchKind::Normalized)));
    }

    // --- find_best_correction ---

    #[test]
    fn test_find_best_correction_exact_match() {
        let candidates = vec!["kubernetes".to_string(), "kubelet".to_string()];
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
            "confguration".to_string(),
            "computation".to_string(),
        ];
        assert_eq!(
            find_best_correction("configuraton", &candidates),
            Some("configuration".to_string())
        );
    }
}
