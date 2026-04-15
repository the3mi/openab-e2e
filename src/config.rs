use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

/// Configuration for the openab-e2e test tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub discord: DiscordConfig,
    #[serde(default)]
    pub test: TestConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscordConfig {
    /// Bot token for ClawTriage (used to send messages)
    pub bot_token: String,
    /// 界王神 Bot user ID (used to identify responses)
    pub target_bot_id: String,
    /// Guild (server) ID
    pub guild_id: String,
    /// Default channel for PR tests
    pub pr_channel_id: String,
    /// 天庭 channel ID
    pub tiantian_channel_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestConfig {
    /// Timeout in seconds waiting for bot response
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
    /// Max retries on network errors
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,
    /// Poll interval in milliseconds when waiting for responses
    #[serde(default = "default_poll_interval_ms")]
    pub poll_interval_ms: u64,
}

fn default_timeout() -> u64 {
    180
}
fn default_max_retries() -> u32 {
    3
}
fn default_poll_interval_ms() -> u64 {
    3000
}

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
    /// Returns the default config file path: ~/.openab-e2e/config.toml
    pub fn default_path() -> Result<PathBuf> {
        let home = dirs::home_dir().context("Could not determine home directory")?;
        Ok(home.join(".openab-e2e").join("config.toml"))
    }

    /// Load config from the default path.
    pub fn load() -> Result<Self> {
        let path = Self::default_path()?;
        let content = fs::read_to_string(&path)
            .with_context(|| format!("Failed to read config at {}", path.display()))?;
        let config: Config =
            toml::from_str(&content).context("Failed to parse config.toml")?;
        Ok(config)
    }

    /// Generate a template config file at the default path.
    pub fn init() -> Result<PathBuf> {
        let path = Self::default_path()?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create dir {}", parent.display()))?;
        }

        let template = Config {
            discord: DiscordConfig {
                bot_token: "YOUR_BOT_TOKEN_HERE".to_string(),
                target_bot_id: "1491255095109746709".to_string(),
                guild_id: "1320784060892708904".to_string(),
                pr_channel_id: "1493499891178016821".to_string(),
                tiantian_channel_id: "1491375585124024440".to_string(),
            },
            test: TestConfig::default(),
        };

        let content = toml::to_string_pretty(&template)
            .context("Failed to serialize config template")?;
        fs::write(&path, &content)
            .with_context(|| format!("Failed to write config to {}", path.display()))?;

        // Set file permissions to owner-only (Unix)
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
                guild_id: "456".to_string(),
                pr_channel_id: "789".to_string(),
                tiantian_channel_id: "012".to_string(),
            },
            test: TestConfig::default(),
        };

        let serialized = toml::to_string_pretty(&config).unwrap();
        let deserialized: Config = toml::from_str(&serialized).unwrap();

        assert_eq!(deserialized.discord.bot_token, "test-token");
        assert_eq!(deserialized.test.timeout_secs, 180);
        assert_eq!(deserialized.test.max_retries, 3);
    }

    #[test]
    fn test_config_defaults() {
        let minimal = r#"
[discord]
bot_token = "tok"
target_bot_id = "1"
guild_id = "2"
pr_channel_id = "3"
tiantian_channel_id = "4"
"#;
        let config: Config = toml::from_str(minimal).unwrap();
        assert_eq!(config.test.timeout_secs, 180);
        assert_eq!(config.test.max_retries, 3);
        assert_eq!(config.test.poll_interval_ms, 3000);
    }
}
