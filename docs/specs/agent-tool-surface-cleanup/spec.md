# Spec: Agent Tool Surface Cleanup

- **Status:** Shipped
- **Owner:** phanijapps
- **Plan:** [`plan.md`](plan.md)
- **Constrained by:** RFC-0014 context capability registry notes
- **Brief:** none
- **Contract:** none
- **Shape:** mixed

> **Spec contract:** this document defines what "done" means. The implementing
> PR must match this spec, or update it. Verification must be derivable from it.

<!-- **Light-mode lean fill.** For low-risk work running the `work-loop`
skill's light mode, only Objective + Acceptance Criteria + a short task list
(in `plan.md`) are required. **Boundaries**, **Testing Strategy**, and
**Assumptions** are optional — keep them only if they earn their place. Any
risk trigger (see the `work-loop` skill) escalates to full mode, where every
section is filled. -->

## Objective

Remove dead model-tool code from `agent-tools` and the stale settings/UI surface
that still describes it. Success means the tool surface matches the live gateway
executor: only tools that can actually be registered by the running system remain
in production code, obsolete factories and optional tool toggles are gone, and
the product API/UI keep working without advertising controls that no longer do
anything.

## Boundaries

The three-tier guard that keeps an implementing agent inside the lines.
*Always do* applies without asking; *Ask first* requires human sign-off
before proceeding; *Never do* is a hard rule, even under time pressure.

### Always do

- Delete stranded tool implementations, exports, tests, docs, and config fields
  once repo search and the live gateway registry confirm they are not reachable.
- Keep the live gateway registry in `gateway-execution` authoritative for model
  tools and verify it still registers the retained tools after cleanup.
- Keep the product agent-management HTTP API, including `POST /api/agents`,
  separate from model-visible agent creation tools.
- Keep `file_tools` only for live behavior that the gateway still uses; remove
  its legacy `write`/`edit` meaning.
- Update UI transport types and tests whenever backend settings fields are
  removed.

### Ask first

- Removing or renaming a tool that `build_tool_registry` still registers.
- Removing product APIs used by the dashboard, CLI, or daemon.
- Changing model-visible names or schemas for retained tools.

### Never do

- Preserve dead code for hypothetical compatibility without a live caller.
- Leave accepted API fields or UI toggles that no longer change runtime behavior.
- Re-introduce `core_tools`, `optional_tools`, or `builtin_tools_with_fs` as a
  second registry path.
- Delete `ReadTool`, `WriteFileTool`, `EditFileTool`, `ShellTool`,
  `LoadSkillTool`, `UpdatePlanTool`, memory, ward, graph, ingest, goal,
  multimodal, subagent orchestration, or connector tools unless the live gateway
  registry no longer uses them.

## Testing Strategy

- **Goal-based checks:** `rg` proves deleted symbols and settings fields are
  absent from production code; `cargo check` proves retained registry wiring
  still compiles.
- **Integration checks:** targeted Rust tests cover settings persistence and
  gateway registry construction where existing tests already exercise those
  seams.
- **UI type/test checks:** TypeScript tests or typecheck prove the settings
  transport shape no longer expects removed fields.

## Acceptance Criteria

<!--
The verifiable goals that close this spec. Each item should be checkable
without subjective judgement — a reviewer can read it and know whether it
holds. Notation: `- [ ]` open, `- [x]` met (see CONVENTIONS § 4 Spec
metadata contract).

Two recurring sources of criteria, so they don't slip into the plan as
mere design detail:

- A **UI state** is an acceptance criterion: phrase it as
  *state / trigger / outcome* — "given <state>, when <trigger>, the user
  sees <outcome>" (e.g. "given an empty cart, when the page loads, the
  user sees the empty-state illustration and a 'browse' link"). The
  per-screen design itself lives in the plan's `## Design (LLD)`; the
  observable state belongs here.
- A **non-functional requirement with a pass/fail bar** is an acceptance
  criterion: it must name a threshold a test or audit can check —
  "meets WCAG 2.2 AA", "p99 latency under 200ms at 1k rps", "zero criticals
  in the dependency scan". An NFR with no bar ("should be fast") is not a
  criterion; give it a number or move it to the plan.

- [x] Production Rust no longer contains `ListAgentsTool`, `CreateAgentTool`,
  `PythonTool`, `WebFetchTool`, `RequestInputTool`, `ShowContentTool`,
  `ExecutionGraphTool`, legacy `WriteTool`, legacy `EditTool`, `core_tools`,
  `optional_tools`, or `builtin_tools_with_fs`.
- [x] Gateway execution still compiles and the live registry retains the tools
  it manually registers today.
- [x] Settings API and UI transport types no longer expose no-op
  `python`/`python_enabled`, `web_fetch`/`web_fetch_enabled`, `ui_tools`,
  or `create_agent` toggles.
- [x] `file_tools` no longer implies legacy `write`/`edit`; any remaining use is
  tied only to currently registered gateway behavior.
- [x] `runtime/agent-tools/AGENTS.md` and relevant RFC implementation notes no
  longer describe the deleted factories or retired tools as active.
- [x] `cargo check -p agent-tools -p gateway-execution -p gateway` passes.

A criterion that ships unmet *on purpose* is never left silently unchecked —
mark it deferred with an inline anchor into the backlog register:

- [ ] <observable outcome> (deferred: <backlog-anchor>)

where <backlog-anchor> resolves to a heading in `docs/backlog.md`.

Optional story trace: when this spec was derived from a product brief that
carries user stories (Shape B; see receive-brief), append `Satisfies: US-n`
to each acceptance criterion that satisfies that story, so coverage is
story-granular:

- [x] <observable outcome>. Satisfies: US-2

The marker is optional — omit it for a no-stories brief (Shape A) or a spec
authored directly.
-->

## Assumptions

- Technical: The live gateway executor builds its registry manually and does
  not call `core_tools`, `optional_tools`, or `builtin_tools_with_fs`. (source:
  `gateway/gateway-execution/src/invoke/executor.rs`)
- Technical: Old optional model tools are only referenced by unused factories,
  exports, their own tests, and docs. (source: `rg` and codegraph dead-code
  probes, 2026-07-09)
- Technical: Product agent management is a separate HTTP API and is not the same
  as the model-visible `CreateAgentTool`. (source:
  `gateway/src/http/agents.rs`)
- Product: The user wants dead code deleted rather than retained for possible
  compatibility. (source: user confirmation 2026-07-09)
