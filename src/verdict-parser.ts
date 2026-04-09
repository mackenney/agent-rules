import type { FileCheckRequest, RuleVerdict, Verdict } from "./schema.js";

export function parseVerdicts(rawText: string, request: FileCheckRequest): RuleVerdict[] {
  const rule = request.rules[0]!;

  const fallback = (reasoning: string): RuleVerdict[] => [{
    rule_id: rule.id,
    verdict: "fail" as Verdict,
    rule_severity: rule.severity,
    confidence: 0.0,
    reasoning,
    line_refs: [],
    context_hint: null,
    from_agentic: false,
  }];

  let text = rawText.trim();

  // Strip a markdown code fence if the text opens with one
  if (text.startsWith("```")) {
    const lines = text.split("\n");
    const inner: string[] = [];
    for (let i = 0; i < lines.length; i++) {
      if (i === 0) continue;
      if (lines[i]?.trim() === "```" && i === lines.length - 1) continue;
      inner.push(lines[i] ?? "");
    }
    text = inner.join("\n").trim();
  }

  // If the text doesn't start with '{', the agent wrote preamble before the JSON.
  // Scan backwards to find the last '{' that yields a valid verdict object.
  if (!text.trimStart().startsWith("{")) {
    let extracted = false;
    for (let i = text.length - 1; i >= 0; i--) {
      if (text[i] === "{") {
        const candidate = text.slice(i).replace(/[\x00-\x08\x0a-\x1f]/g, " ");
        try {
          const parsed = JSON.parse(candidate) as Record<string, unknown>;
          if (typeof parsed["verdict"] === "string") {
            text = candidate;
            extracted = true;
            break;
          }
        } catch {
          // not valid JSON from this position, keep scanning
        }
      }
    }
    if (!extracted) {
      // Also check for markdown-fenced JSON anywhere in the text
      const fenceMatch = text.match(/```(?:json)?\s*([\s\S]*?)```/);
      if (fenceMatch?.[1]) text = fenceMatch[1].trim();
    }
  }

  const sanitized = text.replace(/[\x00-\x08\x0a-\x1f]/g, " ");

  let e: Record<string, unknown>;
  try {
    e = JSON.parse(sanitized) as Record<string, unknown>;
  } catch {
    return fallback("JSON parse error");
  }

  let verdict: Verdict;
  const raw = e["verdict"];
  if (["pass", "fail", "needs-more-context"].includes(raw as string)) {
    verdict = raw as Verdict;
  } else {
    return fallback("Model returned unrecognised verdict");
  }

  const contextHintRaw = e["context_hint"];
  const contextHint =
    contextHintRaw && typeof contextHintRaw === "object"
      ? {
          read_files: Array.isArray((contextHintRaw as Record<string, unknown>)["read_files"])
            ? ((contextHintRaw as Record<string, unknown>)["read_files"] as string[])
            : [],
          question:
            typeof (contextHintRaw as Record<string, unknown>)["question"] === "string"
              ? ((contextHintRaw as Record<string, unknown>)["question"] as string)
              : "",
        }
      : null;

  return [{
    rule_id: rule.id,
    verdict,
    rule_severity: rule.severity,
    confidence: typeof e["confidence"] === "number" ? e["confidence"] : 0.5,
    reasoning:
      typeof e["reasoning"] === "string"
        ? e["reasoning"].replace(/\n/g, " ").replace(/\r/g, "").trim()
        : "",
    line_refs: Array.isArray(e["line_refs"]) ? (e["line_refs"] as number[]) : [],
    context_hint: contextHint,
    from_agentic: false,
  }];
}
