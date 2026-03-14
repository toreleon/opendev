//! Integration tests for the ACE memory system.
//!
//! Tests playbook CRUD, file persistence, delta batch operations,
//! and embedding cache with real filesystem I/O.

use std::collections::HashMap;

use opendev_memory::{
    Bullet, BulletSelector, DeltaBatch, DeltaOperation, DeltaOperationType, EmbeddingCache,
    Playbook,
};
use tempfile::TempDir;

// ========================================================================
// Playbook load/save cycle
// ========================================================================

/// Full playbook lifecycle: create, populate, save, load, verify.
#[test]
fn playbook_save_and_load_cycle() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("playbook.json");

    // Create and populate
    let mut pb = Playbook::new();
    pb.add_bullet(
        "testing",
        "Always run tests after changes",
        Some("t-001"),
        None,
    );
    pb.add_bullet("testing", "Check edge cases", Some("t-002"), None);
    pb.add_bullet(
        "code_nav",
        "Use search before reading files",
        Some("cn-001"),
        None,
    );
    pb.tag_bullet("t-001", "helpful", 5);
    pb.tag_bullet("t-001", "harmful", 1);
    pb.tag_bullet("cn-001", "helpful", 3);

    // Save
    pb.save_to_file(&path).unwrap();

    // Load
    let loaded = Playbook::load_from_file(&path).unwrap();
    assert_eq!(loaded.bullet_count(), 3);

    let t001 = loaded.get_bullet("t-001").unwrap();
    assert_eq!(t001.content, "Always run tests after changes");
    assert_eq!(t001.helpful, 5);
    assert_eq!(t001.harmful, 1);
    assert_eq!(t001.section, "testing");

    let cn001 = loaded.get_bullet("cn-001").unwrap();
    assert_eq!(cn001.content, "Use search before reading files");
    assert_eq!(cn001.helpful, 3);
}

/// Playbook JSON string round-trip via dumps/loads.
#[test]
fn playbook_dumps_loads_roundtrip() {
    let mut pb = Playbook::new();
    pb.add_bullet("debugging", "Check logs first", Some("d-001"), None);
    pb.add_bullet(
        "performance",
        "Profile before optimizing",
        Some("p-001"),
        None,
    );

    let json = pb.dumps();
    assert!(!json.is_empty());

    let restored = Playbook::loads(&json).unwrap();
    assert_eq!(restored.bullet_count(), 2);
    assert!(restored.get_bullet("d-001").is_some());
    assert!(restored.get_bullet("p-001").is_some());
}

/// Playbook update modifies content without changing ID.
#[test]
fn playbook_update_preserves_id() {
    let mut pb = Playbook::new();
    pb.add_bullet("testing", "Original", Some("t-001"), None);

    let updated = pb.update_bullet("t-001", Some("Updated content"), None);
    assert!(updated.is_some());
    assert_eq!(updated.unwrap().content, "Updated content");
    assert_eq!(updated.unwrap().id, "t-001");
}

/// Playbook remove cleans up section when last bullet removed.
#[test]
fn playbook_remove_cleans_empty_section() {
    let mut pb = Playbook::new();
    pb.add_bullet("singleton", "Only bullet", Some("s-001"), None);
    assert!(pb.section_names().contains(&"singleton"));

    pb.remove_bullet("s-001");
    assert_eq!(pb.bullet_count(), 0);
    assert!(pb.section_names().is_empty());
}

/// Playbook as_prompt generates readable output.
#[test]
fn playbook_as_prompt_format() {
    let mut pb = Playbook::new();
    pb.add_bullet("code_nav", "Search before read", Some("cn-001"), None);
    pb.add_bullet("testing", "Run tests after changes", Some("t-001"), None);
    pb.tag_bullet("cn-001", "helpful", 3);

    let prompt = pb.as_prompt();
    assert!(prompt.contains("## code_nav"));
    assert!(prompt.contains("## testing"));
    assert!(prompt.contains("[cn-001] Search before read"));
    assert!(prompt.contains("(helpful=3, harmful=0, neutral=0)"));
}

/// Empty playbook returns empty prompt string.
#[test]
fn playbook_empty_prompt() {
    let pb = Playbook::new();
    assert!(pb.as_prompt().is_empty());
}

/// Playbook stats aggregate counters correctly.
#[test]
fn playbook_stats() {
    let mut pb = Playbook::new();
    pb.add_bullet("a", "A", Some("a-001"), None);
    pb.add_bullet("b", "B", Some("b-001"), None);
    pb.tag_bullet("a-001", "helpful", 10);
    pb.tag_bullet("a-001", "harmful", 2);
    pb.tag_bullet("b-001", "neutral", 5);

    let stats = pb.stats();
    assert_eq!(stats.sections, 2);
    assert_eq!(stats.bullets, 2);
    assert_eq!(stats.helpful, 10);
    assert_eq!(stats.harmful, 2);
    assert_eq!(stats.neutral, 5);
}

/// Auto-generated IDs use section prefix.
#[test]
fn playbook_auto_id_uses_section_prefix() {
    let mut pb = Playbook::new();
    let b1_id = pb
        .add_bullet("file operations", "First", None, None)
        .id
        .clone();
    assert!(b1_id.starts_with("file-"));

    let b2_id = pb
        .add_bullet("file operations", "Second", None, None)
        .id
        .clone();
    assert!(b2_id.starts_with("file-"));
    assert_ne!(b1_id, b2_id);
}

// ========================================================================
// Delta batch creation and application
// ========================================================================

/// Delta batch Add operation creates a new bullet.
#[test]
fn delta_add_creates_bullet() {
    let mut pb = Playbook::new();

    let delta = DeltaBatch {
        reasoning: "Adding new insight".to_string(),
        operations: vec![DeltaOperation {
            op_type: DeltaOperationType::Add,
            section: "new_section".to_string(),
            content: Some("Brand new bullet".to_string()),
            bullet_id: Some("new-001".to_string()),
            metadata: HashMap::new(),
        }],
    };

    pb.apply_delta(&delta);
    assert_eq!(pb.bullet_count(), 1);
    let bullet = pb.get_bullet("new-001").unwrap();
    assert_eq!(bullet.content, "Brand new bullet");
    assert_eq!(bullet.section, "new_section");
}

/// Delta batch Update operation modifies content.
#[test]
fn delta_update_modifies_content() {
    let mut pb = Playbook::new();
    pb.add_bullet("testing", "Old content", Some("t-001"), None);

    let delta = DeltaBatch {
        reasoning: "Improving wording".to_string(),
        operations: vec![DeltaOperation {
            op_type: DeltaOperationType::Update,
            section: "testing".to_string(),
            content: Some("Improved content".to_string()),
            bullet_id: Some("t-001".to_string()),
            metadata: HashMap::new(),
        }],
    };

    pb.apply_delta(&delta);
    assert_eq!(pb.get_bullet("t-001").unwrap().content, "Improved content");
}

/// Delta batch Tag operation increments counters.
#[test]
fn delta_tag_increments_counters() {
    let mut pb = Playbook::new();
    pb.add_bullet("testing", "Test", Some("t-001"), None);

    let delta = DeltaBatch {
        reasoning: "Tagging based on feedback".to_string(),
        operations: vec![DeltaOperation {
            op_type: DeltaOperationType::Tag,
            section: "testing".to_string(),
            content: None,
            bullet_id: Some("t-001".to_string()),
            metadata: [("helpful".to_string(), 3), ("harmful".to_string(), 1)]
                .into_iter()
                .collect(),
        }],
    };

    pb.apply_delta(&delta);
    let bullet = pb.get_bullet("t-001").unwrap();
    assert_eq!(bullet.helpful, 3);
    assert_eq!(bullet.harmful, 1);
}

/// Delta batch Remove operation deletes bullet.
#[test]
fn delta_remove_deletes_bullet() {
    let mut pb = Playbook::new();
    pb.add_bullet("old", "Outdated", Some("old-001"), None);
    assert_eq!(pb.bullet_count(), 1);

    let delta = DeltaBatch {
        reasoning: "Removing outdated bullet".to_string(),
        operations: vec![DeltaOperation {
            op_type: DeltaOperationType::Remove,
            section: "old".to_string(),
            content: None,
            bullet_id: Some("old-001".to_string()),
            metadata: HashMap::new(),
        }],
    };

    pb.apply_delta(&delta);
    assert_eq!(pb.bullet_count(), 0);
}

/// Multiple delta operations in a single batch.
#[test]
fn delta_batch_multiple_operations() {
    let mut pb = Playbook::new();
    pb.add_bullet("testing", "Existing", Some("t-001"), None);

    let delta = DeltaBatch {
        reasoning: "Batch update".to_string(),
        operations: vec![
            DeltaOperation {
                op_type: DeltaOperationType::Add,
                section: "code_nav".to_string(),
                content: Some("New navigation tip".to_string()),
                bullet_id: Some("cn-001".to_string()),
                metadata: HashMap::new(),
            },
            DeltaOperation {
                op_type: DeltaOperationType::Tag,
                section: "testing".to_string(),
                content: None,
                bullet_id: Some("t-001".to_string()),
                metadata: [("helpful".to_string(), 1)].into_iter().collect(),
            },
            DeltaOperation {
                op_type: DeltaOperationType::Update,
                section: "testing".to_string(),
                content: Some("Updated existing".to_string()),
                bullet_id: Some("t-001".to_string()),
                metadata: HashMap::new(),
            },
        ],
    };

    pb.apply_delta(&delta);
    assert_eq!(pb.bullet_count(), 2);
    assert_eq!(pb.get_bullet("t-001").unwrap().content, "Updated existing");
    assert_eq!(pb.get_bullet("t-001").unwrap().helpful, 1);
    assert!(pb.get_bullet("cn-001").is_some());
}

/// DeltaBatch JSON round-trip.
#[test]
fn delta_batch_json_roundtrip() {
    let batch = DeltaBatch {
        reasoning: "test reasoning".to_string(),
        operations: vec![
            DeltaOperation {
                op_type: DeltaOperationType::Add,
                section: "s1".to_string(),
                content: Some("content".to_string()),
                bullet_id: Some("b1".to_string()),
                metadata: HashMap::new(),
            },
            DeltaOperation {
                op_type: DeltaOperationType::Tag,
                section: "s2".to_string(),
                content: None,
                bullet_id: Some("b2".to_string()),
                metadata: [("helpful".to_string(), 1)].into_iter().collect(),
            },
        ],
    };

    let json = batch.to_json();
    let restored = DeltaBatch::from_json(&json);
    assert_eq!(restored.reasoning, "test reasoning");
    assert_eq!(restored.operations.len(), 2);
    assert_eq!(restored.operations[0].op_type, DeltaOperationType::Add);
    assert_eq!(restored.operations[1].op_type, DeltaOperationType::Tag);
}

/// DeltaOperation TAG filters invalid metadata keys.
#[test]
fn delta_tag_filters_invalid_metadata() {
    let json = serde_json::json!({
        "type": "TAG",
        "section": "testing",
        "bullet_id": "t-001",
        "metadata": {"helpful": 1, "invalid_key": 5, "neutral": 2}
    });
    let op = DeltaOperation::from_json(&json).unwrap();
    assert_eq!(op.metadata.len(), 2);
    assert!(op.metadata.contains_key("helpful"));
    assert!(op.metadata.contains_key("neutral"));
    assert!(!op.metadata.contains_key("invalid_key"));
}

// ========================================================================
// Embedding cache
// ========================================================================

/// Embedding cache set and get.
#[test]
fn embedding_cache_set_get() {
    let mut cache = EmbeddingCache::new("test-model");
    let embedding = vec![0.1, 0.2, 0.3, 0.4];

    cache.set("hello world", embedding.clone(), None);
    assert_eq!(cache.size(), 1);

    let result = cache.get("hello world", None);
    assert!(result.is_some());
    assert_eq!(result.unwrap(), &embedding);
}

/// Embedding cache is scoped by model.
#[test]
fn embedding_cache_model_scoping() {
    let mut cache = EmbeddingCache::new("model-a");
    cache.set("text", vec![1.0, 2.0], None);

    assert!(cache.get("text", Some("model-a")).is_some());
    assert!(cache.get("text", Some("model-b")).is_none());
}

/// Embedding cache miss returns None.
#[test]
fn embedding_cache_miss() {
    let mut cache = EmbeddingCache::new("test");
    assert!(cache.get("not cached", None).is_none());
}

/// Embedding cache clear removes all entries.
#[test]
fn embedding_cache_clear() {
    let mut cache = EmbeddingCache::new("test");
    cache.set("a", vec![1.0], None);
    cache.set("b", vec![2.0], None);
    assert_eq!(cache.size(), 2);

    cache.clear();
    assert_eq!(cache.size(), 0);
}

/// Embedding cache file persistence round-trip.
#[test]
fn embedding_cache_file_persistence() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("embeddings.json");

    let mut cache = EmbeddingCache::new("text-embedding-3-small");
    cache.set("hello", vec![0.1, 0.2, 0.3], None);
    cache.set("world", vec![0.4, 0.5, 0.6], None);
    cache.save_to_file(&path).unwrap();

    let mut loaded = EmbeddingCache::load_from_file(&path).unwrap();
    assert_eq!(loaded.size(), 2);
    assert_eq!(loaded.model, "text-embedding-3-small");
    assert!(loaded.get("hello", None).is_some());
    assert!(loaded.get("world", None).is_some());
}

/// Loading nonexistent cache file returns None.
#[test]
fn embedding_cache_load_missing_file() {
    let result = EmbeddingCache::load_from_file(std::path::Path::new("/nonexistent/path.json"));
    assert!(result.is_none());
}

/// Embedding cache serialization round-trip via to_dict/from_dict.
#[test]
fn embedding_cache_dict_roundtrip() {
    let mut cache = EmbeddingCache::new("test-model");
    cache.set("text1", vec![0.1, 0.2], None);
    cache.set("text2", vec![0.3, 0.4], None);

    let dict = cache.to_dict();
    let restored = EmbeddingCache::from_dict(&dict);
    assert_eq!(restored.model, "test-model");
    assert_eq!(restored.size(), 2);
}

/// Cosine similarity calculations.
#[test]
fn cosine_similarity_calculations() {
    use opendev_memory::embeddings::{batch_cosine_similarity, cosine_similarity};

    // Identical vectors -> 1.0
    let v = vec![1.0, 0.0, 0.0];
    assert!((cosine_similarity(&v, &v) - 1.0).abs() < 1e-10);

    // Orthogonal vectors -> 0.0
    let v1 = vec![1.0, 0.0];
    let v2 = vec![0.0, 1.0];
    assert!(cosine_similarity(&v1, &v2).abs() < 1e-10);

    // Opposite vectors -> -1.0
    let v3 = vec![-1.0, 0.0];
    assert!((cosine_similarity(&v1, &v3) - (-1.0)).abs() < 1e-10);

    // Batch similarity
    let query = vec![1.0, 0.0];
    let vectors = vec![vec![1.0, 0.0], vec![0.0, 1.0], vec![-1.0, 0.0]];
    let results = batch_cosine_similarity(&query, &vectors);
    assert_eq!(results.len(), 3);
    assert!((results[0] - 1.0).abs() < 1e-10);
    assert!(results[1].abs() < 1e-10);
    assert!((results[2] - (-1.0)).abs() < 1e-10);
}
