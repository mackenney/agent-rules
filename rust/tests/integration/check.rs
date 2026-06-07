//! Integration tests for `check` that do not require an API key.

use predicates::prelude::*;

use crate::common::{cmd, test_repo};

/// `check` without ANTHROPIC_API_KEY must exit 3 (config error) before any LLM calls.
#[test]
fn missing_api_key_exits_3() {
    cmd()
        .args(["check", "--files", "src/api/bad_controller.py", "--repo"])
        .arg(test_repo())
        .env_remove("ANTHROPIC_API_KEY")
        .assert()
        .code(3);
}

/// `check --provider openrouter` without OPENROUTER_API_KEY must exit 3.
#[test]
fn missing_openrouter_api_key_exits_3() {
    cmd()
        .args([
            "check",
            "--provider",
            "openrouter",
            "--files",
            "src/api/bad_controller.py",
            "--repo",
        ])
        .arg(test_repo())
        .env_remove("OPENROUTER_API_KEY")
        .assert()
        .code(3)
        .stderr(predicates::str::contains("OPENROUTER_API_KEY"));
}

/// Model with '/' and provider=anthropic should show a helpful error.
#[test]
fn model_slash_guard_exits_3() {
    cmd()
        .args([
            "check",
            "--model",
            "anthropic/claude-haiku-4-5",
            "--files",
            "src/api/bad_controller.py",
            "--repo",
        ])
        .arg(test_repo())
        .env("ANTHROPIC_API_KEY", "test-key")
        .assert()
        .code(3)
        .stderr(predicates::str::contains("looks like an OpenRouter model"));
}

/// `check --help` should document the --provider flag.
#[test]
fn help_shows_provider_flag() {
    cmd()
        .args(["check", "--help"])
        .assert()
        .success()
        .stdout(predicates::str::contains("--provider"));
}
