//! Bridge between the ratatui TUI and the AgentRuntime.
//!
//! Spawns a background task that listens for user messages from the TUI,
//! runs them through the agent pipeline, and sends events back to update
//! the UI.

use std::io;
use std::time::Duration;

use tokio::sync::mpsc;
use tracing::{info, warn};

use opendev_agents::traits::AgentEventCallback;
use opendev_history::SessionManager;
use opendev_runtime::InterruptToken;
use opendev_tui::app::AppState;
use opendev_tui::{App, AppEvent};

use crate::runtime::AgentRuntime;

/// Event callback that forwards agent events to the TUI via AppEvent channel.
struct TuiEventCallback {
    tx: mpsc::UnboundedSender<AppEvent>,
}

impl AgentEventCallback for TuiEventCallback {
    fn on_tool_started(
        &self,
        tool_id: &str,
        tool_name: &str,
        args: &std::collections::HashMap<String, serde_json::Value>,
    ) {
        let _ = self.tx.send(AppEvent::ToolStarted {
            tool_id: tool_id.to_string(),
            tool_name: tool_name.to_string(),
            args: args.clone(),
        });
    }

    fn on_tool_finished(&self, tool_id: &str, success: bool) {
        let _ = self.tx.send(AppEvent::ToolFinished {
            tool_id: tool_id.to_string(),
            success,
        });
    }

    fn on_tool_result(&self, tool_id: &str, tool_name: &str, output: &str, success: bool) {
        // Args will be looked up from the stored ToolExecution in app.rs
        let _ = self.tx.send(AppEvent::ToolResult {
            tool_id: tool_id.to_string(),
            tool_name: tool_name.to_string(),
            output: output.to_string(),
            success,
            args: std::collections::HashMap::new(),
        });
    }

    fn on_agent_chunk(&self, text: &str) {
        let _ = self.tx.send(AppEvent::AgentChunk(text.to_string()));
    }

    fn on_reasoning(&self, content: &str) {
        let _ = self
            .tx
            .send(AppEvent::ReasoningContent(content.to_string()));
    }

    fn on_context_usage(&self, pct: f64) {
        let _ = self.tx.send(AppEvent::ContextUsage(pct));
    }

    fn on_file_changed(&self, files: usize, additions: u64, deletions: u64) {
        let _ = self.tx.send(AppEvent::FileChangeSummary {
            files,
            additions,
            deletions,
        });
    }
}

/// Bridges the TUI event loop with the AgentRuntime.
pub struct TuiRunner {
    runtime: AgentRuntime,
    system_prompt: String,
    initial_message: Option<String>,
}

impl TuiRunner {
    /// Create a new TUI runner.
    pub fn new(runtime: AgentRuntime, system_prompt: String) -> Self {
        Self {
            runtime,
            system_prompt,
            initial_message: None,
        }
    }

    /// Set an initial message to send to the agent when the TUI starts.
    pub fn with_initial_message(mut self, msg: Option<String>) -> Self {
        self.initial_message = msg;
        self
    }

    /// Run the TUI application with the agent backend.
    ///
    /// Sets up message forwarding between the TUI and the AgentRuntime,
    /// then runs the TUI event loop.
    pub async fn run(mut self, mut state: AppState) -> io::Result<()> {
        // Channel for forwarding user messages from TUI to agent task
        let (user_tx, mut user_rx) = mpsc::unbounded_channel::<String>();

        // Create the TUI app with the message channel
        let mut app = App::new().with_message_channel(user_tx.clone());

        // Apply initial state
        std::mem::swap(&mut app.state, &mut state);

        // Get event sender so the agent task can push UI updates
        let event_tx = app.event_sender();

        // Bridge channel receivers to AppEvents
        if let Some(receivers) = self.runtime.channel_receivers.take() {
            // Ask-user channel bridge
            let ask_tx = event_tx.clone();
            let mut ask_rx = receivers.ask_user_rx;
            tokio::spawn(async move {
                while let Some(req) = ask_rx.recv().await {
                    let _ = ask_tx.send(AppEvent::AskUserRequested {
                        question: req.question,
                        options: req.options,
                        default: req.default,
                        response_tx: req.response_tx,
                    });
                }
            });

            // Plan-approval channel bridge
            let plan_tx = event_tx.clone();
            let mut plan_rx = receivers.plan_approval_rx;
            tokio::spawn(async move {
                while let Some(req) = plan_rx.recv().await {
                    let _ = plan_tx.send(AppEvent::PlanApprovalRequested {
                        plan_content: req.plan_content,
                        response_tx: req.response_tx,
                    });
                }
            });

            // Tool-approval channel bridge
            let approval_tx = event_tx.clone();
            let mut tool_rx = receivers.tool_approval_rx;
            tokio::spawn(async move {
                while let Some(req) = tool_rx.recv().await {
                    let _ = approval_tx.send(AppEvent::ToolApprovalRequested {
                        command: req.command,
                        working_dir: req.working_dir,
                        response_tx: req.response_tx,
                    });
                }
            });

            // Subagent event channel bridge
            if let Some(mut subagent_rx) = receivers.subagent_event_rx {
                let sa_tx = event_tx.clone();
                tokio::spawn(async move {
                    use opendev_tools_impl::SubagentEvent;
                    while let Some(evt) = subagent_rx.recv().await {
                        let app_event = match evt {
                            SubagentEvent::Started {
                                subagent_id,
                                subagent_name,
                                task,
                            } => AppEvent::SubagentStarted {
                                subagent_id,
                                subagent_name,
                                task,
                            },
                            SubagentEvent::ToolCall {
                                subagent_id,
                                subagent_name,
                                tool_name,
                                tool_id,
                                args,
                            } => AppEvent::SubagentToolCall {
                                subagent_id,
                                subagent_name,
                                tool_name,
                                tool_id,
                                args,
                            },
                            SubagentEvent::ToolComplete {
                                subagent_id,
                                subagent_name,
                                tool_name,
                                tool_id,
                                success,
                            } => AppEvent::SubagentToolComplete {
                                subagent_id,
                                subagent_name,
                                tool_name,
                                tool_id,
                                success,
                            },
                            SubagentEvent::Finished {
                                subagent_id,
                                subagent_name,
                                success,
                                result_summary,
                                tool_call_count,
                                shallow_warning,
                            } => AppEvent::SubagentFinished {
                                subagent_id,
                                subagent_name,
                                success,
                                result_summary,
                                tool_call_count,
                                shallow_warning,
                            },
                            SubagentEvent::TokenUpdate {
                                subagent_id,
                                subagent_name,
                                input_tokens,
                                output_tokens,
                            } => AppEvent::SubagentTokenUpdate {
                                subagent_id,
                                subagent_name,
                                input_tokens,
                                output_tokens,
                            },
                        };
                        if sa_tx.send(app_event).is_err() {
                            break;
                        }
                    }
                });
            }
        }

        // Create the event callback for tool/agent events
        let callback = TuiEventCallback {
            tx: event_tx.clone(),
        };

        // Spawn the agent listener task
        let system_prompt = self.system_prompt;
        let mut runtime = self.runtime;

        tokio::spawn(async move {
            while let Some(msg) = user_rx.recv().await {
                // Handle reasoning effort change sentinel
                if let Some(effort) = msg.strip_prefix("\x00__REASONING_EFFORT__") {
                    let new_effort = if effort == "none" {
                        None
                    } else {
                        Some(effort.to_string())
                    };
                    info!(effort = ?new_effort, "Reasoning effort changed");
                    runtime.llm_caller.config.reasoning_effort = new_effort;
                    continue;
                }

                // Handle model change sentinel
                if let Some(model) = msg.strip_prefix("\x00__MODEL_CHANGE__") {
                    info!(model = model, "Model changed via /models");
                    runtime.llm_caller.config.model = model.to_string();
                    continue;
                }

                // Handle manual compaction sentinel
                if msg == "\x00__COMPACT__" {
                    info!("TUI: manual compaction requested");
                    let _ = event_tx.send(AppEvent::CompactionStarted);

                    match runtime.run_compaction().await {
                        Ok(summary) => {
                            let _ = event_tx.send(AppEvent::CompactionFinished {
                                success: true,
                                message: summary,
                            });
                        }
                        Err(e) => {
                            let _ = event_tx.send(AppEvent::CompactionFinished {
                                success: false,
                                message: e,
                            });
                        }
                    }
                    continue;
                }

                info!(msg_len = msg.len(), "TUI: user submitted message");

                // Create fresh interrupt token for this query
                let interrupt_token = InterruptToken::new();
                let _ = event_tx.send(AppEvent::SetInterruptToken(interrupt_token.clone()));

                // Signal agent started
                let _ = event_tx.send(AppEvent::AgentStarted);
                let _ = event_tx.send(AppEvent::TaskProgressStarted {
                    description: "Thinking".to_string(),
                });

                // Run the query through the agent pipeline with event callback
                match runtime
                    .run_query(
                        &msg,
                        &system_prompt,
                        Some(&callback),
                        Some(&interrupt_token),
                    )
                    .await
                {
                    Ok(result) if result.backgrounded => {
                        let _ = event_tx.send(AppEvent::TaskProgressFinished);

                        // Generate short hex task ID
                        let task_id = format!(
                            "{:07x}",
                            std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default()
                                .subsec_nanos()
                        );
                        let query_summary: String = msg.chars().take(60).collect();

                        // Fork session for background task
                        let forked_sm = if let Some(session) =
                            runtime.session_manager.current_session()
                        {
                            let session_id = session.id.clone();
                            match runtime.session_manager.fork_session(&session_id, None) {
                                Ok(forked_session) => {
                                    // Create a new SessionManager for the background task
                                    let session_dir =
                                        runtime.session_manager.session_dir().to_path_buf();
                                    match SessionManager::new(session_dir) {
                                        Ok(mut sm) => {
                                            sm.set_current_session(forked_session);
                                            Some(sm)
                                        }
                                        Err(e) => {
                                            warn!(
                                                "Failed to create background session manager: {e}"
                                            );
                                            None
                                        }
                                    }
                                }
                                Err(e) => {
                                    warn!("Failed to fork session for background: {e}");
                                    None
                                }
                            }
                        } else {
                            None
                        };

                        if let Some(bg_session_manager) = forked_sm {
                            let session_id = bg_session_manager
                                .current_session()
                                .map(|s| s.id.clone())
                                .unwrap_or_default();

                            // Create background runtime
                            match runtime.create_background_runtime(bg_session_manager) {
                                Ok(mut bg_runtime) => {
                                    // Fresh tokens for background task
                                    let bg_interrupt = InterruptToken::new();
                                    let bg_interrupt_for_mgr = bg_interrupt.clone();

                                    let bg_tx = event_tx.clone();
                                    let bg_task_id = task_id.clone();
                                    let bg_msg = msg.clone();
                                    let bg_system_prompt = system_prompt.clone();

                                    // Register in the manager via event
                                    let _ = event_tx.send(AppEvent::AgentBackgrounded {
                                        task_id: task_id.clone(),
                                        query_summary: query_summary.clone(),
                                    });

                                    // Spawn background task
                                    tokio::spawn(async move {
                                        let bg_callback = TuiEventCallback { tx: bg_tx.clone() };

                                        let result = bg_runtime
                                            .run_query(
                                                &bg_msg,
                                                &bg_system_prompt,
                                                Some(&bg_callback),
                                                Some(&bg_interrupt),
                                            )
                                            .await;

                                        let cost_usd = bg_runtime.total_cost_usd();

                                        match result {
                                            Ok(r) => {
                                                let _ = bg_tx.send(
                                                    AppEvent::BackgroundAgentCompleted {
                                                        task_id: bg_task_id,
                                                        success: r.success,
                                                        result_summary: if r.content.len() > 200 {
                                                            format!("{}...", &r.content[..200])
                                                        } else {
                                                            r.content
                                                        },
                                                        cost_usd,
                                                        tool_call_count: 0, // approximate
                                                    },
                                                );
                                            }
                                            Err(e) => {
                                                let _ = bg_tx.send(
                                                    AppEvent::BackgroundAgentCompleted {
                                                        task_id: bg_task_id,
                                                        success: false,
                                                        result_summary: e.to_string(),
                                                        cost_usd,
                                                        tool_call_count: 0,
                                                    },
                                                );
                                            }
                                        }
                                    });

                                    // Register task in bg_agent_manager via a closure
                                    // (manager is in TUI state — we communicate via events)
                                    // The AgentBackgrounded event handler in event_dispatch
                                    // will display the message. We need to also register
                                    // the task — send it through the state.
                                    // We'll handle registration in event_dispatch.
                                    // For now, emit the token info so event_dispatch can register.
                                    // Actually, we need to register from TUI side.
                                    // Let's add a dedicated registration event.
                                    let _ = event_tx.send(AppEvent::SetBackgroundAgentToken {
                                        task_id: task_id.clone(),
                                        query: msg.clone(),
                                        session_id,
                                        interrupt_token: bg_interrupt_for_mgr,
                                    });
                                }
                                Err(e) => {
                                    warn!("Failed to create background runtime: {e}");
                                    let _ = event_tx.send(AppEvent::AgentError(format!(
                                        "Failed to background agent: {e}"
                                    )));
                                }
                            }
                        } else {
                            let _ = event_tx.send(AppEvent::AgentError(
                                "Failed to fork session for background agent.".to_string(),
                            ));
                        }

                        // Free the foreground
                        let _ = event_tx.send(AppEvent::AgentFinished);
                    }
                    Ok(result) => {
                        let _ = event_tx.send(AppEvent::TaskProgressFinished);
                        if result.interrupted {
                            let _ = event_tx.send(AppEvent::AgentInterrupted);
                        } else {
                            let _ = event_tx.send(AppEvent::AgentFinished);
                            if !result.success {
                                let _ = event_tx.send(AppEvent::AgentError(
                                    "Agent completed with errors".to_string(),
                                ));
                            }
                        }
                    }
                    Err(e) => {
                        let _ = event_tx.send(AppEvent::TaskProgressFinished);
                        let _ = event_tx.send(AppEvent::AgentError(e.to_string()));
                    }
                }
            }
        });

        // If there's an initial message, inject it after a brief delay
        if let Some(msg) = self.initial_message {
            let init_tx = user_tx;
            tokio::spawn(async move {
                // Small delay to let the TUI initialize
                tokio::time::sleep(Duration::from_millis(100)).await;
                let _ = init_tx.send(msg);
            });
        }

        // Run the TUI (blocks until quit)
        app.run().await
    }
}
