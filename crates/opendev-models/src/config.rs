//! Configuration models.

use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Permission settings for a specific tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolPermission {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub always_allow: bool,
    #[serde(default)]
    pub deny_patterns: Vec<String>,
}

fn default_true() -> bool {
    true
}

impl Default for ToolPermission {
    fn default() -> Self {
        Self {
            enabled: true,
            always_allow: false,
            deny_patterns: Vec::new(),
        }
    }
}

impl ToolPermission {
    /// Check if a target (file path, command, etc.) is allowed.
    pub fn is_allowed(&self, target: &str) -> bool {
        if !self.enabled {
            return false;
        }
        if self.always_allow {
            return true;
        }
        !self.deny_patterns.iter().any(|pattern| {
            Regex::new(pattern)
                .map(|re| re.is_match(target))
                .unwrap_or(false)
        })
    }
}

/// Global permission configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionConfig {
    #[serde(default)]
    pub file_write: ToolPermission,
    #[serde(default)]
    pub file_read: ToolPermission,
    #[serde(default = "default_bash_permission")]
    pub bash: ToolPermission,
    #[serde(default)]
    pub git: ToolPermission,
    #[serde(default)]
    pub web_fetch: ToolPermission,
}

fn default_bash_permission() -> ToolPermission {
    ToolPermission {
        enabled: true,
        always_allow: false,
        deny_patterns: vec![
            "rm -rf /".to_string(),
            "sudo rm -rf /*".to_string(),
            "chmod -R 777 /*".to_string(),
        ],
    }
}

impl Default for PermissionConfig {
    fn default() -> Self {
        Self {
            file_write: ToolPermission::default(),
            file_read: ToolPermission::default(),
            bash: default_bash_permission(),
            git: ToolPermission::default(),
            web_fetch: ToolPermission::default(),
        }
    }
}

/// Auto mode configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutoModeConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_max_operations")]
    pub max_operations: u32,
    #[serde(default = "default_require_confirmation_after")]
    pub require_confirmation_after: u32,
    #[serde(default = "default_true")]
    pub dangerous_operations_require_approval: bool,
}

fn default_max_operations() -> u32 {
    10
}
fn default_require_confirmation_after() -> u32 {
    5
}

impl Default for AutoModeConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_operations: 10,
            require_confirmation_after: 5,
            dangerous_operations_require_approval: true,
        }
    }
}

/// Operation-specific settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationConfig {
    #[serde(default = "default_true")]
    pub show_diffs: bool,
    #[serde(default = "default_true")]
    pub backup_before_edit: bool,
    #[serde(default = "default_max_file_size")]
    pub max_file_size: u64,
    #[serde(default)]
    pub allowed_extensions: Vec<String>,
}

fn default_max_file_size() -> u64 {
    1_000_000
}

impl Default for OperationConfig {
    fn default() -> Self {
        Self {
            show_diffs: true,
            backup_before_edit: true,
            max_file_size: 1_000_000,
            allowed_extensions: Vec::new(),
        }
    }
}

/// Scoring weights for ACE playbook bullet selection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaybookScoringWeights {
    #[serde(default = "default_effectiveness")]
    pub effectiveness: f64,
    #[serde(default = "default_recency")]
    pub recency: f64,
    #[serde(default = "default_semantic")]
    pub semantic: f64,
}

fn default_effectiveness() -> f64 {
    0.5
}
fn default_recency() -> f64 {
    0.3
}
fn default_semantic() -> f64 {
    0.2
}

impl Default for PlaybookScoringWeights {
    fn default() -> Self {
        Self {
            effectiveness: 0.5,
            recency: 0.3,
            semantic: 0.2,
        }
    }
}

impl PlaybookScoringWeights {
    /// Validate that all weights are between 0.0 and 1.0.
    pub fn validate(&self) -> Result<(), String> {
        for (name, value) in [
            ("effectiveness", self.effectiveness),
            ("recency", self.recency),
            ("semantic", self.semantic),
        ] {
            if !(0.0..=1.0).contains(&value) {
                return Err(format!(
                    "{name} weight must be between 0.0 and 1.0, got {value}"
                ));
            }
        }
        Ok(())
    }
}

/// ACE playbook configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaybookConfig {
    #[serde(default = "default_max_strategies")]
    pub max_strategies: u32,
    #[serde(default = "default_true")]
    pub use_selection: bool,
    #[serde(default = "default_embedding_model")]
    pub embedding_model: String,
    #[serde(default = "default_embedding_provider")]
    pub embedding_provider: String,
    #[serde(default)]
    pub scoring_weights: PlaybookScoringWeights,
    #[serde(default = "default_true")]
    pub cache_embeddings: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_file: Option<String>,
}

fn default_max_strategies() -> u32 {
    30
}
fn default_embedding_model() -> String {
    "text-embedding-3-small".to_string()
}
fn default_embedding_provider() -> String {
    "openai".to_string()
}

impl Default for PlaybookConfig {
    fn default() -> Self {
        Self {
            max_strategies: 30,
            use_selection: true,
            embedding_model: "text-embedding-3-small".to_string(),
            embedding_provider: "openai".to_string(),
            scoring_weights: PlaybookScoringWeights::default(),
            cache_embeddings: true,
            cache_file: None,
        }
    }
}

/// A named model configuration variant.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelVariant {
    pub name: String,
    pub model: String,
    pub provider: String,
    #[serde(default = "default_temperature")]
    pub temperature: f64,
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,
    #[serde(default)]
    pub description: String,
}

fn default_temperature() -> f64 {
    0.6
}
fn default_max_tokens() -> u32 {
    16384
}

/// Inline agent configuration from opendev.json.
///
/// Allows defining new agents or overriding builtin agents directly
/// in the config file. All fields are optional — only specified fields
/// are applied as overrides.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AgentConfigInline {
    /// Model override (e.g. "gpt-4o", "claude-opus-4-5").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// System prompt override or addition.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    /// Description of when to use this agent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Temperature override.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    /// Top-p override.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f64>,
    /// Max iterations (steps) for the react loop.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_steps: Option<usize>,
    /// Agent mode: "primary", "subagent", or "all".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    /// Display color (hex string like "#FF6600").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// Hide from autocomplete listings.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hidden: Option<bool>,
    /// Disable/remove this agent entirely.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disable: Option<bool>,
    /// Per-tool permission rules (tool pattern → "allow"/"deny"/"ask").
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub permission: HashMap<String, String>,
}

/// Application configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    // AI Provider settings - Three model system
    #[serde(default = "default_model_provider")]
    pub model_provider: String,
    #[serde(default = "default_model")]
    pub model: String,

    // Thinking model
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_thinking: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_thinking_provider: Option<String>,

    // Vision/Multi-modal model
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_vlm: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_vlm_provider: Option<String>,

    // Critique model
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_critique: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_critique_provider: Option<String>,

    // Compact model
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_compact: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_compact_provider: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_base_url: Option<String>,
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,
    #[serde(default = "default_temperature")]
    pub temperature: f64,

    // Session settings
    #[serde(default = "default_auto_save_interval")]
    pub auto_save_interval: u32,
    #[serde(default = "default_max_context_tokens")]
    pub max_context_tokens: u64,

    // UI settings
    #[serde(default)]
    pub verbose: bool,
    #[serde(default)]
    pub debug_logging: bool,
    #[serde(default = "default_color_scheme")]
    pub color_scheme: String,
    #[serde(default = "default_true")]
    pub show_token_count: bool,
    #[serde(default = "default_true")]
    pub enable_sound: bool,

    // Permissions
    #[serde(default)]
    pub permissions: PermissionConfig,

    // Operation settings
    #[serde(default = "default_true")]
    pub enable_bash: bool,
    #[serde(default = "default_bash_timeout")]
    pub bash_timeout: u32,
    #[serde(default)]
    pub auto_mode: AutoModeConfig,
    #[serde(default)]
    pub operation: OperationConfig,
    #[serde(default = "default_max_undo_history")]
    pub max_undo_history: u32,

    // Session intelligence
    #[serde(default = "default_true")]
    pub topic_detection: bool,

    // ACE Playbook settings
    #[serde(default)]
    pub playbook: PlaybookConfig,

    // Plan mode configuration
    #[serde(default = "default_plan_mode_workflow")]
    pub plan_mode_workflow: String,
    #[serde(default = "default_plan_mode_explore_agent_count")]
    pub plan_mode_explore_agent_count: u32,
    #[serde(default = "default_plan_mode_plan_agent_count")]
    pub plan_mode_plan_agent_count: u32,
    #[serde(default = "default_plan_mode_explore_variant")]
    pub plan_mode_explore_variant: String,

    // Custom instructions — file paths, glob patterns, or `~/` paths
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub instructions: Vec<String>,

    // Additional skill directories — file paths or `~/` paths
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skill_paths: Vec<String>,

    // Remote URLs to discover skills from (fetches index.json)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skill_urls: Vec<String>,

    // Default agent to use for new sessions (e.g. "general", "explore")
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_agent: Option<String>,

    // Inline agent definitions/overrides from config.
    // Keys are agent identifiers (e.g. "build", "explore", or custom names).
    // Overrides merge onto builtin agents; new keys create custom agents.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub agents: HashMap<String, AgentConfigInline>,

    // Model variants
    #[serde(default)]
    pub model_variants: HashMap<String, ModelVariant>,

    // Config version for migration support
    #[serde(default = "default_config_version")]
    pub config_version: u32,
}

fn default_config_version() -> u32 {
    1
}
fn default_model_provider() -> String {
    "fireworks".to_string()
}
fn default_model() -> String {
    "accounts/fireworks/models/kimi-k2-instruct-0905".to_string()
}
fn default_auto_save_interval() -> u32 {
    5
}
fn default_max_context_tokens() -> u64 {
    100_000
}
fn default_color_scheme() -> String {
    "monokai".to_string()
}
fn default_bash_timeout() -> u32 {
    30
}
fn default_max_undo_history() -> u32 {
    50
}
fn default_plan_mode_workflow() -> String {
    "5-phase".to_string()
}
fn default_plan_mode_explore_agent_count() -> u32 {
    3
}
fn default_plan_mode_plan_agent_count() -> u32 {
    1
}
fn default_plan_mode_explore_variant() -> String {
    "enabled".to_string()
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            model_provider: default_model_provider(),
            model: default_model(),
            model_thinking: None,
            model_thinking_provider: None,
            model_vlm: None,
            model_vlm_provider: None,
            model_critique: None,
            model_critique_provider: None,
            model_compact: None,
            model_compact_provider: None,
            api_key: None,
            api_base_url: None,
            max_tokens: 16384,
            temperature: 0.6,
            auto_save_interval: 5,
            max_context_tokens: 100_000,
            verbose: false,
            debug_logging: false,
            color_scheme: "monokai".to_string(),
            show_token_count: true,
            enable_sound: true,
            permissions: PermissionConfig::default(),
            enable_bash: true,
            bash_timeout: 30,
            auto_mode: AutoModeConfig::default(),
            operation: OperationConfig::default(),
            max_undo_history: 50,
            topic_detection: true,
            playbook: PlaybookConfig::default(),
            plan_mode_workflow: "5-phase".to_string(),
            plan_mode_explore_agent_count: 3,
            plan_mode_plan_agent_count: 1,
            plan_mode_explore_variant: "enabled".to_string(),
            instructions: Vec::new(),
            skill_paths: Vec::new(),
            skill_urls: Vec::new(),
            default_agent: None,
            agents: HashMap::new(),
            model_variants: HashMap::new(),
            config_version: default_config_version(),
        }
    }
}

impl AppConfig {
    /// Get the API key from config or the environment.
    pub fn get_api_key(&self) -> Result<String, String> {
        if let Some(ref key) = self.api_key {
            return Ok(key.clone());
        }

        let env_var = match self.model_provider.as_str() {
            "fireworks" => "FIREWORKS_API_KEY",
            "anthropic" => "ANTHROPIC_API_KEY",
            "openai" => "OPENAI_API_KEY",
            "azure" => "AZURE_OPENAI_API_KEY",
            "groq" => "GROQ_API_KEY",
            "mistral" => "MISTRAL_API_KEY",
            "deepinfra" => "DEEPINFRA_API_KEY",
            "openrouter" => "OPENROUTER_API_KEY",
            _ => "OPENAI_API_KEY",
        };

        std::env::var(env_var)
            .map_err(|_| format!("No API key found. Set {} environment variable", env_var))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = AppConfig::default();
        assert_eq!(config.model_provider, "fireworks");
        assert_eq!(config.temperature, 0.6);
        assert_eq!(config.max_tokens, 16384);
        assert!(config.enable_bash);
    }

    #[test]
    fn test_config_roundtrip() {
        let config = AppConfig::default();
        let json = serde_json::to_string(&config).unwrap();
        let deserialized: AppConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.model_provider, config.model_provider);
        assert_eq!(deserialized.model, config.model);
    }

    #[test]
    fn test_tool_permission_is_allowed() {
        let perm = ToolPermission {
            enabled: true,
            always_allow: false,
            deny_patterns: vec!["rm -rf /".to_string()],
        };
        assert!(perm.is_allowed("ls -la"));
        assert!(!perm.is_allowed("rm -rf /"));

        let disabled = ToolPermission {
            enabled: false,
            ..Default::default()
        };
        assert!(!disabled.is_allowed("anything"));

        let allow_all = ToolPermission {
            enabled: true,
            always_allow: true,
            deny_patterns: vec![".*".to_string()],
        };
        assert!(allow_all.is_allowed("anything"));
    }

    #[test]
    fn test_scoring_weights_validation() {
        let valid = PlaybookScoringWeights::default();
        assert!(valid.validate().is_ok());

        let invalid = PlaybookScoringWeights {
            effectiveness: 1.5,
            ..Default::default()
        };
        assert!(invalid.validate().is_err());
    }

    #[test]
    fn test_partial_config_deserialization() {
        // Should fill in defaults for missing fields
        let json = r#"{"model_provider": "openai", "model": "gpt-4"}"#;
        let config: AppConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.model_provider, "openai");
        assert_eq!(config.model, "gpt-4");
        assert_eq!(config.temperature, 0.6); // default
        assert!(config.enable_bash); // default
    }
}
