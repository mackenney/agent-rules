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
    #[allow(dead_code)] // read in tests to assert color state
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
}

impl Default for Stylesheet {
    fn default() -> Self {
        Self::new(true)
    }
}

/// Print a report in the specified format.
/// `repo_root` is used in verbose mode to load source context.
pub fn print_report<W: Write>(
    report: &PRReport,
    format: OutputFormat,
    verbose: bool,
    repo_root: Option<&std::path::Path>,
    writer: &mut W,
    colors: &Stylesheet,
) -> std::io::Result<()> {
    match format {
        OutputFormat::Text => print_text_report(report, verbose, repo_root, writer, colors),
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
    repo_root: Option<&std::path::Path>,
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

    violations.sort_by(|(fa, va), (fb, vb)| {
        fa.file_path.cmp(&fb.file_path).then_with(|| {
            let line_a = va.line_refs.first().copied().or(va.line).unwrap_or(0);
            let line_b = vb.line_refs.first().copied().or(vb.line).unwrap_or(0);
            line_a.cmp(&line_b)
        })
    });

    for (file, verdict) in &violations {
        if verbose {
            print_verbose_violation(file, verdict, repo_root, writer, colors)?;
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
    repo_root: Option<&std::path::Path>,
    writer: &mut W,
    colors: &Stylesheet,
) -> std::io::Result<()> {
    let severity_str = match verdict.severity {
        Severity::Error => "error".style(colors.error),
        Severity::Warn => "warning".style(colors.warning),
    };

    // Header: severity[rule-id]: reasoning
    writeln!(
        writer,
        "{}[{}]: {}",
        severity_str,
        verdict.rule_id.style(colors.note),
        verdict.reasoning,
    )?;

    // Location: --> file:first_line
    let first_line = verdict.line_refs.first().copied().or(verdict.line);
    let line_part = first_line.map(|l| format!(":{l}")).unwrap_or_default();
    writeln!(
        writer,
        "  {} {}{}",
        "-->".style(colors.gutter),
        file.file_path.style(colors.file_path),
        line_part,
    )?;

    // Load source lines from disk (fall back silently)
    let source_lines: Option<Vec<String>> = repo_root.and_then(|root| {
        let abs = root.join(&file.file_path);
        std::fs::read_to_string(&abs)
            .ok()
            .map(|s| s.lines().map(|l| l.to_string()).collect())
    });

    if let (Some(src), Some(first)) = (&source_lines, first_line) {
        let highlight_set: std::collections::HashSet<u32> =
            verdict.line_refs.iter().copied().collect();
        let last = verdict.line_refs.last().copied().unwrap_or(first);
        // Show 2 lines before first ref through 2 lines after last ref
        let ctx_start = first.saturating_sub(2).max(1) as usize;
        let ctx_end = ((last + 2) as usize).min(src.len());
        let gutter_w = ctx_end.to_string().len();

        let gutter_line = format!(
            " {:>w$} {}",
            "",
            "│".style(colors.vertical_bar),
            w = gutter_w
        );
        writeln!(writer, "{}", gutter_line)?;

        for n in ctx_start..=ctx_end {
            let src_line = src.get(n - 1).map(|s| s.as_str()).unwrap_or("");
            let line_no = format!(" {:>w$} ", n, w = gutter_w);
            writeln!(
                writer,
                "{}{}  {}",
                line_no.style(colors.line_number),
                "│".style(colors.vertical_bar),
                src_line,
            )?;

            if highlight_set.contains(&(n as u32)) {
                // Underline the non-whitespace content with carets
                let indent = src_line.len() - src_line.trim_start().len();
                let content_len = src_line.trim_end().len().saturating_sub(indent).max(1);
                let carets = "^".repeat(content_len);
                let caret_str = match verdict.severity {
                    Severity::Error => carets.style(colors.error).to_string(),
                    Severity::Warn => carets.style(colors.warning).to_string(),
                };
                // Gutter + spaces matching indent + carets
                let pad = " ".repeat(gutter_w + 3 + indent); // line_no width + " │  " + indent
                writeln!(
                    writer,
                    "{}{}  {}{}",
                    " ".repeat(gutter_w + 1).style(colors.line_number),
                    "│".style(colors.vertical_bar),
                    " ".repeat(indent),
                    caret_str,
                )?;
                let _ = pad; // suppress unused warning
            }
        }

        writeln!(writer, "{}", gutter_line)?;
    } else {
        // No source: just show a thin divider
        let gutter = format!(" {:>3} {}", "", "│".style(colors.vertical_bar));
        writeln!(writer, "{}", gutter)?;
    }

    // Confidence note
    let cached_note = if verdict.cached { " (cached)" } else { "" };
    writeln!(
        writer,
        "{}: confidence {}%{}",
        "note".style(colors.dim),
        (verdict.confidence * 100.0) as u32,
        cached_note,
    )?;
    writeln!(writer)?;

    Ok(())
}

fn print_summary<W: Write>(
    report: &PRReport,
    writer: &mut W,
    colors: &Stylesheet,
) -> std::io::Result<()> {
    let cache_part = if report.cache_hits > 0 {
        format!(", {} cached", report.cache_hits)
    } else {
        String::new()
    };

    if report.overall_verdict == OverallVerdict::Pass {
        writeln!(
            writer,
            "{} {} files passed ({}, {}ms{})",
            "✓".style(colors.success),
            report.files_checked,
            report.model,
            report.duration_ms,
            cache_part,
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
            "{} Found {} issue{} ({} error{}, {} warning{}) in {} file{} ({}, {}ms{})",
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
            report.model,
            report.duration_ms,
            cache_part,
        )?;
    }

    Ok(())
}

fn print_github_report<W: Write>(report: &PRReport, writer: &mut W) -> std::io::Result<()> {
    let (icon, verdict_code) = match report.overall_verdict {
        OverallVerdict::Pass => ("✅", "PASS"),
        OverallVerdict::Warn => ("⚠️", "WARN"),
        OverallVerdict::Fail => ("❌", "FAIL"),
    };

    writeln!(writer, "## {} agent-rules — {}", icon, verdict_code)?;
    writeln!(writer)?;

    let pr_label = match &report.pr_url {
        Some(url) => url.clone(),
        None => format!("`{}..{}`", report.base_ref, report.head_ref),
    };
    writeln!(writer, "**PR:** {}", pr_label)?;
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
    writeln!(writer, "<!-- agent-rules-report -->")?;

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
            model: "test-model".to_string(),
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
        print_report(&report, OutputFormat::Text, false, None, &mut out, &colors).unwrap();
        let s = String::from_utf8(out).unwrap();
        assert!(s.contains("files passed"));
    }

    #[test]
    fn reporter_print_report_json() {
        let report = make_test_report(OverallVerdict::Fail);
        let colors = Stylesheet::new(false);
        let mut out = Vec::new();
        print_report(&report, OutputFormat::Json, false, None, &mut out, &colors).unwrap();
        let json: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(json["overall_verdict"], "error");
    }

    #[test]
    fn reporter_print_report_github() {
        let report = make_test_report(OverallVerdict::Pass);
        let colors = Stylesheet::new(false);
        let mut out = Vec::new();
        print_report(
            &report,
            OutputFormat::Github,
            false,
            None,
            &mut out,
            &colors,
        )
        .unwrap();
        let s = String::from_utf8(out).unwrap();
        assert!(s.contains("<!-- agent-rules-report"));
    }
}
