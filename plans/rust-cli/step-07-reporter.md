# Step 07: Reporter & Progress

## Context

### Overall Objective
Build a Rust CLI that checks PR diffs against LLM-powered rules defined in `.agent-rules.toml` files. Commands: `check`, `cache stats`, `cache clear`, `rules list`, `rules validate`.

### Phase Context
Wave 3 — This step can run in parallel with step-05 (prompt/llm). It only depends on step-02 (schema types). Implements output formatting and progress indication.

### This Step
Implement three reporters (Text, JSON, GitHub) and progress indication (TTY progress bar, CI mode). The text reporter follows ruff/rustc diagnostic style. Colors use owo-colors with NO_COLOR support.

## Prerequisites
- Step 02 complete (schema types for FileVerdict, PRReport)

## Files to Read Before Starting
- `rust/src/reporter.rs` — Replace the placeholder stub
- `rust/src/progress.rs` — Replace the placeholder stub
- `rust/src/schema.rs` — FileVerdict, PRReport, RuleVerdict, Severity, Verdict

## Implementation

### Task 1: Implement progress.rs

Replace `rust/src/progress.rs` with:

```rust
//! Progress reporting: TTY progress bar, CI output, null reporter
//!
//! Uses indicatif for TTY progress bars with spinner.

use indicatif::{ProgressBar, ProgressStyle};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

/// Progress reporter trait
pub trait ProgressReporter: Send + Sync {
    fn set_total(&self, n: usize);
    fn on_file_start(&self, path: &str);
    fn on_file_done(&self, path: &str);
    fn log(&self, msg: &str);
    fn finish(&self);
}

/// TTY progress bar using indicatif
pub struct TtyProgress {
    bar: Mutex<ProgressBar>,
    completed: AtomicUsize,
    total: AtomicUsize,
}

impl TtyProgress {
    pub fn new() -> Self {
        let bar = ProgressBar::new(0);
        bar.set_style(
            ProgressStyle::default_bar()
                .template("{spinner:.cyan} [{bar:40.cyan/dim}] {pos}/{len} {msg}")
                .expect("invalid template")
                .progress_chars("█░░"),
        );
        bar.enable_steady_tick(std::time::Duration::from_millis(80));

        Self {
            bar: Mutex::new(bar),
            completed: AtomicUsize::new(0),
            total: AtomicUsize::new(0),
        }
    }
}

impl Default for TtyProgress {
    fn default() -> Self {
        Self::new()
    }
}

impl ProgressReporter for TtyProgress {
    fn set_total(&self, n: usize) {
        self.total.store(n, Ordering::SeqCst);
        if let Ok(bar) = self.bar.lock() {
            bar.set_length(n as u64);
        }
    }

    fn on_file_start(&self, path: &str) {
        if let Ok(bar) = self.bar.lock() {
            // Truncate path to fit
            let display_path = if path.len() > 40 {
                format!("...{}", &path[path.len() - 37..])
            } else {
                path.to_string()
            };
            bar.set_message(display_path);
        }
    }

    fn on_file_done(&self, _path: &str) {
        let completed = self.completed.fetch_add(1, Ordering::SeqCst) + 1;
        if let Ok(bar) = self.bar.lock() {
            bar.set_position(completed as u64);
        }
    }

    fn log(&self, msg: &str) {
        if let Ok(bar) = self.bar.lock() {
            bar.println(msg);
        }
    }

    fn finish(&self) {
        if let Ok(bar) = self.bar.lock() {
            bar.finish_and_clear();
        }
    }
}

/// CI progress (non-interactive, one line per completed file)
pub struct CiProgress {
    completed: AtomicUsize,
    total: AtomicUsize,
}

impl CiProgress {
    pub fn new() -> Self {
        Self {
            completed: AtomicUsize::new(0),
            total: AtomicUsize::new(0),
        }
    }
}

impl Default for CiProgress {
    fn default() -> Self {
        Self::new()
    }
}

impl ProgressReporter for CiProgress {
    fn set_total(&self, n: usize) {
        self.total.store(n, Ordering::SeqCst);
        eprintln!("Checking {} files...", n);
    }

    fn on_file_start(&self, _path: &str) {
        // No output on start in CI mode
    }

    fn on_file_done(&self, path: &str) {
        let completed = self.completed.fetch_add(1, Ordering::SeqCst) + 1;
        let total = self.total.load(Ordering::SeqCst);
        eprintln!("[{}/{}] {}", completed, total, path);
    }

    fn log(&self, msg: &str) {
        eprintln!("{}", msg);
    }

    fn finish(&self) {
        // Nothing to clean up
    }
}

/// Null progress (no output)
pub struct NullProgress;

impl ProgressReporter for NullProgress {
    fn set_total(&self, _n: usize) {}
    fn on_file_start(&self, _path: &str) {}
    fn on_file_done(&self, _path: &str) {}
    fn log(&self, _msg: &str) {}
    fn finish(&self) {}
}

/// Detect if we should use TTY progress
pub fn should_use_tty() -> bool {
    // Check if stderr is a terminal
    atty::is(atty::Stream::Stderr)
        && std::env::var("CI").is_err()
        && std::env::var("GITHUB_ACTIONS").is_err()
}

/// Create appropriate progress reporter for environment
pub fn create_progress(force_ci: bool) -> Box<dyn ProgressReporter> {
    if force_ci || !should_use_tty() {
        Box::new(CiProgress::new())
    } else {
        Box::new(TtyProgress::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_null_progress() {
        let p = NullProgress;
        p.set_total(10);
        p.on_file_start("test.rs");
        p.on_file_done("test.rs");
        p.log("message");
        p.finish();
        // Should not panic
    }

    #[test]
    fn test_ci_progress_counting() {
        let p = CiProgress::new();
        p.set_total(3);
        assert_eq!(p.total.load(Ordering::SeqCst), 3);

        p.on_file_done("a.rs");
        assert_eq!(p.completed.load(Ordering::SeqCst), 1);

        p.on_file_done("b.rs");
        assert_eq!(p.completed.load(Ordering::SeqCst), 2);
    }
}
```

### Task 2: Update progress.rs for correct TTY detection

Use `std::io::IsTerminal` (stable since Rust 1.70) instead of the deprecated `atty` crate.
In `rust/src/progress.rs`, add the factory function that creates the right reporter:

```rust
use std::io::IsTerminal;

pub fn create_progress_reporter(total: usize) -> Box<dyn ProgressReporter> {
    let is_tty = std::io::stderr().is_terminal();
    let is_ci = std::env::var("CI").is_ok() || std::env::var("GITHUB_ACTIONS").is_ok();
    if is_tty && !is_ci {
        Box::new(TtyProgress::new())
    } else {
        Box::new(NullProgress)
    }
}
```

Do NOT add `atty = "0.2"` to Cargo.toml — it is unmaintained with a RUSTSEC advisory.
Do NOT add `is-terminal` crate either — `std::io::IsTerminal` from std covers the need.

### Task 3: Implement reporter.rs

Replace `rust/src/reporter.rs` with:

```rust
//! Output formatting: Text (ruff-style), JSON, GitHub markdown
//!
//! Text reporter follows ruff/rustc diagnostic style.
//! Colors respect NO_COLOR env var and --color flag.

use owo_colors::{OwoColorize, Style};
use std::io::Write;

use crate::config::OutputFormat;
use crate::schema::{
    FileVerdict, OverallVerdict, PRReport, ResolvedVerdict, RuleVerdict, Severity, Verdict,
};

/// Color configuration (respects NO_COLOR)
pub struct Stylesheet {
    pub error: Style,
    pub warning: Style,
    pub note: Style,
    pub file_path: Style,
    pub line_number: Style,
    pub dim: Style,
    pub success: Style,
    enabled: bool,
}

impl Stylesheet {
    pub fn new(color_enabled: bool) -> Self {
        let enabled = color_enabled && std::env::var("NO_COLOR").is_err();

        if enabled {
            Self {
                error: Style::new().red().bold(),
                warning: Style::new().yellow().bold(),
                note: Style::new().cyan(),
                file_path: Style::new().bold(),
                line_number: Style::new().blue().bold(),
                dim: Style::new().dimmed(),
                success: Style::new().green().bold(),
                enabled,
            }
        } else {
            Self {
                error: Style::new(),
                warning: Style::new(),
                note: Style::new(),
                file_path: Style::new(),
                line_number: Style::new(),
                dim: Style::new(),
                success: Style::new(),
                enabled,
            }
        }
    }

    pub fn default_enabled() -> Self {
        Self::new(true)
    }
}

impl Default for Stylesheet {
    fn default() -> Self {
        Self::new(true)
    }
}

/// Print a report in the specified format
pub fn print_report<W: Write>(
    report: &PRReport,
    format: OutputFormat,
    verbose: bool,
    writer: &mut W,
    colors: &Stylesheet,
) -> std::io::Result<()> {
    match format {
        OutputFormat::Text => print_text_report(report, verbose, writer, colors),
        OutputFormat::Json => print_json_report(report, writer),
        OutputFormat::Github => print_github_report(report, writer),
    }
}

/// Print ruff-style text report
fn print_text_report<W: Write>(
    report: &PRReport,
    verbose: bool,
    writer: &mut W,
    colors: &Stylesheet,
) -> std::io::Result<()> {
    let mut violations = Vec::new();

    // Collect all violations
    for file in &report.files {
        if file.skipped {
            continue;
        }
        for verdict in &file.verdicts {
            if verdict.verdict.resolve() == ResolvedVerdict::Fail {
                violations.push((file, verdict));
            }
        }
    }

    // Print violations
    for (file, verdict) in &violations {
        if verbose {
            print_verbose_violation(file, verdict, writer, colors)?;
        } else {
            print_concise_violation(file, verdict, writer, colors)?;
        }
    }

    // Print summary
    writeln!(writer)?;
    print_summary(report, writer, colors)?;

    Ok(())
}

/// Print concise violation (one line per violation)
/// Format: path:line: severity[rule-id] message (confidence%)
fn print_concise_violation<W: Write>(
    file: &FileVerdict,
    verdict: &RuleVerdict,
    writer: &mut W,
    colors: &Stylesheet,
) -> std::io::Result<()> {
    let line_part = verdict
        .line
        .map(|l| format!(":{}", l))
        .unwrap_or_default();

    let severity_str = match verdict.severity {
        Severity::Error => "error".style(colors.error),
        Severity::Warn => "warning".style(colors.warning),
    };

    let cached_marker = if verdict.cached {
        format!(" {}", "(cached)".style(colors.dim))
    } else {
        String::new()
    };

    writeln!(
        writer,
        "{}{}: {}[{}] {} ({}%){}", 
        file.file_path.style(colors.file_path),
        line_part.style(colors.line_number),
        severity_str,
        verdict.rule_id.style(colors.note),
        verdict.reasoning,
        (verdict.confidence * 100.0) as u32,
        cached_marker,
    )
}

/// Print verbose violation (multi-line, rustc-style)
fn print_verbose_violation<W: Write>(
    file: &FileVerdict,
    verdict: &RuleVerdict,
    writer: &mut W,
    colors: &Stylesheet,
) -> std::io::Result<()> {
    let severity_str = match verdict.severity {
        Severity::Error => "error".style(colors.error),
        Severity::Warn => "warning".style(colors.warning),
    };

    writeln!(
        writer,
        "{}[{}]: {}",
        severity_str,
        verdict.rule_id.style(colors.note),
        verdict.rule_name,
    )?;

    let line_part = verdict
        .line
        .map(|l| format!(":{}", l))
        .unwrap_or_default();

    writeln!(
        writer,
        "  {} {}{}",
        "-->".style(colors.dim),
        file.file_path.style(colors.file_path),
        line_part.style(colors.line_number),
    )?;

    writeln!(writer, "   {}", "|".style(colors.dim))?;
    writeln!(
        writer,
        "   {} {}",
        "=".style(colors.dim),
        verdict.reasoning,
    )?;
    writeln!(
        writer,
        "   {} confidence: {}%{}",
        "=".style(colors.dim),
        (verdict.confidence * 100.0) as u32,
        if verdict.cached { " (cached)" } else { "" },
    )?;
    writeln!(writer)?;

    Ok(())
}

/// Print summary line
fn print_summary<W: Write>(
    report: &PRReport,
    writer: &mut W,
    colors: &Stylesheet,
) -> std::io::Result<()> {
    let verdict_style = match report.overall_verdict {
        OverallVerdict::Pass => colors.success,
        OverallVerdict::Warn => colors.warning,
        OverallVerdict::Fail => colors.error,
    };

    let verdict_icon = match report.overall_verdict {
        OverallVerdict::Pass => "✓",
        OverallVerdict::Warn => "⚠",
        OverallVerdict::Fail => "✗",
    };

    writeln!(
        writer,
        "{} {} | {} files checked, {} passed, {} failed, {} skipped",
        verdict_icon.style(verdict_style),
        format!("{}", report.overall_verdict).to_uppercase().style(verdict_style),
        report.files_checked,
        report.files_passed,
        report.files_failed,
        report.files_skipped,
    )?;

    if report.cache_hits > 0 {
        writeln!(
            writer,
            "  {} cache hits",
            report.cache_hits.style(colors.dim),
        )?;
    }

    Ok(())
}

/// Print JSON report
fn print_json_report<W: Write>(report: &PRReport, writer: &mut W) -> std::io::Result<()> {
    let json = serde_json::to_string_pretty(report).map_err(|e| {
        std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string())
    })?;
    writeln!(writer, "{}", json)
}

/// Print GitHub markdown report
fn print_github_report<W: Write>(report: &PRReport, writer: &mut W) -> std::io::Result<()> {
    // Header with icon
    let (icon, title) = match report.overall_verdict {
        OverallVerdict::Pass => ("✅", "All checks passed"),
        OverallVerdict::Warn => ("⚠️", "Checks passed with warnings"),
        OverallVerdict::Fail => ("❌", "Some checks failed"),
    };

    writeln!(writer, "## {} {}", icon, title)?;
    writeln!(writer)?;

    // Stats line
    writeln!(
        writer,
        "**{} files** checked | **{}** passed | **{}** failed | **{}** skipped",
        report.files_checked,
        report.files_passed,
        report.files_failed,
        report.files_skipped,
    )?;
    writeln!(writer)?;

    // File details
    let failed_files: Vec<_> = report
        .files
        .iter()
        .filter(|f| !f.skipped && !f.passed)
        .collect();

    if !failed_files.is_empty() {
        writeln!(writer, "### Violations")?;
        writeln!(writer)?;

        for file in failed_files {
            writeln!(writer, "<details>")?;
            writeln!(writer, "<summary><code>{}</code></summary>", file.file_path)?;
            writeln!(writer)?;
            writeln!(writer, "| Rule | Verdict | Confidence | Reasoning |")?;
            writeln!(writer, "|------|---------|------------|-----------|")?;

            for v in &file.verdicts {
                if v.verdict.resolve() == ResolvedVerdict::Fail {
                    let verdict_emoji = match v.verdict {
                        Verdict::Fail => "❌",
                        Verdict::NeedsMoreContext => "❓",
                        Verdict::Pass => "✅",
                    };
                    // Escape pipe characters in reasoning
                    let reasoning = v.reasoning.replace('|', "\\|");
                    writeln!(
                        writer,
                        "| `{}` | {} | {}% | {} |",
                        v.rule_id,
                        verdict_emoji,
                        (v.confidence * 100.0) as u32,
                        reasoning,
                    )?;
                }
            }

            writeln!(writer)?;
            writeln!(writer, "</details>")?;
            writeln!(writer)?;
        }
    }

    // Sentinel for comment upsert
    writeln!(writer)?;
    writeln!(
        writer,
        "<!-- agent-rules-report base={} head={} -->",
        report.base_ref, report.head_ref
    )?;

    Ok(())
}

/// Get exit code for a report
pub fn exit_code_for_report(report: &PRReport, warn_as_error: bool) -> i32 {
    match report.overall_verdict {
        OverallVerdict::Pass => 0,
        OverallVerdict::Warn => {
            if warn_as_error {
                1
            } else {
                0
            }
        }
        OverallVerdict::Fail => 2,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_report(overall: OverallVerdict) -> PRReport {
        PRReport {
            base_ref: "main".to_string(),
            head_ref: "HEAD".to_string(),
            pr_url: None,
            files: vec![],
            overall_verdict: overall,
            files_checked: 5,
            files_passed: 3,
            files_failed: 2,
            files_skipped: 0,
            rules_evaluated: 10,
            rules_passed: 8,
            rules_failed: 2,
            cache_hits: 1,
        }
    }

    #[test]
    fn test_exit_code_pass() {
        let report = make_test_report(OverallVerdict::Pass);
        assert_eq!(exit_code_for_report(&report, false), 0);
        assert_eq!(exit_code_for_report(&report, true), 0);
    }

    #[test]
    fn test_exit_code_warn() {
        let report = make_test_report(OverallVerdict::Warn);
        assert_eq!(exit_code_for_report(&report, false), 0);
        assert_eq!(exit_code_for_report(&report, true), 1);
    }

    #[test]
    fn test_exit_code_fail() {
        let report = make_test_report(OverallVerdict::Fail);
        assert_eq!(exit_code_for_report(&report, false), 2);
        assert_eq!(exit_code_for_report(&report, true), 2);
    }

    #[test]
    fn test_json_output() {
        let report = make_test_report(OverallVerdict::Pass);
        let mut output = Vec::new();
        print_json_report(&report, &mut output).unwrap();
        let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(json["overall_verdict"], "pass");
    }

    #[test]
    fn test_github_output_contains_sentinel() {
        let report = make_test_report(OverallVerdict::Pass);
        let mut output = Vec::new();
        print_github_report(&report, &mut output).unwrap();
        let output_str = String::from_utf8(output).unwrap();
        assert!(output_str.contains("<!-- agent-rules-report"));
    }

    #[test]
    fn test_stylesheet_no_color() {
        std::env::set_var("NO_COLOR", "1");
        let style = Stylesheet::new(true);
        assert!(!style.enabled);
        std::env::remove_var("NO_COLOR");
    }
}
```

## Acceptance Criteria

These must ALL pass before reporting complete:

- [ ] `cd rust && cargo build 2>&1 | grep -E "^error" | wc -l` — outputs `0`
- [ ] `cd rust && cargo test progress:: 2>&1 | grep -E "^test result"` — shows `ok` with 0 failed
- [ ] `cd rust && cargo test reporter:: 2>&1 | grep -E "^test result"` — shows `ok` with 0 failed
- [ ] `grep -c "pub trait ProgressReporter" rust/src/progress.rs` — outputs `1`
- [ ] `grep -c "pub struct TtyProgress" rust/src/progress.rs` — outputs `1`
- [ ] `grep -c "pub struct Stylesheet" rust/src/reporter.rs` — outputs `1`
- [ ] `grep -c "pub fn print_report" rust/src/reporter.rs` — outputs `1`
- [ ] `grep -c "exit_code_for_report" rust/src/reporter.rs` — outputs at least `2`
- [ ] No regressions: `cd rust && cargo test 2>&1 | grep -E "^test result"` — shows 0 failed

## Reviewer Instructions

You are reviewing Step 07. Verify:

1. Run `cd rust && cargo test progress::` — all tests pass
2. Run `cd rust && cargo test reporter::` — all tests pass
3. Check `rust/src/progress.rs` contains:
   - `ProgressReporter` trait with set_total, on_file_start, on_file_done, log, finish
   - `TtyProgress` using indicatif ProgressBar with spinner
   - `CiProgress` printing `[N/M] path` per file
   - `NullProgress` no-op implementation
   - `should_use_tty()` detecting CI environment
4. Check `rust/src/reporter.rs` contains:
   - `Stylesheet` with colors for error, warning, note, file_path, etc.
   - NO_COLOR env var support
   - `print_report()` dispatching to format-specific functions
   - Concise format: `path:line: severity[rule-id] message (confidence%)`
   - Verbose format: multi-line rustc-style
   - JSON: serde_json::to_string_pretty
   - GitHub: markdown with details blocks and sentinel comment
   - `exit_code_for_report()` returning 0/1/2
5. Run `cd rust && cargo clippy 2>&1 | grep "^error"` — no errors

Report: "PASS" with each criterion confirmed, or "FAIL: <criterion> — <what's wrong>"

## Rollback
```bash
git checkout HEAD -- rust/src/progress.rs rust/src/reporter.rs rust/Cargo.toml
```
