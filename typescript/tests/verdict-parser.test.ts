import { describe, it, expect } from "vitest";
import { parseVerdicts } from "../src/verdict-parser.js";
import { FileCheckRequest, Rule } from "../src/schema.js";

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

function makeRequest(overrides?: Partial<FileCheckRequest>): FileCheckRequest {
  return {
    file_path: "src/foo.ts",
    diff: "@@ -1 +1 @@\n-old\n+new",
    content: "const x = 1;",
    rules: [makeRule()],
    repo_root: "/repo",
    ...overrides,
  };
}

describe("parseVerdicts", () => {
  it("parses a valid JSON pass verdict", () => {
    const raw = JSON.stringify({
      reasoning: "All good.",
      line_refs: [],
      confidence: 0.9,
      verdict: "pass",
    });
    const result = parseVerdicts(raw, makeRequest());
    expect(result).toHaveLength(1);
    expect(result[0]!.verdict).toBe("pass");
    expect(result[0]!.rule_id).toBe("test/rule");
    expect(result[0]!.confidence).toBe(0.9);
    expect(result[0]!.reasoning).toBe("All good.");
    expect(result[0]!.line_refs).toEqual([]);
    expect(result[0]!.context_hint).toBeNull();
    expect(result[0]!.from_agentic).toBe(false);
  });

  it("parses a fail verdict", () => {
    const raw = JSON.stringify({ reasoning: "Bad.", line_refs: [3], confidence: 0.6, verdict: "fail" });
    const result = parseVerdicts(raw, makeRequest());
    expect(result[0]!.verdict).toBe("fail");
    expect(result[0]!.line_refs).toEqual([3]);
  });

  it("parses a fail verdict with high confidence", () => {
    const raw = JSON.stringify({ reasoning: "Bad code.", line_refs: [5, 6], confidence: 1.0, verdict: "fail" });
    const result = parseVerdicts(raw, makeRequest());
    expect(result[0]!.verdict).toBe("fail");
  });

  it("strips markdown code fences (```json ... ```)", () => {
    const inner = JSON.stringify({ reasoning: "Fine.", line_refs: [], confidence: 0.8, verdict: "pass" });
    const raw = "```json\n" + inner + "\n```";
    const result = parseVerdicts(raw, makeRequest());
    expect(result[0]!.verdict).toBe("pass");
  });

  it("strips plain markdown fences (``` ... ```)", () => {
    const inner = JSON.stringify({ reasoning: "OK.", line_refs: [], confidence: 0.8, verdict: "pass" });
    const raw = "```\n" + inner + "\n```";
    const result = parseVerdicts(raw, makeRequest());
    expect(result[0]!.verdict).toBe("pass");
  });

  it("extracts JSON from text with extra preamble before the object", () => {
    const obj = JSON.stringify({ reasoning: "Preamble test.", line_refs: [], confidence: 0.7, verdict: "fail" });
    const raw = "Here is my analysis:\n\nSome reasoning text.\n" + obj;
    const result = parseVerdicts(raw, makeRequest());
    expect(result[0]!.verdict).toBe("fail");
    expect(result[0]!.reasoning).toBe("Preamble test.");
  });

  it("parses needs-more-context with context_hint", () => {
    const raw = JSON.stringify({
      reasoning: "Need more info.",
      line_refs: [],
      confidence: 0.4,
      verdict: "needs-more-context",
      context_hint: {
        read_files: ["src/config.ts"],
        question: "What config options exist?",
      },
    });
    const result = parseVerdicts(raw, makeRequest());
    expect(result[0]!.verdict).toBe("needs-more-context");
    expect(result[0]!.context_hint).not.toBeNull();
    expect(result[0]!.context_hint!.read_files).toEqual(["src/config.ts"]);
    expect(result[0]!.context_hint!.question).toBe("What config options exist?");
  });

  it("returns fallback fail verdict for unknown verdict string", () => {
    const raw = JSON.stringify({ reasoning: "Hmm.", line_refs: [], confidence: 0.5, verdict: "unknown-verdict" });
    const result = parseVerdicts(raw, makeRequest());
    expect(result[0]!.verdict).toBe("fail");
    expect(result[0]!.confidence).toBe(0.0);
    expect(result[0]!.reasoning).toBe("Model returned unrecognised verdict");
  });

  it("returns fallback fail verdict for invalid JSON", () => {
    const result = parseVerdicts("not valid json at all", makeRequest());
    expect(result[0]!.verdict).toBe("fail");
    expect(result[0]!.confidence).toBe(0.0);
    expect(result[0]!.reasoning).toBe("JSON parse error");
  });

  it("returns fallback fail verdict for empty text", () => {
    const result = parseVerdicts("", makeRequest());
    expect(result[0]!.verdict).toBe("fail");
    expect(result[0]!.confidence).toBe(0.0);
  });

  it("context_hint: null in JSON → context_hint is null in result", () => {
    const raw = JSON.stringify({
      reasoning: "Clear.",
      line_refs: [],
      confidence: 0.9,
      verdict: "pass",
      context_hint: null,
    });
    const result = parseVerdicts(raw, makeRequest());
    expect(result[0]!.context_hint).toBeNull();
  });

  it("uses rule_id from the first rule in the request", () => {
    const req = makeRequest({ rules: [makeRule({ id: "custom/rule-id" })] });
    const raw = JSON.stringify({ reasoning: "OK.", line_refs: [], confidence: 0.8, verdict: "pass" });
    const result = parseVerdicts(raw, req);
    expect(result[0]!.rule_id).toBe("custom/rule-id");
  });

  it("strips newlines from reasoning", () => {
    const raw = JSON.stringify({
      reasoning: "Line one.\nLine two.",
      line_refs: [],
      confidence: 0.8,
      verdict: "pass",
    });
    const result = parseVerdicts(raw, makeRequest());
    expect(result[0]!.reasoning).not.toContain("\n");
  });

  it("defaults confidence to 0.5 when not a number", () => {
    const raw = JSON.stringify({
      reasoning: "OK.",
      line_refs: [],
      confidence: "high",
      verdict: "pass",
    });
    const result = parseVerdicts(raw, makeRequest());
    expect(result[0]!.confidence).toBe(0.5);
  });

  it("defaults line_refs to [] when not an array", () => {
    const raw = JSON.stringify({
      reasoning: "OK.",
      line_refs: null,
      confidence: 0.8,
      verdict: "pass",
    });
    const result = parseVerdicts(raw, makeRequest());
    expect(result[0]!.line_refs).toEqual([]);
  });

  it("extracts JSON from markdown fence embedded mid-text (fallback path)", () => {
    const inner = JSON.stringify({ reasoning: "Mid fence.", line_refs: [], confidence: 0.7, verdict: "fail" });
    const raw = "Some analysis text.\n\n```json\n" + inner + "\n```\n\nMore text after.";
    const result = parseVerdicts(raw, makeRequest());
    expect(result[0]!.verdict).toBe("fail");
  });
});
