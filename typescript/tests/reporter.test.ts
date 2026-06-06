import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { mkdirSync, writeFileSync, rmSync } from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";
import {
  printReport,
  formatJson,
  formatGithubComment,
  formatStepSummary,
} from "../src/reporter.js";
import type { PRReport, FileVerdict, RuleVerdict } from "../src/schema.js";

function createTmpDir(): string {
  const dir = join(tmpdir(), `agent-rules-reporter-test-${Date.now()}-${Math.random().toString(36).slice(2)}`);
  mkdirSync(dir, { recursive: true });
  return dir;
}

function makeRuleVerdict(overrides: Partial<RuleVerdict> = {}): RuleVerdict {
  return {
    rule_id: "test/rule",
    verdict: "pass",
    rule_severity: "warn",
    confidence: 1.0,
    reasoning: "All good",
    line_refs: [],
    context_hint: null,
    from_agentic: false,
    ...overrides,
  };
}

function makeFileVerdict(overrides: Partial<FileVerdict> = {}): FileVerdict {
  return {
    file_path: "src/test.ts",
    verdicts: [makeRuleVerdict()],
    cached: false,
    check_duration_ms: 100,
    agentic_escalations: 0,
    ...overrides,
  };
}

function makeReport(overrides: Partial<PRReport> = {}): PRReport {
  return {
    pr_url: null,
    base_ref: "HEAD~1",
    head_ref: "HEAD",
    files: [makeFileVerdict()],
    model_used: "claude-haiku-4-5",
    stats: {
      total_files: 1,
      files_with_rules: 1,
      cache_hits: 0,
      agentic_escalations: 0,
      pass_count: 1,
      warn_count: 0,
      error_count: 0,
      duration_ms: 100,
    },
    ...overrides,
  };
}

describe("formatJson", () => {
  it("outputs valid JSON", () => {
    const report = makeReport();
    const json = formatJson(report);
    expect(() => JSON.parse(json)).not.toThrow();
  });

  it("includes overall_verdict field", () => {
    const report = makeReport();
    const json = formatJson(report);
    const parsed = JSON.parse(json) as Record<string, unknown>;
    expect(parsed["overall_verdict"]).toBe("pass");
  });

  it("preserves all report fields", () => {
    const report = makeReport({ pr_url: "https://github.com/test/repo/pull/1" });
    const json = formatJson(report);
    const parsed = JSON.parse(json) as Record<string, unknown>;
    expect(parsed["pr_url"]).toBe("https://github.com/test/repo/pull/1");
    expect(parsed["model_used"]).toBe("claude-haiku-4-5");
    expect((parsed["stats"] as Record<string, number>)["total_files"]).toBe(1);
  });

  it("sets overall_verdict to error when any file has fail+error verdict", () => {
    const report = makeReport({
      files: [makeFileVerdict({ verdicts: [makeRuleVerdict({ verdict: "fail", rule_severity: "error" })] })],
    });
    const json = formatJson(report);
    const parsed = JSON.parse(json) as Record<string, unknown>;
    expect(parsed["overall_verdict"]).toBe("error");
  });
});

describe("formatGithubComment", () => {
  it("contains markdown table headers", () => {
    const report = makeReport();
    const comment = formatGithubComment(report);
    expect(comment).toContain("| Rule | Verdict | Confidence | Reasoning |");
    expect(comment).toContain("|------|---------|-----------|-----------|");
  });

  it("escapes pipe characters in reasoning", () => {
    const report = makeReport({
      files: [
        makeFileVerdict({
          verdicts: [makeRuleVerdict({ verdict: "fail", rule_severity: "warn", reasoning: "Use a | b instead" })],
        }),
      ],
    });
    const comment = formatGithubComment(report);
    expect(comment).toContain("Use a \\| b instead");
  });

  it("includes details blocks for each file", () => {
    const report = makeReport({
      files: [
        makeFileVerdict({ file_path: "src/a.ts" }),
        makeFileVerdict({ file_path: "src/b.ts" }),
      ],
    });
    const comment = formatGithubComment(report);
    expect(comment).toContain("<details>");
    expect(comment).toContain("src/a.ts");
    expect(comment).toContain("src/b.ts");
  });

  it("shows correct verdict icons for pass and warn-severity fail", () => {
    const report = makeReport({
      files: [
        makeFileVerdict({
          verdicts: [
            makeRuleVerdict({ verdict: "pass", rule_severity: "warn" }),
            makeRuleVerdict({ verdict: "fail", rule_severity: "warn", rule_id: "warn-rule" }),
            makeRuleVerdict({ verdict: "fail", rule_severity: "error", rule_id: "error-rule" }),
          ],
        }),
      ],
    });
    const comment = formatGithubComment(report);
    expect(comment).toContain("✅ pass");
    expect(comment).toContain("⚠️ warn");
    expect(comment).toContain("❌ error");
  });
});

describe("formatStepSummary", () => {
  it("includes stats table with Error column (not Reject)", () => {
    const report = makeReport();
    const summary = formatStepSummary(report);
    expect(summary).toContain("| Pass | Warn | Error | Cached | Duration |");
    expect(summary).toContain("| 1 | 0 | 0 | 0 |");
  });

  it("includes issues table for non-pass reports", () => {
    const report = makeReport({
      files: [
        makeFileVerdict({
          verdicts: [makeRuleVerdict({ verdict: "fail", rule_severity: "warn", reasoning: "potential issue" })],
        }),
      ],
      stats: { total_files: 1, files_with_rules: 1, cache_hits: 0, agentic_escalations: 0, pass_count: 0, warn_count: 1, error_count: 0, duration_ms: 100 },
    });
    const summary = formatStepSummary(report);
    expect(summary).toContain("### Issues");
    expect(summary).toContain("| File | Rule | Verdict | Reasoning |");
  });

  it("truncates reasoning at 120 chars", () => {
    const longReasoning = "A".repeat(150);
    const report = makeReport({
      files: [
        makeFileVerdict({
          verdicts: [makeRuleVerdict({ verdict: "fail", rule_severity: "warn", reasoning: longReasoning })],
        }),
      ],
    });
    const summary = formatStepSummary(report);
    expect(summary).toContain("…");
    expect(summary).not.toContain("A".repeat(150));
  });
});

describe("printReport", () => {
  let tmpDir: string;
  let consoleLogSpy: ReturnType<typeof vi.spyOn>;
  let stdoutWriteSpy: ReturnType<typeof vi.spyOn>;

  beforeEach(() => {
    tmpDir = createTmpDir();
    consoleLogSpy = vi.spyOn(console, "log").mockImplementation(() => {});
    stdoutWriteSpy = vi.spyOn(process.stdout, "write").mockImplementation(() => true);
  });

  afterEach(() => {
    rmSync(tmpDir, { recursive: true, force: true });
    consoleLogSpy.mockRestore();
    stdoutWriteSpy.mockRestore();
  });

  it("shows 'All checks passed.' for all-pass report", () => {
    const report = makeReport();
    printReport(report, false);
    const calls = consoleLogSpy.mock.calls.flat().join("\n");
    expect(calls).toContain("passed");
  });

  it("renders concise output for non-verbose mode with violations", () => {
    const report = makeReport({
      files: [
        makeFileVerdict({
          verdicts: [makeRuleVerdict({ verdict: "fail", rule_severity: "warn", reasoning: "test issue" })],
        }),
      ],
    });
    printReport(report, false);
    expect(consoleLogSpy).toHaveBeenCalled();
    const calls = consoleLogSpy.mock.calls.flat().join("\n");
    expect(calls).toContain("warning");
    expect(calls).toContain("test issue");
  });

  it("renders full output with source context in verbose mode", () => {
    const srcPath = join(tmpDir, "test.ts");
    writeFileSync(srcPath, "const x = 1;\nconst y = 2;\nconst z = 3;\n");

    const report = makeReport({
      files: [
        makeFileVerdict({
          file_path: "test.ts",
          verdicts: [makeRuleVerdict({ verdict: "fail", rule_severity: "warn", reasoning: "issue", line_refs: [2] })],
        }),
      ],
    });
    printReport(report, true, tmpDir);
    expect(stdoutWriteSpy).toHaveBeenCalled();
    const output = stdoutWriteSpy.mock.calls.flat().join("");
    expect(output).toContain("-->");
    expect(output).toContain("test.ts");
  });
});
