//! Embedding-based semantic search across sessions.
//!
//! Uses the [`EmbeddingCache`] to find sessions whose content is
//! semantically similar to a query string.

use crate::embeddings::{EmbeddingCache, cosine_similarity};
use crate::local_embeddings::{LocalEmbedder, TfIdfEmbedder};

/// A session ID paired with its similarity score.
pub type SessionScore = (String, f64);

/// Search sessions by embedding similarity.
///
/// Given a query string and an [`EmbeddingCache`] containing session
/// embeddings (keyed by session ID), returns sessions ranked by cosine
/// similarity in descending order.
///
/// Each entry in `session_texts` is a `(session_id, text)` pair. The text
/// is typically a concatenation of the session's messages or summary.
///
/// If the query or a session text does not have a cached embedding, one is
/// generated using the provided [`LocalEmbedder`].
///
/// # Arguments
/// * `query` - The search query text.
/// * `cache` - Mutable reference to an embedding cache.
/// * `session_texts` - Pairs of `(session_id, text_content)`.
/// * `embedder` - A local embedder to generate missing embeddings.
/// * `min_score` - Minimum similarity score to include (0.0 to 1.0).
///
/// # Returns
/// A vector of `(session_id, score)` sorted by descending similarity.
pub fn semantic_search_sessions(
    query: &str,
    cache: &mut EmbeddingCache,
    session_texts: &[(String, String)],
    embedder: &dyn LocalEmbedder,
    min_score: f64,
) -> Vec<SessionScore> {
    if query.trim().is_empty() || session_texts.is_empty() {
        return Vec::new();
    }

    // Get or compute query embedding
    let query_embedding = match cache.get(query, None) {
        Some(emb) => emb.clone(),
        None => {
            let emb = embedder.embed(query);
            cache.set(query, emb.clone(), None);
            emb
        }
    };

    let mut results: Vec<SessionScore> = Vec::new();

    for (session_id, text) in session_texts {
        if text.trim().is_empty() {
            continue;
        }

        let session_embedding = match cache.get(text, None) {
            Some(emb) => emb.clone(),
            None => {
                let emb = embedder.embed(text);
                cache.set(text, emb.clone(), None);
                emb
            }
        };

        let score = cosine_similarity(&query_embedding, &session_embedding);
        if score >= min_score {
            results.push((session_id.clone(), score));
        }
    }

    results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    results
}

/// Convenience wrapper that uses a default [`TfIdfEmbedder`].
///
/// Suitable for quick searches when no pre-trained embedder is available.
pub fn semantic_search_sessions_default(
    query: &str,
    cache: &mut EmbeddingCache,
    session_texts: &[(String, String)],
) -> Vec<SessionScore> {
    let embedder = TfIdfEmbedder::default_embedder();
    semantic_search_sessions(query, cache, session_texts, &embedder, 0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_sessions() -> Vec<(String, String)> {
        vec![
            (
                "s1".to_string(),
                "rust programming cargo build system".to_string(),
            ),
            (
                "s2".to_string(),
                "python data science machine learning".to_string(),
            ),
            (
                "s3".to_string(),
                "rust async tokio runtime concurrency".to_string(),
            ),
            (
                "s4".to_string(),
                "cooking recipes italian pasta sauce".to_string(),
            ),
        ]
    }

    #[test]
    fn test_semantic_search_basic() {
        let mut cache = EmbeddingCache::new("local");
        let sessions = make_sessions();
        let embedder = TfIdfEmbedder::default_embedder();

        let results =
            semantic_search_sessions("rust cargo build", &mut cache, &sessions, &embedder, 0.0);

        assert!(!results.is_empty());
        // The "rust programming cargo build" session should rank high
        let top_id = &results[0].0;
        assert!(
            top_id == "s1" || top_id == "s3",
            "expected a rust session at top, got {top_id}"
        );
    }

    #[test]
    fn test_semantic_search_empty_query() {
        let mut cache = EmbeddingCache::new("local");
        let sessions = make_sessions();
        let embedder = TfIdfEmbedder::default_embedder();

        let results = semantic_search_sessions("", &mut cache, &sessions, &embedder, 0.0);
        assert!(results.is_empty());
    }

    #[test]
    fn test_semantic_search_empty_sessions() {
        let mut cache = EmbeddingCache::new("local");
        let embedder = TfIdfEmbedder::default_embedder();

        let results = semantic_search_sessions("query", &mut cache, &[], &embedder, 0.0);
        assert!(results.is_empty());
    }

    #[test]
    fn test_semantic_search_min_score_filter() {
        let mut cache = EmbeddingCache::new("local");
        let sessions = make_sessions();
        let embedder = TfIdfEmbedder::default_embedder();

        // With a very high threshold, most results should be filtered out
        let results =
            semantic_search_sessions("rust programming", &mut cache, &sessions, &embedder, 0.99);
        // Exact match is unlikely with TF-IDF, so most should be filtered
        assert!(results.len() <= 1);
    }

    #[test]
    fn test_semantic_search_sorted_descending() {
        let mut cache = EmbeddingCache::new("local");
        let sessions = make_sessions();
        let embedder = TfIdfEmbedder::default_embedder();

        let results =
            semantic_search_sessions("rust programming", &mut cache, &sessions, &embedder, 0.0);

        for window in results.windows(2) {
            assert!(
                window[0].1 >= window[1].1,
                "results should be sorted descending: {} >= {}",
                window[0].1,
                window[1].1
            );
        }
    }

    #[test]
    fn test_semantic_search_uses_cache() {
        let mut cache = EmbeddingCache::new("local");
        let sessions = make_sessions();
        let embedder = TfIdfEmbedder::default_embedder();

        // First search populates the cache
        let results1 = semantic_search_sessions("rust", &mut cache, &sessions, &embedder, 0.0);
        let cache_size_after_first = cache.size();

        // Second search should use cached embeddings
        let results2 = semantic_search_sessions("rust", &mut cache, &sessions, &embedder, 0.0);

        assert_eq!(results1.len(), results2.len());
        assert_eq!(cache.size(), cache_size_after_first);

        // Scores should be identical since embeddings are cached
        for (r1, r2) in results1.iter().zip(results2.iter()) {
            assert_eq!(r1.0, r2.0);
            assert!((r1.1 - r2.1).abs() < 1e-10);
        }
    }

    #[test]
    fn test_semantic_search_default() {
        let mut cache = EmbeddingCache::new("local");
        let sessions = make_sessions();

        let results = semantic_search_sessions_default("rust cargo", &mut cache, &sessions);
        assert!(!results.is_empty());
    }

    #[test]
    fn test_semantic_search_skips_empty_texts() {
        let mut cache = EmbeddingCache::new("local");
        let sessions = vec![
            ("s1".to_string(), "rust programming".to_string()),
            ("s2".to_string(), "".to_string()),
            ("s3".to_string(), "   ".to_string()),
        ];
        let embedder = TfIdfEmbedder::default_embedder();

        let results = semantic_search_sessions("rust", &mut cache, &sessions, &embedder, 0.0);

        // Only s1 should appear (s2 and s3 are empty/whitespace)
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, "s1");
    }
}
