//! Status bar widget showing model, tokens, mode, autonomy, thinking, git branch, MCP, cost.
//!
//! Mirrors the Python `StatusBar` widget which displays:
//! - Mode (Normal/Plan) with Shift+Tab hint
//! - Autonomy level (Manual/Semi-Auto/Auto) with Ctrl+Shift+A hint
//! - Thinking level (Off/Low/Medium/High) with Ctrl+Shift+T hint
//! - Repo path and git branch
//! - MCP server status
//! - Session cost
//! - Context window remaining percentage

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Widget,
};

use crate::app::{AutonomyLevel, OperationMode, ThinkingLevel};
use crate::formatters::style_tokens;

/// Bottom status bar widget.
#[allow(dead_code)]
pub struct StatusBarWidget<'a> {
    model: &'a str,
    working_dir: &'a str,
    git_branch: Option<&'a str>,
    tokens_used: u64,
    tokens_limit: u64,
    mode: OperationMode,
    // New fields from Python StatusBar
    autonomy: AutonomyLevel,
    thinking_level: ThinkingLevel,
    context_usage_pct: f64,
    session_cost: f64,
    mcp_status: Option<(usize, usize)>,
    mcp_has_errors: bool,
    background_tasks: usize,
}

impl<'a> StatusBarWidget<'a> {
    pub fn new(
        model: &'a str,
        working_dir: &'a str,
        git_branch: Option<&'a str>,
        tokens_used: u64,
        tokens_limit: u64,
        mode: OperationMode,
    ) -> Self {
        Self {
            model,
            working_dir,
            git_branch,
            tokens_used,
            tokens_limit,
            mode,
            autonomy: AutonomyLevel::Manual,
            thinking_level: ThinkingLevel::Medium,
            context_usage_pct: 0.0,
            session_cost: 0.0,
            mcp_status: None,
            mcp_has_errors: false,
            background_tasks: 0,
        }
    }

    pub fn autonomy(mut self, autonomy: AutonomyLevel) -> Self {
        self.autonomy = autonomy;
        self
    }

    pub fn thinking_level(mut self, level: ThinkingLevel) -> Self {
        self.thinking_level = level;
        self
    }

    pub fn context_usage_pct(mut self, pct: f64) -> Self {
        self.context_usage_pct = pct;
        self
    }

    pub fn session_cost(mut self, cost: f64) -> Self {
        self.session_cost = cost;
        self
    }

    pub fn mcp_status(mut self, status: Option<(usize, usize)>, has_errors: bool) -> Self {
        self.mcp_status = status;
        self.mcp_has_errors = has_errors;
        self
    }

    pub fn background_tasks(mut self, count: usize) -> Self {
        self.background_tasks = count;
        self
    }

    #[allow(dead_code)]
    fn format_tokens(n: u64) -> String {
        if n >= 1_000_000 {
            format!("{:.1}M", n as f64 / 1_000_000.0)
        } else if n >= 1_000 {
            format!("{:.1}k", n as f64 / 1_000.0)
        } else {
            n.to_string()
        }
    }
}

impl Widget for StatusBarWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.height == 0 {
            return;
        }

        let mut spans: Vec<Span> = Vec::new();

        // Autonomy level
        let autonomy_color = match self.autonomy {
            AutonomyLevel::Manual => style_tokens::ORANGE_CAUTION,
            AutonomyLevel::SemiAuto => style_tokens::CYAN,
            AutonomyLevel::Auto => style_tokens::GREEN_BRIGHT,
        };
        spans.push(Span::styled(
            "Autonomy: ",
            Style::default().fg(style_tokens::GREY),
        ));
        spans.push(Span::styled(
            self.autonomy.to_string(),
            Style::default()
                .fg(autonomy_color)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(
            " (Ctrl+Shift+A)",
            Style::default().fg(style_tokens::GREY),
        ));

        // Separator
        spans.push(Span::styled(
            "  \u{2502}  ",
            Style::default().fg(style_tokens::GREY),
        ));

        // Thinking level
        let thinking_color = match self.thinking_level {
            ThinkingLevel::Off => style_tokens::GREY,
            ThinkingLevel::Low => style_tokens::CYAN,
            ThinkingLevel::Medium => style_tokens::GREEN_BRIGHT,
            ThinkingLevel::High => style_tokens::GOLD,
        };
        spans.push(Span::styled(
            "Thinking: ",
            Style::default().fg(style_tokens::GREY),
        ));
        spans.push(Span::styled(
            self.thinking_level.to_string(),
            Style::default()
                .fg(thinking_color)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(
            " (Ctrl+Shift+T)",
            Style::default().fg(style_tokens::GREY),
        ));

        // Repo info (path + git branch)
        let repo_display = self.build_repo_display();
        if !repo_display.is_empty() {
            spans.push(Span::styled(
                "  \u{2502}  ",
                Style::default().fg(style_tokens::GREY),
            ));
            spans.push(Span::styled(
                repo_display,
                Style::default().fg(style_tokens::BLUE_PATH),
            ));
        }

        // MCP status (only when servers configured)
        if let Some((connected, total)) = self.mcp_status {
            spans.push(Span::styled(
                "  \u{2502}  ",
                Style::default().fg(style_tokens::GREY),
            ));
            let mcp_label = format!("MCP: {connected}/{total}");
            let mcp_color = if self.mcp_has_errors {
                style_tokens::ORANGE
            } else if connected < total {
                style_tokens::GOLD
            } else {
                style_tokens::GREEN_LIGHT
            };
            spans.push(Span::styled(
                mcp_label,
                Style::default().fg(mcp_color).add_modifier(Modifier::BOLD),
            ));
        }

        // Background tasks
        if self.background_tasks > 0 {
            spans.push(Span::styled(
                "  \u{2502}  ",
                Style::default().fg(style_tokens::GREY),
            ));
            let task_word = if self.background_tasks == 1 {
                "task"
            } else {
                "tasks"
            };
            spans.push(Span::styled(
                format!("{} background {task_word}", self.background_tasks),
                Style::default().fg(style_tokens::BLUE_TASK),
            ));
            spans.push(Span::styled(
                " (Ctrl+B)",
                Style::default().fg(style_tokens::GREY),
            ));
        }

        // Right-aligned section: cost + context remaining
        let context_left = (100.0 - self.context_usage_pct).max(0.0);
        let pct_str = format!("{context_left:.1}");
        let pct_color = if context_left > 50.0 {
            style_tokens::GREEN_LIGHT
        } else if context_left > 25.0 {
            style_tokens::GOLD
        } else {
            style_tokens::ORANGE
        };

        let cost_str = if self.session_cost > 0.0 {
            if self.session_cost < 0.01 {
                format!("${:.4}", self.session_cost)
            } else {
                format!("${:.2}", self.session_cost)
            }
        } else {
            String::new()
        };

        // Calculate total left-side width for gap
        let left_len: usize = spans.iter().map(|s| s.content.len()).sum();
        let right_text = if cost_str.is_empty() {
            format!("Context left {pct_str}%")
        } else {
            format!("Cost {cost_str}  \u{2502}  Context left {pct_str}%")
        };
        let right_len = right_text.len();

        let available_width = area.width as usize;
        let gap = available_width.saturating_sub(left_len + right_len);
        if gap >= 2 {
            spans.push(Span::raw(" ".repeat(gap)));
        } else {
            spans.push(Span::styled(
                "  \u{2502}  ",
                Style::default().fg(style_tokens::GREY),
            ));
        }

        // Cost display
        if !cost_str.is_empty() {
            spans.push(Span::styled(
                "Cost ",
                Style::default().fg(style_tokens::GREY),
            ));
            spans.push(Span::styled(
                cost_str,
                Style::default()
                    .fg(style_tokens::CYAN)
                    .add_modifier(Modifier::BOLD),
            ));
            spans.push(Span::styled(
                "  \u{2502}  ",
                Style::default().fg(style_tokens::GREY),
            ));
        }

        // Context remaining
        spans.push(Span::styled(
            format!("Context left {pct_str}%"),
            Style::default().fg(pct_color).add_modifier(Modifier::BOLD),
        ));

        // Render a thin border line at top, then text below
        let line = Line::from(spans);
        if area.height >= 2 {
            // Draw thin border
            let border_line: String = "\u{2500}".repeat(area.width as usize);
            buf.set_string(
                area.left(),
                area.top(),
                &border_line,
                Style::default().fg(style_tokens::BORDER),
            );
            // Render text on second row
            buf.set_line(area.left(), area.top() + 1, &line, area.width);
        } else {
            // Single row: just render text, no border
            buf.set_line(area.left(), area.top(), &line, area.width);
        }
    }
}

impl StatusBarWidget<'_> {
    /// Build repo display string with path and git branch.
    fn build_repo_display(&self) -> String {
        if self.working_dir.is_empty() || self.working_dir == "." {
            return String::new();
        }

        let dir_display = shorten_path(self.working_dir);
        match self.git_branch {
            Some(branch) => format!("{dir_display} ({branch})"),
            None => dir_display,
        }
    }
}

/// Shorten a path for display (show last 2 components, replace home with ~).
fn shorten_path(path: &str) -> String {
    // Replace home directory with ~
    let display = if let Some(home) = dirs_home(path) {
        home
    } else {
        path.to_string()
    };

    let parts: Vec<&str> = display.split('/').filter(|p| !p.is_empty()).collect();
    if parts.len() <= 2 {
        return display;
    }
    format!(".../{}", parts[parts.len() - 2..].join("/"))
}

/// Replace home directory prefix with ~ if possible.
fn dirs_home(path: &str) -> Option<String> {
    // Try to detect home directory from env
    if let Ok(home) = std::env::var("HOME")
        && path.starts_with(&home)
    {
        return Some(format!("~{}", &path[home.len()..]));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_tokens() {
        assert_eq!(StatusBarWidget::format_tokens(500), "500");
        assert_eq!(StatusBarWidget::format_tokens(1_500), "1.5k");
        assert_eq!(StatusBarWidget::format_tokens(1_500_000), "1.5M");
    }

    #[test]
    fn test_shorten_path() {
        assert_eq!(shorten_path("/home/user"), "/home/user");
        assert_eq!(
            shorten_path("/home/user/projects/myapp"),
            ".../projects/myapp"
        );
    }

    #[test]
    fn test_status_bar_creation() {
        let _widget = StatusBarWidget::new(
            "claude-sonnet-4",
            "/home/user/project",
            Some("main"),
            5000,
            200_000,
            OperationMode::Normal,
        )
        .autonomy(AutonomyLevel::Manual)
        .thinking_level(ThinkingLevel::Medium)
        .context_usage_pct(25.0)
        .session_cost(0.05)
        .mcp_status(Some((2, 3)), false)
        .background_tasks(1);
    }

    #[test]
    fn test_autonomy_display() {
        assert_eq!(AutonomyLevel::Manual.to_string(), "Manual");
        assert_eq!(AutonomyLevel::SemiAuto.to_string(), "Semi-Auto");
        assert_eq!(AutonomyLevel::Auto.to_string(), "Auto");
    }

    #[test]
    fn test_thinking_display() {
        assert_eq!(ThinkingLevel::Off.to_string(), "Off");
        assert_eq!(ThinkingLevel::Low.to_string(), "Low");
        assert_eq!(ThinkingLevel::Medium.to_string(), "Medium");
        assert_eq!(ThinkingLevel::High.to_string(), "High");
    }
}
