//! User input/prompt widget.

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Widget},
};

use crate::formatters::style_tokens;

/// Widget for the user input area.
pub struct InputWidget<'a> {
    buffer: &'a str,
    cursor: usize,
    agent_active: bool,
    mode: &'a str,
}

impl<'a> InputWidget<'a> {
    pub fn new(buffer: &'a str, cursor: usize, agent_active: bool, mode: &'a str) -> Self {
        Self {
            buffer,
            cursor,
            agent_active,
            mode,
        }
    }
}

impl Widget for InputWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.height < 2 {
            return;
        }

        let accent = if self.agent_active {
            style_tokens::GOLD
        } else {
            style_tokens::ACCENT
        };

        let placeholder = if self.agent_active {
            "Agent is thinking... (ESC to interrupt)"
        } else {
            "Type a message..."
        };

        // Row 0: separator line with embedded mode indicator
        // e.g. "── Normal (Shift+Tab) ──────────"
        let mode_label = match self.mode {
            "NORMAL" => "Normal",
            "PLAN" => "Plan",
            other => other,
        };
        let mode_text = format!(" {mode_label} ");
        let hint_text = "(Shift+Tab) ";
        let prefix_dashes = 2; // "── " before mode label
        let used = prefix_dashes + mode_text.len() + hint_text.len();
        let remaining_dashes = (area.width as usize).saturating_sub(used);

        let sep_style = Style::default().fg(accent);
        let sep_line = Line::from(vec![
            Span::styled("── ", sep_style),
            Span::styled(
                mode_text,
                Style::default().fg(accent).add_modifier(Modifier::BOLD),
            ),
            Span::styled(hint_text, Style::default().fg(style_tokens::GREY)),
            Span::styled("─".repeat(remaining_dashes), sep_style),
        ]);
        buf.set_line(area.left(), area.top(), &sep_line, area.width);

        // Row 1: "> " prefix + input text
        let text_area = Rect {
            x: area.x,
            y: area.y + 1,
            width: area.width,
            height: 1,
        };

        let prefix = Span::styled(
            "> ".to_string(),
            Style::default().fg(accent).add_modifier(Modifier::BOLD),
        );

        let content = if self.buffer.is_empty() {
            vec![
                prefix,
                Span::styled(placeholder, Style::default().fg(style_tokens::SUBTLE)),
            ]
        } else {
            let before = &self.buffer[..self.cursor];
            let cursor_char = self.buffer.get(self.cursor..self.cursor + 1).unwrap_or(" ");
            let after = if self.cursor < self.buffer.len() {
                &self.buffer[self.cursor + 1..]
            } else {
                ""
            };

            vec![
                prefix,
                Span::raw(before.to_string()),
                Span::styled(
                    cursor_char.to_string(),
                    Style::default().fg(Color::Black).bg(Color::White),
                ),
                Span::raw(after.to_string()),
            ]
        };

        Paragraph::new(Line::from(content)).render(text_area, buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_input_widget_creation() {
        let _widget = InputWidget::new("hello", 3, false, "NORMAL");
    }

    #[test]
    fn test_input_widget_empty() {
        let _widget = InputWidget::new("", 0, false, "NORMAL");
    }

    #[test]
    fn test_input_widget_agent_active() {
        let _widget = InputWidget::new("", 0, true, "NORMAL");
    }
}
