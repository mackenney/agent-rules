import { readFileSync } from "node:fs";
import { parse as parseToml } from "smol-toml";
import { z } from "zod";
import { Rule, RuleFile, RuleFileSchema, RuleSchema } from "./schema.js";

function buildRule(rawRule: Record<string, unknown>): Rule {
  const raw = { ...rawRule };

  // Map TOML kebab-case keys to internal underscore names
  if (raw["glob-include"] !== undefined) {
    raw["glob_include"] = raw["glob-include"];
    delete raw["glob-include"];
  }
  if (raw["glob-exclude"] !== undefined) {
    raw["glob_exclude"] = raw["glob-exclude"];
    delete raw["glob-exclude"];
  }

  return RuleSchema.parse(raw);
}

function formatValidationError(path: string, err: unknown): Error {
  if (err instanceof z.ZodError) {
    const messages = err.errors.map((e) => {
      const loc = e.path.length > 0 ? e.path.join(" -> ") : "<root>";
      return `  ${loc}: ${e.message}`;
    });
    return new Error(`Schema validation error in '${path}':\n${messages.join("\n")}`);
  }
  return err instanceof Error ? err : new Error(String(err));
}

export function loadRuleFile(path: string): RuleFile {
  let text: string;
  try {
    text = readFileSync(path, "utf-8");
  } catch (err) {
    throw new Error(`Cannot read rule file '${path}': ${(err as Error).message}`);
  }

  let raw: Record<string, unknown>;
  try {
    raw = parseToml(text) as Record<string, unknown>;
  } catch (err) {
    throw new Error(`TOML parse error in '${path}': ${(err as Error).message}`);
  }

  const rawRules = (raw["rules"] as Record<string, unknown>[] | undefined) ?? [];
  delete raw["rules"];

  let ruleFile: RuleFile;
  try {
    ruleFile = RuleFileSchema.parse(raw);
  } catch (err) {
    throw formatValidationError(path, err);
  }

  ruleFile.rules = rawRules.map((r) => {
    try {
      return buildRule(r);
    } catch (err) {
      throw formatValidationError(path, err);
    }
  });

  const ids = new Set<string>();
  for (const rule of ruleFile.rules) {
    if (ids.has(rule.id)) {
      throw new Error(`Duplicate rule id '${rule.id}' in ${path}`);
    }
    ids.add(rule.id);
  }

  ruleFile.source_path = path;
  return ruleFile;
}

export function allRules(ruleFile: RuleFile): Rule[] {
  return ruleFile.rules;
}
