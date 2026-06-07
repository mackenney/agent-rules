//! Integration tests for `cache stats` and `cache clear`.
//! No LLM calls.

use crate::common::cmd;

/// `cache stats` on a repo with no cache must exit 0 and report 0 entries.
#[test]
fn stats_on_empty_repo() {
    let dir = tempfile::tempdir().unwrap();
    let out = cmd()
        .args(["cache", "stats", "--repo"])
        .arg(dir.path())
        .output()
        .unwrap();
    assert!(out.status.success(), "cache stats failed: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains('0'),
        "expected 0 entries in output:\n{stdout}"
    );
}

/// `cache clear -y` on a repo with no cache must exit 0.
#[test]
fn clear_with_yes_flag_succeeds() {
    let dir = tempfile::tempdir().unwrap();
    cmd()
        .args(["cache", "clear", "--repo"])
        .arg(dir.path())
        .arg("-y")
        .assert()
        .success();
}
