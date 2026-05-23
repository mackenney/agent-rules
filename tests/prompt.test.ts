import { describe, it, expect } from "vitest";
import {
  buildFileContext,
  buildRuleSection,
  buildAgenticTask,
  STATELESS_SYSTEM_PROMPT,
} from "../src/prompt.js";
import { FileCheckRequest, Rule } from "../src/schema.js";
import { DEFAULTS } from "../src/config.js";

function makeRequest(overrides?: Partial<FileCheckRequest>): FileCheckRequest {
  return {
    file_path: "src/foo.ts",
    diff: "--- a/src/foo.ts\n+++ b/src/foo.ts\n@@ -1,3 +1,4 @@\n line1\n+added\n line2\n line3",
    content: "line1\nline2\nline3",
    rules: [],
    repo_root: "/repo",
    ...overrides,
  };
}

function makeRule(overrides?: Partial<Rule>): Rule {
  return {
    id: "test/rule",
    name: "Test Rule",
    severity: "warn",
    enabled: true,
    scope: "file",
    context: "stateless",
    prompt: "Check this code.",
    glob_include: [],
    glob_exclude: [],
    examples: [],
    needs_more_context_when: "",
    ...overrides,
  };
}

describe("buildFileContext", () => {
  it("includes FILE header with file path", () => {
    const out = buildFileContext(makeRequest(), makeRule());
    expect(out).toContain("FILE: src/foo.ts");
  });

  it("includes CHANGED LINES section when diff is present", () => {
    const out = buildFileContext(makeRequest(), makeRule());
    expect(out).toContain("CHANGED LINES");
  });

  it("includes FULL FILE CONTENT section when content is present", () => {
    const out = buildFileContext(makeRequest(), makeRule());
    expect(out).toContain("FULL FILE CONTENT");
  });

  it("content null → no FULL FILE CONTENT section", () => {
    const out = buildFileContext(makeRequest({ content: null }), makeRule());
    expect(out).not.toContain("FULL FILE CONTENT");
  });

  it("diff empty string → no CHANGED LINES section", () => {
    const out = buildFileContext(makeRequest({ diff: "" }), makeRule());
    expect(out).not.toContain("CHANGED LINES");
  });

  it("diff undefined → no CHANGED LINES section", () => {
    // FileCheckRequest.diff is a required string, but verify empty behaves correctly
    const out = buildFileContext(makeRequest({ diff: "" }), makeRule());
    expect(out).not.toContain("CHANGED LINES");
  });

  it("content null + diff present → has CHANGED LINES but not FULL FILE CONTENT", () => {
    const out = buildFileContext(makeRequest({ content: null }), makeRule());
    expect(out).toContain("CHANGED LINES");
    expect(out).not.toContain("FULL FILE CONTENT");
  });

  it("truncates content exceeding maxContentChars", () => {
    const hugeLine = "x".repeat(100);
    const content = Array.from({ length: 300 }, () => hugeLine).join("\n");
    expect(content.length).toBeGreaterThan(DEFAULTS.maxContentChars);
    const out = buildFileContext(makeRequest({ content }), makeRule());
    // The output should be much smaller than the untruncated content
    expect(out.length).toBeLessThan(content.length);
  });

  it("truncates diff exceeding maxDiffChars", () => {
    // Each "+line\n" is 6 chars; repeat 2000 = 12000 chars of +lines
    const hugeDiff = "@@ -1,1 +1,1 @@\n" + "+line\n".repeat(2000);
    expect(hugeDiff.length).toBeGreaterThan(DEFAULTS.maxDiffChars);
    const out = buildFileContext(makeRequest({ diff: hugeDiff }), makeRule());
    // Count occurrences of "+line" in the output — if truncated, far fewer than 2000
    const matches = (out.match(/\+line/g) ?? []).length;
    expect(matches).toBeLessThan(2000);
    // The number of +line occurrences should be bounded by maxDiffChars / len("+line\n")
    expect(matches).toBeLessThanOrEqual(Math.ceil(DEFAULTS.maxDiffChars / 6) + 1);
  });

  it("annotates diff lines with absolute line numbers", () => {
    const out = buildFileContext(makeRequest(), makeRule());
    // Lines annotated by annotateDiff will have " | " separating number and content
    expect(out).toMatch(/\d+ \| /);
  });

  it("content with multiple lines → lines numbered in FULL FILE CONTENT", () => {
    const out = buildFileContext(makeRequest({ content: "a\nb\nc" }), makeRule());
    expect(out).toContain("1 | a");
    expect(out).toContain("2 | b");
    expect(out).toContain("3 | c");
  });
});

describe("buildRuleSection", () => {
  it("includes rule name", () => {
    const out = buildRuleSection(makeRule({ name: "MyRule" }));
    expect(out).toContain("MyRule");
  });

  it("includes RULE TO EVALUATE header", () => {
    const out = buildRuleSection(makeRule());
    expect(out).toContain("RULE TO EVALUATE");
  });

  it("includes severity", () => {
    const out = buildRuleSection(makeRule({ severity: "error" }));
    expect(out).toContain("error");
  });

  it("includes prompt instruction", () => {
    const out = buildRuleSection(makeRule({ prompt: "Do not use var." }));
    expect(out).toContain("Do not use var.");
  });

  it("examples appear in output when rule has examples", () => {
    const rule = makeRule({
      examples: [
        { description: "bad usage", code: "var x = 1;", verdict: "fail" },
        { description: "good usage", code: "const x = 1;", verdict: "pass" },
      ],
    });
    const out = buildRuleSection(rule);
    expect(out).toContain("bad usage");
    expect(out).toContain("var x = 1;");
    expect(out).toContain("good usage");
    expect(out).toContain("const x = 1;");
    expect(out).toContain("FAIL");
    expect(out).toContain("PASS");
  });

  it("no examples section when rule has empty examples", () => {
    const out = buildRuleSection(makeRule({ examples: [] }));
    expect(out).not.toContain("examples:");
  });

  it("needs_more_context_when → escalation guidance appears", () => {
    const rule = makeRule({ needs_more_context_when: "When the file imports X." });
    const out = buildRuleSection(rule);
    expect(out).toContain("escalation guidance");
    expect(out).toContain("When the file imports X.");
  });

  it("no escalation guidance when needs_more_context_when is empty", () => {
    const out = buildRuleSection(makeRule({ needs_more_context_when: "" }));
    expect(out).not.toContain("escalation guidance");
  });
});

describe("buildAgenticTask", () => {
  it("includes the file block (FILE: header)", () => {
    const out = buildAgenticTask(makeRequest(), makeRule());
    expect(out).toContain("FILE: src/foo.ts");
  });

  it("includes the rule section", () => {
    const out = buildAgenticTask(makeRequest(), makeRule({ name: "AgenticRule" }));
    expect(out).toContain("AgenticRule");
    expect(out).toContain("RULE TO EVALUATE");
  });

  it("context hints included when hints array has items", () => {
    const hints = [{ read_files: ["src/config.ts"], question: "What config options exist?" }];
    const out = buildAgenticTask(makeRequest(), makeRule(), hints);
    expect(out).toContain("Context hints");
    expect(out).toContain("src/config.ts");
    expect(out).toContain("What config options exist?");
  });

  it("no context hints section when hints is empty", () => {
    const out = buildAgenticTask(makeRequest(), makeRule(), []);
    expect(out).not.toContain("Context hints");
  });

  it("no context hints section when hints is omitted", () => {
    const out = buildAgenticTask(makeRequest(), makeRule());
    expect(out).not.toContain("Context hints");
  });

  it("instructs model not to emit needs-more-context", () => {
    const out = buildAgenticTask(makeRequest(), makeRule());
    expect(out).toContain("needs-more-context");
    expect(out).toContain("terminal verdict");
  });
});

describe("STATELESS_SYSTEM_PROMPT", () => {
  it("is a non-empty string", () => {
    expect(typeof STATELESS_SYSTEM_PROMPT).toBe("string");
    expect(STATELESS_SYSTEM_PROMPT.length).toBeGreaterThan(0);
  });

  it("describes verdict meanings and field guidance", () => {
    expect(STATELESS_SYSTEM_PROMPT).toContain("verdict");
    expect(STATELESS_SYSTEM_PROMPT).toContain("submit_verdict");
    expect(STATELESS_SYSTEM_PROMPT).toContain("confidence");
  });
});
