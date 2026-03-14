//! Conversation summarization for thinking context.
//!
//! Implements episodic memory through LLM-generated conversation summaries.
//! Uses incremental summarization - only sends new messages to the LLM and
//! merges them with the existing summary to save tokens.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Cached conversation summary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationSummary {
    /// The summary text.
    pub summary: String,
    /// Total number of messages when summary was created.
    pub message_count: usize,
    /// Index in filtered messages list up to which we've summarized.
    pub last_summarized_index: usize,
}

/// Generates and caches conversation summaries for episodic memory.
///
/// Uses LLM to create concise summaries of conversation history.
/// Implements incremental summarization - only new messages since the
/// last summary are sent to the LLM, merged with the previous summary.
pub struct ConversationSummarizer {
    cache: Option<ConversationSummary>,
    regenerate_threshold: usize,
    max_summary_length: usize,
    exclude_last_n: usize,
}

impl Default for ConversationSummarizer {
    fn default() -> Self {
        Self::new()
    }
}

impl ConversationSummarizer {
    /// Create a new summarizer with default settings.
    pub fn new() -> Self {
        Self {
            cache: None,
            regenerate_threshold: 5,
            max_summary_length: 500,
            exclude_last_n: 6,
        }
    }

    /// Set the regeneration threshold (number of new messages before re-summarizing).
    pub fn with_threshold(mut self, n: usize) -> Self {
        self.regenerate_threshold = n;
        self
    }

    /// Set the maximum summary length in characters.
    pub fn with_max_length(mut self, n: usize) -> Self {
        self.max_summary_length = n;
        self
    }

    /// Set how many recent messages to exclude from summarization.
    pub fn with_exclude_last(mut self, n: usize) -> Self {
        self.exclude_last_n = n;
        self
    }

    /// Check if summary needs regeneration based on message count delta.
    pub fn needs_regeneration(&self, current_message_count: usize) -> bool {
        match &self.cache {
            None => true,
            Some(cached) => {
                let messages_since_update =
                    current_message_count.saturating_sub(cached.message_count);
                messages_since_update >= self.regenerate_threshold
            }
        }
    }

    /// Get cached summary if available.
    pub fn get_cached_summary(&self) -> Option<&str> {
        self.cache.as_ref().map(|c| c.summary.as_str())
    }

    /// Generate summary of conversation history using incremental approach.
    ///
    /// Only sends new messages since the last summary to the LLM,
    /// along with the previous summary for context merging.
    ///
    /// The `llm_caller` receives a slice of message values (system + user prompt)
    /// and should return `Some(summary_text)` on success.
    pub fn generate_summary<F>(&mut self, messages: &[Value], llm_caller: F) -> String
    where
        F: Fn(&[Value]) -> Option<String>,
    {
        // Filter out system messages
        let filtered: Vec<&Value> = messages
            .iter()
            .filter(|m| m.get("role").and_then(Value::as_str) != Some("system"))
            .collect();

        // Calculate the end index (excluding last N messages for short-term memory)
        let end_index = if filtered.len() > self.exclude_last_n {
            filtered.len() - self.exclude_last_n
        } else {
            // Not enough history to summarize
            return self.cached_summary_or_empty();
        };

        if end_index == 0 {
            return self.cached_summary_or_empty();
        }

        // Determine which messages are new
        let (new_start, previous_summary) = match &self.cache {
            Some(cached) => (cached.last_summarized_index, cached.summary.clone()),
            None => (0, String::new()),
        };

        // Extract only new messages
        if new_start >= end_index {
            return self.cached_summary_or_empty();
        }
        let new_messages = &filtered[new_start..end_index];

        if new_messages.is_empty() {
            return self.cached_summary_or_empty();
        }

        // Build prompt
        let prev_summary_text = if previous_summary.is_empty() {
            "(No previous summary)".to_string()
        } else {
            previous_summary
        };

        let formatted = Self::format_conversation_slice(new_messages);

        let prompt = format!(
            "Summarize the following conversation, incorporating the previous summary.\n\n\
             Previous summary:\n{}\n\n\
             New messages:\n{}",
            prev_summary_text, formatted
        );

        // Build LLM call messages
        let call_messages = vec![
            serde_json::json!({
                "role": "system",
                "content": "You are a helpful assistant that summarizes conversations concisely."
            }),
            serde_json::json!({
                "role": "user",
                "content": prompt
            }),
        ];

        // Call LLM for summary
        if let Some(raw_summary) = llm_caller(&call_messages) {
            let summary = truncate_str(&raw_summary, self.max_summary_length).to_string();

            self.cache = Some(ConversationSummary {
                summary: summary.clone(),
                message_count: messages.len(),
                last_summarized_index: end_index,
            });

            return summary;
        }

        self.cached_summary_or_empty()
    }

    /// Format messages into readable conversation text.
    pub fn format_conversation(messages: &[Value]) -> String {
        let refs: Vec<&Value> = messages.iter().collect();
        Self::format_conversation_slice(&refs)
    }

    /// Clear the cached summary.
    pub fn clear_cache(&mut self) {
        self.cache = None;
    }

    /// Serialize cache for session persistence.
    pub fn to_json(&self) -> Option<Value> {
        self.cache.as_ref().map(|c| {
            serde_json::json!({
                "summary": c.summary,
                "message_count": c.message_count,
                "last_summarized_index": c.last_summarized_index,
            })
        })
    }

    /// Load cache from session persistence.
    pub fn from_json(&mut self, data: Option<&Value>) {
        match data {
            None => self.cache = None,
            Some(val) => {
                self.cache = Some(ConversationSummary {
                    summary: val
                        .get("summary")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                    message_count: val
                        .get("message_count")
                        .and_then(Value::as_u64)
                        .unwrap_or(0) as usize,
                    last_summarized_index: val
                        .get("last_summarized_index")
                        .and_then(Value::as_u64)
                        .unwrap_or(0) as usize,
                });
            }
        }
    }

    // -- Private helpers --

    fn cached_summary_or_empty(&self) -> String {
        self.cache
            .as_ref()
            .map(|c| c.summary.clone())
            .unwrap_or_default()
    }

    fn format_conversation_slice(messages: &[&Value]) -> String {
        let mut lines = Vec::new();

        for msg in messages {
            let role = msg
                .get("role")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_uppercase();
            let content = msg.get("content").and_then(Value::as_str).unwrap_or("");

            match role.as_str() {
                "USER" => {
                    lines.push(format!("User: {}", truncate_str(content, 200)));
                }
                "ASSISTANT" => {
                    if let Some(tool_calls) = msg.get("tool_calls").and_then(Value::as_array) {
                        let tool_names: Vec<&str> = tool_calls
                            .iter()
                            .filter_map(|tc| {
                                tc.get("function")
                                    .and_then(|f| f.get("name"))
                                    .and_then(Value::as_str)
                            })
                            .collect();
                        if !tool_names.is_empty() {
                            lines.push(format!(
                                "Assistant: [Called tools: {}]",
                                tool_names.join(", ")
                            ));
                        } else if !content.is_empty() {
                            lines.push(format!("Assistant: {}", truncate_str(content, 200)));
                        }
                    } else if !content.is_empty() {
                        lines.push(format!("Assistant: {}", truncate_str(content, 200)));
                    }
                }
                "TOOL" => {
                    lines.push("Tool: [result received]".to_string());
                }
                _ => {}
            }
        }

        lines.join("\n")
    }
}

/// Extract key learnings from a conversation for playbook consolidation.
///
/// Scans messages for patterns worth remembering:
/// - Error patterns that were fixed (error followed by a successful resolution)
/// - Configuration/environment discoveries (paths, settings, commands)
/// - File patterns and project structure learned
///
/// Returns a list of concise learning strings suitable for playbook bullets.
pub fn consolidate_learnings(messages: &[Value]) -> Vec<String> {
    let mut learnings = Vec::new();

    // Track error -> fix patterns
    let mut last_error: Option<String> = None;
    let mut seen_configs: Vec<String> = Vec::new();
    let mut seen_file_patterns: Vec<String> = Vec::new();

    for msg in messages {
        let role = msg.get("role").and_then(Value::as_str).unwrap_or("");
        let content = msg.get("content").and_then(Value::as_str).unwrap_or("");

        // Detect error messages in tool results
        if role == "tool" {
            let lower = content.to_lowercase();
            if lower.contains("error")
                || lower.contains("failed")
                || lower.contains("not found")
                || lower.contains("permission denied")
            {
                // Extract a short summary of the error
                let error_summary = content.lines().next().unwrap_or(content);
                let truncated = if error_summary.len() > 120 {
                    &error_summary[..120]
                } else {
                    error_summary
                };
                last_error = Some(truncated.to_string());
            }
        }

        // Detect fixes following errors
        if role == "assistant" && last_error.is_some() {
            let lower = content.to_lowercase();
            if lower.contains("fixed")
                || lower.contains("resolved")
                || lower.contains("the issue was")
                || lower.contains("the problem was")
                || lower.contains("solution")
            {
                if let Some(ref err) = last_error {
                    let fix_summary = content.lines().next().unwrap_or(content);
                    let truncated_fix = if fix_summary.len() > 120 {
                        &fix_summary[..120]
                    } else {
                        fix_summary
                    };
                    learnings.push(format!(
                        "Error pattern fixed: '{}' -> '{}'",
                        err, truncated_fix
                    ));
                }
                last_error = None;
            }
        }

        // Detect configuration discoveries
        if role == "assistant" {
            let lower = content.to_lowercase();
            let config_keywords = [
                "config",
                "configuration",
                "setting",
                "environment variable",
                "env var",
                ".env",
            ];
            for keyword in &config_keywords {
                if lower.contains(keyword) {
                    let config_line = content.lines().next().unwrap_or(content);
                    let truncated = if config_line.len() > 150 {
                        &config_line[..150]
                    } else {
                        config_line
                    };
                    if !seen_configs.contains(&truncated.to_string()) {
                        seen_configs.push(truncated.to_string());
                        learnings.push(format!("Configuration discovered: {}", truncated));
                    }
                    break;
                }
            }
        }

        // Detect file pattern usage in tool calls
        if let Some(tool_calls) = msg.get("tool_calls").and_then(Value::as_array) {
            for tc in tool_calls {
                let name = tc
                    .get("function")
                    .and_then(|f| f.get("name"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let args = tc
                    .get("function")
                    .and_then(|f| f.get("arguments"))
                    .and_then(Value::as_str)
                    .unwrap_or("");

                if (name == "read_file" || name == "write_file" || name == "edit_file")
                    && args.contains("path")
                {
                    // Extract file extension pattern
                    if let Some(ext_start) = args.rfind('.') {
                        let ext = &args[ext_start..];
                        let ext_clean: String = ext
                            .chars()
                            .take_while(|c| c.is_alphanumeric() || *c == '.')
                            .collect();
                        if ext_clean.len() > 1 && !seen_file_patterns.contains(&ext_clean) {
                            seen_file_patterns.push(ext_clean.clone());
                        }
                    }
                }
            }
        }
    }

    // Add file pattern summary if multiple patterns observed
    if seen_file_patterns.len() >= 2 {
        learnings.push(format!(
            "File patterns used: {}",
            seen_file_patterns.join(", ")
        ));
    }

    learnings
}

/// Truncate a string to at most `max_len` characters.
fn truncate_str(s: &str, max_len: usize) -> &str {
    if s.len() <= max_len {
        return s;
    }
    // Find a valid char boundary at or before max_len
    let mut end = max_len;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn make_msg(role: &str, content: &str) -> Value {
        json!({"role": role, "content": content})
    }

    fn make_assistant_tool_call(tool_name: &str) -> Value {
        json!({
            "role": "assistant",
            "content": "",
            "tool_calls": [{"function": {"name": tool_name}}]
        })
    }

    #[test]
    fn test_new_defaults() {
        let s = ConversationSummarizer::new();
        assert!(s.cache.is_none());
        assert_eq!(s.regenerate_threshold, 5);
        assert_eq!(s.max_summary_length, 500);
        assert_eq!(s.exclude_last_n, 6);
    }

    #[test]
    fn test_builder_pattern() {
        let s = ConversationSummarizer::new()
            .with_threshold(10)
            .with_max_length(200)
            .with_exclude_last(3);
        assert_eq!(s.regenerate_threshold, 10);
        assert_eq!(s.max_summary_length, 200);
        assert_eq!(s.exclude_last_n, 3);
    }

    #[test]
    fn test_needs_regeneration_no_cache() {
        let s = ConversationSummarizer::new();
        assert!(s.needs_regeneration(0));
        assert!(s.needs_regeneration(100));
    }

    #[test]
    fn test_needs_regeneration_with_cache() {
        let mut s = ConversationSummarizer::new().with_threshold(5);
        s.cache = Some(ConversationSummary {
            summary: "test".into(),
            message_count: 10,
            last_summarized_index: 4,
        });
        assert!(!s.needs_regeneration(10));
        assert!(!s.needs_regeneration(14));
        assert!(s.needs_regeneration(15));
        assert!(s.needs_regeneration(20));
    }

    #[test]
    fn test_get_cached_summary() {
        let mut s = ConversationSummarizer::new();
        assert_eq!(s.get_cached_summary(), None);

        s.cache = Some(ConversationSummary {
            summary: "hello world".into(),
            message_count: 5,
            last_summarized_index: 2,
        });
        assert_eq!(s.get_cached_summary(), Some("hello world"));
    }

    #[test]
    fn test_generate_summary_too_few_messages() {
        let mut s = ConversationSummarizer::new().with_exclude_last(6);
        let messages: Vec<Value> = (0..5)
            .map(|i| make_msg("user", &format!("msg {i}")))
            .collect();
        let result = s.generate_summary(&messages, |_| Some("summary".into()));
        assert_eq!(result, ""); // Not enough messages after excluding last 6
    }

    #[test]
    fn test_generate_summary_basic() {
        let mut s = ConversationSummarizer::new()
            .with_exclude_last(2)
            .with_max_length(100);

        let messages = vec![
            make_msg("user", "hello"),
            make_msg("assistant", "hi there"),
            make_msg("user", "how are you?"),
            make_msg("assistant", "I'm good"),
            make_msg("user", "recent 1"),
            make_msg("assistant", "recent 2"),
        ];

        let result = s.generate_summary(&messages, |_| Some("A greeting exchange.".into()));
        assert_eq!(result, "A greeting exchange.");
        assert!(s.cache.is_some());

        let cached = s.cache.as_ref().unwrap();
        assert_eq!(cached.message_count, 6);
        assert_eq!(cached.last_summarized_index, 4); // 6 - 2
    }

    #[test]
    fn test_generate_summary_truncates() {
        let mut s = ConversationSummarizer::new()
            .with_exclude_last(1)
            .with_max_length(10);

        let messages = vec![
            make_msg("user", "hello"),
            make_msg("assistant", "world"),
            make_msg("user", "last"),
        ];

        let result = s.generate_summary(&messages, |_| {
            Some("This is a very long summary text".into())
        });
        assert_eq!(result.len(), 10);
        assert_eq!(result, "This is a ");
    }

    #[test]
    fn test_generate_summary_filters_system() {
        let mut s = ConversationSummarizer::new()
            .with_exclude_last(1)
            .with_max_length(500);

        let messages = vec![
            make_msg("system", "You are a bot"),
            make_msg("user", "hello"),
            make_msg("assistant", "hi"),
            make_msg("user", "last"),
        ];

        // After filtering system, we have 3 messages. Exclude last 1 -> end_index=2.
        // new_messages = filtered[0..2] = ["hello", "hi"]
        let result = s.generate_summary(&messages, |call_msgs| {
            // Verify system messages are filtered from the conversation
            let prompt = call_msgs[1]["content"].as_str().unwrap();
            assert!(prompt.contains("User: hello"));
            assert!(prompt.contains("Assistant: hi"));
            assert!(!prompt.contains("You are a bot"));
            Some("Summary without system msg.".into())
        });
        assert_eq!(result, "Summary without system msg.");
    }

    #[test]
    fn test_format_conversation() {
        let messages = vec![
            make_msg("user", "hello"),
            make_msg("assistant", "hi there"),
            make_assistant_tool_call("bash"),
            make_msg("tool", "some output"),
        ];

        let formatted = ConversationSummarizer::format_conversation(&messages);
        assert!(formatted.contains("User: hello"));
        assert!(formatted.contains("Assistant: hi there"));
        assert!(formatted.contains("Assistant: [Called tools: bash]"));
        assert!(formatted.contains("Tool: [result received]"));
    }

    // ------------------------------------------------------------------ //
    // consolidate_learnings tests
    // ------------------------------------------------------------------ //

    #[test]
    fn test_consolidate_learnings_error_fix_pattern() {
        let messages = vec![
            make_msg("user", "fix the build"),
            json!({
                "role": "tool",
                "content": "error: module 'foo' not found"
            }),
            json!({
                "role": "assistant",
                "content": "The issue was a missing import. Fixed by adding the module."
            }),
        ];
        let learnings = consolidate_learnings(&messages);
        assert!(
            learnings.iter().any(|l| l.contains("Error pattern fixed")),
            "Should detect error->fix pattern, got: {learnings:?}"
        );
    }

    #[test]
    fn test_consolidate_learnings_config_discovery() {
        let messages = vec![
            make_msg("user", "how to connect to the database?"),
            json!({
                "role": "assistant",
                "content": "The database configuration is in config/database.yml with host and port settings."
            }),
        ];
        let learnings = consolidate_learnings(&messages);
        assert!(
            learnings
                .iter()
                .any(|l| l.contains("Configuration discovered")),
            "Should detect config discovery, got: {learnings:?}"
        );
    }

    #[test]
    fn test_consolidate_learnings_file_patterns() {
        let messages = vec![json!({
            "role": "assistant",
            "content": "",
            "tool_calls": [
                {"function": {"name": "read_file", "arguments": "{\"path\": \"src/main.rs\"}"}},
                {"function": {"name": "edit_file", "arguments": "{\"path\": \"src/lib.rs\"}"}},
                {"function": {"name": "read_file", "arguments": "{\"path\": \"tests/test.py\"}"}}
            ]
        })];
        let learnings = consolidate_learnings(&messages);
        assert!(
            learnings.iter().any(|l| l.contains("File patterns used")),
            "Should detect file patterns, got: {learnings:?}"
        );
    }

    #[test]
    fn test_consolidate_learnings_empty() {
        let messages: Vec<Value> = vec![];
        let learnings = consolidate_learnings(&messages);
        assert!(learnings.is_empty());
    }

    #[test]
    fn test_consolidate_learnings_no_patterns() {
        let messages = vec![make_msg("user", "hello"), make_msg("assistant", "hi there")];
        let learnings = consolidate_learnings(&messages);
        assert!(learnings.is_empty());
    }

    #[test]
    fn test_clear_cache_and_json_roundtrip() {
        let mut s = ConversationSummarizer::new();

        // No cache -> to_json returns None
        assert!(s.to_json().is_none());

        // Set cache
        s.cache = Some(ConversationSummary {
            summary: "test summary".into(),
            message_count: 10,
            last_summarized_index: 4,
        });

        // to_json
        let json_val = s.to_json().unwrap();
        assert_eq!(json_val["summary"], "test summary");
        assert_eq!(json_val["message_count"], 10);
        assert_eq!(json_val["last_summarized_index"], 4);

        // clear
        s.clear_cache();
        assert!(s.cache.is_none());
        assert_eq!(s.get_cached_summary(), None);

        // from_json
        s.from_json(Some(&json_val));
        assert_eq!(s.get_cached_summary(), Some("test summary"));

        let cached = s.cache.as_ref().unwrap();
        assert_eq!(cached.message_count, 10);
        assert_eq!(cached.last_summarized_index, 4);

        // from_json(None) clears
        s.from_json(None);
        assert!(s.cache.is_none());
    }
}
