//! E2E tests for the `check` command.
//!
//! Each test requires ANTHROPIC_API_KEY (Anthropic tests) and/or OPENROUTER_API_KEY
//! Tests mirror the TypeScript integration suite (stateless.test.ts).

use serde_json::Value;

use crate::common::{cmd, require_env, test_repo};

fn api_key() -> Option<String> {
    require_env("ANTHROPIC_API_KEY")
}

/// bad_controller.py has a hardcoded secret (error) and raw SQL (error) → exit 2.
#[test]
fn bad_controller_exits_2() {
    let Some(key) = api_key() else { return };
    cmd()
        .current_dir(test_repo())
        .args([
            "check",
            "--files",
            "src/api/bad_controller.py",
            "--repo",
            ".",
        ])
        .env("ANTHROPIC_API_KEY", &key)
        .assert()
        .code(2);
}

/// clean_controller.py has no violations → exit 0.
#[test]
fn clean_controller_exits_0() {
    let Some(key) = api_key() else { return };
    cmd()
        .current_dir(test_repo())
        .args([
            "check",
            "--files",
            "src/api/clean_controller.py",
            "--repo",
            ".",
        ])
        .env("ANTHROPIC_API_KEY", &key)
        .assert()
        .code(0);
}

/// payment_controller.py has a hardcoded secret (error) + raw SQL (error) → exit 2.
#[test]
fn payment_controller_exits_2() {
    let Some(key) = api_key() else { return };
    cmd()
        .current_dir(test_repo())
        .args([
            "check",
            "--files",
            "src/api/payment_controller.py",
            "--repo",
            ".",
        ])
        .env("ANTHROPIC_API_KEY", &key)
        .assert()
        .code(2);
}

/// `--output json` produces valid JSON with expected top-level fields.
#[test]
fn json_output_is_valid() {
    let Some(key) = api_key() else { return };
    let out = cmd()
        .current_dir(test_repo())
        .args([
            "check",
            "--files",
            "src/api/bad_controller.py",
            "--repo",
            ".",
            "--output",
            "json",
        ])
        .env("ANTHROPIC_API_KEY", &key)
        .output()
        .unwrap();

    assert_eq!(
        out.status.code(),
        Some(2),
        "expected exit 2 for bad_controller"
    );

    let report: Value =
        serde_json::from_slice(&out.stdout).expect("--output json must produce valid JSON");

    assert!(report["files"].is_array(), "report must have files array");
    assert_eq!(
        report["overall_verdict"], "error",
        "overall_verdict must be error"
    );
    let files = report["files"].as_array().unwrap();
    assert!(!files.is_empty(), "files must not be empty");
    assert!(
        files[0]["verdicts"].is_array(),
        "each file must have a verdicts array"
    );
}

/// `--output github` must include the sentinel comment used for PR comment upserts.
#[test]
fn github_output_has_sentinel() {
    let Some(key) = api_key() else { return };
    let out = cmd()
        .current_dir(test_repo())
        .args([
            "check",
            "--files",
            "src/api/bad_controller.py",
            "--repo",
            ".",
            "--output",
            "github",
        ])
        .env("ANTHROPIC_API_KEY", &key)
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("<!-- agent-rules-report -->"),
        "github output must contain sentinel marker;\ngot:\n{stdout}"
    );
}

/// A clean file with `--warn-as-error` must still exit 0 (no violations at all).
#[test]
fn warn_as_error_on_clean_file_exits_0() {
    let Some(key) = api_key() else { return };
    cmd()
        .current_dir(test_repo())
        .args([
            "check",
            "--files",
            "src/api/clean_controller.py",
            "--repo",
            ".",
            "--warn-as-error",
        ])
        .env("ANTHROPIC_API_KEY", &key)
        .assert()
        .code(0);
}

/// Running the same check twice hits the cache on the second run.
/// Verified by inspecting the JSON report's `cached` flag on the second run.
#[test]
fn second_run_hits_cache() {
    let Some(key) = api_key() else { return };

    // Use an isolated temp dir as repo root so the cache doesn't leak into test-repo.
    let repo = tempfile::tempdir().unwrap();
    // Copy the root .agent-rules.toml so rules apply.
    std::fs::copy(
        test_repo().join(".agent-rules.toml"),
        repo.path().join(".agent-rules.toml"),
    )
    .unwrap();
    // Write a simple clean file (no violations expected).
    let src_dir = repo.path().join("src");
    std::fs::create_dir_all(&src_dir).unwrap();
    let file = src_dir.join("clean.py");
    std::fs::write(&file, b"x = 1\n").unwrap();

    let run = |key: &str| {
        cmd()
            .args(["check", "--files"])
            .arg(&file)
            .args(["--repo"])
            .arg(repo.path())
            .args(["--output", "json"])
            .env("ANTHROPIC_API_KEY", key)
            .output()
            .unwrap()
    };

    // First run — populates cache.
    let out1 = run(&key);
    assert_eq!(out1.status.code(), Some(0), "first run must exit 0");

    // Second run — all (file, rule) pairs must be served from cache.
    let out2 = run(&key);
    assert_eq!(out2.status.code(), Some(0), "second run must exit 0");

    let report: Value =
        serde_json::from_slice(&out2.stdout).expect("second run must produce valid JSON");
    let files = report["files"].as_array().unwrap();
    assert!(!files.is_empty(), "files array must not be empty");
    let all_cached = files.iter().all(|f| f["cached"].as_bool().unwrap_or(false));
    assert!(
        all_cached,
        "all files must be cached on second run;\nreport: {report}"
    );
}

/// OpenRouter model used for e2e tests — reads OPENROUTER_MODEL env var,
/// falls back to deepseek/deepseek-v4-flash.
fn openrouter_model() -> String {
    std::env::var("OPENROUTER_MODEL").unwrap_or_else(|_| "deepseek/deepseek-v4-flash".to_string())
}

/// OpenRouter: bad controller should fail (exit 2) — requires OPENROUTER_API_KEY.
#[test]
fn openrouter_bad_controller_exits_2() {
    let Some(key) = require_env("OPENROUTER_API_KEY") else {
        return;
    };
    let model = openrouter_model();

    cmd()
        .args([
            "check",
            "--provider",
            "openrouter",
            "--model",
            &model,
            "--agentic-model",
            &model,
            "--files",
            "src/api/bad_controller.py",
            "--repo",
        ])
        .arg(test_repo())
        .env("OPENROUTER_API_KEY", &key)
        .assert()
        .code(2);
}

/// OpenRouter: clean controller should pass (exit 0) — requires OPENROUTER_API_KEY.
#[test]
fn openrouter_clean_controller_exits_0() {
    let Some(key) = require_env("OPENROUTER_API_KEY") else {
        return;
    };
    let model = openrouter_model();

    cmd()
        .args([
            "check",
            "--provider",
            "openrouter",
            "--model",
            &model,
            "--agentic-model",
            &model,
            "--files",
            "src/api/clean_controller.py",
            "--repo",
        ])
        .arg(test_repo())
        .env("OPENROUTER_API_KEY", &key)
        .assert()
        .code(0);
}

/// OpenRouter: payment controller triggers agentic rules (context=agentic) → exit 2.
/// Exercises both stateless and agentic evaluators via the same provider/model.
#[test]
fn openrouter_payment_controller_exits_2() {
    let Some(key) = require_env("OPENROUTER_API_KEY") else {
        return;
    };
    let model = openrouter_model();

    cmd()
        .args([
            "check",
            "--provider",
            "openrouter",
            "--model",
            &model,
            "--agentic-model",
            &model,
            "--files",
            "src/api/payment_controller.py",
            "--repo",
        ])
        .arg(test_repo())
        .env("OPENROUTER_API_KEY", &key)
        .assert()
        .code(2);
}
