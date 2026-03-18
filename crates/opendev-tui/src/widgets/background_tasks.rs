//! Background task panel overlay widget.
//!
//! Renders a centered overlay showing the status of background tasks
//! (running, completed, failed, killed) with task details.
//!
//! Also provides the unified Task Watcher panel (Alt+B) that merges
//! both agent tasks and process tasks into a two-pane overlay.

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
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

// --- Unified Task Watcher types ---

/// Kind of unified task (agent or process).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnifiedTaskKind {
    Agent,
    Process,
}

/// A single unified task item for the watcher panel.
#[derive(Debug, Clone)]
pub struct UnifiedTaskItem {
    pub task_id: String,
    pub kind: UnifiedTaskKind,
    pub description: String,
    pub state: String,
    pub runtime_secs: f64,
    pub tool_count: Option<usize>,
    pub cost_usd: Option<f64>,
    pub current_tool: Option<String>,
    pub pid: Option<u32>,
    pub has_output: bool,
}

/// Which pane has focus in the watcher.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskWatcherFocus {
    List,
    Output,
}

/// Two-pane task watcher overlay widget.
pub struct TaskWatcherPanel<'a> {
    tasks: &'a [UnifiedTaskItem],
    selected: usize,
    focus: TaskWatcherFocus,
    output_lines: &'a [String],
    output_scroll: u16,
    spinner_tick: usize,
}

impl<'a> TaskWatcherPanel<'a> {
    pub fn new(
        tasks: &'a [UnifiedTaskItem],
        selected: usize,
        focus: TaskWatcherFocus,
        output_lines: &'a [String],
        output_scroll: u16,
        spinner_tick: usize,
    ) -> Self {
        Self {
            tasks,
            selected,
            focus,
            output_lines,
            output_scroll,
            spinner_tick,
        }
    }

    /// Compute the panel rectangle (~85% width, ~80% height) centered in area.
    pub fn panel_rect(area: Rect) -> Rect {
        let width = ((area.width as f32 * 0.85) as u16).clamp(40, area.width);
        let height = ((area.height as f32 * 0.80) as u16).clamp(12, area.height);
        let x = area.x + (area.width.saturating_sub(width)) / 2;
        let y = area.y + (area.height.saturating_sub(height)) / 2;
        Rect::new(x, y, width, height)
    }
}

impl Widget for TaskWatcherPanel<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let panel = Self::panel_rect(area);
        Clear.render(panel, buf);

        let running = self.tasks.iter().filter(|t| t.state == "running").count();
        let total = self.tasks.len();

        let spinner_frames: &[char] = &[
            '\u{280b}', '\u{2819}', '\u{2839}', '\u{2838}', '\u{283c}', '\u{2834}', '\u{2826}',
            '\u{2827}', '\u{2807}', '\u{280f}',
        ];
        let spinner = if running > 0 {
            format!(
                "{} ",
                spinner_frames[self.spinner_tick % spinner_frames.len()]
            )
        } else {
            String::new()
        };

        let title = format!(" {spinner}Task Watcher ({running} running / {total} total) ");

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(style_tokens::CYAN))
            .title(Span::styled(
                title,
                Style::default()
                    .fg(style_tokens::CYAN)
                    .add_modifier(Modifier::BOLD),
            ));

        // Inner area after borders
        let inner = block.inner(panel);
        block.render(panel, buf);

        if inner.height < 4 || inner.width < 10 {
            return;
        }

        // Split inner into list (~40%) and detail (~60%) with a separator line
        let list_height = (inner.height * 2 / 5).max(3);
        let detail_height = inner.height.saturating_sub(list_height + 1); // 1 for separator

        let list_area = Rect::new(inner.x, inner.y, inner.width, list_height);
        let sep_y = inner.y + list_height;
        let detail_area = Rect::new(inner.x, sep_y + 1, inner.width, detail_height);

        // --- Render task list ---
        let active_bg = Color::Rgb(31, 45, 58);
        let list_focus = self.focus == TaskWatcherFocus::List;

        if self.tasks.is_empty() {
            let line = Line::from(Span::styled(
                "  No background tasks.",
                Style::default().fg(style_tokens::SUBTLE),
            ));
            buf.set_line(list_area.x, list_area.y, &line, list_area.width);
        } else {
            for (i, task) in self.tasks.iter().enumerate() {
                if i as u16 >= list_height {
                    break;
                }
                let is_selected = i == self.selected;
                let y = list_area.y + i as u16;

                let state_color = match task.state.as_str() {
                    "running" => style_tokens::GREEN_BRIGHT,
                    "completed" => style_tokens::SUCCESS,
                    "failed" => style_tokens::ERROR,
                    "killed" => style_tokens::WARNING,
                    _ => style_tokens::SUBTLE,
                };

                let pointer = if is_selected && list_focus {
                    "\u{25b8}"
                } else {
                    " "
                };
                let kind_char = match task.kind {
                    UnifiedTaskKind::Agent => "A",
                    UnifiedTaskKind::Process => "$",
                };

                let id_short = if task.task_id.len() > 7 {
                    &task.task_id[..7]
                } else {
                    &task.task_id
                };

                let desc = if task.description.len() > 35 {
                    format!("{}...", &task.description[..32])
                } else {
                    task.description.clone()
                };

                let line = Line::from(vec![
                    Span::styled(
                        format!(" {pointer} "),
                        if is_selected && list_focus {
                            Style::default()
                                .fg(style_tokens::CYAN)
                                .add_modifier(Modifier::BOLD)
                        } else {
                            Style::default().fg(style_tokens::SUBTLE)
                        },
                    ),
                    Span::styled(
                        format!("{kind_char} "),
                        Style::default()
                            .fg(style_tokens::GREY)
                            .add_modifier(Modifier::BOLD),
                    ),
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
                ]);

                if is_selected {
                    // Fill background for selected row
                    for x in list_area.x..list_area.x + list_area.width {
                        buf[(x, y)].set_bg(active_bg);
                    }
                }
                buf.set_line(list_area.x, y, &line, list_area.width);
            }
        }

        // --- Separator line ---
        let sep: String = "\u{2500}".repeat(inner.width as usize);
        buf.set_string(
            inner.x,
            sep_y,
            &sep,
            Style::default().fg(style_tokens::BORDER),
        );

        // --- Render detail pane ---
        if let Some(task) = self.tasks.get(self.selected) {
            let mut lines: Vec<Line> = Vec::new();

            // Task header
            let kind_label = match task.kind {
                UnifiedTaskKind::Agent => "Agent",
                UnifiedTaskKind::Process => "Process",
            };
            lines.push(Line::from(vec![
                Span::styled(
                    format!(" {kind_label} {}: ", task.task_id),
                    Style::default()
                        .fg(style_tokens::CYAN)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    task.description.clone(),
                    Style::default().fg(style_tokens::PRIMARY),
                ),
            ]));

            // Metadata line
            let mut meta_spans = vec![Span::styled(
                format!(" State: {}", task.state),
                Style::default().fg(style_tokens::GREY),
            )];
            if let Some(tc) = task.tool_count {
                meta_spans.push(Span::styled(
                    format!(" | Tools: {tc}"),
                    Style::default().fg(style_tokens::GREY),
                ));
            }
            if let Some(cost) = task.cost_usd {
                meta_spans.push(Span::styled(
                    format!(" | Cost: ${cost:.4}"),
                    Style::default().fg(style_tokens::GREY),
                ));
            }
            if let Some(pid) = task.pid {
                meta_spans.push(Span::styled(
                    format!(" | PID: {pid}"),
                    Style::default().fg(style_tokens::GREY),
                ));
            }
            lines.push(Line::from(meta_spans));

            // Current tool
            if let Some(ref tool) = task.current_tool {
                lines.push(Line::from(Span::styled(
                    format!(" Current: {tool}"),
                    Style::default().fg(style_tokens::BLUE_TASK),
                )));
            }

            // Blank separator
            lines.push(Line::from(""));

            // Output lines
            if self.output_lines.is_empty() {
                lines.push(Line::from(Span::styled(
                    " (no output)",
                    Style::default().fg(style_tokens::SUBTLE),
                )));
            } else {
                let scroll = self.output_scroll as usize;
                let visible = detail_height.saturating_sub(lines.len() as u16) as usize;
                let start = scroll.min(self.output_lines.len().saturating_sub(1));
                for line in self.output_lines.iter().skip(start).take(visible.max(1)) {
                    let truncated = if line.len() > inner.width as usize {
                        &line[..inner.width as usize]
                    } else {
                        line
                    };
                    lines.push(Line::from(Span::styled(
                        format!(" {truncated}"),
                        Style::default().fg(style_tokens::PRIMARY),
                    )));
                }
            }

            // Render detail lines
            for (i, line) in lines.iter().enumerate() {
                if i as u16 >= detail_height {
                    break;
                }
                buf.set_line(
                    detail_area.x,
                    detail_area.y + i as u16,
                    line,
                    detail_area.width,
                );
            }
        }

        // --- Footer hint ---
        let footer_y = panel.y + panel.height - 1;
        let focus_indicator = match self.focus {
            TaskWatcherFocus::List => "list",
            TaskWatcherFocus::Output => "output",
        };
        let hint = format!(
            " j/k:nav  x:kill  d:dismiss  Enter:focus({focus_indicator})  Ctrl+D/U:scroll  q:close "
        );
        let hint_x = panel.x + 1;
        buf.set_string(
            hint_x,
            footer_y,
            &hint,
            Style::default()
                .fg(style_tokens::SUBTLE)
                .add_modifier(Modifier::ITALIC),
        );
    }
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
