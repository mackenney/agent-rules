import Anthropic from "@anthropic-ai/sdk";
import { resolve } from "node:path";
import {
  buildFileContext,
  buildRuleSection,
  buildAgenticTask,
  STATELESS_SYSTEM_PROMPT,
} from "./prompt.js";
import { DEFAULTS } from "./config.js";
import {
  AuthStorage,
  createAgentSession,
  SessionManager,
  type AgentSessionEvent,
} from "@mariozechner/pi-coding-agent";
import { getModel } from "@mariozechner/pi-ai";
import {
  ContextHint,
  FileCheckRequest,
  Rule,
  RuleVerdict,
  Verdict,
} from "./schema.js";
import type { ProgressReporter } from "./reporter.js";
import { withTimeout, delay } from "./concurrency.js";
import { parseVerdicts } from "./verdict-parser.js";

const MAX_RETRIES = 3;
const RETRY_BASE_DELAY_MS = 1000;

const VERDICT_TOOL: Anthropic.Tool = {
  name: "submit_verdict",
  description: "Submit the rule evaluation verdict as structured data.",
  input_schema: {
    type: "object" as const,
    properties: {
      reasoning: {
        type: "string",
        description: "1-3 sentences explaining the verdict, referencing specific code.",
      },
      line_refs: {
        type: "array",
        items: { type: "integer" },
        description: "Absolute line numbers of violations. Empty array for pass.",
      },
      context_hint: {
        type: "object",
        description: "Required only when verdict is needs-more-context.",
        properties: {
          read_files: { type: "array", items: { type: "string" } },
          question: { type: "string" },
        },
        required: ["read_files", "question"],
      },
      confidence: {
        type: "number",
        description: "Certainty 0.0–1.0. Use < 0.7 when genuinely ambiguous.",
      },
      verdict: {
        type: "string",
        enum: ["pass", "fail", "needs-more-context"],
      },
    },
    required: ["reasoning", "line_refs", "confidence", "verdict"],
  },
};

function buildVerdictFromToolInput(
  input: Record<string, unknown>,
  request: FileCheckRequest,
  fromAgentic = false,
): RuleVerdict {
  const rule = request.rules[0]!;
  const rawVerdict = input["verdict"] as string;
  const verdict: Verdict = ["pass", "fail", "needs-more-context"].includes(rawVerdict)
    ? (rawVerdict as Verdict)
    : "fail";

  const rawHint = input["context_hint"];
  const contextHint =
    rawHint && typeof rawHint === "object"
      ? {
          read_files: Array.isArray((rawHint as Record<string, unknown>)["read_files"])
            ? ((rawHint as Record<string, unknown>)["read_files"] as string[])
            : [],
          question:
            typeof (rawHint as Record<string, unknown>)["question"] === "string"
              ? ((rawHint as Record<string, unknown>)["question"] as string)
              : "",
        }
      : null;

  return {
    rule_id: rule.id,
    verdict,
    rule_severity: rule.severity,
    confidence: typeof input["confidence"] === "number" ? (input["confidence"] as number) : 0.5,
    reasoning:
      typeof input["reasoning"] === "string"
        ? (input["reasoning"] as string).replace(/\n/g, " ").replace(/\r/g, "").trim()
        : "",
    line_refs: Array.isArray(input["line_refs"]) ? (input["line_refs"] as number[]) : [],
    context_hint: contextHint,
    from_agentic: fromAgentic,
  };
}

/**
 * Internal agentic options with resolved values.
 * Used only inside llm.ts — callers use AgenticConfig from runner.ts.
 */
export interface AgenticOpts {
  timeoutMs: number;
  allowBash: boolean;
  model: string;
  // NOTE: apiKey removed — read from process.env["ANTHROPIC_API_KEY"] directly
}

export async function runStateless(
  client: Anthropic,
  request: FileCheckRequest,
  model: string,
  timeoutMs: number,
  cacheSystemPrompt: boolean = false,
  cacheFileContext: boolean = false,
  trace: boolean = false,
  progress?: ProgressReporter,
): Promise<RuleVerdict[]> {
  const rule = request.rules[0]!;
  const fileContext = buildFileContext(request, rule);
  const ruleSection = buildRuleSection(rule);

  const system: Anthropic.TextBlockParam[] | string = cacheSystemPrompt
    ? [{ type: "text", text: STATELESS_SYSTEM_PROMPT, cache_control: { type: "ephemeral" } }]
    : STATELESS_SYSTEM_PROMPT;

  const userContent: Anthropic.TextBlockParam[] | string = cacheFileContext
    ? [
        { type: "text", text: fileContext, cache_control: { type: "ephemeral" } },
        { type: "text", text: ruleSection },
      ]
    : fileContext + ruleSection;

  if (trace) {
    const sysText = typeof system === "string" ? system : system.map((b) => b.text).join("");
    const userText = typeof userContent === "string" ? userContent : userContent.map((b) => b.text).join("");
    process.stderr.write(`\n[TRACE] ── stateless prompt for ${request.file_path} [${request.rules[0]?.id ?? "?"}] ──\n`);
    process.stderr.write(`[TRACE] SYSTEM:\n${sysText}\n`);
    process.stderr.write(`[TRACE] USER:\n${userText}\n`);
  }

  for (let attempt = 0; attempt < MAX_RETRIES; attempt++) {
    try {
      const response = await withTimeout(
        client.messages.create({
          model,
          max_tokens: DEFAULTS.maxStatelessTokens,
          system,
          messages: [{ role: "user", content: userContent }],
          tools: [VERDICT_TOOL],
          tool_choice: { type: "tool", name: "submit_verdict" },
        }),
        timeoutMs
      );
      if (trace) {
        const toolUseBlock = response.content.find((b) => b.type === "tool_use");
        process.stderr.write(
          `[TRACE] RESPONSE for ${request.file_path} [${request.rules[0]?.id ?? "?"}]:\n` +
            JSON.stringify(toolUseBlock?.type === "tool_use" ? toolUseBlock.input : response.content, null, 2) + "\n"
        );
      }
      const toolUse = response.content.find((b): b is Anthropic.ToolUseBlock => b.type === "tool_use");
      if (toolUse) {
        return [buildVerdictFromToolInput(toolUse.input as Record<string, unknown>, request)];
      }
      // Fallback: model returned text despite forced tool_choice (should not happen)
      const rawText = response.content
        .filter((b): b is Anthropic.TextBlock => b.type === "text")
        .map((b) => b.text)
        .join("");
      return parseVerdicts(rawText, request);
    } catch (err) {
      const e = err as Error & { status?: number };
      if (e.message === "timeout") {
        const msg = `Stateless pass timed out (attempt ${attempt + 1}/${MAX_RETRIES}) [${request.file_path}]`;
        progress ? progress.log(msg) : process.stderr.write(msg + "\n");
      } else if (e.status === 429) {
        const msg = `Rate limited (attempt ${attempt + 1}/${MAX_RETRIES}) [${request.file_path}]`;
        progress ? progress.log(msg) : process.stderr.write(msg + "\n");
      } else if (e.status !== undefined && e.status >= 500) {
        const msg = `API server error ${e.status} (attempt ${attempt + 1}/${MAX_RETRIES}) [${request.file_path}]`;
        progress ? progress.log(msg) : process.stderr.write(msg + "\n");
      } else {
        throw err;
      }
    }
    if (attempt < MAX_RETRIES - 1) await delay(RETRY_BASE_DELAY_MS * Math.pow(2, attempt));
  }

  return request.rules.map((r) => ({
    rule_id: r.id,
    verdict: "fail" as Verdict,
    rule_severity: r.severity,
    confidence: 0.0,
    reasoning: "LLM call failed",
    line_refs: [],
    context_hint: null,
    from_agentic: false,
  }));
}

export async function runAgenticEscalation(
  request: FileCheckRequest,
  escalatedRules: [Rule],
  hints: (ContextHint | null)[],
  opts: AgenticOpts,
  progress?: ProgressReporter,
  trace: boolean = false
): Promise<RuleVerdict[]> {
  const repoRoot = resolve(request.repo_root);

  const tools = opts.allowBash
    ? ["read", "grep", "find", "ls", "bash"]
    : ["read", "grep", "find", "ls"];

  const model = getModel("anthropic", opts.model as Parameters<typeof getModel>[1]);
  if (!model) throw new Error(`Unknown model: ${opts.model}`);

  const authStorage = AuthStorage.create();
  const apiKey = process.env["ANTHROPIC_API_KEY"];
  if (!apiKey) {
    throw new Error("ANTHROPIC_API_KEY environment variable is required for agentic escalation");
  }
  authStorage.setRuntimeApiKey("anthropic", apiKey);

  const { session } = await createAgentSession({
    cwd: repoRoot,
    tools,
    model,
    authStorage,
    sessionManager: SessionManager.inMemory(),
  });

  const textBuffer: string[] = [];
  session.subscribe((event: AgentSessionEvent) => {
    if (
      event.type === "message_update" &&
      event.assistantMessageEvent.type === "text_delta"
    ) {
      textBuffer.push(event.assistantMessageEvent.delta);
    }
  });

  const agenticTask = buildAgenticTask(request, escalatedRules[0], hints.filter((h): h is ContextHint => h !== null));
  if (trace) {
    process.stderr.write(`\n[TRACE] ── agentic task for ${request.file_path} ──\n`);
    process.stderr.write(`[TRACE] TASK:\n${agenticTask}\n`);
  }

  try {
    const taskPromise = session.prompt(agenticTask);
    const timeoutPromise = new Promise<never>((_, reject) =>
      setTimeout(() => reject(new Error("agentic_timeout")), opts.timeoutMs)
    );
    await Promise.race([taskPromise, timeoutPromise]);
  } catch (err) {
    const e = err as Error;
    await session.abort();
    if (e.message === "agentic_timeout") {
      console.error(
        `Agentic escalation timed out after ${opts.timeoutMs}ms for ${request.file_path}`
      );
    } else {
      console.error(`Agentic escalation error for ${request.file_path}: ${e.message}`);
    }
  } finally {
    session.dispose();
  }

  const rawAgenticText = textBuffer.join("");
  if (trace) {
    process.stderr.write(`[TRACE] AGENTIC RESPONSE for ${request.file_path}:\n${rawAgenticText}\n`);
  }
  const subRequest: FileCheckRequest = { ...request, rules: escalatedRules };
  let verdicts = parseVerdicts(rawAgenticText, subRequest);

  // If the agent's final message wasn't parseable JSON, normalize via a single tool-use call.
  if (verdicts[0]?.reasoning === "JSON parse error" || verdicts[0]?.reasoning === "Model returned unrecognised verdict") {
    try {
      const normClient = new Anthropic({ apiKey: process.env["ANTHROPIC_API_KEY"] ?? "" });
      const normResponse = await normClient.messages.create({
        model: opts.model,
        max_tokens: 1024,
        system: "Extract a structured rule evaluation verdict from the agent's analysis. The agent has already done the investigation.",
        messages: [{
          role: "user",
          content:
            `File: ${request.file_path}\nRule: ${escalatedRules[0].id} — ${escalatedRules[0].name}\n\n` +
            `Agent analysis:\n${rawAgenticText.slice(0, 8000)}\n\nExtract the verdict.`,
        }],
        tools: [VERDICT_TOOL],
        tool_choice: { type: "tool", name: "submit_verdict" },
      });
      const toolUse = normResponse.content.find((b): b is Anthropic.ToolUseBlock => b.type === "tool_use");
      if (toolUse) {
        verdicts = [buildVerdictFromToolInput(toolUse.input as Record<string, unknown>, subRequest, true)];
      }
    } catch {
      // normalization failed — keep the parse-error fallback verdict
    }
  }

  return verdicts.map((v) => ({
    ...v,
    verdict: v.verdict === "needs-more-context" ? ("fail" as Verdict) : v.verdict,
    from_agentic: true,
  }));
}
