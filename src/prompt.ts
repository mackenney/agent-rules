import { FileCheckRequest, Rule, ContextHint } from "./schema.js";
import { DEFAULTS } from "./config.js";

export const STATELESS_SYSTEM_PROMPT = `\
You are a code review agent. Your job is to evaluate a source file against a single rule
and call submit_verdict with your evaluation.

Verdict meanings:
- "pass": the code satisfies this rule. No violation found.
- "fail": the code violates this rule.
- "needs-more-context": you cannot determine compliance without reading other files
  that are not in the diff. Use sparingly — only when the answer genuinely depends
  on external state. Do not use to express uncertainty about borderline cases; use
  "fail" when in doubt.
  When emitting needs-more-context you MUST populate context_hint.

Field guidance:
- "confidence": certainty 0.0–1.0. Use < 0.7 when genuinely ambiguous.
- "line_refs": absolute line numbers in the final file (from the numbered FULL FILE
  CONTENT block). These must match the " N | " prefix shown on each line. Empty for pass.
- "context_hint": required only for needs-more-context.
- If the rule doesn't apply to this file type, return "pass" with confidence 1.0.
- Prefer concrete verdicts over needs-more-context when you have reasonable evidence.
`;

/**
 * Prefix each line with its 1-based line number so the model can cite accurate
 * line references without having to count manually.
 */
function addLineNumbers(content: string): string {
  const lines = content.split("\n");
  const width = String(lines.length).length;
  return lines.map((l, i) => `${String(i + 1).padStart(width)} | ${l}`).join("\n");
}

/**
 * Annotate each line of a unified diff with the absolute new-file line number.
 *
 * The standard unified diff only records the hunk start in the @@ header; the
 * model would otherwise have to count lines across multiple hunks to determine
 * where a violation actually sits in the final file. We parse each @@ header,
 * maintain a running new-file line counter, and prepend every line with that
 * number so the model can read it directly instead of computing it.
 *
 * Format (width-padded to match addLineNumbers):
 *   "   7 | -old line"   (removed — still shows the original file line)
 *   "   7 | +new line"   (added)
 *   "   8 |  context"    (unchanged context)
 *   (hunk header lines are left as-is)
 */
function annotateDiff(diff: string, totalLines: number): string {
  const width = String(totalLines).length;
  const pad = (n: number) => String(n).padStart(width);

  const output: string[] = [];
  let newLine = 0;

  for (const raw of diff.split("\n")) {
    // @@ -old_start[,old_count] +new_start[,new_count] @@
    const hunkMatch = raw.match(/^@@ -\d+(?:,\d+)? \+(\d+)(?:,\d+)? @@/);
    if (hunkMatch) {
      newLine = parseInt(hunkMatch[1]!, 10);
      output.push(raw);
      continue;
    }

    if (newLine === 0) {
      // File header lines (---, +++, diff --git …) before the first hunk
      output.push(raw);
      continue;
    }

    const marker = raw[0];
    if (marker === "+") {
      output.push(`${pad(newLine)} | ${raw}`);
      newLine++;
    } else if (marker === "-") {
      // Removed lines don't advance the new-file counter but we still label
      // them with the line number they would occupy in the new file context
      // (i.e. the same slot as the next + or context line).
      output.push(`${pad(newLine)} | ${raw}`);
    } else {
      // Context line (space) or \ No newline at end of file
      output.push(`${pad(newLine)} | ${raw}`);
      if (marker === " ") newLine++;
    }
  }

  return output.join("\n");
}

/**
 * Builds the FILE header + diff section + full file content section shared
 * between buildFileContext and buildAgenticTask.
 */
function buildFileBlock(request: FileCheckRequest): string {
  const parts: string[] = [];
  parts.push(`FILE: ${request.file_path}`);

  const content = request.content ?? "";
  const totalLines = content ? content.split("\n").length : 0;

  if (request.diff) {
    parts.push("\nCHANGED LINES (unified diff with absolute new-file line numbers):");
    parts.push("```diff");
    parts.push(annotateDiff(request.diff.slice(0, DEFAULTS.maxDiffChars), totalLines));
    parts.push("```");
  }

  if (content) {
    parts.push("\nFULL FILE CONTENT (each line prefixed \"N | \"; use N verbatim in line_refs):");
    parts.push("```");
    parts.push(addLineNumbers(content.slice(0, DEFAULTS.maxContentChars)));
    parts.push("```");
  }

  return parts.join("\n");
}

/**
 * Returns the file/diff/content section of the user prompt for a single-rule check.
 * This is the cacheable prefix — identical across all rules for the same file.
 */
export function buildFileContext(request: FileCheckRequest, _rule: Rule): string {
  return buildFileBlock(request);
}

/**
 * Returns the rule-evaluation section of the user prompt for a single rule.
 * This is the non-cached suffix appended after the file context.
 */
export function buildRuleSection(rule: Rule): string {
  const parts: string[] = [];
  parts.push(`\nRULE TO EVALUATE:`);
  parts.push(serializeRule(rule));
  return parts.join("\n");
}

export function buildAgenticTask(
  request: FileCheckRequest,
  rule: Rule,
  hints: ContextHint[] = []
): string {
  const parts: string[] = [];

  parts.push(buildFileBlock(request));

  parts.push("\nRULE TO EVALUATE:");
  parts.push(serializeRule(rule));

  if (hints.length > 0) {
    const hintLines: string[] = [];
    for (const h of hints) {
      if (h.read_files.length > 0)
        hintLines.push(`Suggested files to read: ${h.read_files.join(", ")}`);
      if (h.question) hintLines.push(`Question to answer: ${h.question}`);
    }
    if (hintLines.length > 0)
      parts.push("\nContext hints from stateless pass:\n" + hintLines.join("\n"));
  }

  parts.push(`
IMPORTANT: Use your file-reading tools to gather whatever context you need, then
emit your verdict. Your FINAL message must be EXACTLY the following JSON object
and nothing else — no preamble, no explanation, no markdown fences:

{"reasoning":"<1-3 sentences>","line_refs":[],"confidence":0.0,"verdict":"pass|fail"}

Do NOT emit needs-more-context. You must reach a terminal verdict (pass/fail).`);

  return parts.join("\n");
}

function serializeRule(rule: Rule): string {
  const lines: string[] = [
    `  name: ${rule.name}`,
    `  severity: ${rule.severity}`,
    `  instruction: ${rule.prompt.trim()}`,
  ];
  if (rule.needs_more_context_when) {
    lines.push(`  escalation guidance: ${rule.needs_more_context_when.trim()}`);
  }
  if (rule.examples.length > 0) {
    lines.push("  examples:");
    for (const ex of rule.examples) {
      const label = ex.verdict.toUpperCase();
      lines.push(`    [${label}] ${ex.description}`);
      for (const codeLine of ex.code.trim().split("\n")) {
        lines.push(`      ${codeLine}`);
      }
    }
  }
  return lines.join("\n");
}
