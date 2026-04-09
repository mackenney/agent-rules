import { appendFileSync } from "node:fs";
import { PRReport, effectiveSeverity } from "./schema.js";
import { formatStepSummary } from "./reporter.js";

export function maybeWriteStepSummary(report: PRReport): void {
  const summaryPath = process.env["GITHUB_STEP_SUMMARY"];
  if (!summaryPath) return;
  try {
    appendFileSync(summaryPath, formatStepSummary(report), "utf-8");
  } catch (err) {
    console.error(`Could not write step summary: ${(err as Error).message}`);
  }
}

/**
 * Emit GitHub Actions workflow commands for each warn/reject verdict so they
 * appear as inline code annotations on the PR diff.
 *
 * Format: ::error file={file},line={line},title={title}::{message}
 * Docs: https://docs.github.com/en/actions/writing-workflows/choosing-what-your-workflow-does/workflow-commands-for-github-actions#setting-an-error-message
 */
export function emitWorkflowAnnotations(report: PRReport): void {
  if (!process.env["GITHUB_ACTIONS"]) return;

  for (const fv of report.files) {
    for (const rv of fv.verdicts) {
      if (rv.verdict === "pass") continue;

      const display = effectiveSeverity(rv);
      const level = display === "error" ? "error" : "warning";
      const title = `agent-rules: ${rv.rule_id}`;
      const message = rv.reasoning.replace(/\n/g, " ").replace(/%/g, "%25").replace(/\r/g, "%0D");
      const file = fv.file_path;

      if (rv.line_refs.length > 0) {
        const line = rv.line_refs[0]!;
        const endLine = rv.line_refs[rv.line_refs.length - 1]!;
        if (line !== endLine) {
          process.stdout.write(`::${level} file=${file},line=${line},endLine=${endLine},title=${title}::${message}\n`);
        } else {
          process.stdout.write(`::${level} file=${file},line=${line},title=${title}::${message}\n`);
        }
      } else {
        process.stdout.write(`::${level} file=${file},title=${title}::${message}\n`);
      }
    }
  }
}
