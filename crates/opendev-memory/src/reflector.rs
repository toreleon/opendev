//! Execution reflector for extracting learnable patterns from tool executions.
//!
//! Mirrors `opendev/core/context_engineering/memory/reflection/reflector.py`.

use std::collections::HashMap;

/// Result of reflecting on a tool execution sequence.
#[derive(Debug, Clone)]
pub struct ReflectionResult {
    pub category: String,
    pub content: String,
    pub confidence: f64,
    pub reasoning: String,
}

/// Lightweight representation of a tool call for reflection analysis.
#[derive(Debug, Clone)]
pub struct ToolCallInfo {
    pub name: String,
    pub parameters: HashMap<String, String>,
}

/// Analyzes tool execution sequences to extract learnable patterns.
///
/// Identifies patterns worth learning from tool executions,
/// such as multi-step workflows, error recovery, and best practices.
pub struct ExecutionReflector {
    pub min_tool_calls: usize,
    pub min_confidence: f64,
}

impl ExecutionReflector {
    /// Create a new execution reflector.
    pub fn new(min_tool_calls: usize, min_confidence: f64) -> Self {
        Self {
            min_tool_calls,
            min_confidence,
        }
    }

    /// Extract a reusable strategy from tool execution.
    pub fn reflect(
        &self,
        _query: &str,
        tool_calls: &[ToolCallInfo],
        outcome: &str,
    ) -> Option<ReflectionResult> {
        if !self.is_worth_learning(tool_calls, outcome) {
            return None;
        }

        let result = self
            .extract_file_operation_pattern(tool_calls)
            .or_else(|| self.extract_code_navigation_pattern(tool_calls))
            .or_else(|| self.extract_testing_pattern(tool_calls))
            .or_else(|| self.extract_shell_command_pattern(tool_calls))
            .or_else(|| self.extract_error_recovery_pattern(tool_calls, outcome));

        result.filter(|r| r.confidence >= self.min_confidence)
    }

    fn is_worth_learning(&self, tool_calls: &[ToolCallInfo], outcome: &str) -> bool {
        // Always learn from error recovery
        if outcome == "error" && !tool_calls.is_empty() {
            return true;
        }

        // Skip single trivial operations
        if tool_calls.len() == 1 {
            let name = &tool_calls[0].name;
            if name == "read_file" || name == "list_files" {
                return false;
            }
        }

        // Learn from multi-step sequences
        if tool_calls.len() >= self.min_tool_calls {
            return true;
        }

        false
    }

    fn extract_file_operation_pattern(
        &self,
        tool_calls: &[ToolCallInfo],
    ) -> Option<ReflectionResult> {
        let names: Vec<&str> = tool_calls.iter().map(|tc| tc.name.as_str()).collect();

        // Pattern: list_files -> read_file
        if let (Some(list_idx), Some(read_idx)) = (
            names.iter().position(|&n| n == "list_files"),
            names.iter().position(|&n| n == "read_file"),
        ) && list_idx < read_idx
        {
            return Some(ReflectionResult {
                category: "file_operations".to_string(),
                content: "List directory contents before reading files to understand \
                              structure and locate files"
                    .to_string(),
                confidence: 0.75,
                reasoning: "Sequential list_files -> read_file pattern shows exploratory \
                                file access"
                    .to_string(),
            });
        }

        // Pattern: read_file -> write_file
        if let (Some(read_idx), Some(write_idx)) = (
            names.iter().position(|&n| n == "read_file"),
            names.iter().position(|&n| n == "write_file"),
        ) && read_idx < write_idx
        {
            return Some(ReflectionResult {
                category: "file_operations".to_string(),
                content: "Read file contents before writing to understand current state \
                              and preserve important data"
                    .to_string(),
                confidence: 0.8,
                reasoning: "Sequential read_file -> write_file shows safe modification \
                                workflow"
                    .to_string(),
            });
        }

        // Pattern: multiple read_file calls
        let read_count = names.iter().filter(|&&n| n == "read_file").count();
        if read_count >= 3 {
            return Some(ReflectionResult {
                category: "code_navigation".to_string(),
                content: "When understanding complex code, read multiple related files to \
                          build complete picture"
                    .to_string(),
                confidence: 0.7,
                reasoning: format!(
                    "Multiple file reads ({read_count}) indicates thorough code exploration"
                ),
            });
        }

        None
    }

    fn extract_code_navigation_pattern(
        &self,
        tool_calls: &[ToolCallInfo],
    ) -> Option<ReflectionResult> {
        let names: Vec<&str> = tool_calls.iter().map(|tc| tc.name.as_str()).collect();

        // Pattern: search -> read_file
        if let (Some(search_idx), Some(read_idx)) = (
            names.iter().position(|&n| n == "search"),
            names.iter().position(|&n| n == "read_file"),
        ) && search_idx < read_idx
        {
            return Some(ReflectionResult {
                category: "code_navigation".to_string(),
                content: "Search for keywords or patterns before reading files to locate \
                              relevant code efficiently"
                    .to_string(),
                confidence: 0.8,
                reasoning: "Search followed by read shows targeted file access".to_string(),
            });
        }

        // Pattern: multiple searches
        let search_count = names.iter().filter(|&&n| n == "search").count();
        if search_count >= 2 {
            return Some(ReflectionResult {
                category: "code_navigation".to_string(),
                content: "Use multiple searches with different keywords to thoroughly explore \
                          codebase and find all relevant locations"
                    .to_string(),
                confidence: 0.7,
                reasoning: format!(
                    "Multiple searches ({search_count}) shows iterative code exploration"
                ),
            });
        }

        None
    }

    fn extract_testing_pattern(&self, tool_calls: &[ToolCallInfo]) -> Option<ReflectionResult> {
        let names: Vec<&str> = tool_calls.iter().map(|tc| tc.name.as_str()).collect();

        let test_keywords = ["test", "pytest", "jest", "npm test"];
        let has_test_command = tool_calls.iter().any(|tc| {
            tc.name == "run_command"
                && tc.parameters.get("command").is_some_and(|cmd| {
                    let lower = cmd.to_lowercase();
                    test_keywords.iter().any(|kw| lower.contains(kw))
                })
        });

        if !has_test_command {
            return None;
        }

        if names.contains(&"write_file") || names.contains(&"edit_file") {
            return Some(ReflectionResult {
                category: "testing".to_string(),
                content: "Run tests after making code changes to verify correctness and \
                          catch regressions early"
                    .to_string(),
                confidence: 0.85,
                reasoning: "Code modification followed by test execution shows good \
                            development practice"
                    .to_string(),
            });
        }

        None
    }

    fn extract_shell_command_pattern(
        &self,
        tool_calls: &[ToolCallInfo],
    ) -> Option<ReflectionResult> {
        let commands: Vec<&str> = tool_calls
            .iter()
            .filter(|tc| tc.name == "run_command")
            .filter_map(|tc| tc.parameters.get("command").map(String::as_str))
            .collect();

        if commands.len() < 2 {
            return None;
        }

        let install_keywords = [
            "npm install",
            "pip install",
            "yarn install",
            "poetry install",
        ];
        let run_keywords = ["npm start", "python", "node", "pytest"];

        let has_install = commands.iter().any(|cmd| {
            install_keywords
                .iter()
                .any(|kw| cmd.to_lowercase().contains(kw))
        });
        let has_run = commands.iter().any(|cmd| {
            run_keywords
                .iter()
                .any(|kw| cmd.to_lowercase().contains(kw))
        });

        if has_install && has_run {
            return Some(ReflectionResult {
                category: "shell_commands".to_string(),
                content: "Install dependencies before running or testing applications to \
                          ensure all requirements are met"
                    .to_string(),
                confidence: 0.8,
                reasoning: "Install followed by run/test shows proper setup workflow".to_string(),
            });
        }

        if commands.iter().any(|cmd| cmd.contains("git status")) {
            return Some(ReflectionResult {
                category: "git_operations".to_string(),
                content: "Check git status before performing git operations to understand \
                          current state and avoid mistakes"
                    .to_string(),
                confidence: 0.75,
                reasoning: "Git status check before operations shows careful version control \
                            practice"
                    .to_string(),
            });
        }

        None
    }

    fn extract_error_recovery_pattern(
        &self,
        tool_calls: &[ToolCallInfo],
        outcome: &str,
    ) -> Option<ReflectionResult> {
        if outcome != "error" {
            return None;
        }

        let names: Vec<&str> = tool_calls.iter().map(|tc| tc.name.as_str()).collect();

        if names.contains(&"read_file") {
            return Some(ReflectionResult {
                category: "error_handling".to_string(),
                content: "When file access fails, list directory first to verify file exists \
                          and check path correctness"
                    .to_string(),
                confidence: 0.7,
                reasoning: "File access error suggests need for directory verification".to_string(),
            });
        }

        if names.contains(&"run_command") {
            return Some(ReflectionResult {
                category: "error_handling".to_string(),
                content: "When commands fail, verify environment setup, dependencies, and \
                          working directory before retrying"
                    .to_string(),
                confidence: 0.65,
                reasoning: "Command execution error suggests environment or dependency issue"
                    .to_string(),
            });
        }

        None
    }
}

impl Default for ExecutionReflector {
    fn default() -> Self {
        Self::new(2, 0.6)
    }
}

/// Default recency decay factor per day.
const RECENCY_DECAY: f64 = 0.95;

/// Score a reflection by evidence count and recency.
///
/// The score combines evidence strength with temporal decay:
///   `score = evidence_count * recency_decay^age_days`
///
/// - More evidence (observations supporting the reflection) increases the score.
/// - Older reflections decay exponentially, encouraging fresh insights.
/// - A reflection with zero evidence scores 0.0 regardless of age.
///
/// # Arguments
/// * `_reflection` - The reflection text (reserved for future content-based scoring).
/// * `evidence_count` - Number of supporting observations.
/// * `age_days` - How many days old the reflection is.
pub fn score_reflection(_reflection: &str, evidence_count: usize, age_days: u64) -> f64 {
    if evidence_count == 0 {
        return 0.0;
    }
    let decay = RECENCY_DECAY.powi(age_days as i32);
    evidence_count as f64 * decay
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool(name: &str) -> ToolCallInfo {
        ToolCallInfo {
            name: name.to_string(),
            parameters: HashMap::new(),
        }
    }

    fn tool_with_param(name: &str, key: &str, value: &str) -> ToolCallInfo {
        let mut params = HashMap::new();
        params.insert(key.to_string(), value.to_string());
        ToolCallInfo {
            name: name.to_string(),
            parameters: params,
        }
    }

    #[test]
    fn test_no_reflection_for_single_read() {
        let reflector = ExecutionReflector::default();
        let result = reflector.reflect("query", &[tool("read_file")], "success");
        assert!(result.is_none());
    }

    #[test]
    fn test_file_operation_list_then_read() {
        let reflector = ExecutionReflector::default();
        let calls = vec![tool("list_files"), tool("read_file")];
        let result = reflector.reflect("check files", &calls, "success");
        assert!(result.is_some());
        let r = result.unwrap();
        assert_eq!(r.category, "file_operations");
        assert!(r.confidence >= 0.6);
    }

    #[test]
    fn test_file_operation_read_then_write() {
        let reflector = ExecutionReflector::default();
        let calls = vec![tool("read_file"), tool("write_file")];
        let result = reflector.reflect("update file", &calls, "success");
        assert!(result.is_some());
        assert_eq!(result.unwrap().category, "file_operations");
    }

    #[test]
    fn test_code_navigation_search_then_read() {
        let reflector = ExecutionReflector::default();
        let calls = vec![tool("search"), tool("read_file")];
        let result = reflector.reflect("find function", &calls, "success");
        assert!(result.is_some());
        assert_eq!(result.unwrap().category, "code_navigation");
    }

    #[test]
    fn test_multiple_reads_pattern() {
        let reflector = ExecutionReflector::default();
        let calls = vec![tool("read_file"), tool("read_file"), tool("read_file")];
        let result = reflector.reflect("understand code", &calls, "success");
        assert!(result.is_some());
        assert_eq!(result.unwrap().category, "code_navigation");
    }

    #[test]
    fn test_testing_pattern() {
        let reflector = ExecutionReflector::default();
        let calls = vec![
            tool("write_file"),
            tool_with_param("run_command", "command", "pytest tests/"),
        ];
        let result = reflector.reflect("fix test", &calls, "success");
        assert!(result.is_some());
        assert_eq!(result.unwrap().category, "testing");
    }

    #[test]
    fn test_shell_install_then_run() {
        let reflector = ExecutionReflector::default();
        let calls = vec![
            tool_with_param("run_command", "command", "pip install -r requirements.txt"),
            tool_with_param("run_command", "command", "python main.py"),
        ];
        let result = reflector.reflect("run app", &calls, "success");
        assert!(result.is_some());
        assert_eq!(result.unwrap().category, "shell_commands");
    }

    #[test]
    fn test_git_status_pattern() {
        let reflector = ExecutionReflector::default();
        let calls = vec![
            tool_with_param("run_command", "command", "git status"),
            tool_with_param("run_command", "command", "git commit -m 'fix'"),
        ];
        let result = reflector.reflect("commit", &calls, "success");
        assert!(result.is_some());
        assert_eq!(result.unwrap().category, "git_operations");
    }

    #[test]
    fn test_error_recovery_file_access() {
        let reflector = ExecutionReflector::default();
        let calls = vec![tool("read_file")];
        let result = reflector.reflect("read config", &calls, "error");
        assert!(result.is_some());
        assert_eq!(result.unwrap().category, "error_handling");
    }

    #[test]
    fn test_error_recovery_command_failure() {
        let reflector = ExecutionReflector::default();
        let calls = vec![tool("run_command")];
        let result = reflector.reflect("build", &calls, "error");
        assert!(result.is_some());
        assert_eq!(result.unwrap().category, "error_handling");
    }

    #[test]
    fn test_no_learning_from_empty() {
        let reflector = ExecutionReflector::default();
        let result = reflector.reflect("query", &[], "success");
        assert!(result.is_none());
    }

    #[test]
    fn test_confidence_threshold() {
        let reflector = ExecutionReflector::new(2, 0.9); // High threshold
        let calls = vec![tool("read_file"), tool("read_file")];
        // Most patterns have confidence < 0.9, so this should fail
        let result = reflector.reflect("query", &calls, "success");
        assert!(result.is_none());
    }

    // ------------------------------------------------------------------ //
    // score_reflection tests
    // ------------------------------------------------------------------ //

    #[test]
    fn test_score_reflection_zero_evidence() {
        let score = score_reflection("some insight", 0, 0);
        assert_eq!(score, 0.0);
    }

    #[test]
    fn test_score_reflection_fresh() {
        let score = score_reflection("fresh insight", 5, 0);
        // 5 * 0.95^0 = 5.0
        assert!((score - 5.0).abs() < 1e-10);
    }

    #[test]
    fn test_score_reflection_one_day_old() {
        let score = score_reflection("day old", 1, 1);
        // 1 * 0.95^1 = 0.95
        assert!((score - 0.95).abs() < 1e-10);
    }

    #[test]
    fn test_score_reflection_decays_over_time() {
        let fresh = score_reflection("insight", 3, 0);
        let week_old = score_reflection("insight", 3, 7);
        let month_old = score_reflection("insight", 3, 30);

        assert!(fresh > week_old, "fresh > week old");
        assert!(week_old > month_old, "week old > month old");
        assert!(month_old > 0.0, "month old still positive");
    }

    #[test]
    fn test_score_reflection_more_evidence_higher_score() {
        let low = score_reflection("insight", 1, 5);
        let high = score_reflection("insight", 10, 5);
        assert!(high > low);
        // Both should have the same decay factor: 0.95^5
        let ratio = high / low;
        assert!((ratio - 10.0).abs() < 1e-10);
    }

    #[test]
    fn test_score_reflection_large_age() {
        let score = score_reflection("ancient", 1, 365);
        // 0.95^365 is very small but positive
        assert!(score > 0.0);
        assert!(score < 0.001, "very old reflection should have tiny score");
    }
}
