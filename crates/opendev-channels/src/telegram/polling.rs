//! Background polling loop for Telegram getUpdates.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use crate::router::{InboundMessage, MessageRouter};
use tokio::sync::watch;
use tracing::{debug, error};

use super::api::TelegramApi;
use super::types::{Message, SendMessageRequest};

/// Background poller that fetches Telegram updates and routes them to the agent.
pub struct TelegramPoller {
    pub(super) api: Arc<TelegramApi>,
    pub(super) router: Arc<MessageRouter>,
    pub(super) bot_username: String,
    pub(super) bot_id: i64,
    pub(super) group_mention_only: bool,
}

impl TelegramPoller {
    /// Spawn the polling loop as a background tokio task.
    /// Returns a shutdown sender — drop or send `true` to stop polling.
    pub fn spawn(self) -> watch::Sender<bool> {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        tokio::spawn(async move {
            self.run(shutdown_rx).await;
        });
        shutdown_tx
    }

    async fn run(self, mut shutdown_rx: watch::Receiver<bool>) {
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
                            for update in updates {
                                offset = update.update_id + 1;
                                if let Some(ref msg) = update.message {
                                    self.handle_message(msg).await;
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

        // Group chat filtering: only respond to @mentions or replies to bot
        if !is_private && self.group_mention_only {
            let mention = format!("@{}", self.bot_username);
            let is_mention = text.contains(&mention);
            let is_reply_to_bot = msg
                .reply_to_message
                .as_ref()
                .and_then(|r| r.from.as_ref())
                .is_some_and(|u| u.id == self.bot_id);

            if !is_mention && !is_reply_to_bot {
                return;
            }
        }

        // Strip @mention from text
        let mention = format!("@{}", self.bot_username);
        let clean_text = text.replace(&mention, "").trim().to_string();

        // Handle built-in commands locally
        if clean_text == "/start" || clean_text == "/help" {
            let help_text = "I'm an OpenDev AI assistant. Send me a message to get started.";
            let _ = self
                .api
                .send_message(SendMessageRequest {
                    chat_id: msg.chat.id,
                    text: help_text.to_string(),
                    parse_mode: None,
                    reply_to_message_id: Some(msg.message_id),
                })
                .await;
            return;
        }

        // Skip empty messages after stripping
        if clean_text.is_empty() {
            return;
        }

        // Build metadata with chat_id for delivery context passthrough
        let mut metadata = HashMap::new();
        metadata.insert("chat_id".to_string(), serde_json::json!(msg.chat.id));
        metadata.insert("message_id".to_string(), serde_json::json!(msg.message_id));
        if let Some(ref username) = from.username {
            metadata.insert("username".to_string(), serde_json::json!(username));
        }

        let chat_type = if is_private { "direct" } else { "group" };

        let inbound = InboundMessage {
            channel: "telegram".to_string(),
            user_id: from.id.to_string(),
            thread_id: Some(msg.chat.id.to_string()),
            text: clean_text,
            timestamp: chrono::DateTime::from_timestamp(msg.date, 0)
                .unwrap_or_else(chrono::Utc::now),
            chat_type: chat_type.to_string(),
            reply_to_message_id: Some(msg.message_id.to_string()),
            metadata,
        };

        if let Err(e) = self.router.handle_inbound(inbound).await {
            error!("Failed to route Telegram message: {}", e);
        }
    }
}
