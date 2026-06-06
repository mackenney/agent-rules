/**
 * Stateless integration tests — real Anthropic API, no mocks.
 *
 * These tests call the LLM directly through checkFile and assert that the
 * model produces the expected verdicts for known violations. They are the
 * end-to-end proof that the stateless path works correctly.
 *
 * Skipped when ANTHROPIC_API_KEY is not set.
 */

import { describe, it, expect } from "vitest";
import Anthropic from "@anthropic-ai/sdk";
import { resolve } from "node:path";
import { checkFile, type CheckInfra } from "../../src/runner.js";
import { resolveRules } from "../../src/resolver.js";
import type { FileCheckRequest } from "../../src/schema.js";
import { getChangedFiles } from "../../src/git.js";

const apiKey = process.env["ANTHROPIC_API_KEY"] ?? "";
const TEST_REPO = resolve(new URL(".", import.meta.url).pathname, "../../../test-repo");

const runIf = (cond: boolean) => cond ? it : it.skip;
const hasKey = Boolean(apiKey);

function makeNullCache() {
  return {
    keyFor: () => "null-cache-key",
    get: () => null as never,
    put: () => {},
    stats: () => ({}),
    clear: () => 0,
  };
}

describe("stateless integration — real API", () => {
  runIf(hasKey)("detects hardcoded secret in payment_controller.py", async () => {
    const rules = resolveRules("src/api/payment_controller.py", TEST_REPO)
      .filter((r) => r.id === "security/no-hardcoded-secrets");
    expect(rules).toHaveLength(1);

    const diffs = getChangedFiles("main", "feature/add-payment-api", TEST_REPO);
    const fd = diffs.find((f) => f.path.includes("payment_controller"));
    expect(fd).toBeDefined();

    const request: FileCheckRequest = {
      file_path: fd!.path,
      diff: fd!.diff,
      content: fd!.content,
      rules,
      repo_root: TEST_REPO,
    };

    const client = new Anthropic({ apiKey });
    const result = await checkFile(
      request,
      { client, cache: makeNullCache() },
      { model: "claude-haiku-4-5" },
    );

    const verdict = result.verdicts.find((v) => v.rule_id === "security/no-hardcoded-secrets");
    expect(verdict?.verdict).toBe("fail");
    expect(verdict?.from_agentic).toBe(false);
    expect(verdict?.line_refs.length).toBeGreaterThan(0);
  });

  runIf(hasKey)("detects raw SQL injection in payment_controller.py", async () => {
    const rules = resolveRules("src/api/payment_controller.py", TEST_REPO)
      .filter((r) => r.id === "api/no-raw-sql");

    const diffs = getChangedFiles("main", "feature/add-payment-api", TEST_REPO);
    const fd = diffs.find((f) => f.path.includes("payment_controller"));

    const request: FileCheckRequest = {
      file_path: fd!.path,
      diff: fd!.diff,
      content: fd!.content,
      rules,
      repo_root: TEST_REPO,
    };

    const client = new Anthropic({ apiKey });
    const result = await checkFile(
      request,
      { client, cache: makeNullCache() },
      { model: "claude-haiku-4-5" },
    );

    const verdict = result.verdicts.find((v) => v.rule_id === "api/no-raw-sql");
    expect(verdict?.verdict).toBe("fail");
    expect(verdict?.from_agentic).toBe(false);
  });

  runIf(hasKey)("warns on debug print statement", async () => {
    const rules = resolveRules("src/api/payment_controller.py", TEST_REPO)
      .filter((r) => r.id === "quality/no-print-debug");

    const diffs = getChangedFiles("main", "feature/add-payment-api", TEST_REPO);
    const fd = diffs.find((f) => f.path.includes("payment_controller"));

    const request: FileCheckRequest = {
      file_path: fd!.path,
      diff: fd!.diff,
      content: fd!.content,
      rules,
      repo_root: TEST_REPO,
    };

    const client = new Anthropic({ apiKey });
    const result = await checkFile(
      request,
      { client, cache: makeNullCache() },
      { model: "claude-haiku-4-5" },
    );

    const verdict = result.verdicts.find((v) => v.rule_id === "quality/no-print-debug");
    expect(verdict?.verdict).toBe("fail");
    expect(verdict?.from_agentic).toBe(false);
  });

  runIf(hasKey)("clean file passes all rules", async () => {
    const rules = resolveRules("src/api/clean_controller.py", TEST_REPO);

    const diffs = getChangedFiles("main", "feature/add-payment-api", TEST_REPO);
    // clean_controller is not in the diff — test against full content directly
    const content = await import("node:fs").then(
      (fs) => fs.readFileSync(`${TEST_REPO}/src/api/clean_controller.py`, "utf-8")
    );

    if (rules.length === 0) {
      // No rules apply to clean_controller — skip
      return;
    }

    const request: FileCheckRequest = {
      file_path: "src/api/clean_controller.py",
      diff: "",
      content,
      rules: rules.filter((r) => r.context === "stateless"),
      repo_root: TEST_REPO,
    };

    const client = new Anthropic({ apiKey });
    const result = await checkFile(
      request,
      { client, cache: makeNullCache() },
      { model: "claude-haiku-4-5" },
    );

    for (const verdict of result.verdicts) {
      expect(verdict.verdict).not.toBe("fail");
    }
  });
});
