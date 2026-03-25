//! Telegram bot adapter for OpenDev channels.
//!
//! Provides a `ChannelAdapter` implementation that connects to the Telegram
//! Bot API via long polling, routing messages through the `MessageRouter`.

pub mod adapter;
pub mod api;
pub mod error;
pub mod polling;
pub mod types;

pub use adapter::TelegramAdapter;
pub use error::TelegramError;
pub use polling::TelegramPoller;

use std::sync::Arc;

use crate::router::MessageRouter;
use tokio::sync::watch;
use tracing::info;

use api::TelegramApi;

/// Token configuration for the Telegram bot.
pub struct TelegramConfig {
    pub bot_token: String,
    pub enabled: bool,
    pub group_mention_only: bool,
}

/// Resolve the bot token from config or environment.
///
/// Priority: explicit config > `TELEGRAM_BOT_TOKEN` env var.
pub fn resolve_token(config: Option<&TelegramConfig>) -> Result<String, TelegramError> {
    if let Some(cfg) = config
        && !cfg.bot_token.is_empty()
    {
        return Ok(cfg.bot_token.clone());
    }

    std::env::var("TELEGRAM_BOT_TOKEN").map_err(|_| TelegramError::InvalidToken)
}

/// Build and start the Telegram adapter and polling loop.
///
/// Validates the bot token via `getMe`, registers the adapter with the router,
/// and spawns a background polling task.
///
/// Returns the adapter and a shutdown handle. Drop the handle to stop polling.
pub async fn start_telegram(
    config: Option<&TelegramConfig>,
    router: Arc<MessageRouter>,
) -> Result<(Arc<TelegramAdapter>, watch::Sender<bool>), TelegramError> {
    let token = resolve_token(config)?;
    let group_mention_only = config.map(|c| c.group_mention_only).unwrap_or(true);

    let api = Arc::new(TelegramApi::new(token));

    // Validate token
    let me = api.get_me().await?;
    let bot_username = me.username.unwrap_or_default();
    let bot_id = me.id;

    info!("Telegram bot authenticated as @{}", bot_username);

    let adapter = Arc::new(TelegramAdapter {
        api: api.clone(),
        bot_username: bot_username.clone(),
    });

    // Register with router
    router.register_adapter(adapter.clone()).await;

    // Start polling
    let poller = TelegramPoller {
        api,
        router,
        bot_username,
        bot_id,
        group_mention_only,
    };
    let shutdown = poller.spawn();

    Ok((adapter, shutdown))
}
