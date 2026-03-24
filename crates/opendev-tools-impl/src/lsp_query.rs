//! LSP query tool — wraps LspWrapper methods for definition, references, hover, and symbols.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::Mutex;

use opendev_tools_core::{BaseTool, ToolContext, ToolDisplayMeta, ToolResult};
use opendev_tools_lsp::LspWrapper;

/// Tool that queries language servers for code intelligence.
///
/// Supports four actions:
/// - `definition` — go to definition of a symbol at a position
/// - `references` — find all references to a symbol at a position
/// - `hover` — get hover information (type info, docs) for a symbol at a position
/// - `symbols` — list all symbols in a file (document symbols)
#[derive(Debug)]
pub struct LspQueryTool {
    /// Shared LSP wrapper (behind a mutex because methods take `&mut self`).
    lsp: Arc<Mutex<LspWrapper>>,
}

impl LspQueryTool {
    /// Create a new LSP query tool with the given LSP wrapper.
    pub fn new(lsp: Arc<Mutex<LspWrapper>>) -> Self {
        Self { lsp }
    }
}

#[async_trait]
impl BaseTool for LspQueryTool {
    fn name(&self) -> &str {
        "lsp_query"
    }

    fn description(&self) -> &str {
        "Query a language server for code intelligence. Supports actions: \
         \"definition\" (go to definition), \"references\" (find all references), \
         \"hover\" (type/doc info), and \"symbols\" (list document symbols)."
    }

    fn parameter_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["definition", "references", "hover", "symbols"],
                    "description": "The LSP action to perform"
                },
                "file_path": {
                    "type": "string",
                    "description": "Path to the file to query"
                },
                "line": {
                    "type": "number",
                    "description": "0-based line number (required for definition, references, hover)"
                },
                "character": {
                    "type": "number",
                    "description": "0-based character offset (required for definition, references, hover)"
                },
                "query": {
                    "type": "string",
                    "description": "Symbol name filter for the symbols action (optional)"
                }
            },
            "required": ["action", "file_path"]
        })
    }

    async fn execute(
        &self,
        args: HashMap<String, serde_json::Value>,
        ctx: &ToolContext,
    ) -> ToolResult {
        let action = match args.get("action").and_then(|v| v.as_str()) {
            Some(a) => a,
            None => return ToolResult::fail("Missing required parameter: action"),
        };

        let file_path_str = match args.get("file_path").and_then(|v| v.as_str()) {
            Some(p) => p,
            None => return ToolResult::fail("Missing required parameter: file_path"),
        };

        let file_path = PathBuf::from(file_path_str);
        let file_path = if file_path.is_relative() {
            ctx.working_dir.join(&file_path)
        } else {
            file_path
        };

        let workspace_root = ctx.working_dir.clone();

        let line = args.get("line").and_then(|v| v.as_u64()).map(|v| v as u32);

        let character = args
            .get("character")
            .and_then(|v| v.as_u64())
            .map(|v| v as u32);

        let mut lsp = self.lsp.lock().await;

        match action {
            "definition" => {
                let (line, character) = match (line, character) {
                    (Some(l), Some(c)) => (l, c),
                    _ => {
                        return ToolResult::fail(
                            "Parameters 'line' and 'character' are required for the 'definition' action",
                        );
                    }
                };

                match lsp
                    .goto_definition(&file_path, line, character, &workspace_root)
                    .await
                {
                    Ok(locations) => {
                        if locations.is_empty() {
                            return ToolResult::ok("No definition found at the given position.");
                        }
                        let mut output = format!("Found {} definition(s):\n\n", locations.len());
                        for loc in &locations {
                            output.push_str(&format!(
                                "  {}:{}:{}\n",
                                loc.file_path.display(),
                                loc.range.start.line + 1,
                                loc.range.start.character + 1,
                            ));
                        }
                        ToolResult::ok(output)
                    }
                    Err(e) => ToolResult::fail(format!("definition request failed: {e}")),
                }
            }

            "references" => {
                let (line, character) = match (line, character) {
                    (Some(l), Some(c)) => (l, c),
                    _ => {
                        return ToolResult::fail(
                            "Parameters 'line' and 'character' are required for the 'references' action",
                        );
                    }
                };

                match lsp
                    .find_references(&file_path, line, character, &workspace_root)
                    .await
                {
                    Ok(locations) => {
                        if locations.is_empty() {
                            return ToolResult::ok("No references found at the given position.");
                        }
                        let mut output = format!("Found {} reference(s):\n\n", locations.len());
                        for loc in &locations {
                            output.push_str(&format!(
                                "  {}:{}:{}\n",
                                loc.file_path.display(),
                                loc.range.start.line + 1,
                                loc.range.start.character + 1,
                            ));
                        }
                        let mut metadata = HashMap::new();
                        metadata.insert("count".to_string(), serde_json::json!(locations.len()));
                        ToolResult::ok_with_metadata(output, metadata)
                    }
                    Err(e) => ToolResult::fail(format!("references request failed: {e}")),
                }
            }

            "hover" => {
                let (line, character) = match (line, character) {
                    (Some(l), Some(c)) => (l, c),
                    _ => {
                        return ToolResult::fail(
                            "Parameters 'line' and 'character' are required for the 'hover' action",
                        );
                    }
                };

                match lsp
                    .hover(&file_path, line, character, &workspace_root)
                    .await
                {
                    Ok(Some(text)) => ToolResult::ok(text),
                    Ok(None) => {
                        ToolResult::ok("No hover information available at the given position.")
                    }
                    Err(e) => ToolResult::fail(format!("hover request failed: {e}")),
                }
            }

            "symbols" => {
                let query_filter = args.get("query").and_then(|v| v.as_str());

                match lsp.document_symbols(&file_path, &workspace_root).await {
                    Ok(symbols) => {
                        let filtered: Vec<_> = if let Some(q) = query_filter {
                            let q_lower = q.to_lowercase();
                            symbols
                                .into_iter()
                                .filter(|s| s.name.to_lowercase().contains(&q_lower))
                                .collect()
                        } else {
                            symbols
                        };

                        if filtered.is_empty() {
                            let msg = if let Some(q) = query_filter {
                                format!(
                                    "No symbols matching '{}' found in {}",
                                    q,
                                    file_path.display()
                                )
                            } else {
                                format!("No symbols found in {}", file_path.display())
                            };
                            return ToolResult::ok(msg);
                        }

                        let mut output = format!(
                            "Found {} symbol(s) in {}:\n\n",
                            filtered.len(),
                            file_path.display()
                        );
                        for sym in &filtered {
                            let container = sym
                                .container_name
                                .as_deref()
                                .map(|c| format!(" (in {c})"))
                                .unwrap_or_default();
                            output.push_str(&format!(
                                "  {} ({}){}  — line {}\n",
                                sym.name,
                                sym.kind.display_name(),
                                container,
                                sym.range.start.line + 1,
                            ));
                        }

                        let mut metadata = HashMap::new();
                        metadata.insert("count".to_string(), serde_json::json!(filtered.len()));
                        ToolResult::ok_with_metadata(output, metadata)
                    }
                    Err(e) => ToolResult::fail(format!("document symbols request failed: {e}")),
                }
            }

            _ => ToolResult::fail(format!(
                "Unknown action '{}'. Valid actions: definition, references, hover, symbols",
                action
            )),
        }
    }

    fn display_meta(&self) -> Option<ToolDisplayMeta> {
        Some(ToolDisplayMeta {
            verb: "LSP",
            label: "query",
            category: "Symbol",
            primary_arg_keys: &["action", "file_path"],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_tool() -> LspQueryTool {
        let lsp = Arc::new(Mutex::new(LspWrapper::new(None)));
        LspQueryTool::new(lsp)
    }

    fn make_args(pairs: &[(&str, serde_json::Value)]) -> HashMap<String, serde_json::Value> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect()
    }

    #[test]
    fn test_lsp_query_tool_metadata() {
        let tool = make_tool();
        assert_eq!(tool.name(), "lsp_query");
        assert!(tool.description().contains("definition"));
        assert!(tool.description().contains("references"));
        assert!(tool.description().contains("hover"));
        assert!(tool.description().contains("symbols"));

        let schema = tool.parameter_schema();
        let props = schema.get("properties").unwrap();
        assert!(props.get("action").is_some());
        assert!(props.get("file_path").is_some());
        assert!(props.get("line").is_some());
        assert!(props.get("character").is_some());
        assert!(props.get("query").is_some());

        let required = schema.get("required").unwrap().as_array().unwrap();
        assert_eq!(required.len(), 2);
        assert!(required.contains(&serde_json::json!("action")));
        assert!(required.contains(&serde_json::json!("file_path")));
    }

    #[tokio::test]
    async fn test_lsp_query_missing_action() {
        let tool = make_tool();
        let ctx = ToolContext::new("/tmp");
        let args = make_args(&[("file_path", serde_json::json!("test.rs"))]);
        let result = tool.execute(args, &ctx).await;
        assert!(!result.success);
        assert!(result.error.unwrap().contains("action"));
    }

    #[tokio::test]
    async fn test_lsp_query_missing_file_path() {
        let tool = make_tool();
        let ctx = ToolContext::new("/tmp");
        let args = make_args(&[("action", serde_json::json!("definition"))]);
        let result = tool.execute(args, &ctx).await;
        assert!(!result.success);
        assert!(result.error.unwrap().contains("file_path"));
    }

    #[tokio::test]
    async fn test_lsp_query_unknown_action() {
        let tool = make_tool();
        let ctx = ToolContext::new("/tmp");
        let args = make_args(&[
            ("action", serde_json::json!("invalid")),
            ("file_path", serde_json::json!("test.rs")),
        ]);
        let result = tool.execute(args, &ctx).await;
        assert!(!result.success);
        assert!(result.error.unwrap().contains("Unknown action"));
    }

    #[tokio::test]
    async fn test_lsp_query_definition_missing_position() {
        let tool = make_tool();
        let ctx = ToolContext::new("/tmp");
        let args = make_args(&[
            ("action", serde_json::json!("definition")),
            ("file_path", serde_json::json!("test.rs")),
        ]);
        let result = tool.execute(args, &ctx).await;
        assert!(!result.success);
        assert!(result.error.unwrap().contains("line"));
    }

    #[tokio::test]
    async fn test_lsp_query_references_missing_position() {
        let tool = make_tool();
        let ctx = ToolContext::new("/tmp");
        let args = make_args(&[
            ("action", serde_json::json!("references")),
            ("file_path", serde_json::json!("test.rs")),
        ]);
        let result = tool.execute(args, &ctx).await;
        assert!(!result.success);
        assert!(result.error.unwrap().contains("line"));
    }

    #[tokio::test]
    async fn test_lsp_query_hover_missing_position() {
        let tool = make_tool();
        let ctx = ToolContext::new("/tmp");
        let args = make_args(&[
            ("action", serde_json::json!("hover")),
            ("file_path", serde_json::json!("test.rs")),
        ]);
        let result = tool.execute(args, &ctx).await;
        assert!(!result.success);
        assert!(result.error.unwrap().contains("line"));
    }
}
