/**
 * Agentic integration tests — mock stateless pass, real pi session.
 *
 * The stateless Anthropic client is replaced with a stub that immediately
 * returns needs-more-context for arch/enforce-payment-limits. This forces
 * checkFile to open a real pi agent session against the test-repo, which
 * will read src/constants.py and produce a verdict using the live API.
 *
 * This exercises the full agentic path end-to-end:
 *   mock stateless → pi createAgentSession → read tool → parse verdict
 *
 * Skipped when ANTHROPIC_API_KEY is not set.
 */

import { describe, it, expect } from "vitest";
import { resolve } from "node:path";
import { checkFile, DEFAULT_AGENTIC_TIMEOUT_MS } from "../../src/runner.js";
import { DEFAULTS } from "../../src/config.js";
import { resolveRules } from "../../src/resolver.js";
import type { FileCheckRequest } from "../../src/schema.js";
import { getChangedFiles } from "../../src/git.js";

const apiKey = process.env["ANTHROPIC_API_KEY"] ?? "";
const TEST_REPO = resolve(new URL(".", import.meta.url).pathname, "../../test-repo");

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

/**
 * Returns a mock Anthropic client whose messages.create always returns
 * needs-more-context for escalatedRuleId. This bypasses the real stateless
 * pass and forces checkFile straight into the agentic escalation path.
 */
function makeNeedsMoreContextClient(
  escalatedRuleId: string,
  hint: { read_files: string[]; question: string }
) {
  return {
    messages: {
      create: () =>
        Promise.resolve({
          content: [
            {
              type: "text",
              text: JSON.stringify({
                verdict: "needs-more-context",
                confidence: 0.2,
                reasoning: "Cannot determine limits without reading constants file.",
                line_refs: [],
                context_hint: hint,
              }),
            },
          ],
          stop_reason: "end_turn",
        }),
    },
  };
}

describe("agentic integration — mock stateless, real pi session", () => {
  runIf(hasKey)(
    "pi session reads constants.py and rejects missing payment limit checks",
    async () => {
      const allRules = resolveRules("src/api/payment_controller.py", TEST_REPO);
      // Only the forced-escalation rule for this test
      const rules = allRules.filter((r) => r.id === "arch/enforce-payment-limits");
      expect(rules).toHaveLength(1);
      expect(rules[0]?.context).toBe("agentic");

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

      const mockClient = makeNeedsMoreContextClient("arch/enforce-payment-limits", {
        read_files: ["src/constants.py"],
        question: "What are MAX_PAYMENT_AMOUNT and MIN_PAYMENT_AMOUNT?",
      });

      const result = await checkFile(
        request,
        { client: mockClient as never, cache: makeNullCache() },
        {
          model: "claude-haiku-4-5",
          agentic: {
            timeoutMs: DEFAULT_AGENTIC_TIMEOUT_MS,
            allowBash: false,
            model: DEFAULTS.agenticModel,
          },
        }
      );

      const verdict = result.verdicts.find((v) => v.rule_id === "arch/enforce-payment-limits");

      // The pi agent should have escalated and produced a terminal verdict
      expect(result.agentic_escalations).toBe(1);
      expect(verdict).toBeDefined();
      expect(verdict?.from_agentic).toBe(true);
      expect(verdict?.verdict).not.toBe("needs-more-context");

      // The agent read constants.py and found the limit is not enforced — expect reject or warn
      expect(["fail"]).toContain(verdict?.verdict);

      // Reasoning should mention the constants or limit values
      const reasoning = verdict?.reasoning?.toLowerCase() ?? "";
      const mentionsLimits =
        reasoning.includes("constant") ||
        reasoning.includes("max") ||
        reasoning.includes("limit") ||
        reasoning.includes("1000") ||
        reasoning.includes("amount");
      expect(mentionsLimits).toBe(true);
    }
  );

  runIf(hasKey)(
    "pi session timeout produces warn fallback with from_agentic: true",
    async () => {
      const rules = resolveRules("src/api/payment_controller.py", TEST_REPO)
        .filter((r) => r.id === "arch/enforce-payment-limits");

      const diffs = getChangedFiles("main", "feature/add-payment-api", TEST_REPO);
      const fd = diffs.find((f) => f.path.includes("payment_controller"));

      const request: FileCheckRequest = {
        file_path: fd!.path,
        diff: fd!.diff,
        content: fd!.content,
        rules,
        repo_root: TEST_REPO,
      };

      const mockClient = makeNeedsMoreContextClient("arch/enforce-payment-limits", {
        read_files: ["src/constants.py"],
        question: "What is MAX_PAYMENT_AMOUNT?",
      });

      const result = await checkFile(
        request,
        { client: mockClient as never, cache: makeNullCache() },
        {
          model: "claude-haiku-4-5",
          agentic: {
            timeoutMs: 500,   // absurdly short — pi session won't finish
            allowBash: false,
            model: DEFAULTS.agenticModel,
          },
        }
      );

      const verdict = result.verdicts.find((v) => v.rule_id === "arch/enforce-payment-limits");
      expect(result.agentic_escalations).toBe(1);
      expect(verdict?.verdict).toBe("fail");
      expect(verdict?.from_agentic).toBe(true);
    }
  );
});
