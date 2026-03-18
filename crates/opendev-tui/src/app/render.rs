//! UI rendering: layout composition, popup panels, and modal dialogs.

use ratatui::layout;

use crate::widgets::{
    ConversationWidget, InputWidget, StatusBarWidget, TodoPanelWidget, WelcomePanelWidget,
};

use super::App;
use super::OperationMode;

impl App {
    pub(super) fn render(&self, frame: &mut ratatui::Frame) {
        let area = frame.area();

        // Layout: conversation (flexible) | todo panel (if active) | subagent display (if active)
        //         | input | status bar
        // Tool spinners and thinking progress are rendered inline in the conversation area.
        let has_todos = !self.state.todo_items.is_empty();
        let todo_height: u16 = if has_todos {
            if self.state.todo_expanded {
                // 2 borders + 1 progress bar + items (capped at 12)
                (self.state.todo_items.len() as u16 + 3).min(12)
            } else {
                // Collapsed: border top + 1 line + border bottom
                3
            }
        } else {
            0
        };
        let chunks = layout::Layout::default()
            .direction(layout::Direction::Vertical)
            .constraints(
                [
                    layout::Constraint::Min(5),              // conversation
                    layout::Constraint::Length(todo_height), // todo panel
                    layout::Constraint::Length({
                        let input_lines = self.state.input_buffer.matches('\n').count() + 1;
                        (input_lines as u16 + 1).min(8) // +1 for separator, cap at 8
                    }), // input
                    layout::Constraint::Length(2),           // status bar
                ]
                .as_ref(),
            )
            .split(area);

        // Conversation
        let mode_str = match self.state.mode {
            OperationMode::Normal => "NORMAL",
            OperationMode::Plan => "PLAN",
        };

        // Show animated welcome panel when no messages (or during fade-out)
        if self.state.messages.is_empty() && !self.state.welcome_panel.fade_complete {
            let wp = WelcomePanelWidget::new(&self.state.welcome_panel)
                .version(&self.state.version)
                .mode(mode_str);
            frame.render_widget(wp, chunks[0]);
        } else {
            let mut conversation =
                ConversationWidget::new(&self.state.messages, self.state.scroll_offset)
                    .version(&self.state.version)
                    .working_dir(&self.state.working_dir)
                    .mode(mode_str)
                    .active_tools(&self.state.active_tools)
                    .active_subagents(&self.state.active_subagents)
                    .task_progress(self.state.task_progress.as_ref())
                    .spinner_char(self.state.spinner.current())
                    .compaction_active(self.state.compaction_active);
            if !self.state.cached_lines.is_empty() {
                conversation = conversation.cached_lines(&self.state.cached_lines);
            }
            frame.render_widget(conversation, chunks[0]);
        }

        // Todo panel (only if plan has todos)
        if has_todos {
            let mut todo_widget = TodoPanelWidget::new(&self.state.todo_items)
                .with_expanded(self.state.todo_expanded)
                .with_spinner_tick(self.state.todo_spinner_tick);
            if let Some(ref name) = self.state.plan_name {
                todo_widget = todo_widget.with_plan_name(name);
            }
            frame.render_widget(todo_widget, chunks[1]);
        }

        // Input
        let input = InputWidget::new(
            &self.state.input_buffer,
            self.state.input_cursor,
            mode_str,
            self.state.pending_messages.len(),
        );
        frame.render_widget(input, chunks[2]);

        // Autocomplete popup (rendered over conversation area)
        if self.state.autocomplete.is_visible() {
            self.render_autocomplete(frame, chunks[2]);
        }

        // Plan approval panel (rendered over input area when active)
        if self.plan_approval_controller.active() {
            self.render_plan_approval(frame, chunks[2]);
        }

        // Ask-user panel (rendered over input area when active)
        if self.ask_user_controller.active() {
            self.render_ask_user(frame, chunks[2]);
        }

        // Tool approval panel (rendered over input area when active)
        if self.approval_controller.active() {
            self.render_approval(frame, chunks[2]);
        }

        // Model picker panel (rendered over input area when active)
        if let Some(ref picker) = self.model_picker_controller
            && picker.active()
        {
            self.render_model_picker(frame, chunks[2]);
        }

        // Status bar
        let status = StatusBarWidget::new(
            &self.state.model,
            &self.state.working_dir,
            self.state.git_branch.as_deref(),
            self.state.tokens_used,
            self.state.tokens_limit,
            self.state.mode,
        )
        .autonomy(self.state.autonomy)
        .reasoning_level(self.state.reasoning_level)
        .context_usage_pct(self.state.context_usage_pct)
        .session_cost(self.state.session_cost)
        .mcp_status(self.state.mcp_status, self.state.mcp_has_errors)
        .background_tasks(self.state.background_task_count)
        .file_changes(self.state.file_changes);
        frame.render_widget(status, chunks[3]);

        // Background task panel overlay (Ctrl+B)
        if self.state.background_panel_open {
            let task_items: Vec<crate::widgets::background_tasks::TaskDisplayItem> =
                if let Ok(mgr) = self.task_manager.try_lock() {
                    mgr.all_tasks()
                        .iter()
                        .map(|t| crate::widgets::background_tasks::TaskDisplayItem {
                            task_id: t.task_id.clone(),
                            description: t.description.clone(),
                            state: t.state.to_string(),
                            runtime_secs: t.runtime_seconds(),
                        })
                        .collect()
                } else {
                    Vec::new()
                };
            let running = task_items.iter().filter(|t| t.state == "running").count();
            let total = task_items.len();
            let panel = crate::widgets::background_tasks::BackgroundTaskPanel::new(
                &task_items,
                running,
                total,
            );
            frame.render_widget(panel, chunks[0]);
        }
    }

    /// Shared helper that renders a popup panel matching the Python Textual style:
    /// bright_cyan border, `▸` pointer, bold white active label, dim descriptions.
    /// Padding (1, 2) = 1 empty line top/bottom, 2 spaces horizontal.
    /// True cyan color for popup panel borders and accents.
    pub(super) const PANEL_CYAN: ratatui::style::Color = ratatui::style::Color::Rgb(0, 255, 255);

    pub(super) fn render_popup_panel(
        frame: &mut ratatui::Frame,
        input_area: layout::Rect,
        title: &str,
        content_lines: &[ratatui::text::Line<'_>],
        option_lines: &[ratatui::text::Line<'_>],
        hint: &str,
        max_width: Option<u16>,
    ) {
        use crate::formatters::style_tokens;
        use ratatui::style::{Modifier, Style};
        use ratatui::text::{Line, Span};
        use ratatui::widgets::{Block, BorderType, Borders, Paragraph};

        let mut lines: Vec<Line> = Vec::new();

        // Top padding (1 empty line)
        lines.push(Line::from(""));

        // Content section
        for line in content_lines {
            lines.push(line.clone());
        }

        // Hint line
        lines.push(Line::from(Span::styled(
            format!("    {hint}"),
            Style::default().fg(style_tokens::DIM_GREY),
        )));

        // Option lines
        for line in option_lines {
            lines.push(line.clone());
        }

        // Bottom padding (1 empty line)
        lines.push(Line::from(""));

        let panel_width = max_width
            .map(|w| input_area.width.min(w))
            .unwrap_or(input_area.width);
        let panel_height = (lines.len() as u16 + 2).min(input_area.y);
        let popup_area = layout::Rect {
            x: input_area.x,
            y: input_area.y.saturating_sub(panel_height),
            width: panel_width,
            height: panel_height,
        };

        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Self::PANEL_CYAN))
            .title(Span::styled(
                title,
                Style::default()
                    .fg(Self::PANEL_CYAN)
                    .add_modifier(Modifier::BOLD),
            ));

        let paragraph = Paragraph::new(lines).block(block);
        frame.render_widget(ratatui::widgets::Clear, popup_area);
        frame.render_widget(paragraph, popup_area);
    }

    /// Build a single option line matching the Python Textual style.
    /// Active: `▸` bright_cyan pointer + dim number + bold white label + dim description.
    /// Inactive: space pointer + dim number + white label + dim description.
    pub(super) fn build_option_line<'a>(
        is_selected: bool,
        number: &str,
        label: &str,
        description: &str,
    ) -> ratatui::text::Line<'a> {
        use crate::formatters::style_tokens;
        use ratatui::style::{Modifier, Style};
        use ratatui::text::{Line, Span};

        let pointer = if is_selected { "\u{25b8}" } else { " " };
        let pointer_style = if is_selected {
            Style::default()
                .fg(Self::PANEL_CYAN)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(style_tokens::DIM_GREY)
        };
        let num_style = Style::default().fg(style_tokens::DIM_GREY);
        let label_style = if is_selected {
            Style::default()
                .fg(style_tokens::PRIMARY)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(style_tokens::PRIMARY)
        };
        let desc_style = Style::default().fg(style_tokens::DIM_GREY);

        let mut spans = vec![
            Span::styled(format!("    {pointer} "), pointer_style),
            Span::styled(format!("{number} "), num_style),
            Span::styled(label.to_string(), label_style),
        ];
        if !description.is_empty() {
            spans.push(Span::styled(format!("  {description}"), desc_style));
        }
        Line::from(spans)
    }

    /// Render autocomplete popup above the input area.
    pub(super) fn render_autocomplete(&self, frame: &mut ratatui::Frame, input_area: layout::Rect) {
        use crate::autocomplete::CompletionKind;
        use crate::formatters::style_tokens;
        use ratatui::style::{Color, Modifier, Style};
        use ratatui::text::{Line, Span};
        use ratatui::widgets::{Block, BorderType, Borders, Paragraph};

        let items = self.state.autocomplete.items();
        let selected_idx = self.state.autocomplete.selected_index();
        let max_show = items.len().min(10);
        let popup_height = max_show as u16 + 2; // +2 for borders

        // Determine title and width based on completion kind
        let is_file_mode = items
            .first()
            .is_some_and(|i| i.kind == CompletionKind::File);
        let popup_width = if is_file_mode { 60 } else { 50 };
        let title = if is_file_mode {
            " Files "
        } else {
            " Commands "
        };

        let popup_area = layout::Rect {
            x: input_area.x,
            y: input_area.y.saturating_sub(popup_height),
            width: input_area.width.min(popup_width),
            height: popup_height,
        };

        // Python uses BLUE_BG_ACTIVE (#1f2d3a) as active row bg
        let active_bg = Color::Rgb(31, 45, 58);

        let lines: Vec<Line> = items
            .iter()
            .take(max_show)
            .enumerate()
            .map(|(i, item)| {
                let selected = i == selected_idx;
                let (left, right) =
                    crate::autocomplete::formatters::CompletionFormatter::format(item);

                let pointer = if selected { "\u{25b8}" } else { "\u{2022}" };
                let pointer_style = if selected {
                    Style::default()
                        .fg(Self::PANEL_CYAN)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(style_tokens::DIM_GREY)
                };
                let label_style = if selected {
                    Style::default()
                        .fg(Self::PANEL_CYAN)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(style_tokens::PRIMARY)
                };
                let desc_style = if selected {
                    Style::default().fg(style_tokens::GREY)
                } else {
                    Style::default().fg(style_tokens::SUBTLE)
                };

                let line = Line::from(vec![
                    Span::styled(format!(" {pointer} "), pointer_style),
                    Span::styled(left, label_style),
                    Span::styled(format!(" {right}"), desc_style),
                ]);
                if selected {
                    line.style(Style::default().bg(active_bg))
                } else {
                    line
                }
            })
            .collect();

        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(style_tokens::BORDER))
            .title(Span::styled(
                title,
                Style::default()
                    .fg(Self::PANEL_CYAN)
                    .add_modifier(Modifier::BOLD),
            ));

        let paragraph = Paragraph::new(lines).block(block);
        frame.render_widget(ratatui::widgets::Clear, popup_area);
        frame.render_widget(paragraph, popup_area);
    }

    /// Render the plan approval panel above the input area.
    pub(super) fn render_plan_approval(
        &self,
        frame: &mut ratatui::Frame,
        input_area: layout::Rect,
    ) {
        use crate::formatters::style_tokens;
        use ratatui::style::{Modifier, Style};
        use ratatui::text::{Line, Span};

        let plan_options = self.plan_approval_controller.options();
        let selected = self.plan_approval_controller.selected_action();

        let content_lines = vec![Line::from(vec![
            Span::styled("    Plan ", Style::default().fg(style_tokens::DIM_GREY)),
            Span::styled("\u{00b7} ", Style::default().fg(style_tokens::DIM_GREY)),
            Span::styled(
                "Ready for review",
                Style::default()
                    .fg(Self::PANEL_CYAN)
                    .add_modifier(Modifier::BOLD),
            ),
        ])];

        let option_lines: Vec<Line> = plan_options
            .iter()
            .enumerate()
            .map(|(i, opt)| {
                Self::build_option_line(
                    i == selected,
                    &format!("{}.", i + 1),
                    &opt.label,
                    &opt.description,
                )
            })
            .collect();

        Self::render_popup_panel(
            frame,
            input_area,
            " Approval ",
            &content_lines,
            &option_lines,
            "\u{2191}/\u{2193} choose \u{00b7} Enter confirm \u{00b7} Esc cancel",
            None,
        );
    }

    /// Render the ask-user prompt panel.
    pub(super) fn render_ask_user(&self, frame: &mut ratatui::Frame, input_area: layout::Rect) {
        use crate::formatters::style_tokens;
        use ratatui::style::Style;
        use ratatui::text::{Line, Span};

        let ask_options = self.ask_user_controller.options();
        let selected = self.ask_user_controller.selected_index();
        let question = self.ask_user_controller.question();

        let content_lines = vec![Line::from(Span::styled(
            format!("    {question}"),
            Style::default().fg(style_tokens::PRIMARY),
        ))];

        let option_lines: Vec<Line> = ask_options
            .iter()
            .enumerate()
            .map(|(i, opt)| Self::build_option_line(i == selected, &format!("{}.", i + 1), opt, ""))
            .collect();

        Self::render_popup_panel(
            frame,
            input_area,
            " Question ",
            &content_lines,
            &option_lines,
            "\u{2191}/\u{2193} choose \u{00b7} Enter confirm \u{00b7} Esc cancel",
            None,
        );
    }

    /// Render the model picker panel above the input area.
    pub(super) fn render_model_picker(&self, frame: &mut ratatui::Frame, input_area: layout::Rect) {
        use crate::controllers::ModelPickerController;
        use crate::formatters::style_tokens;
        use ratatui::style::{Color, Modifier, Style};
        use ratatui::text::{Line, Span};
        use ratatui::widgets::{Block, BorderType, Borders, Paragraph};

        let picker = match self.model_picker_controller {
            Some(ref p) => p,
            None => return,
        };

        let visible = picker.visible_models();
        let selected_idx = picker.selected_index();
        let total = picker.filtered_count();
        let query = picker.search_query();

        let active_bg = Color::Rgb(31, 45, 58);
        let mut lines: Vec<Line> = Vec::new();

        // Search bar
        let search_display = if query.is_empty() {
            "Type to search...".to_string()
        } else {
            query.to_string()
        };
        let search_style = if query.is_empty() {
            Style::default().fg(style_tokens::DIM_GREY)
        } else {
            Style::default()
                .fg(Self::PANEL_CYAN)
                .add_modifier(Modifier::BOLD)
        };
        lines.push(Line::from(vec![
            Span::styled("  \u{1f50d} ", Style::default().fg(style_tokens::DIM_GREY)),
            Span::styled(search_display, search_style),
        ]));

        // Separator
        lines.push(Line::from(Span::styled(
            "  \u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}",
            Style::default().fg(style_tokens::BORDER),
        )));

        // Track current provider for group headers
        let mut current_provider = String::new();

        for (display_idx, model) in &visible {
            // Provider group header
            if model.provider != current_provider {
                current_provider = model.provider.clone();
                lines.push(Line::from(Span::styled(
                    format!("  {} {}", "\u{25cf}", model.provider_display),
                    Style::default()
                        .fg(style_tokens::GREY)
                        .add_modifier(Modifier::BOLD),
                )));
            }

            let selected = *display_idx == selected_idx;
            let is_current = model.id == self.state.model;

            // Pointer
            let pointer = if selected { "\u{25b8}" } else { " " };
            let pointer_style = if selected {
                Style::default()
                    .fg(Self::PANEL_CYAN)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(style_tokens::DIM_GREY)
            };

            // Model name
            let name_style = if selected {
                Style::default()
                    .fg(Self::PANEL_CYAN)
                    .add_modifier(Modifier::BOLD)
            } else if is_current {
                Style::default()
                    .fg(Color::Rgb(0, 200, 100))
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(style_tokens::PRIMARY)
            };

            // Context and pricing info
            let ctx = ModelPickerController::format_context(model.context_length);
            let pricing =
                ModelPickerController::format_pricing(model.pricing_input, model.pricing_output);

            let mut spans = vec![
                Span::styled(format!("    {pointer} "), pointer_style),
                Span::styled(model.name.clone(), name_style),
            ];

            // Current model indicator
            if is_current {
                spans.push(Span::styled(
                    " \u{2713}",
                    Style::default().fg(Color::Rgb(0, 200, 100)),
                ));
            }

            // Recommended badge
            if model.recommended {
                spans.push(Span::styled(
                    " \u{2605}",
                    Style::default().fg(Color::Rgb(255, 200, 50)),
                ));
            }

            // Context length
            spans.push(Span::styled(
                format!("  {ctx}"),
                Style::default().fg(style_tokens::DIM_GREY),
            ));

            // Pricing
            spans.push(Span::styled(
                format!("  {pricing}"),
                Style::default().fg(style_tokens::SUBTLE),
            ));

            let line = Line::from(spans);
            if selected {
                lines.push(line.style(Style::default().bg(active_bg)));
            } else {
                lines.push(line);
            }
        }

        // Empty state
        if visible.is_empty() {
            lines.push(Line::from(Span::styled(
                "    No models match your search.",
                Style::default().fg(style_tokens::DIM_GREY),
            )));
        }

        // Bottom hint with count
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled(
                format!("  {total} model{}", if total == 1 { "" } else { "s" }),
                Style::default().fg(style_tokens::DIM_GREY),
            ),
            Span::styled(
                "  \u{2191}/\u{2193} navigate \u{00b7} Enter select \u{00b7} Esc cancel",
                Style::default().fg(style_tokens::DIM_GREY),
            ),
        ]));

        let panel_height = (lines.len() as u16 + 2).min(input_area.y);
        let panel_width = input_area.width.min(80);
        let popup_area = layout::Rect {
            x: input_area.x,
            y: input_area.y.saturating_sub(panel_height),
            width: panel_width,
            height: panel_height,
        };

        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Self::PANEL_CYAN))
            .title(Span::styled(
                " Models ",
                Style::default()
                    .fg(Self::PANEL_CYAN)
                    .add_modifier(Modifier::BOLD),
            ));

        let paragraph = Paragraph::new(lines).block(block);
        frame.render_widget(ratatui::widgets::Clear, popup_area);
        frame.render_widget(paragraph, popup_area);
    }

    /// Render the tool approval prompt panel.
    pub(super) fn render_approval(&self, frame: &mut ratatui::Frame, input_area: layout::Rect) {
        use crate::formatters::style_tokens;
        use ratatui::style::{Modifier, Style};
        use ratatui::text::{Line, Span};

        let approval_options = self.approval_controller.options();
        let selected = self.approval_controller.selected_index();
        let command = self.approval_controller.command();
        let working_dir = self.approval_controller.working_dir();

        let content_lines = vec![
            Line::from(vec![
                Span::styled("    Command ", Style::default().fg(style_tokens::DIM_GREY)),
                Span::styled("\u{00b7} ", Style::default().fg(style_tokens::DIM_GREY)),
                Span::styled(
                    command.to_string(),
                    Style::default()
                        .fg(Self::PANEL_CYAN)
                        .add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::from(Span::styled(
                format!("    Directory \u{00b7} {working_dir}"),
                Style::default().fg(style_tokens::DIM_GREY),
            )),
        ];

        let option_lines: Vec<Line> = approval_options
            .iter()
            .enumerate()
            .map(|(i, opt)| {
                Self::build_option_line(
                    i == selected,
                    &format!("{}.", opt.choice),
                    &opt.label,
                    &opt.description,
                )
            })
            .collect();

        Self::render_popup_panel(
            frame,
            input_area,
            " Approval ",
            &content_lines,
            &option_lines,
            "\u{2191}/\u{2193} choose \u{00b7} Enter confirm \u{00b7} Esc cancel",
            None,
        );
    }
}
