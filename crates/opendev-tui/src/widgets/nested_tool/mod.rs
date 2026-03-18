//! Nested tool display widget for subagent progress.
//!
//! Renders a tree-structured view of subagent tool calls,
//! showing which subagent is running, its active tool calls,
//! and completion status with tree connectors (similar to the
//! Python `NestedToolMixin`).

mod state;

pub use state::{CompletedToolCall, NestedToolCallState, SubagentDisplayState};

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Widget, Wrap},
};

use crate::formatters::style_tokens;
use crate::formatters::tool_registry::format_tool_call_parts_with_wd;

/// Tree connector characters (UTF-8 box drawing).
const TREE_BRANCH: &str = "\u{251c}\u{2500}";
const TREE_LAST: &str = "\u{2514}\u{2500}";
const TREE_VERTICAL: &str = "\u{2502}";

/// Spinner characters for cycling animation.
const SPINNER_CHARS: &[char] = &['\u{25cb}', '\u{25cf}', '\u{25cb}', '\u{25ce}'];

/// Green gradient colors for spinner animation.
const GREEN_GRADIENT: &[Color] = &[
    Color::Rgb(0, 180, 90),
    Color::Rgb(0, 200, 100),
    Color::Rgb(0, 220, 110),
    Color::Rgb(0, 240, 120),
];

/// Widget that renders the nested subagent tool display.
pub struct NestedToolWidget<'a> {
    subagents: &'a [SubagentDisplayState],
    working_dir: Option<&'a str>,
}

impl<'a> NestedToolWidget<'a> {
    pub fn new(subagents: &'a [SubagentDisplayState]) -> Self {
        Self {
            subagents,
            working_dir: None,
        }
    }

    pub fn working_dir(mut self, wd: &'a str) -> Self {
        self.working_dir = Some(wd);
        self
    }
}

impl Widget for NestedToolWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if self.subagents.is_empty() {
            return;
        }

        let block = Block::default()
            .borders(Borders::TOP)
            .border_style(Style::default().fg(style_tokens::BORDER))
            .title(Span::styled(
                " Subagents ",
                Style::default()
                    .fg(style_tokens::HEADING_1)
                    .add_modifier(Modifier::BOLD),
            ));

        let mut lines: Vec<Line> = Vec::new();

        for (i, subagent) in self.subagents.iter().enumerate() {
            let is_last = i == self.subagents.len() - 1;

            // Subagent header line
            let connector = if is_last { TREE_LAST } else { TREE_BRANCH };
            let (status_icon, status_color) = if subagent.finished {
                if subagent.success {
                    ("\u{23fa}", style_tokens::SUCCESS)
                } else {
                    ("\u{23fa}", style_tokens::ERROR)
                }
            } else {
                let spinner_idx = subagent.tick % SPINNER_CHARS.len();
                let ch = SPINNER_CHARS[spinner_idx];
                let color_idx = subagent.tick % GREEN_GRADIENT.len();
                (&*format!("{ch}"), GREEN_GRADIENT[color_idx])
            };

            // Build a static string for the spinner character when not finished
            let spinner_string;
            let status_str = if subagent.finished {
                status_icon
            } else {
                let spinner_idx = subagent.tick % SPINNER_CHARS.len();
                spinner_string = SPINNER_CHARS[spinner_idx].to_string();
                &spinner_string
            };

            let elapsed = subagent.elapsed_secs();
            let task_text = crate::formatters::replace_wd_in_text(&subagent.task, self.working_dir);
            let task_preview = if task_text.len() > 60 {
                format!("{}...", &task_text[..60])
            } else {
                task_text
            };

            // Format elapsed as Xm Ys or Xs
            let elapsed_str = if elapsed >= 60 {
                format!("{}m {}s", elapsed / 60, elapsed % 60)
            } else {
                format!("{elapsed}s")
            };

            // Format token count
            let token_str = if subagent.token_count > 0 {
                let k = subagent.token_count as f64 / 1000.0;
                format!(" \u{00b7} {k:.1}k tokens")
            } else {
                String::new()
            };

            // Build stats suffix
            let stats = format!(
                " ({} tool uses{} \u{00b7} {})",
                subagent.tool_call_count, token_str, elapsed_str
            );

            lines.push(Line::from(vec![
                Span::styled(
                    format!(" {connector} "),
                    Style::default().fg(style_tokens::SUBTLE),
                ),
                Span::styled(format!("{status_str} "), Style::default().fg(status_color)),
                Span::styled(
                    subagent.name.clone(),
                    Style::default()
                        .fg(style_tokens::CYAN)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!(": {task_preview}"),
                    Style::default().fg(style_tokens::SUBTLE),
                ),
                Span::styled(stats, Style::default().fg(style_tokens::SUBTLE)),
            ]));

            // Show active tool calls
            let vertical = if is_last {
                "   "
            } else {
                &format!(" {TREE_VERTICAL}  ")
            };
            let active_count = subagent.active_tools.len();

            for (j, tool_state) in subagent.active_tools.values().enumerate() {
                let tool_is_last = j == active_count - 1 && subagent.completed_tools.is_empty();
                let tool_connector = if tool_is_last { TREE_LAST } else { TREE_BRANCH };
                let color_idx = tool_state.tick % GREEN_GRADIENT.len();
                let spinner_idx = tool_state.tick % SPINNER_CHARS.len();
                let spinner_ch = SPINNER_CHARS[spinner_idx];
                let tool_elapsed = tool_state.started_at.elapsed().as_secs();
                let (verb, arg) = format_tool_call_parts_with_wd(
                    &tool_state.tool_name,
                    &tool_state.args,
                    self.working_dir,
                );

                lines.push(Line::from(vec![
                    Span::styled(
                        format!(" {vertical}{tool_connector} "),
                        Style::default().fg(style_tokens::SUBTLE),
                    ),
                    Span::styled(
                        format!("{spinner_ch} "),
                        Style::default().fg(GREEN_GRADIENT[color_idx]),
                    ),
                    Span::styled(verb, Style::default().fg(style_tokens::SUBTLE)),
                    Span::styled(
                        format!("({arg})"),
                        Style::default().fg(style_tokens::SUBTLE),
                    ),
                    Span::styled(
                        format!(" ({tool_elapsed}s)"),
                        Style::default().fg(style_tokens::SUBTLE),
                    ),
                ]));
            }

            // Show last few completed tools (max 3)
            let completed_start = subagent.completed_tools.len().saturating_sub(3);
            let visible_completed = &subagent.completed_tools[completed_start..];
            for (j, completed) in visible_completed.iter().enumerate() {
                let is_last_tool = j == visible_completed.len() - 1;
                let tool_connector = if is_last_tool { TREE_LAST } else { TREE_BRANCH };
                let (icon, color) = if completed.success {
                    ("\u{23fa}", style_tokens::SUCCESS)
                } else {
                    ("\u{23fa}", style_tokens::ERROR)
                };
                let (verb, arg) = format_tool_call_parts_with_wd(
                    &completed.tool_name,
                    &completed.args,
                    self.working_dir,
                );

                lines.push(Line::from(vec![
                    Span::styled(
                        format!(" {vertical}{tool_connector} "),
                        Style::default().fg(style_tokens::SUBTLE),
                    ),
                    Span::styled(format!("{icon} "), Style::default().fg(color)),
                    Span::styled(verb, Style::default().fg(style_tokens::SUBTLE)),
                    Span::styled(
                        format!("({arg})"),
                        Style::default().fg(style_tokens::SUBTLE),
                    ),
                    Span::styled(
                        format!(" ({}s)", completed.elapsed.as_secs()),
                        Style::default().fg(style_tokens::SUBTLE),
                    ),
                ]));
            }

            // Show hidden count if there are more completed tools
            if completed_start > 0 {
                lines.push(Line::from(Span::styled(
                    format!(" {vertical}   +{completed_start} more tool uses"),
                    Style::default()
                        .fg(style_tokens::SUBTLE)
                        .add_modifier(Modifier::ITALIC),
                )));
            }

            // Show shallow warning if present
            if let Some(ref warning) = subagent.shallow_warning {
                lines.push(Line::from(Span::styled(
                    format!(" {vertical}   {warning}"),
                    Style::default().fg(style_tokens::WARNING),
                )));
            }
        }

        let paragraph = Paragraph::new(lines).block(block).wrap(Wrap { trim: true });

        paragraph.render(area, buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn test_empty_widget() {
        let subagents: Vec<SubagentDisplayState> = vec![];
        let _widget = NestedToolWidget::new(&subagents);
    }

    #[test]
    fn test_widget_with_active_subagent() {
        let mut state =
            SubagentDisplayState::new("id-1".into(), "Explore".into(), "Find TODOs".into());
        state.add_tool_call("read_file".into(), "tc-1".into(), HashMap::new());
        let subagents = vec![state];
        let _widget = NestedToolWidget::new(&subagents);
    }

    #[test]
    fn test_widget_with_finished_subagent() {
        let mut state =
            SubagentDisplayState::new("id-2".into(), "Planner".into(), "Create plan".into());
        state.add_tool_call("read_file".into(), "tc-1".into(), HashMap::new());
        state.complete_tool_call("tc-1", true);
        state.add_tool_call("write_file".into(), "tc-2".into(), HashMap::new());
        state.complete_tool_call("tc-2", true);
        state.finish(true, "Plan created".into(), 2, None);
        let subagents = vec![state];
        let _widget = NestedToolWidget::new(&subagents);
    }
}
