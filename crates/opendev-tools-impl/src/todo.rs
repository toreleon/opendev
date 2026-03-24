//! Todo tool — list, update, and manage plan execution todos.
//!
//! Works with the `TodoManager` from `opendev-runtime` to let the agent
//! query and update todo progress during plan execution.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use opendev_tools_core::{BaseTool, ToolContext, ToolDisplayMeta, ToolResult};

/// Tool for managing plan execution todos.
#[derive(Debug)]
pub struct TodoTool {
    /// Shared reference to the todo manager.
    ///
    /// Uses `Arc<Mutex<_>>` so the tool can be registered in the tool registry
    /// while the manager is also accessed by the TUI and react loop.
    manager: Arc<Mutex<opendev_runtime::TodoManager>>,
}

impl TodoTool {
    /// Create a new todo tool with a shared manager.
    pub fn new(manager: Arc<Mutex<opendev_runtime::TodoManager>>) -> Self {
        Self { manager }
    }
}

#[async_trait::async_trait]
impl BaseTool for TodoTool {
    fn name(&self) -> &str {
        "todo"
    }

    fn description(&self) -> &str {
        "Manage plan execution todos. List current todos, mark items as \
         in-progress or completed, or add new items."
    }

    fn parameter_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["list", "start", "complete", "add"],
                    "description": "Action to perform on todos"
                },
                "id": {
                    "type": "integer",
                    "description": "Todo item ID (for start/complete)"
                },
                "title": {
                    "type": "string",
                    "description": "Title for a new todo item (for add)"
                }
            },
            "required": ["action"]
        })
    }

    async fn execute(
        &self,
        args: HashMap<String, serde_json::Value>,
        _ctx: &ToolContext,
    ) -> ToolResult {
        let action = match args.get("action").and_then(|v| v.as_str()) {
            Some(a) => a,
            None => return ToolResult::fail("action is required"),
        };

        let mut mgr = match self.manager.lock() {
            Ok(m) => m,
            Err(e) => return ToolResult::fail(format!("Lock error: {e}")),
        };

        match action {
            "list" => {
                if !mgr.has_todos() {
                    return ToolResult::ok("No todos.");
                }
                ToolResult::ok(mgr.format_status())
            }
            "start" => {
                let id = match args.get("id").and_then(|v| v.as_u64()) {
                    Some(id) => id as usize,
                    None => return ToolResult::fail("id is required for start"),
                };
                if mgr.start(id) {
                    ToolResult::ok(format!(
                        "Todo {id} marked as in-progress.\n\n{}",
                        mgr.format_status()
                    ))
                } else {
                    ToolResult::fail(format!("Todo {id} not found"))
                }
            }
            "complete" => {
                let id = match args.get("id").and_then(|v| v.as_u64()) {
                    Some(id) => id as usize,
                    None => return ToolResult::fail("id is required for complete"),
                };
                if mgr.complete(id) {
                    let status = mgr.format_status();
                    if mgr.all_completed() {
                        ToolResult::ok(format!(
                            "Todo {id} completed. All todos are done!\n\n{status}"
                        ))
                    } else {
                        ToolResult::ok(format!("Todo {id} completed.\n\n{status}"))
                    }
                } else {
                    ToolResult::fail(format!("Todo {id} not found"))
                }
            }
            "add" => {
                let title = match args.get("title").and_then(|v| v.as_str()) {
                    Some(t) if !t.is_empty() => t,
                    _ => return ToolResult::fail("title is required for add"),
                };
                let id = mgr.add(title.to_string());
                ToolResult::ok(format!(
                    "Added todo {id}: {title}\n\n{}",
                    mgr.format_status()
                ))
            }
            _ => ToolResult::fail(format!(
                "Unknown action: {action}. Available: list, start, complete, add"
            )),
        }
    }

    fn display_meta(&self) -> Option<ToolDisplayMeta> {
        Some(ToolDisplayMeta {
            verb: "Todo",
            label: "task",
            category: "Plan",
            primary_arg_keys: &["action", "id", "title"],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_tool() -> (TodoTool, Arc<Mutex<opendev_runtime::TodoManager>>) {
        let mgr = Arc::new(Mutex::new(opendev_runtime::TodoManager::from_steps(&[
            "Step A".into(),
            "Step B".into(),
            "Step C".into(),
        ])));
        let tool = TodoTool::new(Arc::clone(&mgr));
        (tool, mgr)
    }

    fn make_args(pairs: &[(&str, serde_json::Value)]) -> HashMap<String, serde_json::Value> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect()
    }

    #[tokio::test]
    async fn test_list() {
        let (tool, _mgr) = make_tool();
        let ctx = ToolContext::new("/tmp");
        let result = tool
            .execute(make_args(&[("action", serde_json::json!("list"))]), &ctx)
            .await;
        assert!(result.success);
        let output = result.output.unwrap();
        assert!(output.contains("0/3 done"));
        assert!(output.contains("Step A"));
    }

    #[tokio::test]
    async fn test_list_empty() {
        let mgr = Arc::new(Mutex::new(opendev_runtime::TodoManager::new()));
        let tool = TodoTool::new(mgr);
        let ctx = ToolContext::new("/tmp");
        let result = tool
            .execute(make_args(&[("action", serde_json::json!("list"))]), &ctx)
            .await;
        assert!(result.success);
        assert!(result.output.unwrap().contains("No todos"));
    }

    #[tokio::test]
    async fn test_start() {
        let (tool, _mgr) = make_tool();
        let ctx = ToolContext::new("/tmp");
        let result = tool
            .execute(
                make_args(&[
                    ("action", serde_json::json!("start")),
                    ("id", serde_json::json!(1)),
                ]),
                &ctx,
            )
            .await;
        assert!(result.success);
        assert!(result.output.unwrap().contains("in-progress"));
    }

    #[tokio::test]
    async fn test_complete() {
        let (tool, _mgr) = make_tool();
        let ctx = ToolContext::new("/tmp");
        let result = tool
            .execute(
                make_args(&[
                    ("action", serde_json::json!("complete")),
                    ("id", serde_json::json!(1)),
                ]),
                &ctx,
            )
            .await;
        assert!(result.success);
        assert!(result.output.unwrap().contains("completed"));
    }

    #[tokio::test]
    async fn test_complete_all() {
        let (tool, _mgr) = make_tool();
        let ctx = ToolContext::new("/tmp");
        for id in 1..=3 {
            tool.execute(
                make_args(&[
                    ("action", serde_json::json!("complete")),
                    ("id", serde_json::json!(id)),
                ]),
                &ctx,
            )
            .await;
        }
        let result = tool
            .execute(make_args(&[("action", serde_json::json!("list"))]), &ctx)
            .await;
        assert!(result.output.unwrap().contains("3/3 done"));
    }

    #[tokio::test]
    async fn test_add() {
        let (tool, mgr) = make_tool();
        let ctx = ToolContext::new("/tmp");
        let result = tool
            .execute(
                make_args(&[
                    ("action", serde_json::json!("add")),
                    ("title", serde_json::json!("New step")),
                ]),
                &ctx,
            )
            .await;
        assert!(result.success);
        assert_eq!(mgr.lock().unwrap().total(), 4);
    }

    #[tokio::test]
    async fn test_missing_action() {
        let (tool, _mgr) = make_tool();
        let ctx = ToolContext::new("/tmp");
        let result = tool.execute(HashMap::new(), &ctx).await;
        assert!(!result.success);
    }

    #[tokio::test]
    async fn test_unknown_action() {
        let (tool, _mgr) = make_tool();
        let ctx = ToolContext::new("/tmp");
        let result = tool
            .execute(make_args(&[("action", serde_json::json!("unknown"))]), &ctx)
            .await;
        assert!(!result.success);
    }

    #[tokio::test]
    async fn test_start_nonexistent() {
        let (tool, _mgr) = make_tool();
        let ctx = ToolContext::new("/tmp");
        let result = tool
            .execute(
                make_args(&[
                    ("action", serde_json::json!("start")),
                    ("id", serde_json::json!(999)),
                ]),
                &ctx,
            )
            .await;
        assert!(!result.success);
    }
}
