use crate::discord::{DiscordClient, Message};
use crate::test_cases::TestCase;
use anyhow::Result;
use std::time::Duration;
use tracing::{info, warn};

/// How long to wait for 界王神 to respond to each message.
const DEFAULT_TIMEOUT_SECS: u64 = 180;
/// How often to poll Discord while waiting.
const POLL_INTERVAL_SECS: u64 = 5;

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
        let status = if self.total_failed == 0 {
            "✅ ALL PASSED"
        } else {
            "❌ SOME FAILED"
        };
        format!(
            "{status} — {}/{} passed in suite '{}'",
            self.total_passed,
            self.results.len(),
            self.suite_name
        )
    }
}

/// The main test execution engine.
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

    /// Run all test cases in a suite, sending each as a separate message
    /// to the given thread/channel and waiting for responses.
    pub async fn run_suite(
        &self,
        suite_name: &str,
        test_cases: &[TestCase],
        channel_id: &str,
        thread_id: Option<&str>,
        bot_id: &str,
    ) -> Result<SuiteResult> {
        let target = thread_id.unwrap_or(channel_id);

        info!(
            suite = suite_name,
            target_channel = channel_id,
            target_thread = %thread_id.unwrap_or("(none)"),
            "Starting test suite"
        );

        let mut results = Vec::new();

        for (i, tc) in test_cases.iter().enumerate() {
            let resolved = tc.resolve(bot_id);
            let result = self
                .run_single(&resolved, target, i == 0)
                .await;
            results.push(result);
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

    /// Run a single test case:
    /// - Send the prompt to target channel/thread
    /// - Wait for 界王神 to respond
    /// - Validate the response
    async fn run_single(&self, tc: &TestCase, target: &str, _is_first: bool) -> TestResult {
        let start = std::time::Instant::now();
        info!(test = %tc.name, "Sending prompt");

        // Send the message
        let sent = match self.discord.send_message(target, &tc.prompt).await {
            Ok(m) => m,
            Err(e) => {
                return TestResult {
                    test_name: tc.name.clone(),
                    passed: false,
                    response: None,
                    error: Some(format!("Send failed: {e}")),
                    duration_secs: start.elapsed().as_secs_f64(),
                };
            }
        };

        // Wait for 界王神 response
        let response_msg = match self
            .discord
            .wait_for_bot_response(target, &sent.id, self.timeout, Duration::from_secs(POLL_INTERVAL_SECS))
            .await
        {
            Ok(m) => m,
            Err(e) => {
                warn!(test = %tc.name, error = %e, "Timeout or error waiting for response");
                return TestResult {
                    test_name: tc.name.clone(),
                    passed: false,
                    response: None,
                    error: Some(format!("Timeout: {e}")),
                    duration_secs: start.elapsed().as_secs_f64(),
                };
            }
        };

        let response_text = response_msg.content.clone();
        let duration = start.elapsed().as_secs_f64();

        // Validate
        match tc.validate(&response_text) {
            Ok(()) => {
                info!(test = %tc.name, passed = true, "Test passed");
                TestResult {
                    test_name: tc.name.clone(),
                    passed: true,
                    response: Some(response_text),
                    error: None,
                    duration_secs: duration,
                }
            }
            Err(errors) => {
                let error = errors.join("; ");
                warn!(test = %tc.name, passed = false, error = %error, "Test failed validation");
                TestResult {
                    test_name: tc.name.clone(),
                    passed: false,
                    response: Some(response_text),
                    error: Some(error),
                    duration_secs: duration,
                }
            }
        }
    }

    /// Create a new thread in channel_id, send the first prompt there,
    /// and return the thread's message ID (root of the thread).
    pub async fn start_thread(
        &self,
        channel_id: &str,
        first_prompt: &str,
    ) -> Result<(String, Message)> {
        // Sending a message with a mention auto-creates a thread.
        let msg = self.discord.send_message(channel_id, first_prompt).await?;
        Ok((msg.id.clone(), msg))
    }
}
