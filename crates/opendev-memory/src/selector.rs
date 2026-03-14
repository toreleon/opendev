//! Bullet selection logic for ACE playbook context optimization.
//!
//! Mirrors `opendev/core/context_engineering/memory/selector.py`.

use std::collections::HashMap;

use crate::embeddings::{EmbeddingCache, cosine_similarity};
use crate::playbook::Bullet;

/// Bullet with its calculated relevance score.
#[derive(Debug, Clone)]
pub struct ScoredBullet {
    pub bullet: Bullet,
    pub score: f64,
    pub score_breakdown: HashMap<String, f64>,
}

/// Selects most relevant bullets for a given query.
///
/// Implements hybrid retrieval with three scoring factors:
/// - Effectiveness: Based on helpful/harmful feedback
/// - Recency: Prefers recently updated bullets
/// - Semantic: Query-to-bullet similarity using embeddings
pub struct BulletSelector {
    pub weights: HashMap<String, f64>,
    pub embedding_model: String,
    pub cache_file: Option<String>,
    pub embedding_cache: EmbeddingCache,
}

impl BulletSelector {
    /// Create a new bullet selector.
    pub fn new(
        weights: Option<HashMap<String, f64>>,
        embedding_model: &str,
        cache_file: Option<&str>,
    ) -> Self {
        let weights = weights.unwrap_or_else(|| {
            let mut w = HashMap::new();
            w.insert("effectiveness".to_string(), 0.6);
            w.insert("recency".to_string(), 0.4);
            w.insert("semantic".to_string(), 0.0);
            w
        });

        let embedding_cache = cache_file
            .and_then(|p| EmbeddingCache::load_from_file(std::path::Path::new(p)))
            .unwrap_or_else(|| EmbeddingCache::new(embedding_model));

        Self {
            weights,
            embedding_model: embedding_model.to_string(),
            cache_file: cache_file.map(String::from),
            embedding_cache,
        }
    }

    /// Select top-K most relevant bullets.
    pub fn select(&self, bullets: &[Bullet], max_count: usize, query: Option<&str>) -> Vec<Bullet> {
        if bullets.len() <= max_count {
            return bullets.to_vec();
        }

        let mut scored: Vec<ScoredBullet> = bullets
            .iter()
            .map(|b| self.score_bullet(b, query))
            .collect();

        scored.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        scored
            .into_iter()
            .take(max_count)
            .map(|sb| sb.bullet)
            .collect()
    }

    /// Score a single bullet.
    pub fn score_bullet(&self, bullet: &Bullet, query: Option<&str>) -> ScoredBullet {
        let mut breakdown = HashMap::new();

        let effectiveness = self.effectiveness_score(bullet);
        breakdown.insert("effectiveness".to_string(), effectiveness);

        let recency = self.recency_score(bullet);
        breakdown.insert("recency".to_string(), recency);

        let semantic = match query {
            Some(q) if self.weights.get("semantic").copied().unwrap_or(0.0) > 0.0 => {
                self.semantic_score(q, bullet)
            }
            _ => 0.0,
        };
        breakdown.insert("semantic".to_string(), semantic);

        let final_score = self.weights.get("effectiveness").unwrap_or(&0.6) * effectiveness
            + self.weights.get("recency").unwrap_or(&0.4) * recency
            + self.weights.get("semantic").unwrap_or(&0.0) * semantic;

        ScoredBullet {
            bullet: bullet.clone(),
            score: final_score,
            score_breakdown: breakdown,
        }
    }

    /// Effectiveness score based on helpful/harmful feedback.
    /// Returns 0.0..1.0. Untested bullets get 0.5.
    fn effectiveness_score(&self, bullet: &Bullet) -> f64 {
        let total = bullet.helpful + bullet.harmful + bullet.neutral;
        if total == 0 {
            return 0.5;
        }
        let weighted =
            bullet.helpful as f64 * 1.0 + bullet.neutral as f64 * 0.5 + bullet.harmful as f64 * 0.0;
        weighted / total as f64
    }

    /// Recency score -- prefer recently updated bullets.
    /// Returns 0.0..1.0 using exponential decay.
    fn recency_score(&self, bullet: &Bullet) -> f64 {
        let updated_at = bullet
            .updated_at
            .replace("Z", "+00:00")
            .parse::<chrono::DateTime<chrono::Utc>>()
            .or_else(|_| {
                chrono::DateTime::parse_from_rfc3339(&bullet.updated_at)
                    .map(|dt| dt.with_timezone(&chrono::Utc))
            });

        match updated_at {
            Ok(dt) => {
                let days_old = (chrono::Utc::now() - dt).num_days().max(0) as f64;
                let decay_rate = 0.1;
                1.0 / (1.0 + days_old * decay_rate)
            }
            Err(_) => 0.5,
        }
    }

    /// Semantic similarity score using cached embeddings.
    fn semantic_score(&self, query: &str, bullet: &Bullet) -> f64 {
        if self.weights.get("semantic").copied().unwrap_or(0.0) <= 0.0 {
            return 0.0;
        }

        let query_emb = self.embedding_cache.peek(query, None);
        let bullet_emb = self.embedding_cache.peek(&bullet.content, None);

        match (query_emb, bullet_emb) {
            (Some(q), Some(b)) => {
                let sim = cosine_similarity(q, b);
                (sim + 1.0) / 2.0 // Normalize from [-1, 1] to [0, 1]
            }
            _ => 0.5,
        }
    }

    /// Get statistics about a selection.
    pub fn selection_stats(
        &self,
        all_bullets: &[Bullet],
        selected: &[Bullet],
    ) -> HashMap<String, f64> {
        let all_scored: Vec<ScoredBullet> = all_bullets
            .iter()
            .map(|b| self.score_bullet(b, None))
            .collect();

        let selected_ids: std::collections::HashSet<&str> =
            selected.iter().map(|b| b.id.as_str()).collect();

        let avg_all = if all_scored.is_empty() {
            0.0
        } else {
            all_scored.iter().map(|s| s.score).sum::<f64>() / all_scored.len() as f64
        };

        let selected_scores: Vec<f64> = all_scored
            .iter()
            .filter(|s| selected_ids.contains(s.bullet.id.as_str()))
            .map(|s| s.score)
            .collect();

        let avg_selected = if selected_scores.is_empty() {
            0.0
        } else {
            selected_scores.iter().sum::<f64>() / selected_scores.len() as f64
        };

        let mut stats = HashMap::new();
        stats.insert("total_bullets".to_string(), all_bullets.len() as f64);
        stats.insert("selected_bullets".to_string(), selected.len() as f64);
        stats.insert(
            "selection_rate".to_string(),
            if all_bullets.is_empty() {
                0.0
            } else {
                selected.len() as f64 / all_bullets.len() as f64
            },
        );
        stats.insert("avg_all_score".to_string(), avg_all);
        stats.insert("avg_selected_score".to_string(), avg_selected);
        stats.insert("score_improvement".to_string(), avg_selected - avg_all);
        stats
    }
}

impl Default for BulletSelector {
    fn default() -> Self {
        Self::new(None, "text-embedding-3-small", None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_bullet(id: &str, section: &str, helpful: i64, harmful: i64) -> Bullet {
        Bullet {
            id: id.to_string(),
            section: section.to_string(),
            content: format!("Content of {id}"),
            helpful,
            harmful,
            neutral: 0,
            created_at: chrono::Utc::now().to_rfc3339(),
            updated_at: chrono::Utc::now().to_rfc3339(),
        }
    }

    fn make_old_bullet(id: &str, days_ago: i64) -> Bullet {
        let ts = (chrono::Utc::now() - chrono::Duration::days(days_ago)).to_rfc3339();
        Bullet {
            id: id.to_string(),
            section: "test".to_string(),
            content: format!("Content of {id}"),
            helpful: 0,
            harmful: 0,
            neutral: 0,
            created_at: ts.clone(),
            updated_at: ts,
        }
    }

    #[test]
    fn test_select_returns_all_when_under_limit() {
        let selector = BulletSelector::default();
        let bullets = vec![
            make_bullet("a", "test", 0, 0),
            make_bullet("b", "test", 0, 0),
        ];
        let selected = selector.select(&bullets, 5, None);
        assert_eq!(selected.len(), 2);
    }

    #[test]
    fn test_select_limits_to_max_count() {
        let selector = BulletSelector::default();
        let bullets: Vec<Bullet> = (0..10)
            .map(|i| make_bullet(&format!("b-{i}"), "test", i, 0))
            .collect();

        let selected = selector.select(&bullets, 3, None);
        assert_eq!(selected.len(), 3);
    }

    #[test]
    fn test_select_prefers_helpful_bullets() {
        let selector = BulletSelector::default();
        let bullets = vec![
            make_bullet("low", "test", 0, 5),      // harmful
            make_bullet("high", "test", 10, 0),    // very helpful
            make_bullet("mid", "test", 3, 3),      // mixed
            make_bullet("untested", "test", 0, 0), // neutral (0.5)
        ];

        let selected = selector.select(&bullets, 2, None);
        let ids: Vec<&str> = selected.iter().map(|b| b.id.as_str()).collect();
        assert!(ids.contains(&"high"));
    }

    #[test]
    fn test_effectiveness_score() {
        let selector = BulletSelector::default();

        // Untested bullet -> 0.5
        let untested = make_bullet("u", "t", 0, 0);
        assert!((selector.effectiveness_score(&untested) - 0.5).abs() < 1e-10);

        // All helpful -> 1.0
        let helpful = make_bullet("h", "t", 10, 0);
        assert!((selector.effectiveness_score(&helpful) - 1.0).abs() < 1e-10);

        // All harmful -> 0.0
        let harmful = make_bullet("x", "t", 0, 10);
        assert!((selector.effectiveness_score(&harmful) - 0.0).abs() < 1e-10);

        // Equal helpful/harmful -> 0.5
        let mixed = make_bullet("m", "t", 5, 5);
        assert!((selector.effectiveness_score(&mixed) - 0.5).abs() < 1e-10);
    }

    #[test]
    fn test_recency_score() {
        let selector = BulletSelector::default();

        // Recent bullet should score high
        let recent = make_old_bullet("r", 0);
        let recent_score = selector.recency_score(&recent);
        assert!(recent_score > 0.9);

        // Old bullet should score low
        let old = make_old_bullet("o", 30);
        let old_score = selector.recency_score(&old);
        assert!(old_score < 0.3);

        // Recent > Old
        assert!(recent_score > old_score);
    }

    #[test]
    fn test_recency_score_invalid_timestamp() {
        let selector = BulletSelector::default();
        let mut bullet = make_bullet("b", "t", 0, 0);
        bullet.updated_at = "not-a-date".to_string();
        assert!((selector.recency_score(&bullet) - 0.5).abs() < 1e-10);
    }

    #[test]
    fn test_selection_stats() {
        let selector = BulletSelector::default();
        let all = vec![
            make_bullet("a", "t", 10, 0),
            make_bullet("b", "t", 0, 10),
            make_bullet("c", "t", 5, 5),
        ];
        let selected = vec![all[0].clone()];

        let stats = selector.selection_stats(&all, &selected);
        assert_eq!(stats["total_bullets"], 3.0);
        assert_eq!(stats["selected_bullets"], 1.0);
        assert!(stats["score_improvement"] > 0.0);
    }

    #[test]
    fn test_score_bullet_breakdown() {
        let selector = BulletSelector::default();
        let bullet = make_bullet("b", "test", 5, 0);

        let scored = selector.score_bullet(&bullet, None);
        assert!(scored.score_breakdown.contains_key("effectiveness"));
        assert!(scored.score_breakdown.contains_key("recency"));
        assert!(scored.score_breakdown.contains_key("semantic"));
        assert_eq!(scored.score_breakdown["semantic"], 0.0);
    }
}
