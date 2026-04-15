use serde::{Deserialize, Serialize};

/// A single test case: prompt sent to 界王神, and patterns to match in response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestCase {
    /// Human-readable test name
    pub name: String,
    /// The message content sent to 界王神
    pub prompt: String,
    /// One or more substrings that should appear in 界王神's reply
    pub expect_contains: Vec<String>,
    /// Optional: substring that should NOT appear
    #[serde(default)]
    pub expect_not_contains: Vec<String>,
}

impl TestCase {
    /// Validate that a response contains expected patterns.
    pub fn validate(&self, response: &str) -> Result<(), Vec<String>> {
        let mut errors = Vec::new();

        for pattern in &self.expect_contains {
            if !response.contains(pattern) {
                errors.push(format!(
                    "Expected to find '{pattern}' in response, but got: {response}"
                ));
            }
        }

        for pattern in &self.expect_not_contains {
            if response.contains(pattern) {
                errors.push(format!(
                    "Expected NOT to find '{pattern}' in response, but got: {response}"
                ));
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

/// All built-in test cases for the bot chain.
pub fn default_test_suites() -> Vec<Vec<TestCase>> {
    vec![
        // Suite 1: Basic identity tests
        vec![
            TestCase {
                name: "say_hi".into(),
                prompt: "<@1491255095109746709> 請說 HI".into(),
                expect_contains: vec!["HI".into()],
                expect_not_contains: vec![],
            },
            TestCase {
                name: "who_are_you".into(),
                prompt: "<@1491255095109746709> 請問你是誰".into(),
                expect_contains: vec!["界王神".into()],
                expect_not_contains: vec![],
            },
            TestCase {
                name: "model_version".into(),
                prompt: "<@1491255095109746709> 請問你的模型是什麼".into(),
                expect_contains: vec!["claude-sonnet".into()],
                expect_not_contains: vec![],
            },
        ],
    ]
}
