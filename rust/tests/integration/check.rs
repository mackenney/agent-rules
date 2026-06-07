//! Integration tests for `check` that do not require an API key.

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
