//! Conversation/chat display widget.
//!
//! Renders the conversation history with role-colored prefixes,
//! tool call summaries, thinking traces, system-reminder filtering,
//! collapsible tool results, and scroll support.

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Widget, Wrap},
};

use crate::app::{DisplayMessage, DisplayRole, DisplayToolCall};
use crate::formatters::display::strip_system_reminders;
use crate::formatters::markdown::MarkdownRenderer;
use crate::formatters::style_tokens;
use crate::formatters::tool_colors::{categorize_tool, format_tool_call_display, tool_color};
use crate::widgets::spinner::{COMPLETED_CHAR, CONTINUATION_CHAR};

/// Widget that renders the conversation log.
pub struct ConversationWidget<'a> {
    messages: &'a [DisplayMessage],
    scroll_offset: u16,
    terminal_width: u16,
    version: &'a str,
    working_dir: &'a str,
    mode: &'a str,
}

impl<'a> ConversationWidget<'a> {
    pub fn new(messages: &'a [DisplayMessage], scroll_offset: u16) -> Self {
        Self {
            messages,
            scroll_offset,
            terminal_width: 80,
            version: "0.1.0",
            working_dir: ".",
            mode: "NORMAL",
        }
    }

    pub fn terminal_width(mut self, width: u16) -> Self {
        self.terminal_width = width;
        self
    }

    pub fn version(mut self, version: &'a str) -> Self {
        self.version = version;
        self
    }

    pub fn working_dir(mut self, wd: &'a str) -> Self {
        self.working_dir = wd;
        self
    }

    pub fn mode(mut self, mode: &'a str) -> Self {
        self.mode = mode;
        self
    }

    /// Build styled lines from messages.
    fn build_lines(&self) -> Vec<Line<'a>> {
        let mut lines: Vec<Line> = Vec::new();

        if self.messages.is_empty() {
            // Welcome panel is now rendered as a separate widget (WelcomePanelWidget)
            return lines;
        }

        for msg in self.messages {
            // Filter system reminders from displayed content
            let content = strip_system_reminders(&msg.content);

            // Skip empty messages after filtering
            if content.is_empty() && msg.tool_call.is_none() {
                continue;
            }

            match msg.role {
                DisplayRole::Assistant => {
                    // Use markdown renderer for assistant messages
                    let md_lines = MarkdownRenderer::render(&content);
                    let mut leading_consumed = false;
                    for md_line in md_lines.into_iter() {
                        // Check if this line has non-empty content
                        let line_text: String = md_line
                            .spans
                            .iter()
                            .map(|s| s.content.to_string())
                            .collect();
                        let has_content = !line_text.trim().is_empty();

                        if !leading_consumed && has_content {
                            // First non-empty line gets ⏺ leading marker (green)
                            let mut spans = vec![Span::styled(
                                format!("{} ", COMPLETED_CHAR),
                                Style::default().fg(style_tokens::GREEN_BRIGHT),
                            )];
                            spans.extend(md_line.spans);
                            lines.push(Line::from(spans));
                            leading_consumed = true;
                        } else {
                            let mut spans = vec![Span::raw("  ".to_string())];
                            spans.extend(md_line.spans);
                            lines.push(Line::from(spans));
                        }
                    }
                }
                DisplayRole::User => {
                    let content_lines: Vec<&str> = content.lines().collect();
                    for (i, content_line) in content_lines.iter().enumerate() {
                        if i == 0 {
                            lines.push(Line::from(vec![
                                Span::styled(
                                    "> ".to_string(),
                                    Style::default()
                                        .fg(style_tokens::ACCENT)
                                        .add_modifier(Modifier::BOLD),
                                ),
                                Span::styled(
                                    content_line.to_string(),
                                    Style::default().fg(style_tokens::PRIMARY),
                                ),
                            ]));
                        } else {
                            lines.push(Line::from(vec![
                                Span::raw("  ".to_string()),
                                Span::styled(
                                    content_line.to_string(),
                                    Style::default().fg(style_tokens::PRIMARY),
                                ),
                            ]));
                        }
                    }
                }
                DisplayRole::System => {
                    let content_lines: Vec<&str> = content.lines().collect();
                    for (i, content_line) in content_lines.iter().enumerate() {
                        if i == 0 {
                            lines.push(Line::from(vec![
                                Span::styled(
                                    "! ".to_string(),
                                    Style::default()
                                        .fg(style_tokens::WARNING)
                                        .add_modifier(Modifier::ITALIC),
                                ),
                                Span::styled(
                                    content_line.to_string(),
                                    Style::default().fg(style_tokens::SUBTLE),
                                ),
                            ]));
                        } else {
                            lines.push(Line::from(vec![
                                Span::raw("  ".to_string()),
                                Span::styled(
                                    content_line.to_string(),
                                    Style::default().fg(style_tokens::SUBTLE),
                                ),
                            ]));
                        }
                    }
                }
                DisplayRole::Thinking => {
                    for (i, content_line) in content.lines().enumerate() {
                        let prefix = if i == 0 {
                            format!("  {} ", style_tokens::THINKING_ICON)
                        } else {
                            "    ".to_string()
                        };
                        lines.push(Line::from(vec![
                            Span::styled(prefix, Style::default().fg(style_tokens::THINKING_BG)),
                            Span::styled(
                                content_line.to_string(),
                                Style::default()
                                    .fg(style_tokens::THINKING_BG)
                                    .add_modifier(Modifier::ITALIC),
                            ),
                        ]));
                    }
                }
            }

            // Tool call summary with color coding
            if let Some(ref tc) = msg.tool_call {
                let tool_line = format_tool_call(tc);
                lines.push(tool_line);

                // Collapsible result lines
                if !tc.collapsed && !tc.result_lines.is_empty() {
                    for (i, result_line) in tc.result_lines.iter().enumerate() {
                        let prefix_char = if i == 0 {
                            format!("  {}  ", CONTINUATION_CHAR)
                        } else {
                            "     ".to_string()
                        };
                        lines.push(Line::from(vec![
                            Span::styled(prefix_char, Style::default().fg(style_tokens::SUBTLE)),
                            Span::styled(
                                result_line.clone(),
                                Style::default().fg(style_tokens::SUBTLE),
                            ),
                        ]));
                    }
                } else if tc.collapsed && !tc.result_lines.is_empty() {
                    // Show collapsed indicator
                    let count = tc.result_lines.len();
                    lines.push(Line::from(Span::styled(
                        format!(
                            "  {}  ({count} lines collapsed, press Ctrl+O to expand)",
                            CONTINUATION_CHAR
                        ),
                        Style::default()
                            .fg(style_tokens::SUBTLE)
                            .add_modifier(Modifier::ITALIC),
                    )));
                }

                // Nested tool calls (from subagent execution)
                for nested in &tc.nested_calls {
                    let nested_line = format_nested_tool_call(nested, 1);
                    lines.push(nested_line);
                }
            }

            // Blank line between messages
            lines.push(Line::from(""));
        }

        lines
    }
}

/// Format a tool call as a styled line with category color coding.
fn format_tool_call(tc: &DisplayToolCall) -> Line<'static> {
    let category = categorize_tool(&tc.name);
    let color = tool_color(category);

    let (icon, icon_color) = if tc.success {
        (COMPLETED_CHAR, style_tokens::GREEN_BRIGHT)
    } else {
        (COMPLETED_CHAR, style_tokens::ERROR)
    };

    let display = format_tool_call_display(&tc.name, &tc.arguments);

    Line::from(vec![
        Span::styled(format!("  {icon} "), Style::default().fg(icon_color)),
        Span::styled(
            display,
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ),
    ])
}

/// Format a nested tool call with tree indent.
fn format_nested_tool_call(tc: &DisplayToolCall, depth: usize) -> Line<'static> {
    let indent = "  ".repeat(depth + 1);
    let category = categorize_tool(&tc.name);
    let color = tool_color(category);

    let (icon, icon_color) = if tc.success {
        (COMPLETED_CHAR, style_tokens::GREEN_BRIGHT)
    } else {
        (COMPLETED_CHAR, style_tokens::ERROR)
    };

    let display = format_tool_call_display(&tc.name, &tc.arguments);

    Line::from(vec![
        Span::styled(
            format!("{indent}\u{2514}\u{2500} "),
            Style::default().fg(style_tokens::SUBTLE),
        ),
        Span::styled(format!("{icon} "), Style::default().fg(icon_color)),
        Span::styled(display, Style::default().fg(color)),
    ])
}

impl Widget for ConversationWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let lines = self.build_lines();
        let paragraph = Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .scroll((self.scroll_offset, 0));

        paragraph.render(area, buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::DisplayMessage;

    #[test]
    fn test_empty_conversation() {
        let msgs: Vec<DisplayMessage> = vec![];
        let widget = ConversationWidget::new(&msgs, 0);
        let lines = widget.build_lines();
        // Welcome panel is now a separate widget; empty conversation returns no lines
        assert!(lines.is_empty());
    }

    #[test]
    fn test_user_message_rendering() {
        let msgs = vec![DisplayMessage {
            role: DisplayRole::User,
            content: "Hello".into(),
            tool_call: None,
        }];
        let widget = ConversationWidget::new(&msgs, 0);
        let lines = widget.build_lines();
        assert!(lines.len() >= 2); // message + blank line
    }

    #[test]
    fn test_tool_call_display() {
        let msgs = vec![DisplayMessage {
            role: DisplayRole::Assistant,
            content: "Running tool...".into(),
            tool_call: Some(DisplayToolCall {
                name: "bash".into(),
                arguments: std::collections::HashMap::new(),
                summary: Some("ls -la".into()),
                success: true,
                collapsed: false,
                result_lines: vec!["file1.rs".into(), "file2.rs".into()],
                nested_calls: vec![],
            }),
        }];
        let widget = ConversationWidget::new(&msgs, 0);
        let lines = widget.build_lines();
        // message line + tool line + 2 result lines + blank
        assert!(lines.len() >= 5);
    }

    #[test]
    fn test_system_reminder_filtered() {
        let msgs = vec![DisplayMessage {
            role: DisplayRole::Assistant,
            content: "Hello<system-reminder>secret</system-reminder> world".into(),
            tool_call: None,
        }];
        let widget = ConversationWidget::new(&msgs, 0);
        let lines = widget.build_lines();
        // Should not contain "secret"
        let text: String = lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .map(|s| s.content.to_string())
            .collect();
        assert!(!text.contains("secret"));
        assert!(text.contains("Hello"));
        assert!(text.contains("world"));
    }

    #[test]
    fn test_collapsed_tool_result() {
        let msgs = vec![DisplayMessage {
            role: DisplayRole::Assistant,
            content: "Done".into(),
            tool_call: Some(DisplayToolCall {
                name: "read_file".into(),
                arguments: std::collections::HashMap::new(),
                summary: Some("Read 100 lines".into()),
                success: true,
                collapsed: true,
                result_lines: vec!["line1".into(), "line2".into()],
                nested_calls: vec![],
            }),
        }];
        let widget = ConversationWidget::new(&msgs, 0);
        let lines = widget.build_lines();
        let text: String = lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .map(|s| s.content.to_string())
            .collect();
        assert!(text.contains("collapsed"));
    }

    #[test]
    fn test_nested_tool_calls() {
        let msgs = vec![DisplayMessage {
            role: DisplayRole::Assistant,
            content: "".into(),
            tool_call: Some(DisplayToolCall {
                name: "spawn_subagent".into(),
                arguments: std::collections::HashMap::new(),
                summary: Some("Exploring codebase".into()),
                success: true,
                collapsed: false,
                result_lines: vec![],
                nested_calls: vec![DisplayToolCall {
                    name: "read_file".into(),
                    arguments: std::collections::HashMap::new(),
                    summary: Some("src/main.rs".into()),
                    success: true,
                    collapsed: false,
                    result_lines: vec![],
                    nested_calls: vec![],
                }],
            }),
        }];
        let widget = ConversationWidget::new(&msgs, 0);
        let lines = widget.build_lines();
        let text: String = lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .map(|s| s.content.to_string())
            .collect();
        // Tool calls now use format_tool_call_display: Spawn(subagent), Read(file)
        assert!(text.contains("Spawn"));
        assert!(text.contains("Read"));
    }
}
