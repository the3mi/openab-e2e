use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

/// Configuration for openab-e2e.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub discord: DiscordConfig,
    #[serde(default)]
    pub test: TestConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscordConfig {
    /// Discord bot token for the tester bot (ClawTriage / devops-bot)
    pub bot_token: String,
    /// Discord user ID of the bot being tested (openab / target-bot)
    pub target_bot_id: String,
    /// Discord channel ID where tests are run
    pub target_channel_id: String,
    /// Optional guild ID (for future use)
    #[serde(default)]
    pub guild_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestConfig {
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,
    #[serde(default = "default_poll_interval_ms")]
    pub poll_interval_ms: u64,
}

fn default_timeout() -> u64 { 180 }
fn default_max_retries() -> u32 { 3 }
fn default_poll_interval_ms() -> u64 { 3000 }

impl Default for TestConfig {
    fn default() -> Self {
        Self {
            timeout_secs: default_timeout(),
            max_retries: default_max_retries(),
            poll_interval_ms: default_poll_interval_ms(),
        }
    }
}

impl Config {
    /// Default config path: ~/.openab-e2e/config.toml
    pub fn default_path() -> Result<PathBuf> {
        let home = dirs::home_dir().context("Could not determine home directory")?;
        Ok(home.join(".openab-e2e").join("config.toml"))
    }

    /// Load config from default path.
    pub fn load() -> Result<Self> {
        let path = Self::default_path()?;
        let content = fs::read_to_string(&path)
            .with_context(|| format!("Failed to read config at {}", path.display()))?;
        let config: Config = toml::from_str(&content)
            .context("Failed to parse config.toml")?;
        Ok(config)
    }

    /// Write a template config to the default path.
    pub fn init() -> Result<PathBuf> {
        let path = Self::default_path()?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create dir {}", parent.display()))?;
        }

        let template = Config {
            discord: DiscordConfig {
                bot_token: "YOUR_DISCORD_BOT_TOKEN".to_string(),
                target_bot_id: "TARGET_BOT_ID".to_string(),
                target_channel_id: "TARGET_CHANNEL_ID".to_string(),
                guild_id: "".to_string(),
            },
            test: TestConfig::default(),
        };

        let content = toml::to_string_pretty(&template)
            .context("Failed to serialize config")?;
        fs::write(&path, &content)
            .with_context(|| format!("Failed to write config to {}", path.display()))?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = fs::Permissions::from_mode(0o600);
            fs::set_permissions(&path, perms)?;
        }

        Ok(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_roundtrip() {
        let config = Config {
            discord: DiscordConfig {
                bot_token: "test-token".to_string(),
                target_bot_id: "123".to_string(),
                target_channel_id: "456".to_string(),
                guild_id: "".to_string(),
            },
            test: TestConfig::default(),
        };
        let serialized = toml::to_string_pretty(&config).unwrap();
        let deserialized: Config = toml::from_str(&serialized).unwrap();
        assert_eq!(deserialized.discord.bot_token, "test-token");
        assert_eq!(deserialized.discord.target_channel_id, "456");
        assert_eq!(deserialized.test.timeout_secs, 180);
    }
}
