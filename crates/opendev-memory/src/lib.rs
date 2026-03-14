//! ACE memory system for OpenDev.
//!
//! This crate implements the Agentic Context Engine (ACE) memory system:
//! - Playbook: Structured store for accumulated strategies and insights
//! - Delta: Batch mutation operations on the playbook
//! - Embeddings: Embedding cache and cosine similarity for semantic search
//! - Selector: Intelligent bullet selection for LLM context
//! - Reflector: Post-turn reflection to extract learnable patterns
//! - Roles: ACE role data models (Reflector, Curator outputs)

pub mod delta;
pub mod embeddings;
pub mod local_embeddings;
pub mod playbook;
pub mod reflector;
pub mod roles;
pub mod selector;
pub mod session_search;
pub mod summarizer;

pub use delta::{DeltaBatch, DeltaOperation, DeltaOperationType};
pub use embeddings::{EmbeddingCache, EmbeddingCacheConfig, EmbeddingMetadata};
pub use local_embeddings::{LocalEmbedder, TfIdfEmbedder};
pub use playbook::{Bullet, Playbook};
pub use reflector::{ExecutionReflector, ReflectionResult, score_reflection};
pub use roles::{AgentResponse, BulletTag, CuratorOutput, ReflectorOutput};
pub use selector::{BulletSelector, ScoredBullet};
pub use session_search::{semantic_search_sessions, semantic_search_sessions_default};
pub use summarizer::{ConversationSummarizer, consolidate_learnings};
