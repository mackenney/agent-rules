import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import type { FileCheckRequest, Rule } from "../src/schema.js";
import { DEFAULTS } from "../src/config.js";

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

function makeOkResponse(verdict = "pass") {
  return {
    content: [{ type: "text", text: JSON.stringify({ verdict, confidence: 1, reasoning: "ok", line_refs: [] }) }],
    stop_reason: "end_turn",
  };
}

describe("runStateless retry logic", () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("retries on 429 rate limit up to MAX_RETRIES times and succeeds", async () => {
    const createFn = vi.fn()
      .mockRejectedValueOnce(Object.assign(new Error("Rate limited"), { status: 429 }))
      .mockRejectedValueOnce(Object.assign(new Error("Rate limited"), { status: 429 }))
      .mockResolvedValue(makeOkResponse());

    const client = { messages: { create: createFn } };
    const { runStateless } = await import("../src/llm.js");
    const request = makeRequest([makeRule("test")]);

    const resultPromise = runStateless(client as never, request, DEFAULTS.statelessModel, 60000);

    await vi.advanceTimersByTimeAsync(1000);
    await vi.advanceTimersByTimeAsync(2000);

    const result = await resultPromise;
    expect(createFn).toHaveBeenCalledTimes(3);
    expect(result[0]?.verdict).toBe("pass");
  });

  it("retries on 5xx server errors", async () => {
    const createFn = vi.fn()
      .mockRejectedValueOnce(Object.assign(new Error("Server error"), { status: 500 }))
      .mockResolvedValue(makeOkResponse());

    const client = { messages: { create: createFn } };
    const { runStateless } = await import("../src/llm.js");
    const request = makeRequest([makeRule("test")]);

    const resultPromise = runStateless(client as never, request, DEFAULTS.statelessModel, 60000);
    await vi.advanceTimersByTimeAsync(1000);

    const result = await resultPromise;
    expect(createFn).toHaveBeenCalledTimes(2);
    expect(result[0]?.verdict).toBe("pass");
  });

  it("throws immediately on 403 auth error (no retry)", async () => {
    const createFn = vi.fn()
      .mockRejectedValue(Object.assign(new Error("Unauthorized"), { status: 403 }));

    const client = { messages: { create: createFn } };
    const { runStateless } = await import("../src/llm.js");
    const request = makeRequest([makeRule("test")]);

    await expect(runStateless(client as never, request, DEFAULTS.statelessModel, 60000))
      .rejects.toThrow("Unauthorized");

    expect(createFn).toHaveBeenCalledTimes(1);
  });

  it("throws immediately on 400 bad request (no retry)", async () => {
    const createFn = vi.fn()
      .mockRejectedValue(Object.assign(new Error("Bad request"), { status: 400 }));

    const client = { messages: { create: createFn } };
    const { runStateless } = await import("../src/llm.js");
    const request = makeRequest([makeRule("test")]);

    await expect(runStateless(client as never, request, DEFAULTS.statelessModel, 60000))
      .rejects.toThrow("Bad request");

    expect(createFn).toHaveBeenCalledTimes(1);
  });

  it("returns fallback fail verdict after all retries exhausted", async () => {
    const createFn = vi.fn()
      .mockRejectedValue(Object.assign(new Error("Rate limited"), { status: 429 }));

    const client = { messages: { create: createFn } };
    const { runStateless } = await import("../src/llm.js");
    const request = makeRequest([makeRule("test-rule")]);

    const resultPromise = runStateless(client as never, request, DEFAULTS.statelessModel, 60000);

    await vi.advanceTimersByTimeAsync(1000);
    await vi.advanceTimersByTimeAsync(2000);
    await vi.advanceTimersByTimeAsync(4000);

    const result = await resultPromise;
    expect(createFn).toHaveBeenCalledTimes(3);
    expect(result).toHaveLength(1);
    expect(result[0]?.verdict).toBe("fail");
    expect(result[0]?.confidence).toBe(0.0);
    expect(result[0]?.reasoning).toBe("LLM call failed");
    expect(result[0]?.rule_id).toBe("test-rule");
  });

  it("propagates rule_severity to fallback verdict", async () => {
    const createFn = vi.fn()
      .mockRejectedValue(Object.assign(new Error("Rate limited"), { status: 429 }));

    const client = { messages: { create: createFn } };
    const { runStateless } = await import("../src/llm.js");
    const request = makeRequest([makeRule("error-rule", { severity: "error" })]);

    const resultPromise = runStateless(client as never, request, DEFAULTS.statelessModel, 60000);

    await vi.advanceTimersByTimeAsync(1000);
    await vi.advanceTimersByTimeAsync(2000);
    await vi.advanceTimersByTimeAsync(4000);

    const result = await resultPromise;
    expect(result[0]?.rule_severity).toBe("error");
  });

  it("returns pass verdict when LLM responds with pass", async () => {
    const createFn = vi.fn().mockResolvedValue(makeOkResponse("pass"));
    const client = { messages: { create: createFn } };
    const { runStateless } = await import("../src/llm.js");
    const request = makeRequest([makeRule("test")]);

    const result = await runStateless(client as never, request, DEFAULTS.statelessModel, 60000);

    expect(result).toHaveLength(1);
    expect(result[0]?.verdict).toBe("pass");
    expect(result[0]?.confidence).toBe(1);
    expect(createFn).toHaveBeenCalledTimes(1);
  });

  it("uses exponential backoff between retries", async () => {
    const delays: number[] = [];
    const originalSetTimeout = globalThis.setTimeout.bind(globalThis);
    vi.spyOn(globalThis, "setTimeout").mockImplementation((fn, delay, ...args) => {
      if (typeof delay === "number" && delay > 0) delays.push(delay);
      return originalSetTimeout(fn as TimerHandler, 0, ...args);
    });

    const createFn = vi.fn()
      .mockRejectedValueOnce(Object.assign(new Error("Rate limited"), { status: 429 }))
      .mockRejectedValueOnce(Object.assign(new Error("Rate limited"), { status: 429 }))
      .mockResolvedValue(makeOkResponse());

    const client = { messages: { create: createFn } };
    const { runStateless } = await import("../src/llm.js");
    const request = makeRequest([makeRule("test")]);

    const resultPromise = runStateless(client as never, request, DEFAULTS.statelessModel, 60000);
    await vi.advanceTimersByTimeAsync(10000);
    await resultPromise;

    expect(delays.some((d) => d >= 1000)).toBe(true);
    expect(delays.some((d) => d >= 2000)).toBe(true);
  });
});
