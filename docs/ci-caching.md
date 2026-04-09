# CI Caching with agent-rules

agent-rules stores LLM verdicts as flat JSON files in `.agent-rules-cache/`.
The cache key is a SHA-256 hash of the rule definitions and the file content, so the
same file on two different branches produces the same cache hit.  This document explains
how to preserve that cache across CI runs so you only pay for LLM calls when something
actually changed.

## How the cache key works

```
cache_key = sha256(
    rule_ids + rule_prompts + rule_severities +   # rule fingerprint
    file_path + diff + full_content               # file fingerprint
)
```

Consequences:
- **Branch-agnostic**: identical file content produces the same key regardless of branch.
- **Rule-sensitive**: changing a rule's prompt or severity busts all keys for that rule.
- **Model-independent**: the model name is stored as metadata, not in the key.  Changing
  the model does *not* bust the cache.  Run `agent-rules cache clear` if you upgrade
  models and want fresh verdicts.

## Verifying cache hits in CI logs

Cache hits are reflected in the summary line printed after each run:

```
Found 2 issues (1 error, 1 warning) in 8 files (6 cached) [claude-haiku-4-5, 1.2s]
```

A full cache-hit run for a 50-file PR should complete in under 5 seconds.

## GitHub Actions setup

Copy `examples/github-actions.yml` to `.github/workflows/agent-rules.yml`.

### Cache key strategy

```yaml
- name: Restore cache
  uses: actions/cache/restore@v4
  with:
    path: .agent-rules-cache
    key: agent-rules-${{ github.sha }}
    restore-keys: |
      agent-rules-${{ github.event.pull_request.base.sha }}-
      agent-rules-
```

**Exact key** (`agent-rules-<sha>`): used for the save step.  Each commit gets its own
slot, so parallel runs on different SHAs never clobber each other.

**Prefix fallbacks** (the `restore-keys` lines): evaluated in order.

1. `agent-rules-<base-sha>-` — tries to restore the cache from the base branch commit.
   On a freshly opened PR this gives you verdicts for all unchanged files immediately.
2. `agent-rules-` — last-resort fallback to any prior cache.  Still useful; most files
   are unchanged across PRs.

### Save even on failure

```yaml
- name: Save cache
  if: always()
  uses: actions/cache/save@v4
  with:
    path: .agent-rules-cache
    key: agent-rules-${{ github.sha }}
```

`if: always()` ensures verdicts for passing files are persisted even when the workflow
fails on a reject.  Without this, a single reject wipes the cache benefit for all other
files on the next push.

## Busting the cache

| Situation | Action |
|-----------|--------|
| Upgraded the LLM model and want fresh verdicts | `agent-rules cache clear` locally; delete the `agent-rules-` Actions cache via the GitHub UI or API |
| Changed a rule's prompt significantly | Cache auto-busts for that rule (key includes prompt content) |
| Suspect stale verdicts | `agent-rules cache clear --yes` in CI by adding a one-off step |
| Cache entry corrupted | Delete `.agent-rules-cache/` directory; each entry is a single JSON file written atomically, so at worst one entry is corrupt and will be skipped and re-evaluated |

## Cache size

Each verdict entry is roughly 500 bytes of JSON.  A project with:
- 200 files checked per PR
- 10 rules per file
- 500 PRs/month

…accumulates ~200 MB over its lifetime — well under GitHub Actions' 10 GB cache limit.

You can inspect the local cache at any time:

```bash
agent-rules cache stats
```

```
Cache statistics  (.agent-rules-cache)
  Total entries : 1423
  Total hits    : 8901
  Oldest entry  : 14.2d ago
```

## Local development

The cache is also active in local runs.  It lives in `.agent-rules-cache/` relative to
the repo root.  Add it to your `.gitignore`:

```
.agent-rules-cache/
```

To disable caching for a single run:

```bash
agent-rules check --base main --head HEAD --no-cache
```
