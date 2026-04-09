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
  createReadOnlyTools,
  createBashTool,
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
  trace: boolean = false
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
        }),
        timeoutMs
      );
      const rawText = response.content
        .filter((b): b is Anthropic.TextBlock => b.type === "text")
        .map((b) => b.text)
        .join("");
      if (trace) {
        process.stderr.write(`[TRACE] RESPONSE for ${request.file_path} [${request.rules[0]?.id ?? "?"}]:\n${rawText}\n`);
      }
      return parseVerdicts(rawText, request);
    } catch (err) {
      const e = err as Error & { status?: number };
      if (e.message === "timeout") {
        process.stderr.write(`Stateless pass timed out (attempt ${attempt + 1}/${MAX_RETRIES})\n`);
      } else if (e.status === 429) {
        process.stderr.write(`Rate limited (attempt ${attempt + 1}/${MAX_RETRIES})\n`);
      } else if (e.status !== undefined && e.status >= 500) {
        process.stderr.write(`API server error ${e.status} (attempt ${attempt + 1}/${MAX_RETRIES})\n`);
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
    ? [...createReadOnlyTools(repoRoot), createBashTool(repoRoot)]
    : createReadOnlyTools(repoRoot);

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
  const verdicts = parseVerdicts(rawAgenticText, subRequest);
  return verdicts.map((v) => ({
    ...v,
    verdict: v.verdict === "needs-more-context" ? ("fail" as Verdict) : v.verdict,
    from_agentic: true,
  }));
}
