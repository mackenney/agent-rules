import { existsSync, statSync, readdirSync } from "node:fs";
import { join, resolve, relative, dirname } from "node:path";
import micromatch from "micromatch";
import { loadRuleFile, allRules } from "./parser.js";
import { Rule, RuleFile } from "./schema.js";

export const RULE_FILE_NAME = ".agent-rules.toml";

export function resolveRules(filePath: string, repoRoot: string): Rule[] {
  const absRepoRoot = resolve(repoRoot);

  let absFile: string;
  if (filePath.startsWith("/")) {
    absFile = filePath;
  } else {
    absFile = join(absRepoRoot, filePath);
  }

  const ruleFilePaths = findRuleFiles(absFile, absRepoRoot);
  const ruleFiles: RuleFile[] = [];
  for (const rp of ruleFilePaths) {
    try {
      ruleFiles.push(loadRuleFile(rp));
    } catch (err) {
      process.stderr.write(`warning: skipping ${rp}: ${(err as Error).message}\n`);
    }
  }

  const merged = mergeRuleFiles(ruleFiles);

  const result: Rule[] = [];
  for (const rule of merged) {
    if (!rule.enabled) continue;
    if (globMatches(absFile, absRepoRoot, rule)) {
      result.push(rule);
    }
  }

  return result;
}

function findRuleFiles(filePath: string, repoRoot: string): string[] {
  let targetDir: string;
  try {
    const stat = statSync(filePath);
    targetDir = stat.isDirectory() ? filePath : dirname(filePath);
  } catch {
    targetDir = dirname(filePath);
  }

  let rel: string;
  try {
    rel = relative(repoRoot, targetDir);
    if (rel.startsWith("..")) {
      // Outside repo root
      const candidate = join(repoRoot, RULE_FILE_NAME);
      return existsSync(candidate) ? [candidate] : [];
    }
  } catch {
    const candidate = join(repoRoot, RULE_FILE_NAME);
    return existsSync(candidate) ? [candidate] : [];
  }

  const parts = rel === "" ? [] : rel.split("/").filter(Boolean);
  const dirs: string[] = [repoRoot];
  for (let i = 0; i < parts.length; i++) {
    dirs.push(join(repoRoot, ...parts.slice(0, i + 1)));
  }

  const result: string[] = [];
  for (const d of dirs) {
    const candidate = join(d, RULE_FILE_NAME);
    if (existsSync(candidate)) {
      result.push(candidate);
    }
  }
  return result;
}

function mergeRuleFiles(ruleFiles: RuleFile[]): Rule[] {
  const merged = new Map<string, Rule>();

  for (const ruleFile of ruleFiles) {
    if (ruleFile.inherit_mode === "replace") {
      merged.clear();
    }
    for (const rule of allRules(ruleFile)) {
      merged.set(rule.id, rule);
    }
  }

  return Array.from(merged.values());
}

const SKIP_DIRS = new Set([".git", "node_modules", ".next", "dist", "__pycache__", ".cache"]);

export function findAllRuleFiles(repoRoot: string): string[] {
  const results: string[] = [];
  function walk(dir: string) {
    let entries: import("node:fs").Dirent[];
    try {
      entries = readdirSync(dir, { withFileTypes: true });
    } catch {
      return;
    }
    for (const entry of entries) {
      if (entry.isDirectory()) {
        if (SKIP_DIRS.has(entry.name)) continue;
        walk(join(dir, entry.name));
      } else if (entry.name === RULE_FILE_NAME) {
        results.push(join(dir, entry.name));
      }
    }
  }
  walk(repoRoot);
  return results;
}

function globMatches(filePath: string, repoRoot: string, rule: Rule): boolean {
  let relPath: string;
  try {
    relPath = relative(repoRoot, filePath);
    if (relPath.startsWith("..")) return false;
  } catch {
    return false;
  }

  // Replace backslashes on Windows
  const relStr = relPath.replace(/\\/g, "/");

  if (rule.glob_include.length > 0) {
    const matched = micromatch([relStr], rule.glob_include, { dot: true });
    if (matched.length === 0) return false;
  }

  if (rule.glob_exclude.length > 0) {
    const excluded = micromatch([relStr], rule.glob_exclude, { dot: true });
    if (excluded.length > 0) return false;
  }

  return true;
}
