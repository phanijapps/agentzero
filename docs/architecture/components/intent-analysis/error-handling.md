# Intent Analysis — Error Handling & Resilience

## Error Points and Fallbacks

### 1. Continuation Turn (Session Gate)

**Trigger**: `has_intent_log(execution_id)` returns true — session already analyzed.

**Behavior**: Entire intent analysis block is skipped. No events emitted. No LLM call. Agent proceeds without new analysis. This is the expected path for follow-up messages.

**Location**: `runner.rs` create_executor(), the `already_analyzed` check.

---

### 2. No Fact Store Available

**Trigger**: `memory_repo` is `None` or `embedding_client` is `None`.

**Behavior**: Intent analysis block is skipped. No events emitted. No logs. Agent proceeds.

**Location**: `runner.rs` create_executor(), the `if let Some(ref fs)` guard.

---

### 3. Resource Indexing Failure

**Trigger**: Service list or filesystem scan fails.

**Behavior**: Individual failures logged. Analysis continues with partial resources. Non-fatal.

**Location**: `intent_analysis.rs` `index_resources()`.

---

### 4. LLM Client Creation Failure

**Trigger**: `OpenAiClient::new()` returns Err.

**Behavior**: Minimal fallback event emitted:
- `primary_intent: "general"`, `ward_name: "scratch"`, `approach: "simple"`

**Location**: `runner.rs` create_executor(), inner Err match.

---

### 5. LLM Call or JSON Parse Failure

**Trigger**: `analyze_intent()` returns Err (network error, rate limit, invalid JSON).

**Behavior**: Same minimal fallback event emitted. No repair attempted — clean failure.

**Location**: `runner.rs` create_executor(), Err match on `analyze_intent()`.

---

### 6. JSON Wrapped in Markdown Fences

**Trigger**: LLM returns `` ```json ... ``` ``.

**Behavior**: `strip_markdown_fences()` removes wrapping before parsing. Transparent to caller.

**Location**: `intent_analysis.rs` inside `analyze_intent()`.

---

## Degradation Hierarchy

```
Full analysis (all fields, injected into agent prompt)
  ↓ continuation turn
Skip entirely (session already has intent)
  ↓ fact store missing
Skip entirely (no events, no logs)
  ↓ indexing fails
Analysis with partial resource lists
  ↓ LLM client or call fails
Minimal fallback event (general/scratch/simple) — NOT injected into prompt
  ↓ JSON parse fails
Same minimal fallback event
```

## Non-Fatal Guarantee

Intent analysis errors NEVER crash the agent execution. All error paths:
1. Log the error at warn level
2. Emit a fallback event (or no event if fact store absent / continuation turn)
3. Continue building the executor

The agent runs regardless. The analysis is enrichment, not a gate.

## What Changed (from previous version)

- **Removed `repair_truncated_json()`** — 120 lines of bracket-tracking code that could produce semantically broken JSON. Clean Err + fallback is safer.
- **Added session gate** — `has_intent_log(execution_id)` prevents redundant analysis on continuation turns.
- **Leaner prompt** — Fewer fields means shorter LLM output, reducing truncation risk.
