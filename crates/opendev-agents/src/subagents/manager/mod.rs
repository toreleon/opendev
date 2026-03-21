//! Subagent manager for registering and executing subagents.
//!
//! Manages a collection of subagent specifications and provides
//! lookup by name or type. Also provides the execution entry point
//! for spawning subagents with isolated ReAct loops.

mod scanning;
mod spawn;
pub mod types;

pub use types::{
    NoopProgressCallback, SubagentEventBridge, SubagentProgressCallback, SubagentRunResult,
    SubagentType,
};

use std::collections::HashMap;

use tracing::info;

use super::spec::SubAgentSpec;

/// Manages subagent registration, lookup, and execution.
#[derive(Debug, Default)]
pub struct SubagentManager {
    specs: HashMap<String, SubAgentSpec>,
}

impl SubagentManager {
    /// Create a new empty manager.
    pub fn new() -> Self {
        Self {
            specs: HashMap::new(),
        }
    }

    /// Create a manager pre-loaded with core built-in subagent specs.
    ///
    /// Registers only the essential subagents (Explore, Planner, project_init).
    /// Additional subagents can be loaded as custom agents from `~/.opendev/agents/*.md`.
    pub fn with_builtins() -> Self {
        use super::spec::builtins;
        use crate::prompts::embedded;

        let mut mgr = Self::new();
        mgr.register(builtins::code_explorer(
            embedded::SUBAGENTS_SUBAGENT_CODE_EXPLORER,
        ));
        mgr.register(builtins::planner(embedded::SUBAGENTS_SUBAGENT_PLANNER));
        mgr.register(builtins::general(
            "You are a versatile coding assistant. Complete the task using all tools available to you. \
             Read files, search code, edit files, run commands, and use web tools as needed. \
             Be thorough and methodical.",
        ));
        mgr.register(builtins::build(
            "You are a build and test runner. Your job is to run builds, analyze errors, \
             fix compilation failures, and ensure tests pass. Focus on the build output \
             and fix issues systematically.",
        ));
        mgr.register(builtins::project_init(
            embedded::SUBAGENTS_SUBAGENT_PROJECT_INIT,
        ));
        mgr
    }

    /// Create a manager with built-in specs plus custom agents loaded from disk.
    ///
    /// Scans agent directories from lowest to highest priority:
    /// 1. Global: `~/.opendev/agents/`
    /// 2. Walk from git root down to working_dir: `.opendev/agents/`
    ///    at each level (monorepo support)
    ///
    /// Custom agents override built-ins with the same name.
    pub fn with_builtins_and_custom(working_dir: &std::path::Path) -> Self {
        let mut mgr = Self::with_builtins();
        let home = dirs::home_dir().unwrap_or_default();

        // Determine git root for monorepo walking
        let git_root = std::process::Command::new("git")
            .args(["rev-parse", "--show-toplevel"])
            .current_dir(working_dir)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .and_then(|o| {
                String::from_utf8(o.stdout)
                    .ok()
                    .map(|s| std::path::PathBuf::from(s.trim()))
            });
        let stop_dir = git_root.as_deref().unwrap_or(working_dir);

        // Order: lowest priority first (later register() calls override earlier).
        let mut dirs = Vec::new();

        // 1. Global dir
        dirs.push(home.join(".opendev").join("agents"));

        // 2. Walk from working_dir up to git root, collect directory levels
        //    Parent dirs have lower priority than child dirs.
        let mut levels: Vec<std::path::PathBuf> = Vec::new();
        {
            let mut current = working_dir.to_path_buf();
            loop {
                levels.push(current.clone());
                if current == stop_dir || !current.pop() {
                    break;
                }
            }
        }
        // Reverse so parent dirs (lower priority) load first, working_dir loads last
        levels.reverse();
        for level in &levels {
            dirs.push(level.join(".opendev").join("agents"));
        }

        for spec in super::custom_loader::load_custom_agents(&dirs) {
            mgr.register(spec);
        }
        mgr
    }

    /// Apply inline agent config overrides from `opendev.json`.
    ///
    /// For each entry in the config map:
    /// - If `disable: true`, removes the agent entirely
    /// - If the agent exists, merges the overrides onto it
    /// - If the agent doesn't exist, creates a new custom agent
    pub fn apply_config_overrides(
        &mut self,
        overrides: &std::collections::HashMap<String, opendev_models::AgentConfigInline>,
    ) {
        use super::spec::PermissionAction;

        for (name, cfg) in overrides {
            // Handle disable
            if cfg.disable == Some(true) {
                if self.specs.remove(name).is_some() {
                    info!(agent = name, "Disabled agent via config override");
                }
                continue;
            }

            let spec = self.specs.entry(name.clone()).or_insert_with(|| {
                info!(agent = name, "Creating new agent from config");
                SubAgentSpec::new(
                    name,
                    cfg.description.as_deref().unwrap_or("Custom agent"),
                    cfg.prompt
                        .as_deref()
                        .unwrap_or("You are a helpful assistant."),
                )
            });

            // Apply overrides
            if let Some(ref model) = cfg.model {
                spec.model = Some(model.clone());
            }
            if let Some(ref prompt) = cfg.prompt {
                spec.system_prompt = prompt.clone();
            }
            if let Some(ref desc) = cfg.description {
                spec.description = desc.clone();
            }
            if let Some(temp) = cfg.temperature {
                spec.temperature = Some(temp as f32);
            }
            if let Some(top_p) = cfg.top_p {
                spec.top_p = Some(top_p as f32);
            }
            if let Some(steps) = cfg.max_steps {
                spec.max_steps = Some(steps as u32);
            }
            if let Some(ref color) = cfg.color {
                spec.color = Some(color.clone());
            }
            if let Some(hidden) = cfg.hidden {
                spec.hidden = hidden;
            }
            if let Some(ref mode) = cfg.mode {
                spec.mode = super::spec::AgentMode::parse_mode(mode);
            }

            // Merge permissions (config overrides existing)
            for (tool_pattern, action_str) in &cfg.permission {
                let action = match action_str.as_str() {
                    "allow" => PermissionAction::Allow,
                    "deny" => PermissionAction::Deny,
                    "ask" => PermissionAction::Ask,
                    _ => continue,
                };
                spec.permission.insert(
                    tool_pattern.clone(),
                    super::spec::PermissionRule::Action(action),
                );
            }
        }
    }

    /// Register a subagent specification.
    pub fn register(&mut self, spec: SubAgentSpec) {
        self.specs.insert(spec.name.clone(), spec);
    }

    /// Get a subagent spec by name.
    pub fn get(&self, name: &str) -> Option<&SubAgentSpec> {
        self.specs.get(name)
    }

    /// Get a subagent spec by type.
    pub fn get_by_type(&self, subagent_type: SubagentType) -> Option<&SubAgentSpec> {
        self.specs.get(subagent_type.canonical_name())
    }

    /// List all registered subagent names (excludes hidden and disabled agents).
    pub fn names(&self) -> Vec<&str> {
        self.specs
            .values()
            .filter(|s| !s.hidden && !s.disable)
            .map(|s| s.name.as_str())
            .collect()
    }

    /// List all registered subagent names including hidden ones.
    pub fn all_names(&self) -> Vec<&str> {
        self.specs.keys().map(|s| s.as_str()).collect()
    }

    /// Get the number of registered subagents.
    pub fn len(&self) -> usize {
        self.specs.len()
    }

    /// Check if the manager is empty.
    pub fn is_empty(&self) -> bool {
        self.specs.is_empty()
    }

    /// Unregister a subagent by name.
    pub fn unregister(&mut self, name: &str) -> Option<SubAgentSpec> {
        self.specs.remove(name)
    }

    /// Resolve the default agent name for new sessions.
    ///
    /// If `configured_default` is `Some`, validates that the agent exists,
    /// is not hidden, and can be used as a primary agent. Falls back to
    /// the first non-hidden primary-capable agent, or `None` if no suitable
    /// agent is found.
    pub fn resolve_default_agent(&self, configured_default: Option<&str>) -> Option<&str> {
        if let Some(name) = configured_default {
            if let Some(spec) = self.specs.get(name) {
                if spec.disable {
                    tracing::warn!(agent = name, "default_agent is disabled, falling back");
                } else if spec.hidden {
                    tracing::warn!(agent = name, "default_agent is hidden, falling back");
                } else if !spec.mode.can_be_primary() {
                    tracing::warn!(agent = name, "default_agent is subagent-only, falling back");
                } else {
                    return Some(&spec.name);
                }
            } else {
                tracing::warn!(agent = name, "default_agent not found, falling back");
            }
        }

        // Fallback: first non-hidden, non-disabled, primary-capable agent
        self.specs
            .values()
            .find(|s| !s.hidden && !s.disable && s.mode.can_be_primary())
            .map(|s| s.name.as_str())
    }

    /// Build tool schemas description listing available subagents.
    ///
    /// Used to populate the `subagent_type` enum in the `spawn_subagent` tool schema.
    /// Excludes hidden and disabled agents.
    pub fn build_enum_description(&self) -> Vec<(String, String)> {
        self.specs
            .values()
            .filter(|s| !s.hidden && !s.disable)
            .map(|s| (s.name.clone(), s.description.clone()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_spec(name: &str) -> SubAgentSpec {
        SubAgentSpec::new(name, format!("Description of {name}"), "system prompt")
    }

    #[test]
    fn test_manager_new_empty() {
        let mgr = SubagentManager::new();
        assert!(mgr.is_empty());
        assert_eq!(mgr.len(), 0);
    }

    #[test]
    fn test_register_and_get() {
        let mut mgr = SubagentManager::new();
        mgr.register(make_spec("Explore"));
        assert_eq!(mgr.len(), 1);
        assert!(mgr.get("Explore").is_some());
        assert!(mgr.get("nonexistent").is_none());
    }

    #[test]
    fn test_get_by_type() {
        let mut mgr = SubagentManager::new();
        mgr.register(make_spec("Explore"));
        assert!(mgr.get_by_type(SubagentType::CodeExplorer).is_some());
        assert!(mgr.get_by_type(SubagentType::Planner).is_none());
    }

    #[test]
    fn test_unregister() {
        let mut mgr = SubagentManager::new();
        mgr.register(make_spec("Planner"));
        assert!(mgr.unregister("Planner").is_some());
        assert!(mgr.is_empty());
    }

    #[test]
    fn test_names() {
        let mut mgr = SubagentManager::new();
        mgr.register(make_spec("A"));
        mgr.register(make_spec("B"));
        let names = mgr.names();
        assert!(names.contains(&"A"));
        assert!(names.contains(&"B"));
    }

    #[test]
    fn test_build_enum_description() {
        let mut mgr = SubagentManager::new();
        mgr.register(make_spec("Explore"));
        let descs = mgr.build_enum_description();
        assert_eq!(descs.len(), 1);
        assert_eq!(descs[0].0, "Explore");
    }

    #[test]
    fn test_with_builtins() {
        let mgr = SubagentManager::with_builtins();
        assert_eq!(mgr.len(), 5);
        assert!(mgr.get("Explore").is_some());
        assert!(mgr.get("Planner").is_some());
        assert!(mgr.get("General").is_some());
        assert!(mgr.get("Build").is_some());
        assert!(mgr.get("project_init").is_some());
    }

    #[test]
    fn test_with_builtins_and_custom() {
        let tmp = tempfile::tempdir().unwrap();
        let agent_dir = tmp.path().join(".opendev").join("agents");
        std::fs::create_dir_all(&agent_dir).unwrap();
        std::fs::write(
            agent_dir.join("test-agent.md"),
            "---\ndescription: Test agent\n---\nYou are a test.",
        )
        .unwrap();

        let mgr = SubagentManager::with_builtins_and_custom(tmp.path());
        assert!(mgr.len() >= 4); // 3 builtins + 1 custom
        assert!(mgr.get("test-agent").is_some());
    }

    #[test]
    fn test_dot_agents_directory_not_loaded() {
        let tmp = tempfile::tempdir().unwrap();
        let agent_dir = tmp.path().join(".agents").join("agents");
        std::fs::create_dir_all(&agent_dir).unwrap();
        std::fs::write(
            agent_dir.join("reviewer.md"),
            "---\ndescription: Code reviewer from .agents\n---\nYou review code.",
        )
        .unwrap();

        let mgr = SubagentManager::with_builtins_and_custom(tmp.path());
        // .agents/ agents should not be loaded
        assert!(mgr.get("reviewer").is_none());
    }

    #[test]
    fn test_only_opendev_agents_dir_loaded() {
        let tmp = tempfile::tempdir().unwrap();

        // Create agents in .opendev, .agents, and .claude dirs
        let opendev_dir = tmp.path().join(".opendev").join("agents");
        let agents_dir = tmp.path().join(".agents").join("agents");
        let claude_dir = tmp.path().join(".claude").join("agents");
        std::fs::create_dir_all(&opendev_dir).unwrap();
        std::fs::create_dir_all(&agents_dir).unwrap();
        std::fs::create_dir_all(&claude_dir).unwrap();

        std::fs::write(
            opendev_dir.join("shared.md"),
            "---\ndescription: From .opendev\n---\nOpenDev version.",
        )
        .unwrap();
        std::fs::write(
            agents_dir.join("shared.md"),
            "---\ndescription: From .agents\n---\nAgents version.",
        )
        .unwrap();
        std::fs::write(
            claude_dir.join("only-claude.md"),
            "---\ndescription: From .claude\n---\nClaude version.",
        )
        .unwrap();

        let mgr = SubagentManager::with_builtins_and_custom(tmp.path());
        let spec = mgr.get("shared").unwrap();
        // Only .opendev is loaded
        assert_eq!(spec.description, "From .opendev");
        // .claude and .agents agents should not be loaded
        assert!(mgr.get("only-claude").is_none());
    }

    #[test]
    fn test_hidden_agents_excluded_from_names() {
        let mut mgr = SubagentManager::new();
        mgr.register(make_spec("visible"));
        mgr.register(SubAgentSpec::new("hidden-agent", "Hidden", "prompt").with_hidden(true));

        let names = mgr.names();
        assert!(names.contains(&"visible"));
        assert!(!names.contains(&"hidden-agent"));

        // all_names includes hidden
        let all = mgr.all_names();
        assert!(all.contains(&"visible"));
        assert!(all.contains(&"hidden-agent"));
    }

    #[test]
    fn test_hidden_agents_excluded_from_enum_description() {
        let mut mgr = SubagentManager::new();
        mgr.register(make_spec("visible"));
        mgr.register(SubAgentSpec::new("hidden-agent", "Hidden", "prompt").with_hidden(true));

        let descs = mgr.build_enum_description();
        assert_eq!(descs.len(), 1);
        assert_eq!(descs[0].0, "visible");
    }

    #[test]
    fn test_hidden_agents_still_gettable() {
        let mut mgr = SubagentManager::new();
        mgr.register(SubAgentSpec::new("hidden-agent", "Hidden", "prompt").with_hidden(true));

        // Hidden agents can still be retrieved by name (for programmatic spawning)
        assert!(mgr.get("hidden-agent").is_some());
    }

    #[test]
    fn test_custom_agent_with_steps_and_temperature() {
        let tmp = tempfile::tempdir().unwrap();
        let agent_dir = tmp.path().join(".opendev").join("agents");
        std::fs::create_dir_all(&agent_dir).unwrap();
        std::fs::write(
            agent_dir.join("custom.md"),
            "---\ndescription: Custom\nsteps: 50\ntemperature: 0.3\nhidden: true\n---\nYou are custom.",
        )
        .unwrap();

        let mgr = SubagentManager::with_builtins_and_custom(tmp.path());
        let spec = mgr.get("custom").unwrap();
        assert_eq!(spec.max_steps, Some(50));
        assert_eq!(spec.temperature, Some(0.3));
        assert!(spec.hidden);

        // Hidden agent excluded from names
        assert!(!mgr.names().contains(&"custom"));
    }

    #[test]
    fn test_custom_agent_overrides_builtin() {
        let tmp = tempfile::tempdir().unwrap();
        let agent_dir = tmp.path().join(".opendev").join("agents");
        std::fs::create_dir_all(&agent_dir).unwrap();
        // Create a custom agent with same name as a builtin
        std::fs::write(
            agent_dir.join("Explore.md"),
            "---\ndescription: Custom explorer\ntemperature: 0.1\n---\nCustom explorer prompt.",
        )
        .unwrap();

        let mgr = SubagentManager::with_builtins_and_custom(tmp.path());
        let spec = mgr.get("Explore").unwrap();
        // Custom should override the builtin
        assert!(spec.system_prompt.contains("Custom explorer prompt"));
        assert_eq!(spec.temperature, Some(0.1));
    }

    #[test]
    fn test_claude_agents_dir_not_loaded() {
        let tmp = tempfile::tempdir().unwrap();
        let claude_dir = tmp.path().join(".claude").join("agents");
        std::fs::create_dir_all(&claude_dir).unwrap();
        std::fs::write(
            claude_dir.join("claude-agent.md"),
            "---\ndescription: Claude agent\n---\nClaude agent prompt.",
        )
        .unwrap();

        let mgr = SubagentManager::with_builtins_and_custom(tmp.path());
        // .claude/ agents should not be loaded
        assert!(mgr.get("claude-agent").is_none());
    }

    // ---- Disabled agent filtering ----

    #[test]
    fn test_disabled_agents_excluded_from_names() {
        let mut mgr = SubagentManager::new();
        mgr.register(make_spec("active"));
        mgr.register(SubAgentSpec::new("disabled-agent", "Disabled", "prompt").with_disable(true));

        let names = mgr.names();
        assert!(names.contains(&"active"));
        assert!(
            !names.contains(&"disabled-agent"),
            "Disabled agents should be excluded from names()"
        );
    }

    #[test]
    fn test_disabled_agents_excluded_from_enum_description() {
        let mut mgr = SubagentManager::new();
        mgr.register(make_spec("active"));
        mgr.register(SubAgentSpec::new("disabled-agent", "Disabled", "prompt").with_disable(true));

        let descs = mgr.build_enum_description();
        assert_eq!(descs.len(), 1);
        assert_eq!(descs[0].0, "active");
    }

    #[test]
    fn test_disabled_agents_still_gettable() {
        let mut mgr = SubagentManager::new();
        mgr.register(SubAgentSpec::new("disabled-agent", "Disabled", "prompt").with_disable(true));

        // Disabled agents can still be looked up (but spawn will fail)
        assert!(mgr.get("disabled-agent").is_some());
    }

    #[test]
    fn test_disabled_agents_in_all_names() {
        let mut mgr = SubagentManager::new();
        mgr.register(SubAgentSpec::new("disabled-agent", "Disabled", "prompt").with_disable(true));

        let all = mgr.all_names();
        assert!(
            all.contains(&"disabled-agent"),
            "all_names() should include disabled agents"
        );
    }

    // ---- apply_config_overrides tests ----

    #[test]
    fn test_config_override_model() {
        let mut mgr = SubagentManager::new();
        mgr.register(SubAgentSpec::new("build", "Build agent", "Build things."));

        let mut overrides = std::collections::HashMap::new();
        overrides.insert(
            "build".to_string(),
            opendev_models::AgentConfigInline {
                model: Some("gpt-4o-mini".to_string()),
                ..Default::default()
            },
        );
        mgr.apply_config_overrides(&overrides);

        let spec = mgr.get("build").unwrap();
        assert_eq!(spec.model.as_deref(), Some("gpt-4o-mini"));
    }

    #[test]
    fn test_config_override_temperature_and_top_p() {
        let mut mgr = SubagentManager::new();
        mgr.register(SubAgentSpec::new("test", "Test", "prompt"));

        let mut overrides = std::collections::HashMap::new();
        overrides.insert(
            "test".to_string(),
            opendev_models::AgentConfigInline {
                temperature: Some(0.7),
                top_p: Some(0.9),
                ..Default::default()
            },
        );
        mgr.apply_config_overrides(&overrides);

        let spec = mgr.get("test").unwrap();
        assert!((spec.temperature.unwrap() - 0.7).abs() < 0.01);
        assert!((spec.top_p.unwrap() - 0.9).abs() < 0.01);
    }

    #[test]
    fn test_config_override_disable_removes_agent() {
        let mut mgr = SubagentManager::new();
        mgr.register(SubAgentSpec::new("build", "Build agent", "prompt"));
        assert!(mgr.get("build").is_some());

        let mut overrides = std::collections::HashMap::new();
        overrides.insert(
            "build".to_string(),
            opendev_models::AgentConfigInline {
                disable: Some(true),
                ..Default::default()
            },
        );
        mgr.apply_config_overrides(&overrides);

        assert!(
            mgr.get("build").is_none(),
            "Disabled agent should be removed"
        );
    }

    #[test]
    fn test_config_override_creates_new_agent() {
        let mut mgr = SubagentManager::new();
        assert!(mgr.get("custom-agent").is_none());

        let mut overrides = std::collections::HashMap::new();
        overrides.insert(
            "custom-agent".to_string(),
            opendev_models::AgentConfigInline {
                description: Some("My custom agent".to_string()),
                prompt: Some("Be creative.".to_string()),
                model: Some("claude-opus-4-5".to_string()),
                ..Default::default()
            },
        );
        mgr.apply_config_overrides(&overrides);

        let spec = mgr.get("custom-agent").unwrap();
        assert_eq!(spec.description, "My custom agent");
        assert_eq!(spec.system_prompt, "Be creative.");
        assert_eq!(spec.model.as_deref(), Some("claude-opus-4-5"));
    }

    #[test]
    fn test_config_override_prompt_and_description() {
        let mut mgr = SubagentManager::new();
        mgr.register(SubAgentSpec::new("build", "Old desc", "Old prompt"));

        let mut overrides = std::collections::HashMap::new();
        overrides.insert(
            "build".to_string(),
            opendev_models::AgentConfigInline {
                description: Some("New description".to_string()),
                prompt: Some("New system prompt".to_string()),
                ..Default::default()
            },
        );
        mgr.apply_config_overrides(&overrides);

        let spec = mgr.get("build").unwrap();
        assert_eq!(spec.description, "New description");
        assert_eq!(spec.system_prompt, "New system prompt");
    }

    #[test]
    fn test_config_override_max_steps_and_color() {
        let mut mgr = SubagentManager::new();
        mgr.register(SubAgentSpec::new("build", "Build", "prompt"));

        let mut overrides = std::collections::HashMap::new();
        overrides.insert(
            "build".to_string(),
            opendev_models::AgentConfigInline {
                max_steps: Some(50),
                color: Some("#FF6600".to_string()),
                ..Default::default()
            },
        );
        mgr.apply_config_overrides(&overrides);

        let spec = mgr.get("build").unwrap();
        assert_eq!(spec.max_steps, Some(50));
        assert_eq!(spec.color.as_deref(), Some("#FF6600"));
    }

    #[test]
    fn test_config_override_hidden() {
        let mut mgr = SubagentManager::new();
        mgr.register(SubAgentSpec::new("build", "Build", "prompt"));

        let mut overrides = std::collections::HashMap::new();
        overrides.insert(
            "build".to_string(),
            opendev_models::AgentConfigInline {
                hidden: Some(true),
                ..Default::default()
            },
        );
        mgr.apply_config_overrides(&overrides);

        let spec = mgr.get("build").unwrap();
        assert!(spec.hidden);
        assert!(!mgr.names().contains(&"build"));
    }

    #[test]
    fn test_config_override_mode() {
        let mut mgr = SubagentManager::new();
        mgr.register(SubAgentSpec::new("build", "Build", "prompt"));

        let mut overrides = std::collections::HashMap::new();
        overrides.insert(
            "build".to_string(),
            opendev_models::AgentConfigInline {
                mode: Some("primary".to_string()),
                ..Default::default()
            },
        );
        mgr.apply_config_overrides(&overrides);

        let spec = mgr.get("build").unwrap();
        assert!(spec.mode.can_be_primary());
    }

    #[test]
    fn test_config_override_permission_rules() {
        let mut mgr = SubagentManager::new();
        mgr.register(SubAgentSpec::new("build", "Build", "prompt"));

        let mut perms = std::collections::HashMap::new();
        perms.insert("bash".to_string(), "deny".to_string());
        perms.insert("edit".to_string(), "allow".to_string());

        let mut overrides = std::collections::HashMap::new();
        overrides.insert(
            "build".to_string(),
            opendev_models::AgentConfigInline {
                permission: perms,
                ..Default::default()
            },
        );
        mgr.apply_config_overrides(&overrides);

        let spec = mgr.get("build").unwrap();
        // Check bash is denied
        let bash_action = spec.evaluate_permission("bash", "");
        assert_eq!(bash_action, Some(crate::subagents::PermissionAction::Deny));
        // Check edit is allowed
        let edit_action = spec.evaluate_permission("edit", "");
        assert_eq!(edit_action, Some(crate::subagents::PermissionAction::Allow));
    }

    #[test]
    fn test_config_override_invalid_permission_action_skipped() {
        let mut mgr = SubagentManager::new();
        mgr.register(SubAgentSpec::new("build", "Build", "prompt"));

        let mut perms = std::collections::HashMap::new();
        perms.insert("bash".to_string(), "invalid_action".to_string());

        let mut overrides = std::collections::HashMap::new();
        overrides.insert(
            "build".to_string(),
            opendev_models::AgentConfigInline {
                permission: perms,
                ..Default::default()
            },
        );
        mgr.apply_config_overrides(&overrides);

        let spec = mgr.get("build").unwrap();
        // Invalid action should be skipped, so no permission rule for bash
        assert_eq!(spec.evaluate_permission("bash", ""), None);
    }

    #[test]
    fn test_config_override_multiple_agents() {
        let mut mgr = SubagentManager::new();
        mgr.register(SubAgentSpec::new("build", "Build", "prompt1"));
        mgr.register(SubAgentSpec::new("explore", "Explore", "prompt2"));

        let mut overrides = std::collections::HashMap::new();
        overrides.insert(
            "build".to_string(),
            opendev_models::AgentConfigInline {
                model: Some("gpt-4o".to_string()),
                ..Default::default()
            },
        );
        overrides.insert(
            "explore".to_string(),
            opendev_models::AgentConfigInline {
                temperature: Some(0.2),
                ..Default::default()
            },
        );
        mgr.apply_config_overrides(&overrides);

        assert_eq!(mgr.get("build").unwrap().model.as_deref(), Some("gpt-4o"));
        assert!((mgr.get("explore").unwrap().temperature.unwrap() - 0.2).abs() < 0.01);
    }

    // ---- resolve_default_agent tests ----

    #[test]
    fn test_resolve_default_agent_configured() {
        let mut mgr = SubagentManager::new();
        mgr.register(
            SubAgentSpec::new("build", "Build", "prompt")
                .with_mode(crate::subagents::AgentMode::All),
        );

        let result = mgr.resolve_default_agent(Some("build"));
        assert_eq!(result, Some("build"));
    }

    #[test]
    fn test_resolve_default_agent_not_found_falls_back() {
        let mut mgr = SubagentManager::new();
        mgr.register(
            SubAgentSpec::new("build", "Build", "prompt")
                .with_mode(crate::subagents::AgentMode::All),
        );

        let result = mgr.resolve_default_agent(Some("nonexistent"));
        assert_eq!(
            result,
            Some("build"),
            "Should fall back to first primary-capable agent"
        );
    }

    #[test]
    fn test_resolve_default_agent_disabled_falls_back() {
        let mut mgr = SubagentManager::new();
        mgr.register(
            SubAgentSpec::new("build", "Build", "prompt")
                .with_mode(crate::subagents::AgentMode::All)
                .with_disable(true),
        );
        mgr.register(
            SubAgentSpec::new("general", "General", "prompt")
                .with_mode(crate::subagents::AgentMode::Primary),
        );

        let result = mgr.resolve_default_agent(Some("build"));
        assert_eq!(
            result,
            Some("general"),
            "Should skip disabled and fall back"
        );
    }

    #[test]
    fn test_resolve_default_agent_hidden_falls_back() {
        let mut mgr = SubagentManager::new();
        let mut hidden = SubAgentSpec::new("hidden-agent", "Hidden", "prompt");
        hidden.hidden = true;
        hidden.mode = crate::subagents::AgentMode::All;
        mgr.register(hidden);
        mgr.register(
            SubAgentSpec::new("visible", "Visible", "prompt")
                .with_mode(crate::subagents::AgentMode::Primary),
        );

        let result = mgr.resolve_default_agent(Some("hidden-agent"));
        assert_eq!(result, Some("visible"), "Should skip hidden and fall back");
    }

    #[test]
    fn test_resolve_default_agent_subagent_only_falls_back() {
        let mut mgr = SubagentManager::new();
        mgr.register(SubAgentSpec::new("helper", "Helper", "prompt"));
        // Default mode is Subagent, which can't be primary
        mgr.register(
            SubAgentSpec::new("primary", "Primary", "prompt")
                .with_mode(crate::subagents::AgentMode::Primary),
        );

        let result = mgr.resolve_default_agent(Some("helper"));
        assert_eq!(
            result,
            Some("primary"),
            "Should skip subagent-only and fall back"
        );
    }

    #[test]
    fn test_resolve_default_agent_none_configured() {
        let mut mgr = SubagentManager::new();
        mgr.register(
            SubAgentSpec::new("build", "Build", "prompt")
                .with_mode(crate::subagents::AgentMode::All),
        );

        let result = mgr.resolve_default_agent(None);
        assert_eq!(
            result,
            Some("build"),
            "Should return first primary-capable agent"
        );
    }

    #[test]
    fn test_resolve_default_agent_no_primary_capable() {
        let mut mgr = SubagentManager::new();
        // Only subagent-mode agents
        mgr.register(SubAgentSpec::new("helper1", "Helper 1", "prompt"));
        mgr.register(SubAgentSpec::new("helper2", "Helper 2", "prompt"));

        let result = mgr.resolve_default_agent(None);
        assert_eq!(result, None, "No primary-capable agents → None");
    }
}
