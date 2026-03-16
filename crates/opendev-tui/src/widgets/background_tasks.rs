//! Background task panel overlay widget.
//!
//! Renders a centered overlay showing the status of background tasks
//! (running, completed, failed, killed) with task details.

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Widget, Wrap},
};

use crate::formatters::style_tokens;

/// Display item for a single background task.
#[derive(Debug, Clone)]
pub struct TaskDisplayItem {
    pub task_id: String,
    pub description: String,
    pub state: String,
    pub runtime_secs: f64,
}

/// Overlay widget that renders background task status.
pub struct BackgroundTaskPanel<'a> {
    tasks: &'a [TaskDisplayItem],
    running_count: usize,
    total_count: usize,
}

impl<'a> BackgroundTaskPanel<'a> {
    pub fn new(tasks: &'a [TaskDisplayItem], running_count: usize, total_count: usize) -> Self {
        Self {
            tasks,
            running_count,
            total_count,
        }
    }

    /// Compute the panel rectangle centered within the given area.
    pub fn panel_rect(area: Rect) -> Rect {
        let width = area.width.clamp(30, 70);
        let height = area.height.clamp(5, 20);
        let x = area.x + (area.width.saturating_sub(width)) / 2;
        let y = area.y + (area.height.saturating_sub(height)) / 2;
        Rect::new(x, y, width, height)
    }
}

impl Widget for BackgroundTaskPanel<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let panel = Self::panel_rect(area);

        // Clear the area behind the panel
        Clear.render(panel, buf);

        let title = format!(
            " Background Tasks ({} running / {} total) ",
            self.running_count, self.total_count
        );

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(style_tokens::CYAN))
            .title(Span::styled(
                title,
                Style::default()
                    .fg(style_tokens::CYAN)
                    .add_modifier(Modifier::BOLD),
            ));

        let mut lines: Vec<Line> = Vec::new();

        if self.tasks.is_empty() {
            lines.push(Line::from(Span::styled(
                "No background tasks.",
                Style::default().fg(style_tokens::SUBTLE),
            )));
        } else {
            // Show up to 10 most recent tasks
            let start = self.tasks.len().saturating_sub(10);
            for task in &self.tasks[start..] {
                let state_color = match task.state.as_str() {
                    "running" => style_tokens::GREEN_BRIGHT,
                    "completed" => style_tokens::SUCCESS,
                    "failed" => style_tokens::ERROR,
                    "killed" => style_tokens::WARNING,
                    _ => style_tokens::SUBTLE,
                };

                let desc = if task.description.len() > 50 {
                    format!("{}...", &task.description[..50])
                } else {
                    task.description.clone()
                };

                let id_short = if task.task_id.len() > 8 {
                    &task.task_id[..8]
                } else {
                    &task.task_id
                };

                lines.push(Line::from(vec![
                    Span::styled(
                        format!("[{id_short}] "),
                        Style::default().fg(style_tokens::SUBTLE),
                    ),
                    Span::styled(
                        format!("{desc} "),
                        Style::default().fg(style_tokens::PRIMARY),
                    ),
                    Span::styled(
                        format!("[{}]", task.state),
                        Style::default().fg(state_color),
                    ),
                    Span::styled(
                        format!(" {:.1}s", task.runtime_secs),
                        Style::default().fg(style_tokens::SUBTLE),
                    ),
                ]));
            }
        }

        // Footer
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "Ctrl+B: close  |  /tasks: details  |  /kill <id>: stop",
            Style::default()
                .fg(style_tokens::SUBTLE)
                .add_modifier(Modifier::ITALIC),
        )));

        let paragraph = Paragraph::new(lines).block(block).wrap(Wrap { trim: true });
        paragraph.render(panel, buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_panel() {
        let tasks: Vec<TaskDisplayItem> = vec![];
        let _panel = BackgroundTaskPanel::new(&tasks, 0, 0);
    }

    #[test]
    fn test_panel_with_tasks() {
        let tasks = vec![
            TaskDisplayItem {
                task_id: "abc123".to_string(),
                description: "Running tests".to_string(),
                state: "running".to_string(),
                runtime_secs: 5.2,
            },
            TaskDisplayItem {
                task_id: "def456".to_string(),
                description: "Build project".to_string(),
                state: "completed".to_string(),
                runtime_secs: 12.0,
            },
        ];
        let _panel = BackgroundTaskPanel::new(&tasks, 1, 2);
    }

    #[test]
    fn test_panel_rect_centering() {
        let area = Rect::new(0, 0, 100, 40);
        let panel = BackgroundTaskPanel::panel_rect(area);
        assert!(panel.x > 0);
        assert!(panel.y > 0);
        assert!(panel.width <= 70);
        assert!(panel.height <= 20);
    }
}
