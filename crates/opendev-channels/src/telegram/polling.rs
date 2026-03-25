//! Background polling loop for Telegram getUpdates.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use crate::router::{InboundMessage, MessageRouter};
use tokio::sync::{watch, RwLock};
use tracing::{debug, error, info, warn};

use super::api::TelegramApi;
use super::types::{EditMessageTextRequest, Message, SendMessageRequest};
use super::DmPolicy;

/// Minimum characters before sending the first draft message (better push notification UX).
const MIN_INITIAL_CHARS: usize = 30;
/// Throttle interval between message edits in milliseconds.
const THROTTLE_MS: u64 = 1000;

/// Background poller that fetches Telegram updates and routes them to the agent.
pub struct TelegramPoller {
    pub(super) api: Arc<TelegramApi>,
    pub(super) router: Arc<MessageRouter>,
    pub(super) bot_username: String,
    pub(super) bot_id: i64,
    pub(super) group_mention_only: bool,
    pub(super) dm_policy: DmPolicy,
    pub(super) allowed_users: Arc<RwLock<HashSet<String>>>,
}

impl TelegramPoller {
    /// Spawn the polling loop as a background tokio task.
    /// Returns a shutdown sender — drop or send `true` to stop polling.
    pub fn spawn(self: Arc<Self>) -> watch::Sender<bool> {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        tokio::spawn(async move {
            self.run(shutdown_rx).await;
        });
        shutdown_tx
    }

    /// Add a user to the runtime allowlist (called when pairing is approved).
    pub async fn approve_user(&self, user_id: &str) {
        let mut allowed = self.allowed_users.write().await;
        allowed.insert(user_id.to_string());
        info!("Telegram: approved user {}", user_id);
    }

    /// Remove a user from the runtime allowlist.
    pub async fn remove_user(&self, user_id: &str) {
        let mut allowed = self.allowed_users.write().await;
        allowed.remove(user_id);
        info!("Telegram: removed user {}", user_id);
    }

    /// Check if a user is in the allowlist.
    async fn is_user_allowed(&self, user_id: &str) -> bool {
        let allowed = self.allowed_users.read().await;
        allowed.contains(user_id)
    }

    async fn run(self: Arc<Self>, mut shutdown_rx: watch::Receiver<bool>) {
        let mut offset: i64 = 0;

        loop {
            tokio::select! {
                _ = shutdown_rx.changed() => {
                    debug!("Telegram poller received shutdown signal");
                    break;
                }
                result = self.api.get_updates(offset) => {
                    match result {
                        Ok(updates) => {
                            if !updates.is_empty() {
                                info!("Telegram: received {} update(s)", updates.len());
                            }
                            for update in updates {
                                offset = update.update_id + 1;
                                if let Some(msg) = update.message {
                                    debug!(
                                        "Telegram: message from user={} chat={} text={:?}",
                                        msg.from.as_ref().map(|u| u.id).unwrap_or(0),
                                        msg.chat.id,
                                        msg.text.as_deref().unwrap_or("(no text)")
                                    );
                                    let poller = Arc::clone(&self);
                                    tokio::spawn(async move {
                                        poller.handle_message(&msg).await;
                                    });
                                } else {
                                    debug!("Telegram: update {} has no message field", update.update_id);
                                }
                            }
                        }
                        Err(e) => {
                            error!("Telegram getUpdates error: {}", e);
                            tokio::time::sleep(Duration::from_secs(5)).await;
                        }
                    }
                }
            }
        }
    }

    async fn handle_message(&self, msg: &Message) {
        let text = match &msg.text {
            Some(t) => t.clone(),
            None => return, // ignore non-text messages
        };

        let from = match &msg.from {
            Some(u) => u,
            None => return,
        };

        let is_private = msg.chat.chat_type == "private";
        let sender_id = from.id.to_string();

        // ── DM access control (pairing) ──
        if is_private
            && self.dm_policy != DmPolicy::Open
            && !self.is_user_allowed(&sender_id).await
        {
            match self.dm_policy {
                DmPolicy::Pairing => {
                    self.send_pairing_challenge(msg, &sender_id).await;
                    return;
                }
                DmPolicy::Allowlist => {
                    debug!("Telegram: ignoring message from non-allowed user {}", sender_id);
                    return;
                }
                DmPolicy::Open => unreachable!(),
            }
        }

        // ── Group chat filtering ──
        if !is_private && self.group_mention_only {
            let mention = format!("@{}", self.bot_username);
            let is_mention = text.contains(&mention);
            let is_reply_to_bot = msg
                .reply_to_message
                .as_ref()
                .and_then(|r| r.from.as_ref())
                .is_some_and(|u| u.id == self.bot_id);

            if !is_mention && !is_reply_to_bot {
                debug!("Telegram: skipping group message (no mention/reply)");
                return;
            }
        }

        // Strip @mention from text
        let mention = format!("@{}", self.bot_username);
        let clean_text = text.replace(&mention, "").trim().to_string();

        // Handle built-in commands locally
        if clean_text == "/start" || clean_text == "/help" {
            let help_text =
                "I'm an OpenDev AI assistant. Send me a message to get started.";
            let _ = self
                .api
                .send_message(SendMessageRequest {
                    chat_id: msg.chat.id,
                    text: help_text.to_string(),
                    parse_mode: None,
                    reply_to_message_id: None,
                })
                .await;
            return;
        }

        // Skip empty messages after stripping
        if clean_text.is_empty() {
            return;
        }

        let chat_id = msg.chat.id;
        // Send typing indicator and keep it alive while processing
        let api_for_typing = Arc::clone(&self.api);
        let typing_cancel = tokio_util::sync::CancellationToken::new();
        let typing_token = typing_cancel.clone();
        tokio::spawn(async move {
            loop {
                let _ = api_for_typing.send_chat_action(chat_id, "typing").await;
                tokio::select! {
                    _ = typing_token.cancelled() => break,
                    _ = tokio::time::sleep(Duration::from_secs(4)) => {}
                }
            }
        });

        // Create streaming channel
        let (chunk_tx, chunk_rx) = tokio::sync::mpsc::unbounded_channel::<String>();

        // Spawn the draft stream editor task (OpenClaw-style progressive editing)
        let api_for_draft = Arc::clone(&self.api);
        let draft_handle = tokio::spawn(Self::run_draft_stream(
            api_for_draft,
            chat_id,
            chunk_rx,
        ));

        // Build metadata with chat_id for delivery context passthrough
        let mut metadata = HashMap::new();
        metadata.insert("chat_id".to_string(), serde_json::json!(chat_id));
        metadata.insert("message_id".to_string(), serde_json::json!(msg.message_id));
        if let Some(ref username) = from.username {
            metadata.insert("username".to_string(), serde_json::json!(username));
        }

        let chat_type = if is_private { "direct" } else { "group" };

        let inbound = InboundMessage {
            channel: "telegram".to_string(),
            user_id: sender_id,
            thread_id: Some(msg.chat.id.to_string()),
            text: clean_text,
            timestamp: chrono::DateTime::from_timestamp(msg.date, 0)
                .unwrap_or_else(chrono::Utc::now),
            chat_type: chat_type.to_string(),
            reply_to_message_id: None,
            metadata,
        };

        debug!("Telegram: routing message to agent (text={:?})", inbound.text);

        // Execute with streaming — chunks flow to the draft stream task
        let result = self
            .router
            .handle_inbound_streaming(inbound, chunk_tx)
            .await;

        // Wait for draft stream to finish flushing
        let draft_message_id = draft_handle.await.unwrap_or(None);

        // Stop typing indicator
        typing_cancel.cancel();

        // ── Final delivery ──
        // Convert the final response to Telegram HTML and send/edit
        match result {
            Ok(final_text) => {
                let trimmed = final_text.trim();
                if trimmed.is_empty() {
                    if let Some(mid) = draft_message_id {
                        let _ = self
                            .api
                            .edit_message_text(EditMessageTextRequest {
                                chat_id,
                                message_id: mid,
                                text: "Done.".to_string(),
                                parse_mode: None,
                            })
                            .await;
                    }
                    return;
                }

                // Convert markdown to Telegram HTML
                let html = super::format::markdown_to_telegram_html(trimmed);

                // Split into chunks respecting HTML tag boundaries
                let chunks = super::format::split_telegram_html(&html, 4000);

                for (i, chunk) in chunks.iter().enumerate() {
                    if i == 0 {
                        if let Some(mid) = draft_message_id {
                            // Edit the draft message with final HTML content
                            let edit_result = self
                                .api
                                .edit_message_text(EditMessageTextRequest {
                                    chat_id,
                                    message_id: mid,
                                    text: chunk.clone(),
                                    parse_mode: Some("HTML".to_string()),
                                })
                                .await;

                            // Fall back to plain text if HTML parsing fails
                            if edit_result.is_err() {
                                debug!("Telegram: HTML edit failed, falling back to plain text");
                                let _ = self
                                    .api
                                    .edit_message_text(EditMessageTextRequest {
                                        chat_id,
                                        message_id: mid,
                                        text: trimmed.to_string(),
                                        parse_mode: None,
                                    })
                                    .await;
                                return;
                            }
                        } else {
                            // No draft was sent (very short response) — send fresh
                            let send_result = self
                                .api
                                .send_message(SendMessageRequest {
                                    chat_id,
                                    text: chunk.clone(),
                                    parse_mode: Some("HTML".to_string()),
                                    reply_to_message_id: None,
                                })
                                .await;

                            if send_result.is_err() {
                                let _ = self
                                    .api
                                    .send_message(SendMessageRequest {
                                        chat_id,
                                        text: trimmed.to_string(),
                                        parse_mode: None,
                                        reply_to_message_id: None,
                                    })
                                    .await;
                                return;
                            }
                        }
                    } else {
                        // Additional chunks — send as new messages
                        let send_result = self
                            .api
                            .send_message(SendMessageRequest {
                                chat_id,
                                text: chunk.clone(),
                                parse_mode: Some("HTML".to_string()),
                                reply_to_message_id: None,
                            })
                            .await;

                        if send_result.is_err() {
                            // Fall back to plain text for this chunk
                            let plain_chunk = strip_html_tags(chunk);
                            let _ = self
                                .api
                                .send_message(SendMessageRequest {
                                    chat_id,
                                    text: plain_chunk,
                                    parse_mode: None,
                                    reply_to_message_id: None,
                                })
                                .await;
                        }
                    }
                }
            }
            Err(e) => {
                error!("Telegram: agent execution failed: {}", e);
                let error_text = format!("Error: {}", e);
                if let Some(mid) = draft_message_id {
                    let _ = self
                        .api
                        .edit_message_text(EditMessageTextRequest {
                            chat_id,
                            message_id: mid,
                            text: error_text,
                            parse_mode: None,
                        })
                        .await;
                } else {
                    let _ = self
                        .api
                        .send_message(SendMessageRequest {
                            chat_id,
                            text: error_text,
                            parse_mode: None,
                            reply_to_message_id: None,
                        })
                        .await;
                }
            }
        }
    }

    /// OpenClaw-style draft stream: progressively edits a Telegram message
    /// as LLM tokens arrive. Waits for `MIN_INITIAL_CHARS` before sending the
    /// first message, then throttles edits at `THROTTLE_MS` intervals.
    ///
    /// Returns the message_id of the draft (or None if nothing was sent).
    async fn run_draft_stream(
        api: Arc<TelegramApi>,
        chat_id: i64,
        mut chunk_rx: tokio::sync::mpsc::UnboundedReceiver<String>,
    ) -> Option<i64> {
        let mut accumulated = String::new();
        let mut draft_message_id: Option<i64> = None;
        let mut last_edit = tokio::time::Instant::now();
        let mut pending = false;

        loop {
            let chunk = tokio::time::timeout(
                Duration::from_millis(THROTTLE_MS),
                chunk_rx.recv(),
            )
            .await;

            match chunk {
                Ok(Some(text)) => {
                    accumulated.push_str(&text);
                    pending = true;
                }
                Ok(None) => break, // channel closed — done
                Err(_) => {}       // timeout — flush below
            }

            if !pending || accumulated.trim().is_empty() {
                continue;
            }

            let should_edit = last_edit.elapsed() >= Duration::from_millis(THROTTLE_MS);

            if let Some(mid) = draft_message_id {
                if should_edit {
                    // Subsequent edits with cursor
                    let display = format!("{}▍", accumulated.trim());
                    let _ = api
                        .edit_message_text(EditMessageTextRequest {
                            chat_id,
                            message_id: mid,
                            text: display,
                            parse_mode: None,
                        })
                        .await;
                    last_edit = tokio::time::Instant::now();
                    pending = false;
                }
            } else if accumulated.trim().len() >= MIN_INITIAL_CHARS {
                // First send: wait until we have enough content
                let display = format!("{}▍", accumulated.trim());
                match api
                    .send_message(SendMessageRequest {
                        chat_id,
                        text: display,
                        parse_mode: None,
                        reply_to_message_id: None,
                    })
                    .await
                {
                    Ok(m) => {
                        draft_message_id = Some(m.message_id);
                        last_edit = tokio::time::Instant::now();
                        pending = false;
                    }
                    Err(e) => {
                        error!("Telegram: failed to send draft: {}", e);
                    }
                }
            }
        }

        // Final flush — remove cursor, show accumulated text as-is
        if let Some(mid) = draft_message_id
            && pending
            && !accumulated.trim().is_empty()
        {
            let _ = api
                .edit_message_text(EditMessageTextRequest {
                    chat_id,
                    message_id: mid,
                    text: accumulated.trim().to_string(),
                    parse_mode: None,
                })
                .await;
        }

        draft_message_id
    }

    /// Send a pairing challenge to an unknown user.
    async fn send_pairing_challenge(&self, msg: &Message, sender_id: &str) {
        let sender_name = msg
            .from
            .as_ref()
            .map(|u| {
                u.username
                    .as_ref()
                    .map(|n| format!("@{n}"))
                    .unwrap_or_else(|| u.first_name.clone())
            })
            .unwrap_or_else(|| "Unknown".to_string());

        warn!(
            "Telegram: pairing request from {} (ID: {})",
            sender_name, sender_id
        );

        let challenge_text = format!(
            "🔒 Access required.\n\n\
             Your Telegram ID: `{}`\n\n\
             Ask the bot owner to approve you:\n\
             ```\nopendev channel pair telegram {}\n```",
            sender_id, sender_id,
        );

        let _ = self
            .api
            .send_message(SendMessageRequest {
                chat_id: msg.chat.id,
                text: challenge_text,
                parse_mode: Some("Markdown".to_string()),
                reply_to_message_id: None,
            })
            .await;
    }
}

/// Strip HTML tags for plain-text fallback.
fn strip_html_tags(html: &str) -> String {
    let mut result = String::with_capacity(html.len());
    let mut in_tag = false;
    for ch in html.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => result.push(ch),
            _ => {}
        }
    }
    // Unescape HTML entities
    result
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
}
