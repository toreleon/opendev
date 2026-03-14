//! Local embedding generation using TF-IDF bag-of-words.
//!
//! Provides a simple local embedder that generates embeddings without
//! requiring an external API. Uses TF-IDF (Term Frequency - Inverse Document
//! Frequency) to produce fixed-dimension embedding vectors.
//!
//! This module defines the `LocalEmbedder` trait interface that could be
//! backed by ONNX runtime when the `ort` feature is available. The default
//! implementation uses TF-IDF for basic local embeddings.

use std::collections::HashMap;

/// Trait for local embedding generation.
///
/// Implementations should produce normalized embedding vectors suitable for
/// cosine similarity comparisons.
pub trait LocalEmbedder: Send + Sync {
    /// Generate an embedding vector for the given text.
    fn embed(&self, text: &str) -> Vec<f64>;

    /// Generate embeddings for multiple texts.
    fn embed_batch(&self, texts: &[&str]) -> Vec<Vec<f64>> {
        texts.iter().map(|t| self.embed(t)).collect()
    }

    /// Return the dimensionality of the embedding vectors.
    fn dimension(&self) -> usize;
}

/// A simple TF-IDF based local embedder.
///
/// Uses a fixed vocabulary built from a set of seed documents. Each text is
/// represented as a normalized TF-IDF vector over the vocabulary.
///
/// This provides reasonable similarity detection for related texts without
/// requiring any external model or API.
#[derive(Debug, Clone)]
pub struct TfIdfEmbedder {
    /// Vocabulary: word -> index mapping.
    vocabulary: HashMap<String, usize>,
    /// Inverse document frequency for each term.
    idf: Vec<f64>,
    /// Dimensionality of output vectors.
    dim: usize,
}

/// Default vocabulary size when no seed documents are provided.
const DEFAULT_DIM: usize = 256;

impl TfIdfEmbedder {
    /// Create a new TF-IDF embedder with a pre-built vocabulary.
    ///
    /// The vocabulary is built from the provided seed documents. Each unique
    /// word (lowercased, alphanumeric only) becomes a dimension in the
    /// embedding space, up to `max_dim` dimensions.
    pub fn new(seed_documents: &[&str], max_dim: usize) -> Self {
        let mut word_doc_count: HashMap<String, usize> = HashMap::new();
        let mut word_freq: HashMap<String, usize> = HashMap::new();
        let total_docs = seed_documents.len().max(1);

        for doc in seed_documents {
            let mut seen_in_doc: std::collections::HashSet<String> =
                std::collections::HashSet::new();
            for word in tokenize(doc) {
                *word_freq.entry(word.clone()).or_insert(0) += 1;
                if seen_in_doc.insert(word.clone()) {
                    *word_doc_count.entry(word).or_insert(0) += 1;
                }
            }
        }

        // Select top-N words by frequency as the vocabulary
        let mut words_by_freq: Vec<(String, usize)> = word_freq.into_iter().collect();
        words_by_freq.sort_by(|a, b| b.1.cmp(&a.1));
        words_by_freq.truncate(max_dim);

        let mut vocabulary = HashMap::new();
        let mut idf = Vec::new();
        for (idx, (word, _freq)) in words_by_freq.into_iter().enumerate() {
            let doc_count = word_doc_count.get(&word).copied().unwrap_or(1);
            let idf_value = ((total_docs as f64) / (doc_count as f64 + 1.0)).ln() + 1.0;
            vocabulary.insert(word, idx);
            idf.push(idf_value);
        }

        let dim = vocabulary.len().max(1);

        Self {
            vocabulary,
            idf,
            dim,
        }
    }

    /// Create a TF-IDF embedder with default settings and no seed documents.
    ///
    /// Uses a hash-based approach to map words to dimensions, suitable when
    /// no training corpus is available.
    pub fn default_embedder() -> Self {
        Self {
            vocabulary: HashMap::new(),
            idf: Vec::new(),
            dim: DEFAULT_DIM,
        }
    }
}

impl LocalEmbedder for TfIdfEmbedder {
    fn embed(&self, text: &str) -> Vec<f64> {
        let tokens = tokenize(text);
        let total_tokens = tokens.len().max(1) as f64;

        if self.vocabulary.is_empty() {
            // Hash-based fallback: map words to dimensions via hash
            let mut vec = vec![0.0f64; self.dim];
            for token in &tokens {
                let hash = simple_hash(token) % self.dim;
                vec[hash] += 1.0 / total_tokens;
            }
            normalize(&mut vec);
            return vec;
        }

        // TF-IDF computation
        let mut vec = vec![0.0f64; self.dim];
        for token in &tokens {
            if let Some(&idx) = self.vocabulary.get(token) {
                let tf = 1.0 / total_tokens;
                vec[idx] += tf * self.idf[idx];
            }
        }
        normalize(&mut vec);
        vec
    }

    fn dimension(&self) -> usize {
        self.dim
    }
}

impl Default for TfIdfEmbedder {
    fn default() -> Self {
        Self::default_embedder()
    }
}

/// Tokenize text into lowercase alphanumeric words.
fn tokenize(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|w| w.len() >= 2)
        .map(|w| w.to_lowercase())
        .collect()
}

/// Simple deterministic hash for a string.
fn simple_hash(s: &str) -> usize {
    let mut hash: usize = 5381;
    for byte in s.bytes() {
        hash = hash.wrapping_mul(33).wrapping_add(byte as usize);
    }
    hash
}

/// L2-normalize a vector in place.
fn normalize(vec: &mut [f64]) {
    let norm: f64 = vec.iter().map(|x| x * x).sum::<f64>().sqrt();
    if norm > 0.0 {
        for x in vec.iter_mut() {
            *x /= norm;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tokenize() {
        let tokens = tokenize("Hello, world! This is a test.");
        assert!(tokens.contains(&"hello".to_string()));
        assert!(tokens.contains(&"world".to_string()));
        assert!(tokens.contains(&"this".to_string()));
        assert!(tokens.contains(&"test".to_string()));
        // Single-char words are filtered out
        assert!(!tokens.contains(&"a".to_string()));
    }

    #[test]
    fn test_tokenize_empty() {
        let tokens = tokenize("");
        assert!(tokens.is_empty());
    }

    #[test]
    fn test_normalize() {
        let mut vec = vec![3.0, 4.0];
        normalize(&mut vec);
        let norm: f64 = vec.iter().map(|x| x * x).sum::<f64>().sqrt();
        assert!((norm - 1.0).abs() < 1e-10);
    }

    #[test]
    fn test_normalize_zero_vector() {
        let mut vec = vec![0.0, 0.0, 0.0];
        normalize(&mut vec);
        assert_eq!(vec, vec![0.0, 0.0, 0.0]);
    }

    #[test]
    fn test_default_embedder() {
        let embedder = TfIdfEmbedder::default_embedder();
        assert_eq!(embedder.dimension(), DEFAULT_DIM);

        let emb = embedder.embed("hello world");
        assert_eq!(emb.len(), DEFAULT_DIM);

        // Should be normalized
        let norm: f64 = emb.iter().map(|x| x * x).sum::<f64>().sqrt();
        assert!((norm - 1.0).abs() < 1e-10);
    }

    #[test]
    fn test_default_embedder_empty_text() {
        let embedder = TfIdfEmbedder::default_embedder();
        let emb = embedder.embed("");
        assert_eq!(emb.len(), DEFAULT_DIM);
        // All zeros since no tokens
        assert!(emb.iter().all(|&x| x == 0.0));
    }

    #[test]
    fn test_tfidf_embedder_with_seeds() {
        let seeds = &[
            "rust programming language systems",
            "python scripting language dynamic",
            "rust cargo build system package",
        ];
        let embedder = TfIdfEmbedder::new(seeds, 50);

        let emb1 = embedder.embed("rust programming");
        let emb2 = embedder.embed("python scripting");

        assert_eq!(emb1.len(), embedder.dimension());
        assert_eq!(emb2.len(), embedder.dimension());

        // Embeddings for different texts should differ
        assert_ne!(emb1, emb2);
    }

    #[test]
    fn test_tfidf_similar_texts_closer() {
        let seeds = &[
            "rust programming language",
            "python scripting language",
            "cooking recipes food",
            "rust cargo build tools",
        ];
        let embedder = TfIdfEmbedder::new(seeds, 100);

        let rust1 = embedder.embed("rust programming cargo");
        let rust2 = embedder.embed("rust language build");
        let cooking = embedder.embed("cooking food recipes");

        let sim_rust = cosine_sim(&rust1, &rust2);
        let sim_diff = cosine_sim(&rust1, &cooking);

        assert!(
            sim_rust > sim_diff,
            "similar topics should be closer: rust-rust={sim_rust} vs rust-cooking={sim_diff}"
        );
    }

    #[test]
    fn test_embed_batch() {
        let embedder = TfIdfEmbedder::default_embedder();
        let texts = &["hello world", "goodbye world", "foo bar"];
        let embeddings = embedder.embed_batch(texts);
        assert_eq!(embeddings.len(), 3);
        for emb in &embeddings {
            assert_eq!(emb.len(), DEFAULT_DIM);
        }
    }

    #[test]
    fn test_deterministic_embeddings() {
        let embedder = TfIdfEmbedder::default_embedder();
        let emb1 = embedder.embed("consistent output");
        let emb2 = embedder.embed("consistent output");
        assert_eq!(emb1, emb2);
    }

    #[test]
    fn test_simple_hash_deterministic() {
        let h1 = simple_hash("test");
        let h2 = simple_hash("test");
        assert_eq!(h1, h2);
    }

    #[test]
    fn test_simple_hash_different() {
        let h1 = simple_hash("abc");
        let h2 = simple_hash("xyz");
        assert_ne!(h1, h2);
    }

    /// Helper: cosine similarity between two vectors.
    fn cosine_sim(a: &[f64], b: &[f64]) -> f64 {
        let dot: f64 = a.iter().zip(b).map(|(x, y)| x * y).sum();
        let na: f64 = a.iter().map(|x| x * x).sum::<f64>().sqrt();
        let nb: f64 = b.iter().map(|x| x * x).sum::<f64>().sqrt();
        if na == 0.0 || nb == 0.0 {
            return 0.0;
        }
        dot / (na * nb)
    }
}
