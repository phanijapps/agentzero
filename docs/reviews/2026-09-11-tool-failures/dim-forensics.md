# Dimension 1 — Forensics (all sessions, data-driven)

Sources: `~/Documents/zbot/data/traces/*.jsonl.gz` (5,024 events, 30 sessions with tool calls) + `conversations.db` messages joined to `agent_executions` for agent attribution.

## lookup_capabilities

| Metric | Value |
|---|---|
| Total calls (traces) | **25** (fd588249 shows 12 — traces rotated/compacted; messages DB corroborates ~25) |
| By agent | **planner-agent: 25 (100%)** |
| Sessions using it | 3 of 30 (10%) — but these are the graph/planner-routed sessions |
| Calls per using-session | 4, 8, 13 — **≥3 in 100% of using sessions** |
| Query clusters | roster-discovery ~100%: "python code generation image diffusion", "coding", "developer", "executor builder implementer", "data analyst", "agent" (limit 50/25), "spec-builder plan-composer" |

**Sharpest fact 1**: lookup_capabilities is *exclusively a planner phenomenon*, and every query is "who can do X" — enumeration-via-semantic-search of a ~10-item roster. Zero self-capability queries from root or executors in the sampled window.

## Tool-call errors (messages DB, all history)

Taxonomy by tool + class (top classes; "other" rows dominated by wrapper text `Tool execution failed: Tool(...)` — inner message parsed below):

| Class | Count | Dominant tools |
|---|---|---|
| ward shape (missing `action` 132 / missing `name` for use-create 92 / missing name lint 13) | **~237** | ward (delegated executors + ward agents) |
| shell timeout | 101 | shell |
| file-not-found (read 34 / exec 20) | 54 | read, shell |
| present_surface component shape (invalid property 30 / invalid shape 13 / unsupported property 11) | 54 | present_surface |
| memory_write content-too-long | 23 | memory_write (policy) |
| update_plan missing array | 21 | update_plan |
| multimodal shape ('content' must be array) | 1 in fd588249 (+ civitai URL environmental 1) | multimodal |
| steering size cap | 1 | steer_agent (policy) |

By agent: builder-agent 310, root 211, ward agents ~145, research-agent 65 — errors concentrate in **delegated executors**.

**Sharpest fact 2**: the #1 recurring error is the **ward tool's parameter contract** (~237 occurrences — an order of magnitude above any other).

## Retry behavior

**Sharpest fact 3**: consecutive identical retries — execution `exec-04114647` rows 4327→4335: **five back-to-back `Missing 'name' parameter for use/create` errors**, same call shape, no adaptation between retries. The error text does not teach the correct form, and the model does not infer it.

**Sharpest fact 4**: multimodal's shape error (fd588249) recovered in ~2 calls; single-occurrence classes self-heal fast — repeated-failure classes are exactly those whose error text lacks the target shape.

**Sharpest fact 5**: `[SYSTEM: Tool failed…]` failure-feedback injections appear ~200× in history — the remediation loop exists and fires, but generic "read the error" guidance cannot compensate for error text that omits the correct argument shape.

Model attribution: not present in trace/message payloads (provider logged at session start only: glm-5.2:cloud, deepseek-family via Ollama). Cannot split error rates by model from this data — noted as a telemetry gap.
