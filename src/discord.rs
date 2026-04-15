use anyhow::{bail, Context, Result};
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use std::time::Duration;
use tracing::{debug, info, warn};

const DISCORD_API: &str = "https://discord.com/api/v10";

/// Discord REST API client.
pub struct DiscordClient {
    client: reqwest::Client,
    bot_token: String,
    target_bot_id: String,
    max_retries: u32,
    /// Cached webhook for cap reset (id, token)
    webhook: Mutex<Option<(String, String)>>,
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
            bot_token: bot_token.to_string(),
            target_bot_id: target_bot_id.to_string(),
            max_retries,
            webhook: Mutex::new(None),
        })
    }

    /// Send a non-bot message to reset the bot-turn cap.
    /// Uses a temporary webhook — webhook messages don't count as bot messages.
    pub async fn send_webhook_cap_reset(&self, channel_id: &str) -> Result<()> {
        // Get or create webhook
        let (webhook_id, webhook_token) = {
            let mut cache = self.webhook.lock().unwrap();
            if let Some(ref c) = *cache {
                c.clone()
            } else {
                let url = format!("{DISCORD_API}/channels/{channel_id}/webhooks");
                #[derive(Serialize)]
                struct CreateWebhook { name: String }
                let value: serde_json::Value = self
                    .request_with_retry(|| {
                        self.client.post(&url).json(&CreateWebhook { name: "cap-reset".into() }).send()
                    })
                    .await
                    .context("Failed to create webhook")?;
                let id = value["id"].as_str().unwrap_or("").to_string();
                let token = value["token"].as_str().unwrap_or("").to_string();
                *cache = Some((id.clone(), token.clone()));
                (id, token)
            }
        };

        // Use a simple POST without the generic retry wrapper (which expects JSON)
        let url = format!("{DISCORD_API}/webhooks/{webhook_id}/{webhook_token}");
        #[derive(Serialize)]
        struct WebhookMsg { content: String }
        let resp = self.client
            .post(&url)
            .json(&WebhookMsg { content: "cap reset".into() })
            .send()
            .await
            .context("Failed to send webhook HTTP request")?;

        if !resp.status().is_success() {
            bail!("Webhook POST failed: {}", resp.status());
        }

        info!("Sent webhook cap-reset to channel {}", channel_id);
        Ok(())
    }

    /// Send a message to a channel or thread.
    pub async fn send_message(&self, channel_id: &str, content: &str) -> Result<Message> {
        let url = format!("{DISCORD_API}/channels/{channel_id}/messages");
        let body = CreateMessage { content: content.to_string() };

        let value = self
            .request_with_retry(|| self.client.post(&url).json(&body).send())
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

    /// Wait for a response from the target bot.
    ///
    /// Strategy: Poll the sent message directly for a thread (bot creates thread on reply).
    /// Then poll the thread for the bot's actual response text.
    pub async fn wait_for_bot_response(
        &self,
        channel_id: &str,
        after_message_id: &str,
        timeout: Duration,
        poll_interval: Duration,
    ) -> Result<Message> {
        let start = tokio::time::Instant::now();

        info!(
            channel = channel_id,
            sent_msg_id = after_message_id,
            timeout_secs = timeout.as_secs(),
            "Waiting for bot response"
        );

        // Phase 1: Poll the sent message until it gets a thread
        loop {
            if start.elapsed() > timeout {
                bail!(
                    "Timeout after {}s — no thread for message {}",
                    timeout.as_secs(),
                    after_message_id
                );
            }

            let msg = self.get_message(channel_id, after_message_id).await?;

            if let Some(ref thread_info) = msg.thread {
                let thread_id = thread_info.id.clone();
                info!(thread_id = %thread_id, "Thread found, reading bot response from thread");

                // Phase 2: Poll the thread for the bot's actual response
                return self.wait_for_bot_text_in_thread(&thread_id, start, timeout, poll_interval).await;
            }

            debug!("Sent message has no thread yet, polling again...");
            tokio::time::sleep(poll_interval).await;
        }
    }

    /// Poll a thread until we find a bot message with actual text content.
    async fn wait_for_bot_text_in_thread(
        &self,
        thread_id: &str,
        start: tokio::time::Instant,
        timeout: Duration,
        poll_interval: Duration,
    ) -> Result<Message> {
        loop {
            if start.elapsed() > timeout {
                bail!(
                    "Timeout after {}s waiting for bot response in thread {}",
                    timeout.as_secs(),
                    thread_id
                );
            }

            let messages = self.get_messages(thread_id, 10).await?;

            for msg in &messages {
                if msg.author.id == self.target_bot_id && !msg.content.is_empty() {
                    info!(
                        message_id = %msg.id,
                        author = %msg.author.username,
                        content = %msg.content,
                        "Bot response found"
                    );
                    return Ok(msg.clone());
                }
            }

            debug!("No bot text in thread yet, polling again...");
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
