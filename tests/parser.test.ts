import { describe, it, expect, beforeEach, afterEach } from "vitest";
import { mkdtempSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { loadRuleFile } from "../src/parser.js";

describe("loadRuleFile", () => {
  let dir: string;

  beforeEach(() => {
    dir = mkdtempSync(join(tmpdir(), "parser-test-"));
  });

  afterEach(() => {
    rmSync(dir, { recursive: true });
  });

  function writeToml(content: string): string {
    const path = join(dir, ".agent-rules.toml");
    writeFileSync(path, content);
    return path;
  }

  it("parses a valid TOML file with one rule", () => {
    const path = writeToml(`
[[rules]]
id = "style/no-var"
name = "No var"
prompt = "Do not use var."
`);
    const result = loadRuleFile(path);
    expect(result.rules).toHaveLength(1);
    expect(result.rules[0]!.id).toBe("style/no-var");
    expect(result.rules[0]!.name).toBe("No var");
    expect(result.rules[0]!.prompt).toBe("Do not use var.");
  });

  it("maps glob-include / glob-exclude to glob_include / glob_exclude", () => {
    const path = writeToml(`
[[rules]]
id = "style/ts-only"
name = "TS only"
prompt = "Check types."
glob-include = ["**/*.ts"]
glob-exclude = ["**/*.d.ts"]
`);
    const result = loadRuleFile(path);
    expect(result.rules[0]!.glob_include).toEqual(["**/*.ts"]);
    expect(result.rules[0]!.glob_exclude).toEqual(["**/*.d.ts"]);
  });

  it("returns empty rules array for an empty rules section", () => {
    const path = writeToml(`version = "1"\n`);
    const result = loadRuleFile(path);
    expect(result.rules).toHaveLength(0);
  });

  it("sets source_path on the returned RuleFile", () => {
    const path = writeToml(`version = "1"\n`);
    const result = loadRuleFile(path);
    expect(result.source_path).toBe(path);
  });

  it("applies default values from schema (severity, enabled, context)", () => {
    const path = writeToml(`
[[rules]]
id = "test/defaults"
name = "Defaults"
prompt = "Check."
`);
    const result = loadRuleFile(path);
    const rule = result.rules[0]!;
    expect(rule.severity).toBe("warn");
    expect(rule.enabled).toBe(true);
    expect(rule.context).toBe("stateless");
    expect(rule.examples).toEqual([]);
    expect(rule.needs_more_context_when).toBe("");
  });

  it("throws a parse error for malformed TOML (unclosed bracket)", () => {
    const path = writeToml(`[[rules\nid = "bad"\n`);
    expect(() => loadRuleFile(path)).toThrow(/TOML parse error/);
  });

  it("throws a schema error for invalid severity type", () => {
    const path = writeToml(`
[[rules]]
id = "style/bad-severity"
name = "Bad"
prompt = "Check."
severity = "critical"
`);
    expect(() => loadRuleFile(path)).toThrow(/Schema validation error|Invalid enum/);
  });

  it("ignores unknown fields like scope (schema strips extras via Zod)", () => {
    const path = writeToml(`
[[rules]]
id = "repo/check"
name = "Repo check"
prompt = "Check."
scope = "repo"
`);
    // Unknown fields are silently stripped by Zod; no error should be thrown
    expect(() => loadRuleFile(path)).not.toThrow();
  });

  it("throws with 'Duplicate rule id' for duplicate IDs", () => {
    const path = writeToml(`
[[rules]]
id = "dup/rule"
name = "First"
prompt = "Check."

[[rules]]
id = "dup/rule"
name = "Second"
prompt = "Check."
`);
    expect(() => loadRuleFile(path)).toThrow(/Duplicate rule id/);
  });

  it("throws for empty rule ID (schema requires min(1))", () => {
    const path = writeToml(`
[[rules]]
id = ""
name = "Empty ID"
prompt = "Check."
`);
    expect(() => loadRuleFile(path)).toThrow();
  });

  it("throws when file does not exist", () => {
    const missing = join(dir, "nonexistent.toml");
    expect(() => loadRuleFile(missing)).toThrow(/Cannot read rule file/);
  });

  it("parses multiple rules from one file", () => {
    const path = writeToml(`
[[rules]]
id = "style/rule-a"
name = "Rule A"
prompt = "Check A."

[[rules]]
id = "style/rule-b"
name = "Rule B"
prompt = "Check B."
`);
    const result = loadRuleFile(path);
    expect(result.rules).toHaveLength(2);
    expect(result.rules.map((r) => r.id)).toEqual(["style/rule-a", "style/rule-b"]);
  });

  it("parses examples embedded in a rule", () => {
    const path = writeToml(`
[[rules]]
id = "style/no-var"
name = "No var"
prompt = "Avoid var."

[[rules.examples]]
description = "bad"
code = "var x = 1;"
verdict = "fail"

[[rules.examples]]
description = "good"
code = "const x = 1;"
verdict = "pass"
`);
    const result = loadRuleFile(path);
    const rule = result.rules[0]!;
    expect(rule.examples).toHaveLength(2);
    expect(rule.examples[0]!.verdict).toBe("fail");
    expect(rule.examples[1]!.verdict).toBe("pass");
  });
});
