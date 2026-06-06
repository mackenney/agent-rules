import { describe, it, expect, vi, beforeEach, type MockInstance } from "vitest";
import { parseVerdicts, checkFile, checkPr, DEFAULT_AGENTIC_TIMEOUT_MS } from "../src/runner.js";
import { DEFAULTS } from "../src/config.js";
import type { FileCheckRequest, Rule, RuleVerdict, FileVerdict } from "../src/schema.js";
import { effectiveSeverity, fileVerdictOverall } from "../src/schema.js";

// ─── mocks ───────────────────────────────────────────────────────────────────

// Capture the subscribe callback so tests can fire events into the session
let capturedSubscribe: ((event: unknown) => void) | null = null;
let mockPromptResolve: (() => void) | null = null;

const mockSession = {
  subscribe: vi.fn((cb: (event: unknown) => void) => {
    capturedSubscribe = cb;
    return () => {};
  }),
  prompt: vi.fn(() => new Promise<void>((resolve) => { mockPromptResolve = resolve; })),
  abort: vi.fn(() => Promise.resolve()),
  dispose: vi.fn(),
};

vi.mock("@mariozechner/pi-coding-agent", () => ({
  createAgentSession: vi.fn(() => Promise.resolve({ session: mockSession })),
  SessionManager: { inMemory: vi.fn(() => ({})) },
  createReadOnlyTools: vi.fn(() => []),
  createBashTool: vi.fn(() => ({})),
  AuthStorage: {
    create: vi.fn(() => ({ setRuntimeApiKey: vi.fn() })),
  },
}));

vi.mock("@mariozechner/pi-ai", () => ({
  getModel: vi.fn(() => ({ id: "claude-sonnet-4-5", provider: "anthropic" })),
}));

// ─── helpers ─────────────────────────────────────────────────────────────────

function makeRule(id: string, overrides: Partial<Rule> = {}): Rule {
  return {
    id,
    name: `Rule ${id}`,
    severity: "warn",
    enabled: true,
    scope: "file",
    context: "stateless",
    prompt: `Check for ${id}`,
    glob_include: ["**/*"],
    glob_exclude: [],
    examples: [],
    needs_more_context_when: "",
    ...overrides,
  };
}

function makeRequest(rules: Rule[]): FileCheckRequest {
  return {
    file_path: "src/test.ts",
    diff: "@@ -1 +1 @@\n+const x = 1;",
    content: "const x = 1;",
    rules,
    repo_root: ".",
  };
}

function makeAnthropicClient(verdict: Omit<RuleVerdict, "from_agentic"> | null): unknown {
  const text = verdict
    ? JSON.stringify({
        verdict: verdict.verdict,
        confidence: verdict.confidence,
        reasoning: verdict.reasoning,
        line_refs: verdict.line_refs,
        ...(verdict.context_hint ? { context_hint: verdict.context_hint } : {}),
      })
    : "{}";
  return {
    messages: {
      create: vi.fn(() =>
        Promise.resolve({
          content: [{ type: "text", text }],
          stop_reason: "end_turn",
        })
      ),
    },
  };
}

function makeNullCache() {
  return {
    get: vi.fn(() => null),
    put: vi.fn(),
    keyFor: vi.fn(() => "test-cache-key"),
    stats: vi.fn(() => ({})),
    clear: vi.fn(() => 0),
  };
}

function fireTextDeltas(chunks: string[]) {
  if (!capturedSubscribe) throw new Error("subscribe not called yet");
  for (const chunk of chunks) {
    capturedSubscribe({
      type: "message_update",
      assistantMessageEvent: { type: "text_delta", delta: chunk },
    });
  }
}

// ─── parseVerdicts ────────────────────────────────────────────────────────────

describe("parseVerdicts", () => {
  it("parses valid JSON response", () => {
    const rules = [makeRule("rule-1")];
    const request = makeRequest(rules);
    const rawText = JSON.stringify({
      verdict: "pass", confidence: 1.0, reasoning: "All good", line_refs: [],
    });

    const result = parseVerdicts(rawText, request);
    expect(result).toHaveLength(1);
    expect(result[0]?.rule_id).toBe("rule-1");
    expect(result[0]?.verdict).toBe("pass");
  });

  it("parses line_refs from response", () => {
    const rules = [makeRule("rule-1")];
    const request = makeRequest(rules);
    const rawText = JSON.stringify({
      verdict: "fail", confidence: 0.7, reasoning: "Found an issue", line_refs: [5, 12],
    });

    const result = parseVerdicts(rawText, request);
    expect(result[0]?.verdict).toBe("fail");
    expect(result[0]?.line_refs).toEqual([5, 12]);
  });

  it("strips markdown code fences", () => {
    const rules = [makeRule("rule-1")];
    const request = makeRequest(rules);
    const rawText = `\`\`\`json
{"verdict":"pass","confidence":1.0,"reasoning":"ok","line_refs":[]}
\`\`\``;

    const result = parseVerdicts(rawText, request);
    expect(result).toHaveLength(1);
    expect(result[0]?.verdict).toBe("pass");
  });

  it("returns fail fallback on invalid JSON", () => {
    const rules = [makeRule("rule-1")];
    const request = makeRequest(rules);
    const result = parseVerdicts("this is not json", request);

    expect(result).toHaveLength(1);
    expect(result[0]?.verdict).toBe("fail");
    expect(result[0]?.confidence).toBe(0.0);
    expect(result[0]?.reasoning).toBe("JSON parse error");
  });

  it("falls back to fail for invalid verdict values", () => {
    const rules = [makeRule("rule-1")];
    const request = makeRequest(rules);
    const rawText = JSON.stringify({
      verdict: "invalid-verdict", confidence: 0.5, reasoning: "hmm", line_refs: [],
    });

    const result = parseVerdicts(rawText, request);
    expect(result[0]?.verdict).toBe("fail");
  });

  it("parses context_hint from needs-more-context verdicts", () => {
    const rules = [makeRule("rule-1")];
    const request = makeRequest(rules);
    const rawText = JSON.stringify({
      verdict: "needs-more-context",
      confidence: 0.4,
      reasoning: "Need more info",
      line_refs: [],
      context_hint: {
        read_files: ["src/config.ts"],
        question: "What does this config do?",
      },
    });

    const result = parseVerdicts(rawText, request);
    expect(result[0]?.verdict).toBe("needs-more-context");
    expect(result[0]?.context_hint?.read_files).toEqual(["src/config.ts"]);
    expect(result[0]?.context_hint?.question).toBe("What does this config do?");
  });

  it("sanitizes control characters in JSON strings", () => {
    const rules = [makeRule("rule-1")];
    const request = makeRequest(rules);
    const rawText =
      '{"verdict":"pass","confidence":1.0,"reasoning":"line1\nline2","line_refs":[]}';

    const result = parseVerdicts(rawText, request);
    expect(result).toHaveLength(1);
    expect(result[0]?.verdict).toBe("pass");
  });

  it("trims and normalizes reasoning", () => {
    const rules = [makeRule("rule-1")];
    const request = makeRequest(rules);
    const rawText = JSON.stringify({
      verdict: "fail",
      confidence: 0.8,
      reasoning: "  line1\r\nline2\n  ",
      line_refs: [],
    });

    const result = parseVerdicts(rawText, request);
    expect(result[0]?.reasoning).not.toContain("\n");
    expect(result[0]?.reasoning).not.toContain("\r");
    expect(result[0]?.reasoning).toBe("line1 line2");
  });

  it("extracts JSON from agentic preamble text", () => {
    const rules = [makeRule("rule-1")];
    const request = makeRequest(rules);
    const json = JSON.stringify({
      verdict: "fail", confidence: 0.8, reasoning: "missing check", line_refs: [5],
    });
    const rawText = `After reading the file I found an issue.\n\nHere is my verdict:\n${json}`;

    const result = parseVerdicts(rawText, request);
    expect(result[0]?.verdict).toBe("fail");
    expect(result[0]?.reasoning).toBe("missing check");
  });

  it("extracts JSON when brace has a trailing space (pi agent style)", () => {
    const rules = [makeRule("rule-1")];
    const request = makeRequest(rules);
    const rawText = `Analysis complete.\n\n{ "verdict": "fail", "confidence": 0.9, "reasoning": "bad", "line_refs": [] }`;

    const result = parseVerdicts(rawText, request);
    expect(result[0]?.verdict).toBe("fail");
  });

  it("extracts JSON from markdown fence anywhere in agentic output", () => {
    const rules = [makeRule("rule-1")];
    const request = makeRequest(rules);
    const rawText = [
      "I read the constants file and found the limit.",
      "",
      "```json",
      JSON.stringify({ verdict: "pass", confidence: 1.0, reasoning: "ok", line_refs: [] }),
      "```",
      "",
      "The payment code is compliant.",
    ].join("\n");

    const result = parseVerdicts(rawText, request);
    expect(result[0]?.verdict).toBe("pass");
  });

  it("falls back to fail when JSON has no verdict field", () => {
    const rules = [makeRule("rule-1")];
    const request = makeRequest(rules);
    const rawText = JSON.stringify({ something_else: true });

    const result = parseVerdicts(rawText, request);
    expect(result).toHaveLength(1);
    expect(result[0]?.verdict).toBe("fail");
  });
});

// ─── checkFile routing ────────────────────────────────────────────────────────

describe("checkFile routing", () => {
  let createAgentSession: MockInstance;

  beforeEach(async () => {
    vi.clearAllMocks();
    capturedSubscribe = null;
    mockPromptResolve = null;
    // Set ANTHROPIC_API_KEY so runAgenticEscalation doesn't throw before mocked createAgentSession
    process.env["ANTHROPIC_API_KEY"] = "test-key";
    // Reset prompt to return a fresh controllable promise each test
    mockSession.prompt.mockImplementation(
      () => new Promise<void>((resolve) => { mockPromptResolve = resolve; })
    );
    const piMod = await import("@mariozechner/pi-coding-agent");
    createAgentSession = piMod.createAgentSession as unknown as MockInstance;
  });

  it("stateless-only: no pi session when all verdicts are terminal", async () => {
    const rule = makeRule("sec/no-secret", { context: "stateless" });
    const request = makeRequest([rule]);
    const client = makeAnthropicClient(
      { rule_id: "sec/no-secret", verdict: "pass", rule_severity: "warn", confidence: 1.0, reasoning: "clean", line_refs: [], context_hint: null },
    );

    const result = await checkFile(request, { client: client as never, cache: makeNullCache() });

    expect(createAgentSession).not.toHaveBeenCalled();
    expect(result.agentic_escalations).toBe(0);
    expect(result.verdicts[0]?.verdict).toBe("pass");
    expect(result.verdicts[0]?.from_agentic).toBe(false);
  });

  it("needs-more-context on stateless rule collapses to fail, no pi session", async () => {
    const rule = makeRule("sec/check", { context: "stateless" });
    const request = makeRequest([rule]);
    const client = makeAnthropicClient(
      { rule_id: "sec/check", verdict: "needs-more-context", rule_severity: "warn", confidence: 0.3, reasoning: "unclear", line_refs: [], context_hint: null },
    );

    const result = await checkFile(request, { client: client as never, cache: makeNullCache() });

    expect(createAgentSession).not.toHaveBeenCalled();
    expect(result.agentic_escalations).toBe(0);
    expect(result.verdicts[0]?.verdict).toBe("fail");
    expect(result.verdicts[0]?.reasoning).toContain("collapsed from needs-more-context");
  });

  it("needs-more-context on agentic rule triggers pi session", async () => {
    const rule = makeRule("arch/check-deps", { context: "agentic" });
    const request = makeRequest([rule]);
    const client = makeAnthropicClient(
      { rule_id: "arch/check-deps", verdict: "needs-more-context", rule_severity: "warn", confidence: 0.4,
        reasoning: "need to see imports", line_refs: [], context_hint: null },
    );

    const agenticJson = JSON.stringify({
      verdict: "fail", confidence: 0.8, reasoning: "missing dep injection", line_refs: [3],
    });

    // Start checkFile — it will pause waiting for the pi session prompt to resolve
    const checkPromise = checkFile(
      request,
      { client: client as never, cache: makeNullCache() },
      {
        agentic: {
          timeoutMs: DEFAULT_AGENTIC_TIMEOUT_MS,
          allowBash: false,
          model: DEFAULTS.agenticModel,
        },
      }
    );

    // Let createAgentSession and subscribe happen
    await vi.waitFor(() => expect(capturedSubscribe).not.toBeNull());

    // Fire the agentic verdict as text deltas, then resolve prompt
    fireTextDeltas([agenticJson]);
    mockPromptResolve!();

    const result = await checkPromise;

    expect(createAgentSession).toHaveBeenCalledOnce();
    expect(result.agentic_escalations).toBe(1);
    expect(result.verdicts[0]?.verdict).toBe("fail");
    expect(result.verdicts[0]?.from_agentic).toBe(true);
    expect(result.verdicts[0]?.reasoning).toBe("missing dep injection");
  });

  it("terminal stateless verdicts are not escalated even with agentic rule", async () => {
    const rule = makeRule("arch/check", { context: "agentic" });
    const request = makeRequest([rule]);
    const client = makeAnthropicClient(
      { rule_id: "arch/check", verdict: "fail", rule_severity: "error", confidence: 0.95, reasoning: "clear violation",
        line_refs: [5], context_hint: null },
    );

    const result = await checkFile(request, { client: client as never, cache: makeNullCache() });

    expect(createAgentSession).not.toHaveBeenCalled();
    expect(result.agentic_escalations).toBe(0);
    expect(result.verdicts[0]?.verdict).toBe("fail");
    expect(result.verdicts[0]?.from_agentic).toBe(false);
  });

  it("pi session timeout falls back to fail", async () => {
    const rule = makeRule("arch/check-deps", { context: "agentic" });
    const request = makeRequest([rule]);
    const client = makeAnthropicClient(
      { rule_id: "arch/check-deps", verdict: "needs-more-context", rule_severity: "warn", confidence: 0.4,
        reasoning: "need files", line_refs: [], context_hint: null },
    );

    // session.prompt never resolves — timeout will fire
    mockSession.prompt.mockImplementation(() => new Promise(() => {}));

    const result = await checkFile(
      request,
      { client: client as never, cache: makeNullCache() },
      { agentic: { timeoutMs: 50, allowBash: false, model: DEFAULTS.agenticModel } }
    );

    expect(createAgentSession).toHaveBeenCalledOnce();
    expect(mockSession.abort).toHaveBeenCalled();
    expect(result.verdicts[0]?.verdict).toBe("fail");
    expect(result.verdicts[0]?.from_agentic).toBe(true);
  });

  it("result is served from cache on second call", async () => {
    const rule = makeRule("sec/check");
    const request = makeRequest([rule]);
    const client = makeAnthropicClient(null);

    const cachedVerdict = {
      file_path: request.file_path,
      verdicts: [{ rule_id: "sec/check", verdict: "pass" as const, rule_severity: "warn" as const,
        confidence: 1.0, reasoning: "cached", line_refs: [], context_hint: null, from_agentic: false }],
      cached: true,
      check_duration_ms: 1,
      agentic_escalations: 0,
    };

    const cache = {
      get: vi.fn(() => cachedVerdict),
      put: vi.fn(),
      keyFor: vi.fn(() => "test-cache-key"),
      stats: vi.fn(() => ({})),
      clear: vi.fn(() => 0),
    };

    const result = await checkFile(request, { client: client as never, cache });

    expect((client as { messages: { create: MockInstance } }).messages.create).not.toHaveBeenCalled();
    expect(result.cached).toBe(true);
    expect(result.verdicts[0]?.reasoning).toBe("cached");
  });
});

// ─── effectiveSeverity ────────────────────────────────────────────────────────────────────────

function makeRuleVerdict(overrides: Partial<RuleVerdict> = {}): RuleVerdict {
  return {
    rule_id: "test",
    verdict: "pass",
    rule_severity: "warn",
    confidence: 1.0,
    reasoning: "",
    line_refs: [],
    context_hint: null,
    from_agentic: false,
    ...overrides,
  };
}

function makeFileVerdict(overrides: Partial<FileVerdict> = {}): FileVerdict {
  return {
    file_path: "test.ts",
    verdicts: [makeRuleVerdict()],
    cached: false,
    check_duration_ms: 100,
    agentic_escalations: 0,
    ...overrides,
  };
}

describe("effectiveSeverity", () => {
  it("returns pass for pass verdict", () => {
    expect(effectiveSeverity(makeRuleVerdict({ verdict: "pass", rule_severity: "error" }))).toBe("pass");
  });

  it("returns warn for fail verdict with warn severity", () => {
    expect(effectiveSeverity(makeRuleVerdict({ verdict: "fail", rule_severity: "warn" }))).toBe("warn");
  });

  it("returns error for fail verdict with error severity", () => {
    expect(effectiveSeverity(makeRuleVerdict({ verdict: "fail", rule_severity: "error" }))).toBe("error");
  });
});

describe("fileVerdictOverall", () => {
  it("returns pass when all verdicts are pass", () => {
    const fv = makeFileVerdict({
      verdicts: [
        makeRuleVerdict({ verdict: "pass", rule_severity: "warn" }),
        makeRuleVerdict({ verdict: "pass", rule_severity: "error" }),
      ],
    });
    expect(fileVerdictOverall(fv)).toBe("pass");
  });

  it("returns warn when worst is fail+warn", () => {
    const fv = makeFileVerdict({
      verdicts: [
        makeRuleVerdict({ verdict: "pass", rule_severity: "warn" }),
        makeRuleVerdict({ verdict: "fail", rule_severity: "warn" }),
      ],
    });
    expect(fileVerdictOverall(fv)).toBe("warn");
  });

  it("returns error when any verdict is fail+error", () => {
    const fv = makeFileVerdict({
      verdicts: [
        makeRuleVerdict({ verdict: "fail", rule_severity: "warn" }),
        makeRuleVerdict({ verdict: "fail", rule_severity: "error" }),
      ],
    });
    expect(fileVerdictOverall(fv)).toBe("error");
  });
});

// ─── checkPr ─────────────────────────────────────────────────────────────────────────────

describe("checkPr", () => {
  beforeEach(async () => {
    vi.clearAllMocks();
    capturedSubscribe = null;
    mockPromptResolve = null;
    process.env["ANTHROPIC_API_KEY"] = "test-key";
    mockSession.prompt.mockImplementation(
      () => new Promise<void>((resolve) => { mockPromptResolve = resolve; })
    );
  });

  function makeFileRequest(filePath: string, rules: Rule[]): FileCheckRequest {
    return {
      file_path: filePath,
      diff: "@@ -1 +1 @@\n+const x = 1;",
      content: "const x = 1;",
      rules,
      repo_root: ".",
    };
  }

  it("returns correct PRReport structure for single file single rule", async () => {
    const rule = makeRule("test/rule");
    const files = [makeFileRequest("src/test.ts", [rule])];
    const client = makeAnthropicClient({
      rule_id: "test/rule",
      verdict: "pass",
      rule_severity: "warn",
      confidence: 1.0,
      reasoning: "ok",
      line_refs: [],
      context_hint: null,
    });

    const report = await checkPr(files, { client: client as never, cache: makeNullCache() });

    expect(report.files).toHaveLength(1);
    expect(report.files[0]?.file_path).toBe("src/test.ts");
    expect(report.files[0]?.verdicts).toHaveLength(1);
    expect(report.files[0]?.verdicts[0]?.verdict).toBe("pass");
    expect(report.stats.total_files).toBe(1);
    expect(report.stats.pass_count).toBe(1);
  });

  it("aggregates verdicts from multiple files correctly", async () => {
    const ruleA = makeRule("rule-a");
    const ruleB = makeRule("rule-b");
    const files = [
      makeFileRequest("src/a.ts", [ruleA]),
      makeFileRequest("src/b.ts", [ruleB]),
    ];

    let callCount = 0;
    const client = {
      messages: {
        create: vi.fn(() => {
          callCount++;
          const verdict = callCount === 1 ? "pass" : "fail";
          return Promise.resolve({
            content: [{
              type: "text",
              text: JSON.stringify({ verdict, confidence: 0.9, reasoning: `verdict ${callCount}`, line_refs: [] }),
            }],
            stop_reason: "end_turn",
          });
        }),
      },
    };

    const report = await checkPr(files, { client: client as never, cache: makeNullCache() });

    expect(report.files).toHaveLength(2);
    expect(report.stats.pass_count).toBe(1);
    expect(report.stats.warn_count).toBe(1);
  });

  it("computes stats correctly for mixed verdicts", async () => {
    const rules = [makeRule("rule-a"), makeRule("rule-b"), makeRule("rule-c")];
    const files = [
      makeFileRequest("src/pass.ts", [rules[0]!]),
      makeFileRequest("src/warn.ts", [rules[1]!]),
      makeFileRequest("src/error.ts", [rules[2]!]),
    ];

    let callCount = 0;
    const verdictSequence = ["pass", "fail", "fail"];
    const client = {
      messages: {
        create: vi.fn(() => {
          const verdict = verdictSequence[callCount++] ?? "pass";
          return Promise.resolve({
            content: [{
              type: "text",
              text: JSON.stringify({ verdict, confidence: 0.9, reasoning: "test", line_refs: [] }),
            }],
            stop_reason: "end_turn",
          });
        }),
      },
    };

    const report = await checkPr(files, { client: client as never, cache: makeNullCache() });

    expect(report.stats.total_files).toBe(3);
    expect(report.stats.pass_count).toBe(1);
    expect(report.stats.warn_count).toBe(2);
    expect(report.stats.error_count).toBe(0);
  });

  it("correctly sets cached flag per file", async () => {
    const rule = makeRule("test/rule");
    const files = [
      makeFileRequest("src/cached.ts", [rule]),
      makeFileRequest("src/uncached.ts", [rule]),
    ];

    let keyCallCount = 0;
    const cachedVerdict: FileVerdict = {
      file_path: "src/cached.ts",
      verdicts: [makeRuleVerdict({ rule_id: "test/rule", verdict: "pass" })],
      cached: true,
      check_duration_ms: 1,
      agentic_escalations: 0,
    };

    const cache = {
      get: vi.fn(() => keyCallCount === 1 ? cachedVerdict : null),
      put: vi.fn(),
      keyFor: vi.fn(() => `key-${keyCallCount++}`),
      stats: vi.fn(() => ({})),
      clear: vi.fn(() => 0),
    };

    const client = makeAnthropicClient({
      rule_id: "test/rule",
      verdict: "pass",
      rule_severity: "warn",
      confidence: 1.0,
      reasoning: "fresh",
      line_refs: [],
      context_hint: null,
    });

    const report = await checkPr(files, { client: client as never, cache });

    const cachedFile = report.files.find((f) => f.file_path === "src/cached.ts");
    const uncachedFile = report.files.find((f) => f.file_path === "src/uncached.ts");
    expect(cachedFile?.cached).toBe(true);
    expect(uncachedFile?.cached).toBe(false);
    expect(report.stats.cache_hits).toBe(1);
  });

  it("sets cacheSystemPrompt true when multiple rule tasks exist", async () => {
    const rule1 = makeRule("rule-1");
    const rule2 = makeRule("rule-2");
    const files = [makeFileRequest("src/multi.ts", [rule1, rule2])];

    const createSpy = vi.fn(() =>
      Promise.resolve({
        content: [{ type: "text", text: JSON.stringify({ verdict: "pass", confidence: 1.0, reasoning: "ok", line_refs: [] }) }],
        stop_reason: "end_turn",
      })
    );
    const client = { messages: { create: createSpy } };

    await checkPr(files, { client: client as never, cache: makeNullCache() });

    expect(createSpy).toHaveBeenCalledTimes(2);
    const firstCall = createSpy.mock.calls[0]?.[0] as { system?: unknown };
    expect(Array.isArray(firstCall?.system)).toBe(true);
  });
});
