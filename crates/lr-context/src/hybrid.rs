//! Reciprocal Rank Fusion (RRF) merge of FTS5 and vector search results.

use crate::types::{ContentType, MatchLayer, SearchHit};
use std::collections::HashMap;

/// A hit from vector (cosine similarity) search.
pub struct VectorSearchHit {
    pub source: String,
    pub title: String,
    pub content: String,
    pub score: f32,
    pub content_type: ContentType,
    pub line_start: usize,
    pub line_end: usize,
}

/// RRF constant (standard value, same as memsearch).
const RRF_K: f64 = 60.0;

/// Merge FTS5 and vector results using Reciprocal Rank Fusion.
///
/// Each result list is ranked independently. For each item, the RRF score
/// is `1/(k + rank)` summed across lists where it appears.
/// Items are identified by `(source, line_start, line_end)`.
pub fn rrf_merge(
    fts_hits: &[SearchHit],
    vector_hits: &[VectorSearchHit],
    limit: usize,
) -> Vec<SearchHit> {
    // Key: (source, line_start, line_end)
    type Key = (String, usize, usize);

    let mut scores: HashMap<Key, f64> = HashMap::new();
    let mut hit_data: HashMap<Key, SearchHit> = HashMap::new();

    // Add FTS5 hits (already ranked by BM25)
    for (rank, hit) in fts_hits.iter().enumerate() {
        let key = (hit.source.clone(), hit.line_start, hit.line_end);
        let rrf_score = 1.0 / (RRF_K + rank as f64 + 1.0);
        *scores.entry(key.clone()).or_default() += rrf_score;
        hit_data.entry(key).or_insert_with(|| hit.clone());
    }

    // Add vector hits (ranked by cosine similarity, highest first)
    for (rank, vhit) in vector_hits.iter().enumerate() {
        let key = (vhit.source.clone(), vhit.line_start, vhit.line_end);
        let rrf_score = 1.0 / (RRF_K + rank as f64 + 1.0);
        *scores.entry(key.clone()).or_default() += rrf_score;
        hit_data.entry(key).or_insert_with(|| SearchHit {
            title: vhit.title.clone(),
            content: vhit.content.clone(),
            source: vhit.source.clone(),
            rank: -(vhit.score as f64), // Negative for consistency with BM25 convention
            content_type: vhit.content_type,
            match_layer: MatchLayer::Porter, // Vector hits surfaced as Porter layer
            line_start: vhit.line_start,
            line_end: vhit.line_end,
        });
    }

    // Sort by combined RRF score descending
    let mut ranked: Vec<(Key, f64)> = scores.into_iter().collect();
    ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    ranked
        .into_iter()
        .take(limit)
        .filter_map(|(key, rrf_score)| {
            hit_data.remove(&key).map(|mut hit| {
                // Store the RRF score as the rank for sorting
                hit.rank = -rrf_score;
                hit
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fts_hit(source: &str, line_start: usize, rank: f64) -> SearchHit {
        SearchHit {
            title: format!("FTS {}", source),
            content: "fts content".to_string(),
            source: source.to_string(),
            rank,
            content_type: ContentType::Prose,
            match_layer: MatchLayer::Porter,
            line_start,
            line_end: line_start + 5,
        }
    }

    fn vec_hit(source: &str, line_start: usize, score: f32) -> VectorSearchHit {
        VectorSearchHit {
            source: source.to_string(),
            title: format!("Vec {}", source),
            content: "vec content".to_string(),
            score,
            content_type: ContentType::Prose,
            line_start,
            line_end: line_start + 5,
        }
    }

    #[test]
    fn rrf_merge_empty_inputs() {
        let result = rrf_merge(&[], &[], 10);
        assert!(result.is_empty());
    }

    #[test]
    fn rrf_merge_fts_only() {
        let fts = vec![fts_hit("a", 1, -1.0), fts_hit("b", 10, -0.5)];
        let result = rrf_merge(&fts, &[], 10);
        assert_eq!(result.len(), 2);
        // First FTS result should be ranked higher
        assert_eq!(result[0].source, "a");
    }

    #[test]
    fn rrf_merge_vector_only() {
        let vec_hits = vec![vec_hit("a", 1, 0.9), vec_hit("b", 10, 0.7)];
        let result = rrf_merge(&[], &vec_hits, 10);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].source, "a");
    }

    #[test]
    fn rrf_merge_overlap_boosted() {
        // Item "a" appears in both lists → should be ranked first due to combined score
        let fts = vec![fts_hit("a", 1, -1.0), fts_hit("b", 10, -0.5)];
        let vec_hits = vec![vec_hit("c", 20, 0.95), vec_hit("a", 1, 0.8)];
        let result = rrf_merge(&fts, &vec_hits, 10);
        // "a" appears in both, so it should be first
        assert_eq!(result[0].source, "a");
    }

    #[test]
    fn rrf_merge_respects_limit() {
        let fts = vec![
            fts_hit("a", 1, -2.0),
            fts_hit("b", 10, -1.5),
            fts_hit("c", 20, -1.0),
        ];
        let result = rrf_merge(&fts, &[], 2);
        assert_eq!(result.len(), 2);
    }
}
