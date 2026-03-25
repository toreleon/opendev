//! Low-level Telegram Bot API HTTP client.

use std::time::Duration;

use reqwest::Client;

use super::error::TelegramError;
use super::types::{Message, SendMessageRequest, TelegramResponse, Update, User};

/// HTTP client for the Telegram Bot API.
pub struct TelegramApi {
    token: String,
    client: Client,
    base_url: String,
}

impl TelegramApi {
    /// Create a new API client for the given bot token.
    pub fn new(token: String) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(35))
            .build()
            .expect("failed to build reqwest client");
        let base_url = format!("https://api.telegram.org/bot{}", token);
        Self {
            token,
            client,
            base_url,
        }
    }

    /// Validate the bot token and retrieve bot info.
    pub async fn get_me(&self) -> Result<User, TelegramError> {
        let url = format!("{}/getMe", self.base_url);
        let resp: TelegramResponse<User> = self.client.get(&url).send().await?.json().await?;

        if resp.ok {
            resp.result
                .ok_or_else(|| TelegramError::Api("getMe returned ok but no result".to_string()))
        } else {
            Err(TelegramError::Api(
                resp.description
                    .unwrap_or_else(|| "unknown error".to_string()),
            ))
        }
    }

    /// Long-poll for updates. Blocks server-side for up to 30 seconds.
    pub async fn get_updates(&self, offset: i64) -> Result<Vec<Update>, TelegramError> {
        let url = format!("{}/getUpdates?offset={}&timeout=30", self.base_url, offset);
        let resp: TelegramResponse<Vec<Update>> =
            self.client.get(&url).send().await?.json().await?;

        if resp.ok {
            Ok(resp.result.unwrap_or_default())
        } else {
            Err(TelegramError::Api(
                resp.description
                    .unwrap_or_else(|| "unknown error".to_string()),
            ))
        }
    }

    /// Send a text message.
    pub async fn send_message(&self, req: SendMessageRequest) -> Result<Message, TelegramError> {
        let url = format!("{}/sendMessage", self.base_url);
        let resp: TelegramResponse<Message> = self
            .client
            .post(&url)
            .json(&req)
            .send()
            .await?
            .json()
            .await?;

        if resp.ok {
            resp.result.ok_or_else(|| {
                TelegramError::Api("sendMessage returned ok but no result".to_string())
            })
        } else {
            Err(TelegramError::Api(
                resp.description
                    .unwrap_or_else(|| "unknown error".to_string()),
            ))
        }
    }

    /// Get the bot token (for diagnostics/logging, not for display).
    pub fn token(&self) -> &str {
        &self.token
    }
}
