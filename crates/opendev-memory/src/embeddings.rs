//! Embedding cache and cosine similarity for semantic bullet selection.
//!
//! Mirrors `opendev/core/context_engineering/memory/embeddings.py`.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::Path;

/// Default maximum number of entries in the embedding cache.
const DEFAULT_MAX_ENTRIES: usize = 10_000;

/// Metadata for a cached embedding.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingMetadata {
    pub text: String,
    pub model: String,
    pub hash: String,
    pub embedding: Vec<f64>,
}

impl EmbeddingMetadata {
    /// Create embedding metadata with computed hash.
    pub fn create(text: &str, model: &str, embedding: Vec<f64>) -> Self {
        let content = format!("{model}:{text}");
        let hash = make_hash(&content);
        Self {
            text: text.to_string(),
            model: model.to_string(),
            hash,
            embedding,
        }
    }
}

/// Cache for bullet embeddings to avoid redundant API calls.
///
/// Stores embeddings in memory and can be persisted to disk.
/// Cache keys are based on content hash + model name.
/// Uses LRU eviction when the cache exceeds `max_entries`.
#[derive(Debug, Clone)]
pub struct EmbeddingCache {
    pub model: String,
    cache: HashMap<String, EmbeddingMetadata>,
    /// Maximum number of entries before LRU eviction kicks in.
    pub max_entries: usize,
    /// Access order tracking: most recently used keys are at the end.
    access_order: Vec<String>,
}

impl EmbeddingCache {
    /// Create a new embedding cache with the default max entries (10,000).
    pub fn new(model: &str) -> Self {
        Self {
            model: model.to_string(),
            cache: HashMap::new(),
            max_entries: DEFAULT_MAX_ENTRIES,
            access_order: Vec::new(),
        }
    }

    /// Create a new embedding cache with a custom max entries limit.
    pub fn with_max_entries(model: &str, max_entries: usize) -> Self {
        Self {
            model: model.to_string(),
            cache: HashMap::new(),
            max_entries,
            access_order: Vec::new(),
        }
    }

    /// Get cached embedding for text.
    ///
    /// Marks the entry as recently used for LRU tracking.
    pub fn get(&mut self, text: &str, model: Option<&str>) -> Option<&Vec<f64>> {
        let model = model.unwrap_or(&self.model);
        let key = make_key(text, model);
        if self.cache.contains_key(&key) {
            self.touch(&key);
            self.cache.get(&key).map(|meta| &meta.embedding)
        } else {
            None
        }
    }

    /// Get cached embedding without updating LRU order (read-only lookup).
    pub fn peek(&self, text: &str, model: Option<&str>) -> Option<&Vec<f64>> {
        let model = model.unwrap_or(&self.model);
        let key = make_key(text, model);
        self.cache.get(&key).map(|meta| &meta.embedding)
    }

    /// Cache an embedding.
    ///
    /// If the cache is at capacity, the least-recently-used entry is evicted.
    pub fn set(&mut self, text: &str, embedding: Vec<f64>, model: Option<&str>) {
        let model_str = model.unwrap_or(&self.model).to_string();
        let key = make_key(text, &model_str);

        // If key already exists, just update it
        if self.cache.contains_key(&key) {
            let metadata = EmbeddingMetadata::create(text, &model_str, embedding);
            self.cache.insert(key.clone(), metadata);
            self.touch(&key);
            return;
        }

        // Evict LRU entry if at capacity
        if self.max_entries > 0 && self.cache.len() >= self.max_entries {
            self.evict_lru();
        }

        let metadata = EmbeddingMetadata::create(text, &model_str, embedding);
        self.cache.insert(key.clone(), metadata);
        self.access_order.push(key);
    }

    /// Move key to end of access_order (most recently used).
    fn touch(&mut self, key: &str) {
        self.access_order.retain(|k| k != key);
        self.access_order.push(key.to_string());
    }

    /// Evict the least-recently-used entry (front of access_order).
    fn evict_lru(&mut self) {
        if let Some(lru_key) = self.access_order.first().cloned() {
            self.cache.remove(&lru_key);
            self.access_order.remove(0);
        }
    }

    /// Clear all cached embeddings.
    pub fn clear(&mut self) {
        self.cache.clear();
        self.access_order.clear();
    }

    /// Get number of cached embeddings.
    pub fn size(&self) -> usize {
        self.cache.len()
    }

    /// Serialize cache to JSON value.
    pub fn to_dict(&self) -> serde_json::Value {
        let cache_map: serde_json::Map<String, serde_json::Value> = self
            .cache
            .iter()
            .map(|(key, meta)| (key.clone(), serde_json::to_value(meta).unwrap_or_default()))
            .collect();
        serde_json::json!({
            "model": self.model,
            "max_entries": self.max_entries,
            "cache": cache_map,
        })
    }

    /// Deserialize cache from JSON value.
    pub fn from_dict(data: &serde_json::Value) -> Self {
        let model = data
            .get("model")
            .and_then(|v| v.as_str())
            .unwrap_or("text-embedding-3-small")
            .to_string();

        let max_entries = data
            .get("max_entries")
            .and_then(|v| v.as_u64())
            .map(|v| v as usize)
            .unwrap_or(DEFAULT_MAX_ENTRIES);

        let mut cache = HashMap::new();
        let mut access_order = Vec::new();
        if let Some(cache_obj) = data.get("cache").and_then(|v| v.as_object()) {
            for (key, val) in cache_obj {
                if let Ok(meta) = serde_json::from_value::<EmbeddingMetadata>(val.clone()) {
                    cache.insert(key.clone(), meta);
                    access_order.push(key.clone());
                }
            }
        }

        Self {
            model,
            cache,
            max_entries,
            access_order,
        }
    }

    /// Save cache to JSON file.
    pub fn save_to_file(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(&self.to_dict()).map_err(std::io::Error::other)?;
        std::fs::write(path, json)
    }

    /// Load cache from JSON file. Returns None if file doesn't exist or is corrupt.
    pub fn load_from_file(path: &Path) -> Option<Self> {
        let content = std::fs::read_to_string(path).ok()?;
        let data: serde_json::Value = serde_json::from_str(&content).ok()?;
        Some(Self::from_dict(&data))
    }
}

impl Default for EmbeddingCache {
    fn default() -> Self {
        Self::new("text-embedding-3-small")
    }
}

/// Configuration for EmbeddingCache.
#[derive(Debug, Clone)]
pub struct EmbeddingCacheConfig {
    pub model: String,
    pub max_entries: usize,
}

impl Default for EmbeddingCacheConfig {
    fn default() -> Self {
        Self {
            model: "text-embedding-3-small".to_string(),
            max_entries: DEFAULT_MAX_ENTRIES,
        }
    }
}

/// Calculate cosine similarity between two vectors.
///
/// Returns a value between -1.0 and 1.0:
/// - 1.0 = identical direction
/// - 0.0 = orthogonal
/// - -1.0 = opposite direction
pub fn cosine_similarity(vec1: &[f64], vec2: &[f64]) -> f64 {
    if vec1.len() != vec2.len() || vec1.is_empty() {
        return 0.0;
    }

    let dot: f64 = vec1.iter().zip(vec2.iter()).map(|(a, b)| a * b).sum();
    let norm1: f64 = vec1.iter().map(|a| a * a).sum::<f64>().sqrt();
    let norm2: f64 = vec2.iter().map(|a| a * a).sum::<f64>().sqrt();

    if norm1 == 0.0 || norm2 == 0.0 {
        return 0.0;
    }

    let similarity = dot / (norm1 * norm2);
    similarity.clamp(-1.0, 1.0)
}

/// Calculate cosine similarity between a query vector and multiple vectors.
pub fn batch_cosine_similarity(query: &[f64], vectors: &[Vec<f64>]) -> Vec<f64> {
    vectors
        .iter()
        .map(|v| cosine_similarity(query, v))
        .collect()
}

/// Create a SHA-256 based cache key (first 16 hex chars).
fn make_key(text: &str, model: &str) -> String {
    make_hash(&format!("{model}:{text}"))
}

/// SHA-256 hash truncated to 16 hex chars.
fn make_hash(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    let result = hasher.finalize();
    hex::encode(&result[..8])
}

/// Inline hex encoding (avoids pulling in the `hex` crate just for this).
mod hex {
    pub fn encode(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_embedding_metadata_create() {
        let embedding = vec![0.1, 0.2, 0.3];
        let meta = EmbeddingMetadata::create("hello", "test-model", embedding.clone());
        assert_eq!(meta.text, "hello");
        assert_eq!(meta.model, "test-model");
        assert!(!meta.hash.is_empty());
        assert_eq!(meta.embedding, embedding);
    }

    #[test]
    fn test_embedding_cache_set_get() {
        let mut cache = EmbeddingCache::new("test-model");
        let embedding = vec![0.1, 0.2, 0.3];

        cache.set("hello", embedding.clone(), None);
        assert_eq!(cache.size(), 1);

        let result = cache.get("hello", None);
        assert!(result.is_some());
        assert_eq!(result.unwrap(), &embedding);

        // Different text returns None
        assert!(cache.get("world", None).is_none());
    }

    #[test]
    fn test_embedding_cache_model_scoping() {
        let mut cache = EmbeddingCache::new("model-a");
        cache.set("hello", vec![1.0], None);

        // Same text, same model -> found
        assert!(cache.peek("hello", Some("model-a")).is_some());

        // Same text, different model -> not found
        assert!(cache.peek("hello", Some("model-b")).is_none());
    }

    #[test]
    fn test_embedding_cache_clear() {
        let mut cache = EmbeddingCache::new("test");
        cache.set("a", vec![1.0], None);
        cache.set("b", vec![2.0], None);
        assert_eq!(cache.size(), 2);

        cache.clear();
        assert_eq!(cache.size(), 0);
    }

    #[test]
    fn test_embedding_cache_serialization() {
        let mut cache = EmbeddingCache::new("test-model");
        cache.set("hello", vec![0.1, 0.2], None);
        cache.set("world", vec![0.3, 0.4], None);

        let dict = cache.to_dict();
        let mut restored = EmbeddingCache::from_dict(&dict);

        assert_eq!(restored.model, "test-model");
        assert_eq!(restored.size(), 2);
        assert!(restored.get("hello", None).is_some());
        assert!(restored.get("world", None).is_some());
    }

    #[test]
    fn test_embedding_cache_file_persistence() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("embeddings.json");

        let mut cache = EmbeddingCache::new("test-model");
        cache.set("hello", vec![0.1, 0.2, 0.3], None);
        cache.save_to_file(&path).unwrap();

        let mut loaded = EmbeddingCache::load_from_file(&path).unwrap();
        assert_eq!(loaded.size(), 1);
        assert!(loaded.get("hello", None).is_some());
    }

    #[test]
    fn test_embedding_cache_load_missing_file() {
        let result = EmbeddingCache::load_from_file(Path::new("/nonexistent/path"));
        assert!(result.is_none());
    }

    #[test]
    fn test_cosine_similarity_identical() {
        let v = vec![1.0, 0.0, 0.0];
        let sim = cosine_similarity(&v, &v);
        assert!((sim - 1.0).abs() < 1e-10);
    }

    #[test]
    fn test_cosine_similarity_orthogonal() {
        let v1 = vec![1.0, 0.0];
        let v2 = vec![0.0, 1.0];
        let sim = cosine_similarity(&v1, &v2);
        assert!(sim.abs() < 1e-10);
    }

    #[test]
    fn test_cosine_similarity_opposite() {
        let v1 = vec![1.0, 0.0];
        let v2 = vec![-1.0, 0.0];
        let sim = cosine_similarity(&v1, &v2);
        assert!((sim - (-1.0)).abs() < 1e-10);
    }

    #[test]
    fn test_cosine_similarity_zero_vector() {
        let v1 = vec![1.0, 2.0];
        let v2 = vec![0.0, 0.0];
        assert_eq!(cosine_similarity(&v1, &v2), 0.0);
    }

    #[test]
    fn test_cosine_similarity_empty() {
        assert_eq!(cosine_similarity(&[], &[]), 0.0);
    }

    #[test]
    fn test_cosine_similarity_different_lengths() {
        let v1 = vec![1.0, 2.0];
        let v2 = vec![1.0];
        assert_eq!(cosine_similarity(&v1, &v2), 0.0);
    }

    #[test]
    fn test_batch_cosine_similarity() {
        let query = vec![1.0, 0.0];
        let vectors = vec![
            vec![1.0, 0.0],  // identical
            vec![0.0, 1.0],  // orthogonal
            vec![-1.0, 0.0], // opposite
        ];
        let results = batch_cosine_similarity(&query, &vectors);
        assert_eq!(results.len(), 3);
        assert!((results[0] - 1.0).abs() < 1e-10);
        assert!(results[1].abs() < 1e-10);
        assert!((results[2] - (-1.0)).abs() < 1e-10);
    }

    #[test]
    fn test_make_hash_deterministic() {
        let h1 = make_hash("test-model:hello");
        let h2 = make_hash("test-model:hello");
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 16); // 8 bytes = 16 hex chars
    }

    #[test]
    fn test_make_hash_different_inputs() {
        let h1 = make_hash("a");
        let h2 = make_hash("b");
        assert_ne!(h1, h2);
    }

    // ------------------------------------------------------------------ //
    // LRU eviction tests
    // ------------------------------------------------------------------ //

    #[test]
    fn test_lru_eviction_at_capacity() {
        let mut cache = EmbeddingCache::with_max_entries("test", 3);
        cache.set("a", vec![1.0], None);
        cache.set("b", vec![2.0], None);
        cache.set("c", vec![3.0], None);
        assert_eq!(cache.size(), 3);

        // Adding a 4th entry should evict "a" (least recently used)
        cache.set("d", vec![4.0], None);
        assert_eq!(cache.size(), 3);
        assert!(cache.peek("a", None).is_none(), "a should be evicted");
        assert!(cache.peek("b", None).is_some());
        assert!(cache.peek("c", None).is_some());
        assert!(cache.peek("d", None).is_some());
    }

    #[test]
    fn test_lru_access_refreshes_order() {
        let mut cache = EmbeddingCache::with_max_entries("test", 3);
        cache.set("a", vec![1.0], None);
        cache.set("b", vec![2.0], None);
        cache.set("c", vec![3.0], None);

        // Access "a" to make it most-recently-used
        cache.get("a", None);

        // Now "b" is LRU; adding "d" should evict "b"
        cache.set("d", vec![4.0], None);
        assert_eq!(cache.size(), 3);
        assert!(cache.peek("a", None).is_some(), "a was recently accessed");
        assert!(cache.peek("b", None).is_none(), "b should be evicted");
        assert!(cache.peek("c", None).is_some());
        assert!(cache.peek("d", None).is_some());
    }

    #[test]
    fn test_lru_update_existing_key_no_eviction() {
        let mut cache = EmbeddingCache::with_max_entries("test", 3);
        cache.set("a", vec![1.0], None);
        cache.set("b", vec![2.0], None);
        cache.set("c", vec![3.0], None);

        // Updating existing key should not trigger eviction
        cache.set("a", vec![10.0], None);
        assert_eq!(cache.size(), 3);
        assert_eq!(cache.get("a", None).unwrap(), &vec![10.0]);
    }

    #[test]
    fn test_lru_default_max_entries() {
        let cache = EmbeddingCache::new("test");
        assert_eq!(cache.max_entries, DEFAULT_MAX_ENTRIES);
    }

    #[test]
    fn test_lru_with_max_entries_constructor() {
        let cache = EmbeddingCache::with_max_entries("test", 500);
        assert_eq!(cache.max_entries, 500);
    }

    #[test]
    fn test_lru_clear_resets_access_order() {
        let mut cache = EmbeddingCache::with_max_entries("test", 3);
        cache.set("a", vec![1.0], None);
        cache.set("b", vec![2.0], None);
        cache.clear();
        assert_eq!(cache.size(), 0);

        // After clear, can fill up again without premature eviction
        cache.set("x", vec![1.0], None);
        cache.set("y", vec![2.0], None);
        cache.set("z", vec![3.0], None);
        assert_eq!(cache.size(), 3);
    }

    #[test]
    fn test_lru_serialization_preserves_max_entries() {
        let mut cache = EmbeddingCache::with_max_entries("test", 500);
        cache.set("a", vec![1.0], None);

        let dict = cache.to_dict();
        let restored = EmbeddingCache::from_dict(&dict);
        assert_eq!(restored.max_entries, 500);
    }

    #[test]
    fn test_embedding_cache_config_default() {
        let config = EmbeddingCacheConfig::default();
        assert_eq!(config.model, "text-embedding-3-small");
        assert_eq!(config.max_entries, DEFAULT_MAX_ENTRIES);
    }
}
