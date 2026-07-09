# Structured Output Passthrough & Executor Simplification

**Date:** 2026-05-10
**Branch:** `feat/structured-output-and-executor-simplification`
**Context:** Gap analysis from [philschmid.de/subagent-patterns-2026](https://www.philschmid.de/subagent-patterns-2026)

---

## Problem 1 — Structured Output Passthrough

### What's broken

When a parent agent delegates to a subagent with `output_schema`, the subagent's JSON result
is wrapped in a markdown code fence and inserted back into the parent's context as a plain
system message. The parent must substring-parse markdown to recover the JSON — no structured
field, no guarantee, no schema enforcement.

**Current flow (lossy):**

```
Subagent emits:  {"analysis": "...", "score": 0.9}
                            ↓
callback.rs:61   format_response_content() → "## From child-agent\n\n```json\n{...}\n```"
                            ↓
callback.rs:168  conversation_repo.append_session_message(role="system", content=<markdown>)
                            ↓
Parent's turn:   Sees a string; must regex-extract JSON from the code fence
```

**Key files:**

| Symptom | File | Lines |
|---------|------|-------|
| `format_response_content()` wraps JSON in markdown fence | `gateway/gateway-execution/src/delegation/callback.rs` | 61–71 |
| `format_callback_message()` assembles the full system message | `callback.rs` | 75–106 |
| `validate_delegation_response()` checks JSON parsability, not schema conformance | `callback.rs` | 133–150 |
| Callback inserted as plain text system message | `callback.rs` | 160–197 |
| `output_schema` defined but schema validation absent | `gateway/gateway-execution/src/delegation/spawn.rs` | 194–200 |

### Fix design

**Goal:** When `output_schema` is set on a delegation, the callback message carries the
validated JSON in a machine-readable field the parent can extract without parsing markdown.

**Approach — dual-mode callback message:**

1. **Keep the human-readable markdown** for the parent LLM's context window (unchanged).
2. **Add a structured envelope comment** at the start of the system message that the
   callback reader can extract deterministically:

```
<!-- structured-result
{"ok":true,"data":{...subagent json...},"agent":"code-agent","schema_valid":true}
-->
## From code-agent
...existing markdown...
```

3. **Wire schema validation**: use `jsonschema` crate (already in workspace?) to actually
   validate the subagent response against `output_schema`. Set `schema_valid: false` if it
   fails, so the parent can decide whether to re-delegate.

4. **Expose a helper** `extract_structured_result(message: &str) -> Option<Value>` that
   parses the HTML comment envelope — usable by tests and by the memory recall path.

**Why this approach:**
- Zero risk: parent LLM sees identical markdown context window, no prompt regression.
- Additive: callers that don't use `output_schema` see no change.
- No new message types or DB schema changes needed.

**Files to change:**

| File | Change |
|------|--------|
| `gateway/gateway-execution/src/delegation/callback.rs` | Add `format_structured_envelope()`, `extract_structured_result()`, wire into `format_callback_message()` when `output_schema` is Some |
| `gateway/gateway-execution/src/delegation/spawn.rs` | Pass `output_schema` through to callback formatter |
| `gateway/gateway-execution/Cargo.toml` | Add `jsonschema` if not present |

**Tests to add** (in `callback.rs` or new `tests/e2e_structured_output_tests.rs`):
- `structured_envelope_present_when_schema_set()`
- `no_envelope_when_no_schema()`
- `extract_structured_result_parses_envelope()`
- `schema_valid_false_when_response_violates_schema()`
- `parent_can_extract_json_without_markdown_parsing()`

---

## Problem 2 — executor.rs Simplification

### What's complex

`runtime/agent-runtime/src/executor.rs` is 4161 lines with at least 5 distinct
responsibilities mixed together. 1714 lines are tests (which stay). The remaining ~2447
lines of logic contain:

| Responsibility | Approx lines | Natural module |
|----------------|--------------|----------------|
| Core LLM loop (`execute_with_tools_loop`) | ~887 | stays in executor (entry point) |
| **ProgressTracker** — loop diagnostics, extension grants, stuck detection | ~293 | `progress.rs` |
| **Context management** — compaction, truncation, sanitize | ~318 | `context_management.rs` |
| **Tool execution** — routing, schema hardening, parallel dispatch | ~200 | `tool_executor.rs` |
| **Schema helpers** — `harden_tool_schema`, `normalize_mcp_parameters`, `build_tools_schema` | ~105 | `tool_executor.rs` |

**Line markers:**
- `ProgressTracker` struct + impl: lines 1757–2050
- Compaction/truncation helpers: `compact_messages` (2127–2285), `truncate_tool_result` (2286–2340), `sanitize_messages` (2341–2444)
- Tool schema builders: lines 1534–1638
- `execute_tool()` core: lines 1398–1533

### Fix design

Same incremental approach as stream.rs: extract easiest modules first, leave core loop
last, keep executor.rs as re-export shim until each piece is stable.

**Extraction order:**

1. **`progress.rs`** — `ProgressTracker` struct + full impl (self-contained, no mut executor state). Extract first because it's the cleanest seam.
2. **`context_management.rs`** — `compact_messages`, `truncate_tool_result`, `sanitize_messages`. Pure functions, testable in isolation.
3. **`tool_executor.rs`** — `execute_tool()`, `harden_tool_schema()`, `normalize_mcp_parameters()`, `build_tools_schema()`. Needs `ExecutorConfig` reference; pass by ref.

After all three extracted, executor.rs core loop shrinks from ~2447 to ~950 lines of logic
(plus tests), well within readable range.

**Files to create:**

| New file | Extracted from | Lines saved |
|----------|---------------|-------------|
| `runtime/agent-runtime/src/progress.rs` | executor.rs:1757–2050 | ~293 |
| `runtime/agent-runtime/src/context_management.rs` | executor.rs:2127–2444 | ~318 |
| `runtime/agent-runtime/src/tool_executor.rs` | executor.rs:1398–1638 | ~241 |

**Files to modify:**

| File | Change |
|------|--------|
| `runtime/agent-runtime/src/executor.rs` | Replace extracted sections with `use super::` re-exports, remove ~850 lines |
| `runtime/agent-runtime/src/lib.rs` | Add `pub(crate) mod progress; pub(crate) mod context_management; pub(crate) mod tool_executor;` |

---

## Implementation Order

1. **Structured output passthrough** (callback.rs changes — smaller, higher impact)
2. **executor.rs — extract ProgressTracker** (safest seam, self-contained)
3. **executor.rs — extract context_management** (pure functions)
4. **executor.rs — extract tool_executor** (needs executor config ref)

Each step: write failing test → implement → cargo check → commit.
