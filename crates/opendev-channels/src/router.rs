//! Message router for multi-channel agent.
//!
//! Routes inbound messages from channels (CLI, web, Telegram, etc.) to
//! sessions and dispatches responses back to the correct channel/user.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, error, info};

use crate::error::{ChannelError, ChannelResult};

/// An inbound message from a channel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InboundMessage {
    /// Channel this message came from (e.g., "cli", "web", "telegram").
    pub channel: String,
    /// User identifier within the channel.
    pub user_id: String,
    /// Optional thread/conversation identifier.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    /// Message text content.
    pub text: String,
    /// Timestamp of the message.
    pub timestamp: DateTime<Utc>,
    /// Chat type (e.g., "direct", "group").
    #[serde(default = "default_chat_type")]
    pub chat_type: String,
    /// Optional message ID to reply to.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply_to_message_id: Option<String>,
    /// Additional channel-specific metadata.
    #[serde(default)]
    pub metadata: HashMap<String, serde_json::Value>,
}

fn default_chat_type() -> String {
    "direct".to_string()
}

/// An outbound message to send via a channel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutboundMessage {
    /// Message text content.
    pub text: String,
    /// Thread/conversation to send to.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    /// Message to reply to.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply_to_message_id: Option<String>,
    /// Parse mode for formatting (e.g., "markdown").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parse_mode: Option<String>,
}

/// Delivery context containing channel-specific addressing info.
pub type DeliveryContext = HashMap<String, serde_json::Value>;

/// Trait for channel adapters that can send/receive messages.
#[async_trait]
pub trait ChannelAdapter: Send + Sync {
    /// Get the channel name (e.g., "telegram", "web", "cli").
    fn channel_name(&self) -> &str;

    /// Send a message to the channel.
    async fn send(
        &self,
        delivery_context: &DeliveryContext,
        message: OutboundMessage,
    ) -> ChannelResult<()>;
}

/// Callback type for agent execution.
///
/// Takes (session_id, message_text) and returns agent response text.
#[async_trait]
pub trait AgentExecutor: Send + Sync {
    async fn execute(&self, session_id: &str, message_text: &str) -> ChannelResult<String>;

    /// Execute with streaming — sends text chunks through the provided channel
    /// as they arrive from the LLM. Returns the final complete response.
    ///
    /// Default implementation falls back to non-streaming `execute`.
    async fn execute_streaming(
        &self,
        session_id: &str,
        message_text: &str,
        _chunk_tx: tokio::sync::mpsc::UnboundedSender<String>,
    ) -> ChannelResult<String> {
        self.execute(session_id, message_text).await
    }
}

/// Routes inbound messages from channels to agent sessions.
///
/// The router is the central coordinator for multi-channel messaging:
/// 1. Receives InboundMessage from channel adapters
/// 2. Resolves or creates appropriate session (by channel+user)
/// 3. Dispatches messages to the agent for processing
/// 4. Routes agent responses back to the correct channel
pub struct MessageRouter {
    adapters: Arc<RwLock<HashMap<String, Arc<dyn ChannelAdapter>>>>,
    executor: Arc<RwLock<Option<Arc<dyn AgentExecutor>>>>,
    /// Maps (channel, user_id, thread_id) -> session_id.
    session_map: Arc<RwLock<HashMap<SessionKey, String>>>,
    /// Delivery contexts keyed by session_id.
    delivery_contexts: Arc<RwLock<HashMap<String, DeliveryContext>>>,
}

/// Key for looking up sessions by channel + user + thread.
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
struct SessionKey {
    channel: String,
    user_id: String,
    thread_id: Option<String>,
}

impl MessageRouter {
    /// Create a new message router.
    pub fn new() -> Self {
        Self {
            adapters: Arc::new(RwLock::new(HashMap::new())),
            executor: Arc::new(RwLock::new(None)),
            session_map: Arc::new(RwLock::new(HashMap::new())),
            delivery_contexts: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Set the agent executor for processing messages.
    pub async fn set_executor(&self, executor: Arc<dyn AgentExecutor>) {
        let mut exec = self.executor.write().await;
        *exec = Some(executor);
    }

    /// Register a channel adapter for routing.
    pub async fn register_adapter(&self, adapter: Arc<dyn ChannelAdapter>) {
        let name = adapter.channel_name().to_string();
        let mut adapters = self.adapters.write().await;
        adapters.insert(name.clone(), adapter);
        info!("Registered channel adapter: {}", name);
    }

    /// Get a registered adapter by channel name.
    pub async fn get_adapter(&self, channel_name: &str) -> Option<Arc<dyn ChannelAdapter>> {
        let adapters = self.adapters.read().await;
        adapters.get(channel_name).cloned()
    }

    /// Route an inbound message to the correct session and agent.
    pub async fn handle_inbound(&self, message: InboundMessage) -> ChannelResult<()> {
        info!(
            "Routing message from {}:{} (thread={:?})",
            message.channel, message.user_id, message.thread_id
        );

        // Get channel adapter
        let adapter = self
            .get_adapter(&message.channel)
            .await
            .ok_or_else(|| ChannelError::AdapterNotFound(message.channel.clone()))?;

        // Resolve or create session
        let session_id = self.resolve_session(&message).await;

        // Store delivery context
        {
            let mut contexts = self.delivery_contexts.write().await;
            let mut ctx = DeliveryContext::new();
            ctx.insert(
                "channel".to_string(),
                serde_json::Value::String(message.channel.clone()),
            );
            ctx.insert(
                "user_id".to_string(),
                serde_json::Value::String(message.user_id.clone()),
            );
            if let Some(ref tid) = message.thread_id {
                ctx.insert(
                    "thread_id".to_string(),
                    serde_json::Value::String(tid.clone()),
                );
            }
            for (k, v) in &message.metadata {
                ctx.insert(k.clone(), v.clone());
            }
            contexts.insert(session_id.clone(), ctx);
        }

        // Dispatch to agent
        let executor = self.executor.read().await;
        if let Some(ref exec) = *executor {
            match exec.execute(&session_id, &message.text).await {
                Ok(response) => {
                    let contexts = self.delivery_contexts.read().await;
                    let delivery_context = contexts.get(&session_id).cloned().unwrap_or_default();

                    let outbound = OutboundMessage {
                        text: response,
                        thread_id: message.thread_id.clone(),
                        reply_to_message_id: message.reply_to_message_id.clone(),
                        parse_mode: None,
                    };

                    adapter.send(&delivery_context, outbound).await?;
                }
                Err(e) => {
                    error!("Error dispatching to agent: {}", e);
                    let contexts = self.delivery_contexts.read().await;
                    let delivery_context = contexts.get(&session_id).cloned().unwrap_or_default();

                    let error_msg = OutboundMessage {
                        text: format!("Sorry, I encountered an error: {}", e),
                        thread_id: message.thread_id.clone(),
                        reply_to_message_id: None,
                        parse_mode: None,
                    };

                    adapter.send(&delivery_context, error_msg).await?;
                }
            }
        }

        Ok(())
    }

    /// Route an inbound message with streaming support.
    ///
    /// Like `handle_inbound` but streams text chunks through `chunk_tx` and
    /// returns the final response text. The caller is responsible for delivering
    /// the response to the channel (e.g., by editing a Telegram message).
    pub async fn handle_inbound_streaming(
        &self,
        message: InboundMessage,
        chunk_tx: tokio::sync::mpsc::UnboundedSender<String>,
    ) -> ChannelResult<String> {
        info!(
            "Streaming route from {}:{} (thread={:?})",
            message.channel, message.user_id, message.thread_id
        );

        // Resolve or create session
        let session_id = self.resolve_session(&message).await;

        // Store delivery context
        {
            let mut contexts = self.delivery_contexts.write().await;
            let mut ctx = DeliveryContext::new();
            ctx.insert(
                "channel".to_string(),
                serde_json::Value::String(message.channel.clone()),
            );
            ctx.insert(
                "user_id".to_string(),
                serde_json::Value::String(message.user_id.clone()),
            );
            if let Some(ref tid) = message.thread_id {
                ctx.insert(
                    "thread_id".to_string(),
                    serde_json::Value::String(tid.clone()),
                );
            }
            for (k, v) in &message.metadata {
                ctx.insert(k.clone(), v.clone());
            }
            contexts.insert(session_id.clone(), ctx);
        }

        // Dispatch to agent with streaming
        let executor = self.executor.read().await;
        if let Some(ref exec) = *executor {
            exec.execute_streaming(&session_id, &message.text, chunk_tx)
                .await
        } else {
            Err(ChannelError::AgentError(
                "no executor configured".to_string(),
            ))
        }
    }

    /// Resolve an existing session or create a new one.
    async fn resolve_session(&self, message: &InboundMessage) -> String {
        let key = SessionKey {
            channel: message.channel.clone(),
            user_id: message.user_id.clone(),
            thread_id: message.thread_id.clone(),
        };

        let session_map = self.session_map.read().await;
        if let Some(session_id) = session_map.get(&key) {
            debug!("Found existing session: {}", session_id);
            return session_id.clone();
        }
        drop(session_map);

        // Create new session ID
        let session_id = uuid::Uuid::new_v4().to_string()[..12].to_string();
        info!(
            "Created new session {} for {}:{}",
            session_id, message.channel, message.user_id
        );

        let mut session_map = self.session_map.write().await;
        session_map.insert(key, session_id.clone());

        session_id
    }

    /// Get the number of registered adapters.
    pub async fn adapter_count(&self) -> usize {
        let adapters = self.adapters.read().await;
        adapters.len()
    }

    /// Get the number of active sessions.
    pub async fn session_count(&self) -> usize {
        let sessions = self.session_map.read().await;
        sessions.len()
    }

    /// Get all registered channel names.
    pub async fn channel_names(&self) -> Vec<String> {
        let adapters = self.adapters.read().await;
        adapters.keys().cloned().collect()
    }
}

impl Default for MessageRouter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A simple test adapter that records sent messages.
    struct TestAdapter {
        name: String,
        sent: Arc<RwLock<Vec<OutboundMessage>>>,
    }

    impl TestAdapter {
        fn new(name: &str) -> (Arc<Self>, Arc<RwLock<Vec<OutboundMessage>>>) {
            let sent = Arc::new(RwLock::new(Vec::new()));
            let adapter = Arc::new(Self {
                name: name.to_string(),
                sent: sent.clone(),
            });
            (adapter, sent)
        }
    }

    #[async_trait]
    impl ChannelAdapter for TestAdapter {
        fn channel_name(&self) -> &str {
            &self.name
        }

        async fn send(
            &self,
            _delivery_context: &DeliveryContext,
            message: OutboundMessage,
        ) -> ChannelResult<()> {
            let mut sent = self.sent.write().await;
            sent.push(message);
            Ok(())
        }
    }

    /// A test executor that echoes messages.
    struct EchoExecutor;

    #[async_trait]
    impl AgentExecutor for EchoExecutor {
        async fn execute(&self, _session_id: &str, message_text: &str) -> ChannelResult<String> {
            Ok(format!("Echo: {}", message_text))
        }
    }

    #[tokio::test]
    async fn test_router_new() {
        let router = MessageRouter::new();
        assert_eq!(router.adapter_count().await, 0);
        assert_eq!(router.session_count().await, 0);
    }

    #[tokio::test]
    async fn test_register_adapter() {
        let router = MessageRouter::new();
        let (adapter, _) = TestAdapter::new("test-channel");

        router.register_adapter(adapter).await;
        assert_eq!(router.adapter_count().await, 1);
        assert!(router.get_adapter("test-channel").await.is_some());
        assert!(router.get_adapter("nonexistent").await.is_none());
    }

    #[tokio::test]
    async fn test_channel_names() {
        let router = MessageRouter::new();
        let (adapter1, _) = TestAdapter::new("cli");
        let (adapter2, _) = TestAdapter::new("web");

        router.register_adapter(adapter1).await;
        router.register_adapter(adapter2).await;

        let mut names = router.channel_names().await;
        names.sort();
        assert_eq!(names, vec!["cli", "web"]);
    }

    #[tokio::test]
    async fn test_handle_inbound_no_adapter() {
        let router = MessageRouter::new();

        let message = InboundMessage {
            channel: "unknown".to_string(),
            user_id: "user1".to_string(),
            thread_id: None,
            text: "hello".to_string(),
            timestamp: Utc::now(),
            chat_type: "direct".to_string(),
            reply_to_message_id: None,
            metadata: HashMap::new(),
        };

        let result = router.handle_inbound(message).await;
        assert!(matches!(result, Err(ChannelError::AdapterNotFound(_))));
    }

    #[tokio::test]
    async fn test_handle_inbound_with_executor() {
        let router = MessageRouter::new();
        let (adapter, sent) = TestAdapter::new("test");
        router.register_adapter(adapter).await;
        router.set_executor(Arc::new(EchoExecutor)).await;

        let message = InboundMessage {
            channel: "test".to_string(),
            user_id: "user1".to_string(),
            thread_id: None,
            text: "hello world".to_string(),
            timestamp: Utc::now(),
            chat_type: "direct".to_string(),
            reply_to_message_id: None,
            metadata: HashMap::new(),
        };

        router.handle_inbound(message).await.unwrap();

        let sent = sent.read().await;
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].text, "Echo: hello world");
    }

    #[tokio::test]
    async fn test_session_reuse() {
        let router = MessageRouter::new();
        let (adapter, sent) = TestAdapter::new("test");
        router.register_adapter(adapter).await;
        router.set_executor(Arc::new(EchoExecutor)).await;

        // Send two messages from the same user
        for text in &["msg1", "msg2"] {
            let message = InboundMessage {
                channel: "test".to_string(),
                user_id: "user1".to_string(),
                thread_id: None,
                text: text.to_string(),
                timestamp: Utc::now(),
                chat_type: "direct".to_string(),
                reply_to_message_id: None,
                metadata: HashMap::new(),
            };
            router.handle_inbound(message).await.unwrap();
        }

        // Should still be 1 session (reused)
        assert_eq!(router.session_count().await, 1);

        let sent = sent.read().await;
        assert_eq!(sent.len(), 2);
    }

    #[tokio::test]
    async fn test_different_users_different_sessions() {
        let router = MessageRouter::new();
        let (adapter, _) = TestAdapter::new("test");
        router.register_adapter(adapter).await;
        router.set_executor(Arc::new(EchoExecutor)).await;

        for user in &["user1", "user2"] {
            let message = InboundMessage {
                channel: "test".to_string(),
                user_id: user.to_string(),
                thread_id: None,
                text: "hello".to_string(),
                timestamp: Utc::now(),
                chat_type: "direct".to_string(),
                reply_to_message_id: None,
                metadata: HashMap::new(),
            };
            router.handle_inbound(message).await.unwrap();
        }

        assert_eq!(router.session_count().await, 2);
    }

    #[tokio::test]
    async fn test_inbound_message_serialization() {
        let msg = InboundMessage {
            channel: "telegram".to_string(),
            user_id: "12345".to_string(),
            thread_id: Some("thread-1".to_string()),
            text: "Hello bot".to_string(),
            timestamp: Utc::now(),
            chat_type: "group".to_string(),
            reply_to_message_id: None,
            metadata: HashMap::new(),
        };

        let json = serde_json::to_string(&msg).unwrap();
        let deserialized: InboundMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.channel, "telegram");
        assert_eq!(deserialized.user_id, "12345");
        assert_eq!(deserialized.thread_id.as_deref(), Some("thread-1"));
    }
}
