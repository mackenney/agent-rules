import chalk from "chalk";
import { readFileSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { join } from "node:path";
import { PRReport, FileVerdict, RuleVerdict, DisplayVerdict, fileVerdictOverall, prReportOverallVerdict, effectiveSeverity } from "./schema.js";
export type { ProgressReporter } from "./progress.js";
export { createProgressReporter } from "./progress.js";

const DISPLAY_ICONS: Record<DisplayVerdict, string> = {
  pass: "✅",
  warn: "⚠️",
  error: "❌",
};

// Ruff/ty stylesheet (mirrors stylesheet.rs in ruff_db)
const sev = {
  error:   (s: string) => chalk.redBright.bold(s),
  warning: (s: string) => chalk.yellow.bold(s),
  note:    (s: string) => chalk.greenBright.bold(s),
  help:    (s: string) => chalk.cyanBright.bold(s),
  lineNo:  (s: string) => chalk.blueBright.bold(s),
  sep:     (s: string) => chalk.cyan(s),
  em:      (s: string) => chalk.bold(s),
  dim:     (s: string) => chalk.dim(s),
};

function displaySev(d: DisplayVerdict): "error" | "warning" {
  return d === "error" ? "error" : "warning";
}

/** One-liner per violation — mirrors ruff's concise output format. */
function renderConcise(fv: FileVerdict, rv: RuleVerdict): string {
  const display = effectiveSeverity(rv);
  const level = displaySev(display);
  const severity = level === "error" ? sev.error("error") : sev.warning("warning");
  const code = sev.em(`[${rv.rule_id}]`);
  const agenticBadge = rv.from_agentic ? chalk.cyan("[agentic] ") : "";
  const conf = sev.dim(`(${Math.round(rv.confidence * 100)}%)`);

  const lineStr = rv.line_refs.length > 0
    ? `${sev.sep(":")}${rv.line_refs[0]}`
    : "";

  return (
    `${sev.em(fv.file_path)}${lineStr}${sev.sep(":")} ` +
    `${severity}${code} ${agenticBadge}${rv.reasoning} ${conf}`
  );
}

/**
 * Annotate-snippets style block — mirrors ruff/rustc full diagnostic format.
 * Reads source lines from disk when available to render the ^^^ underline.
 */
function renderFull(fv: FileVerdict, rv: RuleVerdict, repoRoot?: string, headRef?: string): string {
  const display = effectiveSeverity(rv);
  const level = displaySev(display);
  const severity = level === "error" ? sev.error("error") : sev.warning("warning");
  const code = sev.em(`[${rv.rule_id}]`);
  const agenticBadge = rv.from_agentic ? ` ${chalk.cyan("[agentic]")}` : "";
  const lines: string[] = [];

  // Header: error[rule-id]: message
  lines.push(`${severity}${code}${agenticBadge}: ${rv.reasoning}`);

  const firstLine = rv.line_refs[0];
  const lastLine = rv.line_refs[rv.line_refs.length - 1];

  if (firstLine !== undefined) {
    // Location pointer: " --> file:line"
    const arrow = sev.sep("-->");
    const loc = `${sev.em(fv.file_path)}${sev.sep(":")}${firstLine}`;
    lines.push(` ${arrow} ${loc}`);

    // Try to load source for context
    let sourceLines: string[] | null = null;
    try {
      if (repoRoot && headRef) {
        const raw = execFileSync("git", ["show", `${headRef}:${fv.file_path}`], {
          cwd: repoRoot,
          encoding: "utf-8",
        });
        sourceLines = raw.split("\n");
      } else {
        const absPath = repoRoot ? join(repoRoot, fv.file_path) : fv.file_path;
        sourceLines = readFileSync(absPath, "utf-8").split("\n");
      }
    } catch {
      // file not readable from git or disk — show location header only
    }

    if (sourceLines) {
      const highlightSet = new Set(rv.line_refs);
      // Show 2 lines before first ref through 2 lines after last ref
      const ctxStart = Math.max(1, firstLine - 2);
      const ctxEnd = Math.min(sourceLines.length, (lastLine ?? firstLine) + 2);
      const gutterW = String(ctxEnd).length;
      const gutter = sev.lineNo(" ".repeat(gutterW) + " │");

      lines.push(gutter);
      for (let n = ctxStart; n <= ctxEnd; n++) {
        const srcLine = sourceLines[n - 1] ?? "";
        const lineNo = sev.lineNo(String(n).padStart(gutterW) + " │");
        lines.push(`${lineNo} ${srcLine}`);

        if (highlightSet.has(n)) {
          const indent = srcLine.length - srcLine.trimStart().length;
          const contentLen = Math.max(1, srcLine.trimEnd().length - indent);
          const caretStr = level === "error"
            ? sev.error("^".repeat(contentLen))
            : sev.warning("^".repeat(contentLen));
          lines.push(`${gutter} ${" ".repeat(indent)}${caretStr}`);
        }
      }
      lines.push(gutter);
    }
  }

  // Confidence note
  const conf = Math.round(rv.confidence * 100);
  lines.push(`${sev.dim("note")}: confidence ${conf}%`);
  lines.push("");

  return lines.join("\n");
}

export function printReport(report: PRReport, verbose: boolean = false, repoRoot?: string): void {
  // Flatten to violations only (pass verdicts are silent, like ruff)
  type Violation = { fv: FileVerdict; rv: RuleVerdict };
  const violations: Violation[] = [];
  for (const fv of report.files) {
    for (const rv of fv.verdicts) {
      if (rv.verdict === "pass") continue;
      violations.push({ fv, rv });
    }
  }

  // Sort: file path → first line ref ascending
  violations.sort((a, b) => {
    const fc = a.fv.file_path.localeCompare(b.fv.file_path);
    if (fc !== 0) return fc;
    return (a.rv.line_refs[0] ?? 0) - (b.rv.line_refs[0] ?? 0);
  });

  if (violations.length === 0) {
    console.log(chalk.green("All checks passed."));
  } else if (verbose) {
    const headRef = report.head_ref || undefined;
    for (const { fv, rv } of violations) {
      process.stdout.write(renderFull(fv, rv, repoRoot, headRef) + "\n");
    }
  } else {
    for (const { fv, rv } of violations) {
      console.log(renderConcise(fv, rv));
    }
    console.log();
  }

  // Summary line — count individual rule verdicts, not file-level buckets
  const s = report.stats;
  const overall = prReportOverallVerdict(report);
  const errorCount = violations.filter(({ rv }) => effectiveSeverity(rv) === "error").length;
  const warnCount  = violations.filter(({ rv }) => effectiveSeverity(rv) === "warn").length;
  const totalIssues = errorCount + warnCount;
  const durationS = (s.duration_ms / 1000).toFixed(1);
  const cachedNote = s.cache_hits > 0 ? sev.dim(` (${s.cache_hits} cached)`) : "";

  const summaryColor = overall === "error" ? sev.error : overall === "warn" ? sev.warning : chalk.green;
  const issueWord = totalIssues === 1 ? "issue" : "issues";
  const errorPart = errorCount > 0 ? `${errorCount} error${errorCount > 1 ? "s" : ""}` : "";
  const warnPart  = warnCount  > 0 ? `${warnCount} warning${warnCount > 1 ? "s" : ""}` : "";
  const breakdown = [errorPart, warnPart].filter(Boolean).join(", ");
  const checkedFiles = report.files.length;
  const inFiles = `in ${checkedFiles} file${checkedFiles > 1 ? "s" : ""}`;
  const model = sev.dim(`[${report.model_used || "n/a"}, ${durationS}s]`);

  if (totalIssues === 0) {
    console.log(`${chalk.green(`✓ ${checkedFiles} file${checkedFiles !== 1 ? "s" : ""} passed`)}${cachedNote} ${model}`);
  } else {
    console.log(
      `${summaryColor(`Found ${totalIssues} ${issueWord}`)} ${sev.dim(`(${breakdown})`)} ${sev.dim(inFiles)}${cachedNote} ${model}`
    );
  }
}

export function formatJson(report: PRReport): string {
  const data = JSON.parse(JSON.stringify(report)) as Record<string, unknown>;
  data["overall_verdict"] = prReportOverallVerdict(report);
  return JSON.stringify(data, null, 2);
}

export function formatGithubComment(report: PRReport): string {
  const overall = prReportOverallVerdict(report);
  const icon = DISPLAY_ICONS[overall];
  const lines: string[] = [];

  const prLabel = report.pr_url ?? `\`${report.base_ref}..${report.head_ref}\``;
  lines.push(`## ${icon} agent-rules — ${overall.toUpperCase()}`);
  lines.push(`PR: ${prLabel}`);
  lines.push("");

  const s = report.stats;
  lines.push(
    `**${s.pass_count} pass** · **${s.warn_count} warn** · **${s.error_count} error**` +
      (s.cache_hits > 0 ? ` · ${s.cache_hits} cached` : "")
  );
  lines.push("");

  for (const fv of report.files) {
    if (fv.verdicts.length === 0) continue;
    const fvOverall = fileVerdictOverall(fv);
    const fvIcon = DISPLAY_ICONS[fvOverall];
    lines.push("<details>");
    lines.push(`<summary><b>${fvIcon} ${fv.file_path}</b> — ${fvOverall}</summary>`);
    lines.push("");
    lines.push("| Rule | Verdict | Confidence | Reasoning |");
    lines.push("|------|---------|-----------|-----------|");
    for (const rv of fv.verdicts) {
      const rvDisplay = effectiveSeverity(rv);
      const rvIcon = DISPLAY_ICONS[rvDisplay];
      const reasoning = rv.reasoning.replace(/\|/g, "\\|").replace(/\n/g, " ");
      lines.push(
        `| \`${rv.rule_id}\` | ${rvIcon} ${rvDisplay} | ${Math.round(rv.confidence * 100)}% | ${reasoning} |`
      );
    }
    lines.push("");
    lines.push("</details>");
    lines.push("");
  }

  return lines.join("\n");
}

export function formatStepSummary(report: PRReport): string {
  const overall = prReportOverallVerdict(report);
  const icon = DISPLAY_ICONS[overall];
  const lines: string[] = [];

  const prLabel = report.pr_url ?? `\`${report.base_ref}..${report.head_ref}\``;
  lines.push(`## ${icon} agent-rules — ${overall.toUpperCase()}`);
  lines.push(`**PR:** ${prLabel}`);
  lines.push("");

  const s = report.stats;
  lines.push("| Pass | Warn | Error | Cached | Duration |");
  lines.push("|------|------|-------|--------|----------|");
  lines.push(
    `| ${s.pass_count} | ${s.warn_count} | ${s.error_count} | ${s.cache_hits} | ${(s.duration_ms / 1000).toFixed(1)}s |`
  );
  lines.push("");

  const issues = report.files.flatMap((fv) =>
    fv.verdicts
      .filter((rv) => rv.verdict !== "pass")
      .map((rv) => ({ fv, rv }))
  );

  if (issues.length > 0) {
    lines.push("### Issues");
    lines.push("| File | Rule | Verdict | Reasoning |");
    lines.push("|------|------|---------|-----------|");
    for (const { fv, rv } of issues) {
      const rvDisplay = effectiveSeverity(rv);
      const rvIcon = DISPLAY_ICONS[rvDisplay];
      let reasoning = rv.reasoning.replace(/\|/g, "\\|").replace(/\n/g, " ");
      if (reasoning.length > 120) reasoning = reasoning.slice(0, 117) + "…";
      lines.push(
        `| \`${fv.file_path}\` | \`${rv.rule_id}\` | ${rvIcon} ${rvDisplay} | ${reasoning} |`
      );
    }
    lines.push("");
  }

  return lines.join("\n");
}

// Re-export CI sinks for backward compatibility
export { maybeWriteStepSummary, emitWorkflowAnnotations } from "./ci.js";


