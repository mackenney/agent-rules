import { describe, it, expect, beforeEach, afterEach } from "vitest";
import { mkdirSync, writeFileSync, rmSync } from "node:fs";
import { join, resolve } from "node:path";
import { tmpdir } from "node:os";
import { resolveRules } from "../src/resolver.js";

function createTmpDir(): string {
  const dir = join(tmpdir(), `agent-rules-test-${Date.now()}-${Math.random().toString(36).slice(2)}`);
  mkdirSync(dir, { recursive: true });
  return dir;
}

function writeToml(dir: string, content: string): void {
  writeFileSync(join(dir, ".agent-rules.toml"), content, "utf-8");
}

describe("resolveRules", () => {
  let repoRoot: string;

  beforeEach(() => {
    repoRoot = createTmpDir();
  });

  afterEach(() => {
    rmSync(repoRoot, { recursive: true, force: true });
  });

  it("returns empty array when no rule files exist", () => {
    mkdirSync(join(repoRoot, "src"), { recursive: true });
    const rules = resolveRules("src/foo.ts", repoRoot);
    expect(rules).toEqual([]);
  });

  it("loads rules from root", () => {
    writeToml(
      repoRoot,
      `
[[rules]]
id = "no-console"
name = "No console.log"
prompt = "Do not use console.log in production code"
`
    );
    const rules = resolveRules("src/foo.ts", repoRoot);
    expect(rules).toHaveLength(1);
    expect(rules[0]?.id).toBe("no-console");
  });

  it("merges root and child rules, child overrides parent by ID", () => {
    writeToml(
      repoRoot,
      `
[[rules]]
id = "rule-a"
name = "Rule A root"
prompt = "Root version of rule A"

[[rules]]
id = "rule-b"
name = "Rule B"
prompt = "Rule B from root"
`
    );
    const srcDir = join(repoRoot, "src");
    mkdirSync(srcDir, { recursive: true });
    writeToml(
      srcDir,
      `
[[rules]]
id = "rule-a"
name = "Rule A override"
prompt = "Child version of rule A"
`
    );

    const rules = resolveRules("src/foo.ts", repoRoot);
    expect(rules).toHaveLength(2);

    const ruleA = rules.find((r) => r.id === "rule-a");
    expect(ruleA?.name).toBe("Rule A override");

    const ruleB = rules.find((r) => r.id === "rule-b");
    expect(ruleB?.name).toBe("Rule B");
  });

  it("inherit_mode replace clears parent rules", () => {
    writeToml(
      repoRoot,
      `
[[rules]]
id = "rule-parent"
name = "Parent rule"
prompt = "Parent rule prompt"
`
    );
    const subDir = join(repoRoot, "sub");
    mkdirSync(subDir, { recursive: true });
    writeToml(
      subDir,
      `
inherit_mode = "replace"

[[rules]]
id = "rule-child"
name = "Child rule"
prompt = "Child rule prompt"
`
    );

    const rules = resolveRules("sub/file.ts", repoRoot);
    expect(rules).toHaveLength(1);
    expect(rules[0]?.id).toBe("rule-child");
  });

  it("filters rules by glob-include", () => {
    writeToml(
      repoRoot,
      `
[[rules]]
id = "ts-only"
name = "TypeScript only"
prompt = "Only for TypeScript files"
glob-include = ["**/*.ts"]

[[rules]]
id = "all-files"
name = "All files"
prompt = "For all files"
`
    );

    const tsRules = resolveRules("src/foo.ts", repoRoot);
    expect(tsRules.map((r) => r.id)).toContain("ts-only");
    expect(tsRules.map((r) => r.id)).toContain("all-files");

    const pyRules = resolveRules("src/foo.py", repoRoot);
    expect(pyRules.map((r) => r.id)).not.toContain("ts-only");
    expect(pyRules.map((r) => r.id)).toContain("all-files");
  });

  it("excludes files matching glob-exclude", () => {
    writeToml(
      repoRoot,
      `
[[rules]]
id = "no-test"
name = "Not for tests"
prompt = "Skip test files"
glob-include = ["**/*"]
glob-exclude = ["**/*.test.ts"]
`
    );

    const prodRules = resolveRules("src/foo.ts", repoRoot);
    expect(prodRules.map((r) => r.id)).toContain("no-test");

    const testRules = resolveRules("src/foo.test.ts", repoRoot);
    expect(testRules.map((r) => r.id)).not.toContain("no-test");
  });

  it("skips disabled rules", () => {
    writeToml(
      repoRoot,
      `
[[rules]]
id = "enabled-rule"
name = "Enabled"
prompt = "This is enabled"

[[rules]]
id = "disabled-rule"
name = "Disabled"
prompt = "This is disabled"
enabled = false
`
    );

    const rules = resolveRules("src/foo.ts", repoRoot);
    expect(rules.map((r) => r.id)).toContain("enabled-rule");
    expect(rules.map((r) => r.id)).not.toContain("disabled-rule");
  });

  it("skips repo-scoped rules", () => {
    writeToml(
      repoRoot,
      `
[[rules]]
id = "file-rule"
name = "File rule"
prompt = "File scoped rule"
scope = "file"
`
    );

    const rules = resolveRules("src/foo.ts", repoRoot);
    expect(rules.map((r) => r.id)).toContain("file-rule");
  });

  it("applies rules from deeply nested directories", () => {
    writeToml(
      repoRoot,
      `
[[rules]]
id = "root-rule"
name = "Root rule"
prompt = "From root"
`
    );
    const deepDir = join(repoRoot, "a", "b", "c");
    mkdirSync(deepDir, { recursive: true });
    writeToml(
      deepDir,
      `
[[rules]]
id = "deep-rule"
name = "Deep rule"
prompt = "From deep"
`
    );

    const rules = resolveRules("a/b/c/file.ts", repoRoot);
    const ids = rules.map((r) => r.id);
    expect(ids).toContain("root-rule");
    expect(ids).toContain("deep-rule");
  });

  it("three-level replace chain: root -> parent(replace) -> child(merge)", () => {
    writeToml(repoRoot, `
[[rules]]
id = "rule-root"
name = "Root rule"
prompt = "Root"
`);

    const parentDir = join(repoRoot, "parent");
    mkdirSync(parentDir, { recursive: true });
    writeToml(parentDir, `
inherit_mode = "replace"

[[rules]]
id = "rule-parent"
name = "Parent rule"
prompt = "Parent"
`);

    const childDir = join(parentDir, "child");
    mkdirSync(childDir, { recursive: true });
    writeToml(childDir, `
[[rules]]
id = "rule-child"
name = "Child rule"
prompt = "Child"
`);

    const rules = resolveRules("parent/child/file.ts", repoRoot);
    const ids = rules.map((r) => r.id);

    expect(ids).not.toContain("rule-root");
    expect(ids).toContain("rule-parent");
    expect(ids).toContain("rule-child");
  });

  it("handles absolute file path input", () => {
    writeToml(repoRoot, `
[[rules]]
id = "test-rule"
name = "Test"
prompt = "Test"
`);

    const absPath = resolve(repoRoot, "src/foo.ts");
    const rules = resolveRules(absPath, repoRoot);

    expect(rules.map((r) => r.id)).toContain("test-rule");
  });

  it("includes rules from intermediate directories for deeply nested paths", () => {
    writeToml(repoRoot, `
[[rules]]
id = "root-rule"
name = "Root"
prompt = "Root"
`);

    const srcDir = join(repoRoot, "src");
    mkdirSync(srcDir, { recursive: true });
    writeToml(srcDir, `
[[rules]]
id = "src-rule"
name = "Src"
prompt = "Src"
`);

    const rules = resolveRules("src/components/Button.ts", repoRoot);
    const ids = rules.map((r) => r.id);

    expect(ids).toContain("root-rule");
    expect(ids).toContain("src-rule");
  });
});
