//! Thinking skip heuristic and subagent analysis.

use serde_json::Value;

use super::super::ReactLoop;

impl ReactLoop {
    /// Check if the last tool calls were all read-only and succeeded.
    ///
    /// Used to skip the thinking phase when the previous turn only did
    /// information gathering (no state changes to re-plan around).
    /// Mirrors Python's `IterationMixin._last_tools_were_readonly()`.
    pub fn should_skip_thinking(&self, messages: &[Value]) -> bool {
        let mut found_tools = false;
        // Collect tool names from the most recent assistant tool_calls
        let _last_assistant_tools: Vec<String> = Vec::new();

        for msg in messages.iter().rev() {
            // Skip injected thinking trace messages (fake assistant-user pair)
            if msg.get("_thinking").and_then(|v| v.as_bool()) == Some(true) {
                continue;
            }

            let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("");
            match role {
                "tool" => {
                    let content = msg.get("content").and_then(|c| c.as_str()).unwrap_or("");
                    let tool_name = msg.get("name").and_then(|n| n.as_str()).unwrap_or("");

                    // If any tool errored, don't skip thinking
                    if content.starts_with("Error")
                        || content.to_lowercase().contains("\"success\": false")
                    {
                        return false;
                    }
                    if !tool_name.is_empty() && !self.readonly_tools.contains(tool_name) {
                        return false;
                    }
                    found_tools = true;
                }
                "assistant" if found_tools => {
                    // Check tool_calls in the assistant message for non-readonly tools
                    if let Some(tcs) = msg.get("tool_calls").and_then(|v| v.as_array()) {
                        for tc in tcs {
                            if let Some(name) = tc
                                .get("function")
                                .and_then(|f| f.get("name"))
                                .and_then(|n| n.as_str())
                                && !self.readonly_tools.contains(name)
                            {
                                return false;
                            }
                        }
                    }
                    break;
                }
                "user" if found_tools => break,
                "user" | "assistant" => return false,
                _ => {}
            }
        }
        found_tools
    }

    /// Count the number of assistant messages with tool_calls in a subagent result.
    ///
    /// Used for shallow subagent detection. If a subagent only made <=1 tool
    /// call, the parent could have done it directly.
    pub fn count_subagent_tool_calls(messages: &[Value]) -> usize {
        messages
            .iter()
            .filter(|msg| {
                msg.get("role").and_then(|r| r.as_str()) == Some("assistant")
                    && msg.get("tool_calls").is_some()
                    && !msg
                        .get("tool_calls")
                        .and_then(|tc| tc.as_array())
                        .map(|a| a.is_empty())
                        .unwrap_or(true)
            })
            .count()
    }

    /// Generate a shallow subagent warning suffix if applicable.
    ///
    /// Returns `Some(warning)` if the subagent made <=1 tool calls, `None` otherwise.
    pub fn shallow_subagent_warning(result_messages: &[Value], success: bool) -> Option<String> {
        if !success {
            return None;
        }
        let tool_call_count = Self::count_subagent_tool_calls(result_messages);
        if tool_call_count <= 1 {
            Some(format!(
                "\n\n[SHALLOW SUBAGENT WARNING] This subagent only made \
                 {tool_call_count} tool call(s). Spawning a subagent for a task \
                 that requires ≤1 tool call is wasteful — you should have used a \
                 direct tool call instead. For future similar tasks, use read_file, \
                 search, or list_files directly rather than spawning a subagent."
            ))
        } else {
            None
        }
    }
}
