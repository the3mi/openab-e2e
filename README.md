# openab-e2e

Discord bot chain E2E tester for openab PR testing.

```
devops-bot (tester) ──REST API──▶ Discord ──WebSocket──▶ openab-e2e-target (PR bot)
                                            ▲
                                            │ replies in Thread
                                            │
                                    openab-e2e reads Thread
```

## Architecture

```
src/
├── main.rs       # CLI entry point (clap)
├── config.rs     # TOML config management
├── discord.rs    # Discord REST API client with retry
├── tester.rs     # Test execution engine
└── test_cases.rs # Test case definitions
```

## Security

- **No hardcoded tokens** — all secrets live in `~/.openab-e2e/config.toml`
- Config file permissions: `600` (owner-only) on Unix

## Configuration

First run:

```bash
openab-e2e config init
# → creates ~/.openab-e2e/config.toml
```

Edit `~/.openab-e2e/config.toml`:

```toml
[discord]
bot_token = "YOUR_TESTER_BOT_TOKEN"   # Discord bot token for the tester bot (e.g. devops-bot)
target_bot_id = "TARGET_BOT_ID"        # Discord user ID of the bot being tested (e.g. openab-e2e-target)
guild_id = "YOUR_GUILD_ID"
pr_channel_id = "PR_CHANNEL_ID"         # Discord channel ID for PR tests
tiantian_channel_id = "TIANTIAN_CHANNEL_ID"  # Discord channel ID for 天庭 tests

[test]
timeout_secs = 180        # max wait for bot response per message
max_retries = 3           # exponential backoff on network errors
poll_interval_ms = 3000   # how often to poll Discord while waiting
```

View current config:

```bash
openab-e2e config show
```

## Usage

### Interactive test (new thread)

```bash
# Use default PR channel
openab-e2e test

# Use 天庭 channel
openab-e2e test --channel 1491375585124024440
```

### Interactive test (existing thread)

```bash
openab-e2e test --thread 1493792852780519665 --channel 1493499891178016821
```

### Run a specific test

```bash
openab-e2e test --test-name say_hi
```

### Full suite (CI / cron)

```bash
# Runs all suites, exits non-zero on failure
openab-e2e run-all --channel 1493499891178016821 --fail-fast
```

## Default Test Cases

| Name           | Prompt                                      | Expects          |
|----------------|---------------------------------------------|------------------|
| `say_hi`       | 請說 HI                                     | `HI`             |
| `who_are_you`  | 請問你是誰                                  | `{BOT_NAME}`     |
| `model_version`| 請問你的模型是什麼                           | `claude-sonnet`  |

## Adding New Test Cases

Edit `src/test_cases.rs`:

```rust
TestCase {
    name: "my_test".into(),
    prompt: "<@1491255095109746709> 你的問題".into(),
    expect_contains: vec!["預期回覆".into()],
    expect_not_contains: vec![],   // optional
},
```

## CI / Jenkins-like Setup

### Cron job (macOS/Linux)

```bash
# Run every 5 minutes, log output
*/5 * * * * /usr/local/bin/openab-e2e run-all >> /var/log/openab-e2e.log 2>&1
```

### GitHub Actions

```yaml
name: E2E Bot Chain Test

on:
  schedule:
    - cron: '*/5 * * * *'   # every 5 minutes
  workflow_dispatch:         # manual trigger

jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - name: Build
        run: cargo build --release
      - name: Run tests
        env:
          DISCORD_BOT_TOKEN: ${{ secrets.DISCORD_BOT_TOKEN }}
        run: |
          # Write config
          mkdir -p ~/.openab-e2e
          echo '${{ secrets.OPENAB_E2E_CONFIG }}' > ~/.openab-e2e/config.toml
          ./target/release/openab-e2e run-all --fail-fast
```

### Jenkins Pipeline

```groovy
pipeline {
    agent any

    stages {
        stage('E2E Test') {
            steps {
                sh '''
                    openab-e2e run-all --channel 1493499891178016821 --fail-fast
                '''
            }
        }
    }

    post {
        failure {
            slackSend channel: '#ops',
                     message: "界王神 E2E test FAILED"
        }
        success {
            slackSend channel: '#ops',
                     message: "界王神 E2E test PASSED"
        }
    }
}
```

## Bot Turn Cap

Discord bots have a **10 consecutive bot→bot message limit** per channel.
If you hit the cap, 界王神 will ignore further bot messages until a human posts.

Workaround: have a human (or use a different bot account) post one message to reset the counter.

## Development

```bash
cargo build --release
cargo test
cargo run -- test --channel 1493499891178016821
```

## Project Status

| Dimension      | Status |
|----------------|--------|
| 🔐 Security    | ✅ Token via config file, owner-only perms |
| 🎨 Architecture| ✅ Modular (discord/tester/config/main) |
| ⚙️ Functionality| ✅ Config init + test + run-all |
| 🔀 Complexity  | ✅ Lean, extensible test case system |
| 🧪 Testing     | ✅ Unit tests for config parsing |
| ⚠️ Error Handling | ✅ Exponential backoff, clear errors |
| 📖 Clarity     | ✅ Doc comments, Rust idioms |
| 📚 Documentation| ✅ This README + inline docs |
| 🏛️ Technical Debt | ✅ Fresh project, modern Rust 2021 |
