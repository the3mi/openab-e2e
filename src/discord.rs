use anyhow::{bail, Context, Result};
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::{debug, info};

const DISCORD_API: &str = "https://discord.com/api/v10";
const MAX_RETRIES: u32 = 3;

/// Discord message with thread info.
#[derive(Debug, Clone, Deserialize)]
pub struct Message {
    pub id: String,
    pub content: String,
    pub author: Author,
    #[serde(default)]
    pub thread: Option<ThreadInfo>,
    #[serde(default)]
    pub message_reference: Option<MessageReference>,
    #[serde(rename = "channel_id", default)]
    pub channel_id: String,
    #[serde(rename = "edited_timestamp", default)]
    pub edited_timestamp: Option<String>,
    #[serde(rename = "last_message_id", default)]
    pub last_message_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MessageReference {
    pub message_id: Option<String>,
    #[serde(rename = "channel_id", default)]
    pub channel_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Author {
    pub id: String,
    pub username: String,
    #[serde(default)]
    pub bot: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ThreadInfo {
    pub id: String,
    pub name: String,
    #[serde(rename = "last_message_id", default)]
    pub last_message_id: Option<String>,
    #[serde(rename = "parent_id", default)]
    pub parent_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Channel {
    pub id: String,
    pub name: Option<String>,
    #[serde(rename = "type")]
    pub channel_type: u8,
}

#[derive(Debug, Serialize)]
struct CreateMessage {
    content: String,
}

pub struct DiscordClient {
    client: reqwest::Client,
    target_bot_id: String,
    max_retries: u32,
}

impl DiscordClient {
    pub fn new(bot_token: &str, target_bot_id: &str, max_retries: u32) -> Result<Self> {
        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bot {bot_token}"))
                .context("Invalid bot token")?,
        );
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

        let client = reqwest::Client::builder()
            .default_headers(headers)
            .timeout(Duration::from_secs(30))
            .build()
            .context("Failed to build HTTP client")?;

        Ok(Self {
            client,
            target_bot_id: target_bot_id.to_string(),
            max_retries,
        })
    }

    /// Send a message to a channel or thread.
    pub async fn send_message(&self, channel_or_thread_id: &str, content: &str) -> Result<Message> {
        let url = format!("{}/channels/{}/messages", DISCORD_API, channel_or_thread_id);
        let body = CreateMessage {
            content: content.to_string(),
        };

        let resp = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .context("Failed to send message")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            bail!("Failed to send message: {} — {body}", status);
        }

        let value: serde_json::Value = resp.json().await.context("Failed to parse response")?;
        let msg: Message =
            serde_json::from_value(value).context("Failed to parse message JSON")?;
        info!(
            message_id = %msg.id,
            channel = %msg.channel_id,
            "Sent message"
        );
        Ok(msg)
    }

    /// Fetch a single message from a channel.
    pub async fn get_message(&self, channel_id: &str, message_id: &str) -> Result<Message> {
        let url = format!("{}/channels/{}/messages/{}", DISCORD_API, channel_id, message_id);
        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .context("Failed to fetch message")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            bail!("Failed to fetch message {}: {} — {body}", message_id, status);
        }

        let value: serde_json::Value = resp.json().await.context("Failed to parse response")?;
        let msg: Message =
            serde_json::from_value(value).context("Failed to parse message JSON")?;
        Ok(msg)
    }

    /// Fetch recent messages from a channel.
    pub async fn get_messages(&self, channel_id: &str, limit: u8) -> Result<Vec<Message>> {
        let url = format!(
            "{}/channels/{}/messages?limit={}",
            DISCORD_API,
            channel_id,
            limit.min(100)
        );
        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .context("Failed to fetch messages")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            bail!("Failed to fetch messages from {}: {} — {body}", channel_id, status);
        }

        let value: serde_json::Value = resp.json().await.context("Failed to parse response")?;
        let messages: Vec<Message> = serde_json::from_value(value)
            .context("Failed to parse messages JSON")?;
        debug!(channel = channel_id, count = messages.len(), "Fetched messages");
        Ok(messages)
    }

    /// Wait for a response from the target bot.
    ///
    /// Flow (ALWAYS reads from main channel to avoid thread 403):
    /// 1. Poll the sent message until it has a thread (bot replied)
    /// 2. Poll main channel messages for bot's reply (message_reference -> our message)
    /// 3. Return the bot's message and thread_id
    ///
    /// `target` = where we sent the message (main channel or thread)
    /// `sent_message_id` = the message we sent
    /// `main_channel_id` = the parent channel (for reading messages)
    pub async fn wait_for_bot_response(
        &self,
        target: &str,
        sent_message_id: &str,
        main_channel_id: &str,
        timeout: Duration,
        poll_interval: Duration,
    ) -> Result<(Message, String)> {
        let start = tokio::time::Instant::now();

        info!(
            target,
            sent_msg_id = sent_message_id,
            main_channel = main_channel_id,
            timeout_secs = timeout.as_secs(),
            "Waiting for bot response"
        );

        // Phase 1: If target is main channel, wait for thread creation.
        // If target is already a thread (subsequent tests), skip Phase 1.
        let discovered_thread_id = if target == main_channel_id {
            let discovered = loop {
                if start.elapsed() > timeout {
                    bail!(
                        "Timeout after {}s — no thread for message {}",
                        timeout.as_secs(),
                        sent_message_id
                    );
                }

                let msg = self.get_message(target, sent_message_id).await?;

                if let Some(ref thread_info) = msg.thread {
                    break thread_info.id.clone();
                }

                debug!("Sent message has no thread yet, polling...");
                tokio::time::sleep(poll_interval).await;
            };
            info!(thread_id = %discovered, "Thread created by bot");
            discovered
        } else {
            info!("Already in thread {}, skipping thread creation check", target);
            target.to_string()
        };

        // Phase 2: Poll the thread for the bot's reply message.
        // Bot replies IN THE THREAD it created, so we read from discovered_thread_id.
        let bot_msg = loop {
            if start.elapsed() > timeout {
                bail!(
                    "Timeout after {}s — bot message not found in thread {}",
                    timeout.as_secs(),
                    discovered_thread_id
                );
            }

            let messages = self.get_messages(&discovered_thread_id, 20).await?;

            // Find the LATEST bot message that came AFTER our sent message (newer snowflake ID)
            let found = messages.iter()
                .filter(|msg| msg.author.id == self.target_bot_id && msg.id.as_str() > sent_message_id)
                .max_by_key(|msg| msg.id.clone());

            if let Some(msg) = found {
                let c = msg.content.trim();
                if c.is_empty() || c == "..." {
                    debug!("Bot message still editing, retrying...");
                    tokio::time::sleep(Duration::from_millis(500)).await;
                    continue;
                }
                break msg.clone();
            }

            debug!("Bot reply not yet in thread, polling...");
            tokio::time::sleep(poll_interval).await;
        };

        info!(
            message_id = %bot_msg.id,
            author = %bot_msg.author.username,
            content = %bot_msg.content,
            "Bot response found"
        );

        Ok((bot_msg, discovered_thread_id))
    }

    /// Create a webhook in a channel for cap reset.
    pub async fn create_webhook(&self, channel_id: &str, name: &str) -> Result<Webhook> {
        let url = format!("{}/channels/{}/webhooks", DISCORD_API, channel_id);
        #[derive(Serialize)]
        struct CreateWebhook {
            name: String,
        }
        let resp = self
            .client
            .post(&url)
            .json(&CreateWebhook {
                name: name.to_string(),
            })
            .send()
            .await
            .context("Failed to create webhook")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            bail!("Failed to create webhook: {} — {body}", status);
        }

        let value: serde_json::Value = resp.json().await.context("Failed to parse webhook")?;
        let webhook: Webhook =
            serde_json::from_value(value).context("Failed to parse webhook JSON")?;
        Ok(webhook)
    }

    /// Execute a webhook (send a message via webhook).
    pub async fn execute_webhook(
        &self,
        webhook_url: &str,
        content: &str,
    ) -> Result<()> {
        #[derive(Serialize)]
        struct ExecuteWebhook {
            content: String,
        }
        let resp = self
            .client
            .post(webhook_url)
            .json(&ExecuteWebhook {
                content: content.to_string(),
            })
            .send()
            .await
            .context("Failed to execute webhook")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            bail!("Failed to execute webhook: {} — {body}", status);
        }

        Ok(())
    }

    /// List webhooks in a channel.
    pub async fn list_webhooks(&self, channel_id: &str) -> Result<Vec<Webhook>> {
        let url = format!("{}/channels/{}/webhooks", DISCORD_API, channel_id);
        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .context("Failed to list webhooks")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            bail!("Failed to list webhooks: {} — {body}", status);
        }

        let value: serde_json::Value = resp.json().await.context("Failed to parse webhooks")?;
        let webhooks: Vec<Webhook> =
            serde_json::from_value(value).context("Failed to parse webhooks JSON")?;
        Ok(webhooks)

    }
    /// Reset the bot-turn cap by posting via a temporary webhook in the channel.
    pub async fn send_webhook_cap_reset(&self, channel_id: &str) -> Result<()> {
        let webhooks = self.list_webhooks(channel_id).await?;
        let webhook = webhooks
            .iter()
            .find(|w| w.name == "cap-reset" && !w.token.is_empty())
            .context("No cap-reset webhook found")?;
        let webhook_url = format!("{}/webhooks/{}/{}", DISCORD_API, webhook.id, webhook.token);
        self.execute_webhook(&webhook_url, "cap reset").await?;
        info!(channel = channel_id, "Sent webhook cap-reset to channel");
        Ok(())
    }
}
#[derive(Debug, Clone, Deserialize)]
pub struct Webhook {
    pub id: String,
    pub name: String,
    pub token: String,
}
