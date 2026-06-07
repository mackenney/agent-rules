//! Integration tests for `rules list` and `rules validate`.
//! No LLM calls — pure CLI/filesystem behavior.

use std::io::Write;

use crate::common::{cmd, test_repo};

/// `rules validate` must succeed on the well-formed test-repo.
#[test]
fn validate_passes_on_test_repo() {
    cmd()
        .args(["rules", "validate", "--repo"])
        .arg(test_repo())
        .assert()
        .success();
}

/// `rules list` for payment_controller.py must include all four applicable rules.
#[test]
fn list_payment_controller_has_expected_rules() {
    let out = cmd()
        .args([
            "rules",
            "list",
            "--path",
            "src/api/payment_controller.py",
            "--repo",
        ])
        .arg(test_repo())
        .output()
        .unwrap();
    assert!(out.status.success(), "rules list failed: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    for rule in &[
        "arch/enforce-payment-limits",
        "api/validate-user-before-payment",
        "api/no-raw-sql",
        "security/no-hardcoded-secrets",
    ] {
        assert!(
            stdout.contains(rule),
            "rules list missing {rule}:\n{stdout}"
        );
    }
}

/// `rules list` for clean_controller.py must include root-level rules.
/// (clean_controller.py is under src/api/ so it inherits root + api rules)
#[test]
fn list_clean_controller_includes_root_rules() {
    let out = cmd()
        .args([
            "rules",
            "list",
            "--path",
            "src/api/clean_controller.py",
            "--repo",
        ])
        .arg(test_repo())
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("security/no-hardcoded-secrets"),
        "must include root-level secrets rule:\n{stdout}"
    );
}

/// `api/no-raw-sql` glob-excludes `tests/**`, so test files must not see that rule.
#[test]
fn list_test_file_excluded_from_no_raw_sql() {
    let out = cmd()
        .args(["rules", "list", "--path", "tests/test_user.py", "--repo"])
        .arg(test_repo())
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stdout.contains("api/no-raw-sql"),
        "test file must be excluded from api/no-raw-sql:\n{stdout}"
    );
}

/// `rules validate` must exit 1 on a rule file with duplicate IDs.
#[test]
fn validate_rejects_duplicate_ids() {
    let dir = tempfile::tempdir().unwrap();
    let mut f = std::fs::File::create(dir.path().join(".agent-rules.toml")).unwrap();
    write!(
        f,
        r#"
version = "1"

[[rules]]
id = "dup/rule"
name = "Rule A"
prompt = "Check A"

[[rules]]
id = "dup/rule"
name = "Rule B"
prompt = "Check B"
"#
    )
    .unwrap();

    cmd()
        .args(["rules", "validate", "--repo"])
        .arg(dir.path())
        .assert()
        .failure();
}

/// `rules validate` must exit 1 on a TOML syntax error.
#[test]
fn validate_rejects_invalid_toml() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join(".agent-rules.toml"),
        b"not valid toml ][[\n",
    )
    .unwrap();
    cmd()
        .args(["rules", "validate", "--repo"])
        .arg(dir.path())
        .assert()
        .failure();
}
