//! Diff preview tool — generate and display unified diffs between file versions.
//!
//! Provides diff generation for showing file changes in a human-readable format.
//! Uses a simple LCS-based diff algorithm to produce unified-style output.

use std::collections::HashMap;

use opendev_tools_core::{BaseTool, ToolContext, ToolDisplayMeta, ToolResult};

/// Tool for generating diff previews.
#[derive(Debug)]
pub struct DiffPreviewTool;

#[async_trait::async_trait]
impl BaseTool for DiffPreviewTool {
    fn name(&self) -> &str {
        "diff_preview"
    }

    fn description(&self) -> &str {
        "Generate a unified diff between two versions of a file's content. \
         Shows additions, removals, and change statistics."
    }

    fn parameter_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "Path to the file (used in diff header)"
                },
                "original": {
                    "type": "string",
                    "description": "Original file content"
                },
                "modified": {
                    "type": "string",
                    "description": "Modified file content"
                },
                "context_lines": {
                    "type": "integer",
                    "description": "Number of context lines to show (default: 3)"
                }
            },
            "required": ["file_path", "original", "modified"]
        })
    }

    async fn execute(
        &self,
        args: HashMap<String, serde_json::Value>,
        _ctx: &ToolContext,
    ) -> ToolResult {
        let file_path = match args.get("file_path").and_then(|v| v.as_str()) {
            Some(p) => p,
            None => return ToolResult::fail("file_path is required"),
        };

        let original = match args.get("original").and_then(|v| v.as_str()) {
            Some(o) => o,
            None => return ToolResult::fail("original content is required"),
        };

        let modified = match args.get("modified").and_then(|v| v.as_str()) {
            Some(m) => m,
            None => return ToolResult::fail("modified content is required"),
        };

        let context_lines = args
            .get("context_lines")
            .and_then(|v| v.as_u64())
            .unwrap_or(3) as usize;

        let original_lines: Vec<&str> = original.split('\n').collect();
        let modified_lines: Vec<&str> = modified.split('\n').collect();

        // Generate unified diff
        let diff_output = unified_diff(
            &original_lines,
            &modified_lines,
            &format!("a/{file_path}"),
            &format!("b/{file_path}"),
            context_lines,
        );

        // Calculate stats
        let mut added = 0usize;
        let mut removed = 0usize;
        for line in diff_output.lines() {
            if line.starts_with('+') && !line.starts_with("+++") {
                added += 1;
            } else if line.starts_with('-') && !line.starts_with("---") {
                removed += 1;
            }
        }

        let mut output_parts = Vec::new();
        output_parts.push(format!("File: {file_path}"));
        output_parts.push("\u{2500}".repeat(50));

        if diff_output.is_empty() {
            output_parts.push("No changes detected.".to_string());
        } else {
            output_parts.push(diff_output);
        }

        output_parts.push("\u{2500}".repeat(50));
        output_parts.push(format!("Changes: +{added} -{removed}"));

        let output = output_parts.join("\n");

        let mut metadata = HashMap::new();
        metadata.insert("lines_added".into(), serde_json::json!(added));
        metadata.insert("lines_removed".into(), serde_json::json!(removed));
        metadata.insert("lines_changed".into(), serde_json::json!(added + removed));
        metadata.insert("file_path".into(), serde_json::json!(file_path));

        ToolResult::ok_with_metadata(output, metadata)
    }

    fn display_meta(&self) -> Option<ToolDisplayMeta> {
        Some(ToolDisplayMeta {
            verb: "Diff",
            label: "file",
            category: "FileWrite",
            primary_arg_keys: &["file_path"],
        })
    }
}

/// An edit operation.
#[derive(Debug, Clone, PartialEq)]
enum Edit {
    Keep(usize, usize), // (original_idx, modified_idx)
    Remove(usize),      // original_idx
    Add(usize),         // modified_idx
}

/// Compute the edit script between two sequences using LCS.
fn compute_edit_script(original: &[&str], modified: &[&str]) -> Vec<Edit> {
    let n = original.len();
    let m = modified.len();

    // Build LCS table
    let mut table = vec![vec![0u32; m + 1]; n + 1];
    for i in 1..=n {
        for j in 1..=m {
            if original[i - 1] == modified[j - 1] {
                table[i][j] = table[i - 1][j - 1] + 1;
            } else {
                table[i][j] = table[i - 1][j].max(table[i][j - 1]);
            }
        }
    }

    // Backtrack to get edit script
    let mut edits = Vec::new();
    let mut i = n;
    let mut j = m;

    while i > 0 || j > 0 {
        if i > 0 && j > 0 && original[i - 1] == modified[j - 1] {
            edits.push(Edit::Keep(i - 1, j - 1));
            i -= 1;
            j -= 1;
        } else if j > 0 && (i == 0 || table[i][j - 1] >= table[i - 1][j]) {
            edits.push(Edit::Add(j - 1));
            j -= 1;
        } else {
            edits.push(Edit::Remove(i - 1));
            i -= 1;
        }
    }

    edits.reverse();
    edits
}

/// Generate a unified diff between two sets of lines.
fn unified_diff(
    original: &[&str],
    modified: &[&str],
    from_file: &str,
    to_file: &str,
    context: usize,
) -> String {
    let edits = compute_edit_script(original, modified);

    // Check if there are any actual changes
    if edits.iter().all(|e| matches!(e, Edit::Keep(_, _))) {
        return String::new();
    }

    let mut output = Vec::new();
    output.push(format!("--- {from_file}"));
    output.push(format!("+++ {to_file}"));

    // Identify indices in the edit array that represent changes
    let change_indices: Vec<usize> = edits
        .iter()
        .enumerate()
        .filter(|(_, e)| !matches!(e, Edit::Keep(_, _)))
        .map(|(i, _)| i)
        .collect();

    if change_indices.is_empty() {
        return String::new();
    }

    // Group nearby changes into hunks
    let mut groups: Vec<(usize, usize)> = Vec::new(); // (first_change_idx, last_change_idx) in edits
    let mut group_start = change_indices[0];
    let mut group_end = change_indices[0];

    for &idx in &change_indices[1..] {
        if idx - group_end <= context * 2 + 1 {
            group_end = idx;
        } else {
            groups.push((group_start, group_end));
            group_start = idx;
            group_end = idx;
        }
    }
    groups.push((group_start, group_end));

    // Render each group as a hunk
    for (start, end) in groups {
        let hunk_start = start.saturating_sub(context);
        let hunk_end = (end + context + 1).min(edits.len());

        let mut orig_start = 0usize;
        let mut mod_start = 0usize;
        let mut orig_count = 0usize;
        let mut mod_count = 0usize;
        let mut lines = Vec::new();
        let mut first = true;

        for edit in &edits[hunk_start..hunk_end] {
            match edit {
                Edit::Keep(oi, mi) => {
                    if first {
                        orig_start = *oi;
                        mod_start = *mi;
                        first = false;
                    }
                    lines.push(format!(" {}", original[*oi]));
                    orig_count += 1;
                    mod_count += 1;
                }
                Edit::Remove(oi) => {
                    if first {
                        orig_start = *oi;
                        mod_start = if *oi > 0 { *oi } else { 0 };
                        first = false;
                    }
                    lines.push(format!("-{}", original[*oi]));
                    orig_count += 1;
                }
                Edit::Add(mi) => {
                    if first {
                        orig_start = if *mi > 0 { *mi } else { 0 };
                        mod_start = *mi;
                        first = false;
                    }
                    lines.push(format!("+{}", modified[*mi]));
                    mod_count += 1;
                }
            }
        }

        output.push(format!(
            "@@ -{},{} +{},{} @@",
            orig_start + 1,
            orig_count,
            mod_start + 1,
            mod_count
        ));
        output.extend(lines);
    }

    output.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_args(pairs: &[(&str, serde_json::Value)]) -> HashMap<String, serde_json::Value> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect()
    }

    #[test]
    fn test_compute_edit_script_identical() {
        let original = vec!["a", "b", "c"];
        let modified = vec!["a", "b", "c"];
        let edits = compute_edit_script(&original, &modified);
        assert!(edits.iter().all(|e| matches!(e, Edit::Keep(_, _))));
    }

    #[test]
    fn test_compute_edit_script_addition() {
        let original = vec!["a", "c"];
        let modified = vec!["a", "b", "c"];
        let edits = compute_edit_script(&original, &modified);
        let adds: Vec<_> = edits.iter().filter(|e| matches!(e, Edit::Add(_))).collect();
        assert_eq!(adds.len(), 1);
    }

    #[test]
    fn test_compute_edit_script_removal() {
        let original = vec!["a", "b", "c"];
        let modified = vec!["a", "c"];
        let edits = compute_edit_script(&original, &modified);
        let removes: Vec<_> = edits
            .iter()
            .filter(|e| matches!(e, Edit::Remove(_)))
            .collect();
        assert_eq!(removes.len(), 1);
    }

    #[test]
    fn test_unified_diff_with_changes() {
        let original = vec!["line1", "line2", "line3", "line4"];
        let modified = vec!["line1", "line2 modified", "line3", "line4"];
        let diff = unified_diff(&original, &modified, "a/test.txt", "b/test.txt", 3);
        assert!(diff.contains("--- a/test.txt"));
        assert!(diff.contains("+++ b/test.txt"));
        assert!(diff.contains("-line2"));
        assert!(diff.contains("+line2 modified"));
    }

    #[test]
    fn test_unified_diff_no_changes() {
        let lines = vec!["a", "b", "c"];
        let diff = unified_diff(&lines, &lines, "a/f.txt", "b/f.txt", 3);
        assert!(diff.is_empty());
    }

    #[tokio::test]
    async fn test_diff_preview_missing_args() {
        let tool = DiffPreviewTool;
        let ctx = ToolContext::new("/tmp");

        let result = tool.execute(HashMap::new(), &ctx).await;
        assert!(!result.success);

        let args = make_args(&[("file_path", serde_json::json!("test.txt"))]);
        let result = tool.execute(args, &ctx).await;
        assert!(!result.success);
    }

    #[tokio::test]
    async fn test_diff_preview_with_changes() {
        let tool = DiffPreviewTool;
        let ctx = ToolContext::new("/tmp");
        let args = make_args(&[
            ("file_path", serde_json::json!("test.rs")),
            (
                "original",
                serde_json::json!("fn main() {\n    println!(\"hello\");\n}"),
            ),
            (
                "modified",
                serde_json::json!("fn main() {\n    println!(\"world\");\n}"),
            ),
        ]);
        let result = tool.execute(args, &ctx).await;
        assert!(result.success);
        let output = result.output.unwrap();
        assert!(output.contains("test.rs"));
        assert!(output.contains("Changes:"));
    }

    #[tokio::test]
    async fn test_diff_preview_no_changes() {
        let tool = DiffPreviewTool;
        let ctx = ToolContext::new("/tmp");
        let args = make_args(&[
            ("file_path", serde_json::json!("same.txt")),
            ("original", serde_json::json!("same content")),
            ("modified", serde_json::json!("same content")),
        ]);
        let result = tool.execute(args, &ctx).await;
        assert!(result.success);
        let output = result.output.unwrap();
        assert!(output.contains("+0") && output.contains("-0"));
    }
}
