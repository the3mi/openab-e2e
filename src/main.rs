mod config;
mod discord;
mod test_cases;
mod tester;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use config::Config;
use discord::DiscordClient;
use std::path::PathBuf;
use tester::Tester;
use tracing::Level;
use tracing_subscriber::FmtSubscriber;

// ── CLI ─────────────────────────────────────────────────────────────────────

#[derive(Parser, Debug)]
#[command(
    name = "openab-e2e",
    version = "0.1.0",
    about = "Discord bot chain E2E tester for openab PR testing"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Path to config file (default: ~/.openab-e2e/config.toml)
    #[arg(short, long)]
    config: Option<PathBuf>,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Run E2E tests against the target bot
    Test {
        /// Discord channel ID (default: from config)
        #[arg(short, long)]
        channel: Option<String>,

        /// Existing thread ID to continue conversation
        #[arg(short, long)]
        thread: Option<String>,

        /// Run only a specific test by name
        #[arg(long)]
        test_name: Option<String>,

        /// Send a webhook message to reset the bot-turn cap before testing
        #[arg(long)]
        reset_cap: bool,
    },

    /// Manage configuration
    Config {
        #[command(subcommand)]
        sub: ConfigCommands,
    },

    /// Run all test suites (for CI / cron)
    RunAll {
        /// Discord channel ID (default: from config)
        #[arg(short, long)]
        channel: Option<String>,

        /// Exit with non-zero code if any test fails
        #[arg(short = '1', long)]
        fail_fast: bool,
    },
}

#[derive(Subcommand, Debug)]
enum ConfigCommands {
    /// Show current configuration
    Show,
    /// Create a new config file from template
    Init,
}

// ── Main ─────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<()> {
    let subscriber = FmtSubscriber::builder()
        .with_max_level(Level::INFO)
        .with_target(false)
        .finish();
    let _ = tracing::subscriber::set_global_default(subscriber);

    let cli = Cli::parse();

    match &cli.command {
        Commands::Config { sub } => match sub {
            ConfigCommands::Show => {
                let cfg = Config::load()
                    .or_else(|_| Config::init().map(|_| Config::load().unwrap()))
                    .context("Failed to load or create config")?;
                println!("Discord bot token: {}...", cfg.discord.bot_token.chars().take(10).collect::<String>());
                println!("Target bot ID: {}", cfg.discord.target_bot_id);
                println!("Target channel: {}", cfg.discord.target_channel_id);
                println!("Timeout: {}s", cfg.test.timeout_secs);
                println!("Max retries: {}", cfg.test.max_retries);
                Ok(())
            }
            ConfigCommands::Init => {
                let path = Config::default_path().context("Failed to get default config path")?;
                Config::init().context("Failed to init config")?;
                println!("Config template written to: {}", path.display());
                println!("Please edit the file with your Discord IDs.");
                Ok(())
            }
        },
        Commands::Test {
            channel,
            thread,
            test_name,
            reset_cap,
        } => {
            let cfg = Config::load()
                .or_else(|_| Config::init().map(|_| Config::load().unwrap()))
                .context("Failed to load or create config")?;
            let discord = DiscordClient::new(
                &cfg.discord.bot_token,
                &cfg.discord.target_bot_id,
                cfg.test.max_retries,
            )?;

            // Reset bot-turn cap via webhook before testing
            let channel_id = channel.as_deref().unwrap_or(&cfg.discord.target_channel_id);
            if *reset_cap {
                println!("Resetting bot-turn cap via webhook...");
                discord.send_webhook_cap_reset(channel_id).await
                    .context("Failed to send cap-reset webhook")?;
                println!("Cap reset sent.");
            }

            let tester = Tester::new(discord);
            let thread_id = thread.as_deref();

            let suites = test_cases::default_test_suites();
            let mut all_passed = true;

            for (i, suite) in suites.iter().enumerate() {
                let suite_name = format!("suite-{}", i + 1);

                let cases: Vec<_> = if let Some(name) = test_name {
                    suite.iter().filter(|tc| tc.name == *name).cloned().collect()
                } else {
                    suite.to_vec()
                };

                if cases.is_empty() {
                    continue;
                }

                let result = tester
                    .run_suite(&suite_name, &cases, channel_id, thread_id, &cfg.discord.target_bot_id)
                    .await?;

                print_suite_result(&result);

                if result.total_failed > 0 {
                    all_passed = false;
                }
            }

            if !all_passed {
                std::process::exit(1);
            }
            Ok(())
        }
        Commands::RunAll {
            channel,
            fail_fast,
        } => {
            let cfg = Config::load()
                .or_else(|_| Config::init().map(|_| Config::load().unwrap()))
                .context("Failed to load or create config")?;
            let discord = DiscordClient::new(
                &cfg.discord.bot_token,
                &cfg.discord.target_bot_id,
                cfg.test.max_retries,
            )?;
            let tester = Tester::new(discord);

            let channel_id = channel.as_deref().unwrap_or(&cfg.discord.target_channel_id);

            let suites = test_cases::default_test_suites();
            let mut all_passed = true;

            for (i, suite) in suites.iter().enumerate() {
                let suite_name = format!("suite-{}", i + 1);
                let result = tester
                    .run_suite(&suite_name, suite, channel_id, None, &cfg.discord.target_bot_id)
                    .await?;
                print_suite_result(&result);
                if result.total_failed > 0 {
                    all_passed = false;
                }
            }

            if !all_passed && *fail_fast {
                std::process::exit(1);
            }
            Ok(())
        }
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────────

fn print_suite_result(result: &tester::SuiteResult) {
    println!("\n{}", result.summary());
    for r in &result.results {
        let icon = if r.passed { "✅" } else { "❌" };
        let status = if r.passed { "PASS" } else { "FAIL" };
        println!("  {icon} [{status}] {} ({:.1}s)", r.test_name, r.duration_secs);
        if let Some(err) = &r.error {
            println!("         Error: {}", err);
        }
        if let Some(resp) = &r.response {
            println!("         Response: {}", resp.trim());
        }
    }
}
