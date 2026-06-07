//! Shared test helpers — binary launcher, test-repo path, env-var guard.
#![allow(dead_code)]

use std::path::PathBuf;

/// Absolute path to the shared test-repo fixture (agent-rules/test-repo/).
///
/// Uses `git rev-parse --git-common-dir` so it resolves correctly from both
/// the main repository and any git worktrees (where the gitlink is empty).
pub fn test_repo() -> PathBuf {
    let out = std::process::Command::new("git")
        .args(["rev-parse", "--git-common-dir"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("git rev-parse --git-common-dir failed");
    let git_dir = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
    // The output is absolute in worktrees and relative to CWD in the main repo.
    let git_dir = if git_dir.is_absolute() {
        git_dir
    } else {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(git_dir)
    };
    git_dir
        .parent()
        .expect(".git must have a parent")
        .join("test-repo")
}

/// Return the value of `var` or print a skip message and return `None`.
/// Use with `let Some(key) = require_env("VAR") else { return; }` to skip.
pub fn require_env(var: &str) -> Option<String> {
    match std::env::var(var) {
        Ok(v) if !v.is_empty() => Some(v),
        _ => {
            eprintln!("skipping: {var} not set");
            None
        }
    }
}

/// A pre-configured `assert_cmd::Command` pointing at the agent-rules binary.
pub fn cmd() -> assert_cmd::Command {
    assert_cmd::Command::cargo_bin("agent-rules")
        .expect("agent-rules binary not found; run `cargo build` first")
}
