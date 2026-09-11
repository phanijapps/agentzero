# Dimension 3 — Schema adherence + retry design

## Current state

- **Error path**: tool errors return `AgentError::Tool(msg)` → engine → model sees the raw string in the tool result; `SubagentGuardHook::after_tool` (policy.rs:272+) appends generic `[SYSTEM: Tool failed. Read the error...]` guidance on failures.
- **Schemas**: plain JSON Schema per tool; **zero few-shot call examples anywhere** (grep: no example/`few.shot` in agent-tools descriptions or rig_adapter). Local models (glm/deepseek via Ollama) get schema-only.
- **No per-provider adaptation** — same descriptions/schemas to every provider.
- **Shape errors don't teach**: `"Missing 'name' parameter for use/create"` (ward.rs:1261) — names the missing field but not the full correct call. Forensics: **5 consecutive identical retries** of this exact error in one execution — the model cannot reconstruct the shape from "missing name". The multimodal fix (reports received shape + shows the wrap) recovered in 2 calls — one data point that shape-teaching errors work.
- Ward schema requires `action`+`name` but the SUBAGENT audience description lists actions without an args example, and `reject_unknown_fields` adds a second failure axis (models add fields, get rejected).

## Why local models fumble

Weak-schema models bind to *patterns in context*, not JSON Schema documents. Two reliable lifts in harness practice: (a) a literal example call in the tool description, (b) server-side coercion for canonical fumbles. zbot has neither; it relies on error-text iteration, which costs 3–6 turns per fumble class per session (forensics: ward ~237 errors, present_surface 54, update_plan 21).

## Highest-leverage changes (ranked, file-mapped)

1. **Example-line in the description of the top fumble tools** (ward, present_surface, update_plan, multimodal): one `Example: {"action":"use","name":"finance"}` line. Kills the shape-blind-retry loop the same way the multimodal error fix did, but *before* the first failure. Files: `tools/ward.rs` (audience descriptions), `tools/present_surface.rs`, `execution/update_plan.rs`. Effort **S**. Expected: ward-class errors −80%.
2. **Server-side canonical coercion**: `content: "url"` → `[{type:"image",source:"url"}]`; `name` bare string for ward use/create when `action` present; plan passed as object → wrap array. A tiny `coerce` step in each tool's execute (never silently for destructive ops). Files: multimodal.rs, ward.rs, update_plan.rs. Effort **S–M**. Removes the error class for common fumbles entirely.
3. **Error-as-affordance standard**: every `Missing 'X'` error carries the minimal correct fragment (the multimodal pattern, applied to ward/present_surface/update_plan). Files: same + policy.rs hook could echo the tool's example line on repeated failure (it already tracks failing calls). Effort **S**.

Rationale for ranking: (1) prevents, (2) forgives, (3) teaches-after-fail. Prevention is cheapest per token.
