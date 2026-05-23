//! Output formatting: Text (ruff-style), JSON, GitHub markdown
//!
//! Text reporter follows ruff/rustc diagnostic style.
//! Colors respect NO_COLOR env var via owo-colors.

use owo_colors::{OwoColorize, Style};
use std::io::Write;

use crate::config::OutputFormat;
use crate::schema::{
    FileVerdict, OverallVerdict, PRReport, ResolvedVerdict, RuleVerdict, Severity, Verdict,
};

/// Color configuration. Respects NO_COLOR environment variable.
pub struct Stylesheet {
    pub error: Style,
    pub warning: Style,
    pub note: Style,
    pub file_path: Style,
    pub line_number: Style,
    pub dim: Style,
    pub success: Style,
    pub gutter: Style,
    pub vertical_bar: Style,
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
                success: Style::new().green(),
                gutter: Style::new().cyan(),
                vertical_bar: Style::new().blue().bold(),
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
                gutter: Style::new(),
                vertical_bar: Style::new(),
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
        OutputFormat::Json => {
            let json = format_json(report);
            writeln!(writer, "{}", json)
        }
        OutputFormat::Github => {
            let md = format_github_comment(report);
            write!(writer, "{}", md)
        }
    }
}

/// Format the report as a pretty-printed JSON string
pub fn format_json(report: &PRReport) -> String {
    serde_json::to_string_pretty(report).unwrap_or_else(|e| format!("{{\"error\": \"{}\"}}", e))
}

/// Format the report as a GitHub markdown comment string
pub fn format_github_comment(report: &PRReport) -> String {
    let mut out = Vec::new();
    print_github_report(report, &mut out).unwrap_or_default();
    String::from_utf8_lossy(&out).into_owned()
}

fn print_text_report<W: Write>(
    report: &PRReport,
    verbose: bool,
    writer: &mut W,
    colors: &Stylesheet,
) -> std::io::Result<()> {
    let mut violations = Vec::new();

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

    for (file, verdict) in &violations {
        if verbose {
            print_verbose_violation(file, verdict, writer, colors)?;
        } else {
            print_concise_violation(file, verdict, writer, colors)?;
        }
    }

    writeln!(writer)?;
    print_summary(report, writer, colors)?;

    Ok(())
}

fn print_concise_violation<W: Write>(
    file: &FileVerdict,
    verdict: &RuleVerdict,
    writer: &mut W,
    colors: &Stylesheet,
) -> std::io::Result<()> {
    let line_part = verdict.line.map(|l| format!(":{}", l)).unwrap_or_default();

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

    let line_part = verdict.line.map(|l| format!(":{}", l)).unwrap_or_default();

    writeln!(
        writer,
        "  {} {}{}",
        "-->".style(colors.gutter),
        file.file_path.style(colors.file_path),
        line_part.style(colors.line_number),
    )?;

    writeln!(writer, "   {}", "│".style(colors.vertical_bar))?;
    writeln!(
        writer,
        "   {} {}",
        "=".style(colors.gutter),
        verdict.reasoning,
    )?;
    writeln!(
        writer,
        "   {} confidence: {}%{}",
        "=".style(colors.gutter),
        (verdict.confidence * 100.0) as u32,
        if verdict.cached { " (cached)" } else { "" },
    )?;
    writeln!(writer)?;

    Ok(())
}

fn print_summary<W: Write>(
    report: &PRReport,
    writer: &mut W,
    colors: &Stylesheet,
) -> std::io::Result<()> {
    if report.overall_verdict == OverallVerdict::Pass {
        writeln!(
            writer,
            "{} {} files passed ({} cached)",
            "✓".style(colors.success),
            report.files_checked,
            report.cache_hits,
        )?;
    } else {
        let error_count = report
            .files
            .iter()
            .flat_map(|f| f.verdicts.iter())
            .filter(|v| {
                v.verdict.resolve() == ResolvedVerdict::Fail && v.severity == Severity::Error
            })
            .count();
        let warn_count = report
            .files
            .iter()
            .flat_map(|f| f.verdicts.iter())
            .filter(|v| {
                v.verdict.resolve() == ResolvedVerdict::Fail && v.severity == Severity::Warn
            })
            .count();

        let verdict_style = match report.overall_verdict {
            OverallVerdict::Pass => colors.success,
            OverallVerdict::Warn => colors.warning,
            OverallVerdict::Fail => colors.error,
        };

        writeln!(
            writer,
            "{} Found {} issue{} ({} error{}, {} warning{}) in {} file{} ({} cached)",
            "✗".style(verdict_style),
            error_count + warn_count,
            if error_count + warn_count == 1 {
                ""
            } else {
                "s"
            },
            error_count,
            if error_count == 1 { "" } else { "s" },
            warn_count,
            if warn_count == 1 { "" } else { "s" },
            report.files_checked,
            if report.files_checked == 1 { "" } else { "s" },
            report.cache_hits,
        )?;
    }

    Ok(())
}

fn print_github_report<W: Write>(report: &PRReport, writer: &mut W) -> std::io::Result<()> {
    let (icon, title) = match report.overall_verdict {
        OverallVerdict::Pass => ("✅", "All checks passed"),
        OverallVerdict::Warn => ("⚠️", "Checks passed with warnings"),
        OverallVerdict::Fail => ("❌", "Some checks failed"),
    };

    writeln!(writer, "## {} {}", icon, title)?;
    writeln!(writer)?;

    writeln!(
        writer,
        "**{} files** checked | **{}** passed | **{}** failed | **{}** skipped",
        report.files_checked, report.files_passed, report.files_failed, report.files_skipped,
    )?;
    writeln!(writer)?;

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

    writeln!(writer)?;
    writeln!(
        writer,
        "<!-- agent-rules-report base={} head={} -->",
        report.base_ref, report.head_ref
    )?;

    Ok(())
}

/// Get exit code for a report: 0 = pass, 1 = warn (with --warn-as-error), 2 = fail
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
            duration_ms: 0,
        }
    }

    #[test]
    fn reporter_exit_code_pass() {
        let report = make_test_report(OverallVerdict::Pass);
        assert_eq!(exit_code_for_report(&report, false), 0);
        assert_eq!(exit_code_for_report(&report, true), 0);
    }

    #[test]
    fn reporter_exit_code_warn() {
        let report = make_test_report(OverallVerdict::Warn);
        assert_eq!(exit_code_for_report(&report, false), 0);
        assert_eq!(exit_code_for_report(&report, true), 1);
    }

    #[test]
    fn reporter_exit_code_fail() {
        let report = make_test_report(OverallVerdict::Fail);
        assert_eq!(exit_code_for_report(&report, false), 2);
        assert_eq!(exit_code_for_report(&report, true), 2);
    }

    #[test]
    fn reporter_json_output() {
        let report = make_test_report(OverallVerdict::Pass);
        let json_str = format_json(&report);
        let json: serde_json::Value = serde_json::from_str(&json_str).unwrap();
        assert_eq!(json["overall_verdict"], "pass");
    }

    #[test]
    fn reporter_github_output_contains_sentinel() {
        let report = make_test_report(OverallVerdict::Pass);
        let md = format_github_comment(&report);
        assert!(md.contains("<!-- agent-rules-report"));
    }

    #[test]
    fn reporter_stylesheet_no_color() {
        // Set NO_COLOR temporarily — note: tests may run in parallel so
        // we check the created style object directly.
        let style = Stylesheet::new(false);
        assert!(!style.enabled);
    }

    #[test]
    #[test]
    #[allow(deprecated)]
    fn reporter_stylesheet_color_disabled_by_no_color() {
        // SAFETY: This test sets NO_COLOR env var. Tests run in separate processes by default
        // in cargo test, but parallel threads within a process share env. Acceptable for now.
        std::env::set_var("NO_COLOR", "1");
        let style = Stylesheet::new(true);
        assert!(!style.enabled);
        std::env::remove_var("NO_COLOR");
    }

    #[test]
    fn reporter_print_report_text() {
        let report = make_test_report(OverallVerdict::Pass);
        let colors = Stylesheet::new(false);
        let mut out = Vec::new();
        print_report(&report, OutputFormat::Text, false, &mut out, &colors).unwrap();
        let s = String::from_utf8(out).unwrap();
        assert!(s.contains("files passed"));
    }

    #[test]
    fn reporter_print_report_json() {
        let report = make_test_report(OverallVerdict::Fail);
        let colors = Stylesheet::new(false);
        let mut out = Vec::new();
        print_report(&report, OutputFormat::Json, false, &mut out, &colors).unwrap();
        let json: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(json["overall_verdict"], "fail");
    }

    #[test]
    fn reporter_print_report_github() {
        let report = make_test_report(OverallVerdict::Pass);
        let colors = Stylesheet::new(false);
        let mut out = Vec::new();
        print_report(&report, OutputFormat::Github, false, &mut out, &colors).unwrap();
        let s = String::from_utf8(out).unwrap();
        assert!(s.contains("<!-- agent-rules-report"));
    }
}
