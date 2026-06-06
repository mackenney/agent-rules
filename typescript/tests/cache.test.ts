import { describe, it, expect, beforeEach, afterEach } from "vitest";
import { mkdirSync, rmSync } from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";
import { CacheManager, NullCache } from "../src/cache.js";
import { FileCheckRequest, FileVerdict } from "../src/schema.js";

function createTmpDir(): string {
  const dir = join(
    tmpdir(),
    `agent-rules-cache-test-${Date.now()}-${Math.random().toString(36).slice(2)}`
  );
  mkdirSync(dir, { recursive: true });
  return dir;
}

function makeRequest(overrides: Partial<FileCheckRequest> = {}): FileCheckRequest {
  return {
    file_path: "src/foo.ts",
    diff: "@@ -1 +1 @@\n-old\n+new",
    content: "const x = 1;",
    rules: [
      {
        id: "rule-1",
        name: "Test Rule",
        severity: "warn",
        enabled: true,
        scope: "file",
        context: "stateless",
        prompt: "Check for issues",
        glob_include: ["**/*"],
        glob_exclude: [],
        examples: [],
        needs_more_context_when: "",
      },
    ],
    repo_root: ".",
    ...overrides,
  };
}

function makeVerdict(filePath: string = "src/foo.ts"): FileVerdict {
  return {
    file_path: filePath,
    verdicts: [
      {
        rule_id: "rule-1",
        verdict: "pass",
        confidence: 1.0,
        reasoning: "Looks good",
        line_refs: [],
        context_hint: null,
        from_agentic: false,
      },
    ],
    cached: false,
    check_duration_ms: 100,
    agentic_escalations: 0,
  };
}

describe("CacheManager", () => {
  let tmpDir: string;
  let cacheDir: string;

  beforeEach(() => {
    tmpDir = createTmpDir();
    cacheDir = join(tmpDir, "cache");
  });

  afterEach(() => {
    rmSync(tmpDir, { recursive: true, force: true });
  });

  it("returns null on cache miss", () => {
    const cache = new CacheManager(cacheDir);
    const result = cache.get("nonexistent-key");
    expect(result).toBeNull();
  });

  it("stores and retrieves a verdict", () => {
    const cache = new CacheManager(cacheDir);
    const verdict = makeVerdict();
    cache.put("test-key", verdict, "claude-haiku");
    const retrieved = cache.get("test-key");
    expect(retrieved).not.toBeNull();
    expect(retrieved?.file_path).toBe("src/foo.ts");
    expect(retrieved?.cached).toBe(true);
  });

  it("marks retrieved verdicts as cached", () => {
    const cache = new CacheManager(cacheDir);
    const verdict = makeVerdict();
    cache.put("test-key", verdict, "claude-haiku");
    const retrieved = cache.get("test-key");
    expect(retrieved?.cached).toBe(true);
  });

  it("clears all entries", () => {
    const cache = new CacheManager(cacheDir);
    cache.put("key-1", makeVerdict("file1.ts"), "model");
    cache.put("key-2", makeVerdict("file2.ts"), "model");
    const count = cache.clear();
    expect(count).toBe(2);
    expect(cache.get("key-1")).toBeNull();
    expect(cache.get("key-2")).toBeNull();
  });

  it("returns stats", () => {
    const cache = new CacheManager(cacheDir);
    cache.put("key-1", makeVerdict(), "model");
    const stats = cache.stats();
    expect(stats.totalEntries).toBe(1);
    expect(stats.cachePath).toBe(cacheDir);
  });

  it("updates hit count on repeated gets", () => {
    const cache = new CacheManager(cacheDir);
    cache.put("hit-key", makeVerdict(), "model");
    cache.get("hit-key");
    cache.get("hit-key");
    const stats = cache.stats();
    expect(stats.totalHits).toBeGreaterThanOrEqual(2);
  });
});

describe("deriveKey", () => {
  const keyFor = (req: Parameters<typeof makeRequest>[0] = {}) =>
    new NullCache().keyFor(makeRequest(req));

  it("produces same key for identical requests", () => {
    const req = makeRequest();
    const cache = new NullCache();
    expect(cache.keyFor(req)).toBe(cache.keyFor(req));
  });

  it("produces different key when diff changes", () => {
    expect(keyFor({ diff: "@@ -1 +1 @@\n-old\n+new" })).not.toBe(
      keyFor({ diff: "@@ -1 +1 @@\n-old\n+different" })
    );
  });

  it("produces different key when content changes", () => {
    expect(keyFor({ content: "const x = 1;" })).not.toBe(
      keyFor({ content: "const x = 2;" })
    );
  });

  it("produces different key when rule prompt changes", () => {
    const baseReq = makeRequest();
    const modifiedRules = [{ ...baseReq.rules[0]!, prompt: "Different prompt" }];
    const req2 = makeRequest({ rules: modifiedRules });
    expect(new NullCache().keyFor(baseReq)).not.toBe(new NullCache().keyFor(req2));
  });

  it("is order-independent for rules", () => {
    const rule1 = {
      id: "a",
      name: "A",
      severity: "warn" as const,
      enabled: true,
      scope: "file" as const,
      context: "stateless" as const,
      prompt: "Prompt A",
      glob_include: ["**/*"],
      glob_exclude: [],
      examples: [],
      needs_more_context_when: "",
    };
    const rule2 = { ...rule1, id: "b", name: "B", prompt: "Prompt B" };
    const req1 = makeRequest({ rules: [rule1, rule2] });
    const req2 = makeRequest({ rules: [rule2, rule1] });
    const cache = new NullCache();
    expect(cache.keyFor(req1)).toBe(cache.keyFor(req2));
  });
});

describe("NullCache", () => {
  it("always returns null on get", () => {
    const cache = new NullCache();
    expect(cache.get("any-key")).toBeNull();
  });

  it("put is a no-op", () => {
    const cache = new NullCache();
    cache.put("key", makeVerdict());
    expect(cache.get("key")).toBeNull();
  });

  it("clear returns 0", () => {
    const cache = new NullCache();
    expect(cache.clear()).toBe(0);
  });

  it("stats returns typed zeros", () => {
    const cache = new NullCache();
    const stats = cache.stats();
    expect(stats.totalEntries).toBe(0);
    expect(stats.cachePath).toBeNull();
  });
});
