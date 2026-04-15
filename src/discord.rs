use anyhow::{bail, Context, Result};
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::{debug, info, warn};

const DISCORD_API: &str = "https://discord.com/api/v10";

/// Discord REST API client.
pub struct DiscordClient {
    client: reqwest::Client,
    bot_token: String,
    target_bot_id: String,
    max_retries: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Message {
    pub id: String,
    pub content: String,
    pub author: Author,
    #[serde(default)]
    pub thread: Option<ThreadInfo>,
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

#[derive(Debug, Serialize)]
struct StartThread {
    name: String,
    #[serde(rename = "type")]
    thread_type: u8,
}

impl DiscordClient {
    pub fn new(bot_token: &str, target_bot_id: &str, max_retries: u32) -> Result<Self> {
        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bot {bot_token}"))
                .context("Invalid bot token format")?,
        );
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

        let client = reqwest::Client::builder()
            .default_headers(headers)
            .timeout(Duration::from_secs(30))
            .build()
            .context("Failed to build HTTP client")?;

        Ok(Self {
            client,
            bot_token: bot_token.to_string(),
            target_bot_id: target_bot_id.to_string(),
            max_retries,
        })
    }

    /// Send a message to a channel or thread and return the sent message.
    pub async fn send_message(&self, channel_id: &str, content: &str) -> Result<Message> {
        let url = format!("{DISCORD_API}/channels/{channel_id}/messages");
        let body = CreateMessage {
            content: content.to_string(),
        };

        let value = self
            .request_with_retry(|| {
                self.client.post(&url).json(&body).send()
            })
            .await
            .context("Failed to send message")?;

        let msg: Message = serde_json::from_value(value)
            .context("Failed to parse message JSON")?;

        info!(message_id = %msg.id, channel = channel_id, "Sent message");
        Ok(msg)
    }

    /// Fetch a single message by ID.
    pub async fn get_message(&self, channel_id: &str, message_id: &str) -> Result<Message> {
        let url = format!("{DISCORD_API}/channels/{channel_id}/messages/{message_id}");

        let value = self
            .request_with_retry(|| self.client.get(&url).send())
            .await
            .context("Failed to fetch message")?;

        let msg: Message = serde_json::from_value(value)
            .context("Failed to parse message JSON")?;
        Ok(msg)
    }

    /// Fetch recent messages from a channel/thread, returns newest first.
    pub async fn get_messages(&self, channel_id: &str, limit: u8) -> Result<Vec<Message>> {
        let url = format!("{DISCORD_API}/channels/{channel_id}/messages?limit={limit}");

        let value = self
            .request_with_retry(|| self.client.get(&url).send())
            .await
            .context("Failed to fetch messages")?;
        let messages: Vec<Message> = serde_json::from_value(value)
            .context("Failed to parse messages JSON")?;

        debug!(channel = channel_id, count = messages.len(), "Fetched messages");
        Ok(messages)
    }

    /// Wait for a response from the target bot in a thread/channel.
    /// Returns the bot's response message, or error on timeout.
    pub async fn wait_for_bot_response(
        &self,
        channel_id: &str,
        after_message_id: &str,
        timeout: Duration,
        poll_interval: Duration,
    ) -> Result<Message> {
        let start = tokio::time::Instant::now();
        let after_id: u64 = after_message_id.parse().unwrap_or(0);

        info!(
            channel = channel_id,
            timeout_secs = timeout.as_secs(),
            "Waiting for bot response"
        );

        loop {
            if start.elapsed() > timeout {
                bail!(
                    "Timeout after {}s waiting for bot response in channel {}",
                    timeout.as_secs(),
                    channel_id
                );
            }

            let messages = self.get_messages(channel_id, 10).await?;

            for msg in &messages {
                let msg_id: u64 = msg.id.parse().unwrap_or(0);
                if msg.author.id == self.target_bot_id && msg_id > after_id {
                    info!(
                        message_id = %msg.id,
                        author = %msg.author.username,
                        "Received bot response"
                    );
                    return Ok(msg.clone());
                }
            }

            debug!("No response yet, polling again in {}ms", poll_interval.as_millis());
            tokio::time::sleep(poll_interval).await;
        }
    }

    /// Generic retry wrapper with exponential backoff.
    async fn request_with_retry<F, Fut>(&self, make_request: F) -> Result<serde_json::Value>
    where
        F: Fn() -> Fut,
        Fut: std::future::Future<Output = reqwest::Result<reqwest::Response>>,
    {
        let mut last_err = None;

        for attempt in 0..=self.max_retries {
            if attempt > 0 {
                let delay = Duration::from_millis(1000 * 2u64.pow(attempt - 1));
                warn!(attempt, delay_ms = delay.as_millis() as u64, "Retrying after error");
                tokio::time::sleep(delay).await;
            }

            match make_request().await {
                Ok(resp) => {
                    let status = resp.status();
                    if status.is_success() {
                        let body = resp.text().await.context("Failed to read response body")?;
                        let value: serde_json::Value =
                            serde_json::from_str(&body).context("Failed to parse JSON response")?;
                        return Ok(value);
                    }

                    if status.as_u16() == 429 {
                        let body = resp.text().await.unwrap_or_default();
                        warn!(status = 429, body = %body, "Rate limited");
                        last_err = Some(anyhow::anyhow!("Rate limited (429): {body}"));
                        continue;
                    }

                    if status.as_u16() == 401 || status.as_u16() == 403 {
                        let body = resp.text().await.unwrap_or_default();
                        bail!("Authentication error ({status}): {body}");
                    }

                    let body = resp.text().await.unwrap_or_default();
                    last_err = Some(anyhow::anyhow!("HTTP {status}: {body}"));
                }
                Err(e) => {
                    warn!(attempt, error = %e, "Request failed");
                    last_err = Some(e.into());
                }
            }
        }

        Err(last_err.unwrap_or_else(|| anyhow::anyhow!("Request failed after retries")))
    }
}
