use crate::discord::{DiscordClient, Message};
use crate::test_cases::TestCase;
use anyhow::Result;
use std::time::Duration;
use tracing::{info, warn};

/// How long to wait for 界王神 to respond to each message.
const DEFAULT_TIMEOUT_SECS: u64 = 30;
/// How often to poll Discord while waiting.
const POLL_INTERVAL_SECS: u64 = 1;

/// Result of a single test case run.
#[derive(Debug)]
pub struct TestResult {
    pub test_name: String,
    pub passed: bool,
    pub response: Option<String>,
    pub error: Option<String>,
    pub duration_secs: f64,
}

/// Result of an entire test suite.
#[derive(Debug)]
pub struct SuiteResult {
    pub suite_name: String,
    pub results: Vec<TestResult>,
    pub total_passed: usize,
    pub total_failed: usize,
}

impl SuiteResult {
    pub fn summary(&self) -> String {
        let mut s = format!(
            "\n\n{} — {} passed, {} failed in suite '{}'\n\n",
            if self.total_passed == self.results.len() {
                "✅ ALL PASSED"
            } else {
                "❌ SOME FAILED"
            },
            self.total_passed,
            self.total_failed,
            self.suite_name
        );
        for r in &self.results {
            let icon = if r.passed { "✅" } else { "❌" };
            s.push_str(&format!(
                "  {} [{}] ({:.1}s)\n         {}\n",
                icon,
                r.test_name,
                r.duration_secs,
                r.error
                    .as_ref()
                    .map(|e| format!("Error: {}", e))
                    .or_else(|| r.response.as_ref().map(|m| format!("Response: {}", m)))
                    .unwrap_or_default()
            ));
        }
        s
    }
}

pub struct Tester {
    discord: DiscordClient,
    timeout: Duration,
}

impl Tester {
    pub fn new(discord: DiscordClient) -> Self {
        Self {
            discord,
            timeout: Duration::from_secs(DEFAULT_TIMEOUT_SECS),
        }
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Run a suite of test cases against a target channel.
    ///
    /// All test cases share a single thread (created by the first test case).
    /// - First test: send to main channel, bot creates thread
    /// - Subsequent tests: send to the discovered thread
    /// - All reads: from main channel (avoids thread 403 permission issues)
    pub async fn run_suite(
        &self,
        suite_name: &str,
        test_cases: &[TestCase],
        channel_id: &str,
        thread_id: Option<&str>,
        bot_id: &str,
    ) -> Result<SuiteResult> {
        let main_channel_id = channel_id.to_string();

        info!(
            suite = suite_name,
            main_channel = %main_channel_id,
            target_thread = %thread_id.unwrap_or("(none)"),
            "Starting test suite"
        );

        let mut results = Vec::new();
        let mut active_thread_id: Option<String> = thread_id.map(String::from);

        for (i, tc) in test_cases.iter().enumerate() {
            let resolved = tc.resolve(bot_id);

            // Where to send: thread if discovered, otherwise main channel
            let target = active_thread_id.as_deref().unwrap_or(&main_channel_id);

            let (result, discovered_thread) = self
                .run_single(&resolved, target, &main_channel_id)
                .await;

            // After first test, capture the thread for subsequent tests
            if i == 0 {
                active_thread_id = discovered_thread;
                if let Some(ref t) = active_thread_id {
                    info!(
                        "First test created thread {}. Subsequent tests will use this thread.",
                        t
                    );
                }
            }

            results.push(result);

            // Delay between tests to avoid bot anti-spam
            if i < test_cases.len() - 1 {
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        }

        let total_passed = results.iter().filter(|r| r.passed).count();
        let total_failed = results.len() - total_passed;

        Ok(SuiteResult {
            suite_name: suite_name.to_string(),
            results,
            total_passed,
            total_failed,
        })
    }

    /// Run a single test case.
    /// Send to `target` (main channel or thread), read from `main_channel_id`.
    async fn run_single(
        &self,
        tc: &TestCase,
        target: &str,
        main_channel_id: &str,
    ) -> (TestResult, Option<String>) {
        let start = std::time::Instant::now();
        info!(test = %tc.name, target, "Sending prompt");

        // Send the message
        let sent = match self.discord.send_message(target, &tc.prompt).await {
            Ok(m) => m,
            Err(e) => {
                return (
                    TestResult {
                        test_name: tc.name.clone(),
                        passed: false,
                        response: None,
                        error: Some(format!("Send failed: {e}")),
                        duration_secs: start.elapsed().as_secs_f64(),
                    },
                    None,
                );
            }
        };

        // Wait for 界王神 response
        let (response_msg, thread_id) = match self
            .discord
            .wait_for_bot_response(
                target,
                &sent.id,
                main_channel_id,
                self.timeout,
                Duration::from_secs(POLL_INTERVAL_SECS),
            )
            .await
        {
            Ok((m, tid)) => (m, Some(tid)),
            Err(e) => {
                warn!(test = %tc.name, error = %e, "Timeout or error waiting for response");
                return (
                    TestResult {
                        test_name: tc.name.clone(),
                        passed: false,
                        response: None,
                        error: Some(format!("Timeout: {e}")),
                        duration_secs: start.elapsed().as_secs_f64(),
                    },
                    None,
                );
            }
        };

        let response_text = response_msg.content.clone();
        let duration = start.elapsed().as_secs_f64();

        // Validate
        match tc.validate(&response_text) {
            Ok(()) => {
                info!(test = %tc.name, passed = true, "Test passed");
                (
                    TestResult {
                        test_name: tc.name.clone(),
                        passed: true,
                        response: Some(response_text),
                        error: None,
                        duration_secs: duration,
                    },
                    thread_id,
                )
            }
            Err(errors) => {
                let error = errors.join("; ");
                warn!(test = %tc.name, passed = false, error = %error, "Test failed validation");
                (
                    TestResult {
                        test_name: tc.name.clone(),
                        passed: false,
                        response: Some(response_text),
                        error: Some(error),
                        duration_secs: duration,
                    },
                    thread_id,
                )
            }
        }
    }

    /// Run tests against an existing thread (all messages in same thread).
    pub async fn run_suite_in_thread(
        &self,
        suite_name: &str,
        test_cases: &[TestCase],
        thread_id: &str,
        main_channel_id: &str,
        bot_id: &str,
    ) -> Result<SuiteResult> {
        info!(
            suite = suite_name,
            thread_id,
            main_channel_id,
            "Starting test suite in existing thread"
        );

        let mut results = Vec::new();

        for (i, tc) in test_cases.iter().enumerate() {
            let resolved = tc.resolve(bot_id);
            let (result, _) = self.run_single(&resolved, thread_id, main_channel_id).await;
            results.push(result);

            if i < test_cases.len() - 1 {
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        }

        let total_passed = results.iter().filter(|r| r.passed).count();
        let total_failed = results.len() - total_passed;

        Ok(SuiteResult {
            suite_name: suite_name.to_string(),
            results,
            total_passed,
            total_failed,
        })
    }
}
