# Plan: Agent Tool Surface Cleanup

- **Spec:** [`spec.md`](spec.md)
- **Status:** Done

> **Plan contract:** this is the implementation strategy. Unlike the spec, this
> document is allowed to change as you learn. When it changes substantially
> (a different approach, not just a re-ordering), note why in the changelog
> at the bottom.

<!-- **Light-mode lean fill.** For low-risk work running the `work-loop`
skill's light mode, only Approach + a short Tasks list are required.
**Constraints**, **Risks**, **Changelog**, and the whole `## Design (LLD)`
section are optional — keep them only if they earn their place. Any risk
trigger (see the `work-loop` skill) escalates to full mode, where every
section is filled. -->

## Approach

Delete from the inside out. First remove the unreachable tool implementations,
factory helpers, and re-exports from `agent-tools`, keeping only tools that the
gateway executor actually registers. Then shrink settings and UI transport types
so they only expose live runtime knobs. Documentation updates happen after the
code shape is final, and the cleanup closes with search gates plus targeted Rust
and UI verification. The riskiest part is accidentally deleting a product API or
a dynamically registered tool, so every deletion is checked against
`build_tool_registry` before it lands.

## Constraints

- RFC-0014 notes classify `execution_graph`, `python`, `web_fetch`,
  `request_input`, and `show_content` as remove/redesign/quarantine candidates.
- `gateway-execution` is the source of truth for model-visible tool
  registration.
- Product agent HTTP APIs stay in place; only model-visible dead tools are
  removed.

## Construction tests

Most construction tests live under **Tasks** below (per-task `Tests:`
subsections). This top-level section is only for cross-cutting tests that
span tasks.

<!--
Construction tests guide implementation. They sit in two layers:

1. **Per-task tests** (the majority) live under each Task below, in the
   `Tests:` subsection. That's where unit, edge-case, and property tests
   for a single task go.
2. **Cross-cutting tests** (this section) live here, listed once: integration
   tests that span tasks, end-to-end smoke tests, and any manual verification
   steps.

Designed up front, before EXECUTE. Revisable if a test over-specifies an
internal detail the plan later changes. The contract itself lives in
`spec.md` (Acceptance Criteria + Testing Strategy); construction tests
that verify it live here.

**Integration tests:** `cargo check -p agent-tools -p gateway-execution -p gateway`
after Rust cleanup; targeted settings tests after API shape changes.
**Manual verification:** `rg` absence checks for deleted symbols and stale
settings fields.

## Design (LLD)

### Design decisions

- The gateway executor remains the only runtime composition point for model
  tools. `agent-tools` supplies concrete tool types, not alternate registry
  factories.
- Old same-purpose aliases are deleted instead of hidden. The retained write and
  edit path is `write_file`/`edit_file`; the retained read path is `read`.
- Settings are treated as runtime contracts only when a running component reads
  them. Fields that only fed retired factories are removed from backend and UI
  types.

### Interfaces & contracts

No new formal contract file is introduced. The existing settings JSON shape is
reduced by removing dead fields. Retained fields keep their current names unless
a live caller requires a follow-up rename.

### Component / module decomposition

- `runtime/agent-tools`: concrete retained tools, shared `ToolSettings`, and
  public re-exports.
- `gateway/gateway-execution`: live tool registry, unchanged except compile
  fixes caused by removed exports.
- `gateway`: settings request/response conversion.
- `apps/ui`: transport settings types and tests.

## Tasks

The work-breakdown. Tasks are sized so each one is a coherent commit or PR.
**Phrase each task as a verifiable goal, not a procedure.** The task name
*is* the success criterion: *"Add validation"* → *"All invalid-input tests
pass"*; *"Refactor X"* → *"Tests for X green before and after; public
surface unchanged"*. **Within each task, `Tests:` comes before `Approach:`** —
tests drive implementation, not the other way around. Use red-green-refactor
with separate commits when the change is non-trivial.

**Every task must declare `Depends on:` explicitly** — list prior task IDs
or `none`. Don't omit the field; "obvious from order" is the failure mode
that hides serial-by-default thinking. `none` is a valid and common answer.

**`Depends on:` grammar** (so the supervisor-mode scheduler —
`loop-cohort schedule` — can read it). The field is a comma-separated list of:
local task IDs (`T1`, `T1a`), ranges (`T1-T6`), or a **cross-spec marker**
`spec:<name>/TN` for a dependency on another spec's task (e.g.
`spec:auth-tokens/T7`). Parenthetical prose after the IDs is
ignored, so `T11 (lands after the shim)` is fine. Cross-spec deps are
*spec-sequencing*, not intra-plan waves, and are excluded from this plan's
DAG. The scheduler **fails on a dependency cycle** and **warns on a
forward-reference** (a dep authored later — it still schedules correctly by
running the dep first).

**Optional `Touches:` grammar** (read by `loop-cohort schedule`).
A task *may* add a `**Touches:**` line listing the file globs it expects to
touch — a comma-separated list of paths/globs (`src/api/*.py, docs/api.md`),
trailing prose ignored. `loop-cohort schedule` uses it to predict, per wave,
`predicted-disjoint: yes|no|unknown` **before** dispatch — a cheap
*serialize-only* screen. It **never greenlights** parallel: a predicted overlap
serializes early, but `yes`/`unknown` still require the authoritative post-write
`git merge-tree` check to actually parallelize (under-declaration is unsafe).
The field is **optional** — omit it freely; a task with no `Touches:` makes its
wave `unknown`, never an error.

### T1: Dead tool implementations and factories are gone from Rust production code

**Depends on:** none

**Status:** Done

**Touches:** runtime/agent-tools/src/tools/**, runtime/agent-tools/src/lib.rs

**Tests:**
- `rg -n "\b(ListAgentsTool|CreateAgentTool|PythonTool|WebFetchTool|RequestInputTool|ShowContentTool|ExecutionGraphTool|WriteTool|EditTool|core_tools|optional_tools|builtin_tools_with_fs)\b" runtime/agent-tools/src` returns no production-code hits.
- `cargo check -p agent-tools` passes.

**Approach:**
- Remove unused modules: `agent.rs`, `web.rs`, `ui.rs`, and
  `execution/graph.rs`.
- Remove `PythonTool` from `execution/mod.rs`.
- Remove legacy `WriteTool` and `EditTool` from `file.rs`; keep `ReadTool`.
- Remove factories and exports from `tools/mod.rs` and `lib.rs`.

**Done when:** deleted symbols are absent from `runtime/agent-tools/src` and
`agent-tools` compiles.

### T2: Settings and UI no longer expose no-op tool toggles

**Depends on:** T1

**Status:** Done

**Touches:** runtime/agent-tools/src/tools/mod.rs, gateway/src/http/settings.rs, apps/ui/**

**Tests:**
- `rg -n "(python_enabled|web_fetch_enabled|ui_tools|create_agent|webFetch|uiTools|createAgent)" gateway apps/ui runtime/agent-tools/src` has no production-code hits for retired settings.
- Targeted settings tests pass.

**Approach:**
- Shrink `ToolSettings` to live fields.
- Remove retired fields from gateway request/response conversion and OpenAPI or
  typed docs if present.
- Update UI transport types and tests to match the new settings shape.
- Keep `file_tools` only if it still gates live gateway behavior; clarify its
  meaning in comments and docs.

**Done when:** backend and UI compile/test against the reduced settings shape.

### T3: Documentation describes the current tool surface only

**Depends on:** T1-T2

**Status:** Done

**Touches:** runtime/agent-tools/AGENTS.md, docs/rfc/0014-notes/**, docs/specs/README.md

**Tests:**
- `rg -n "(core_tools|optional_tools|builtin_tools_with_fs|ExecutionGraphTool|WebFetchTool|PythonTool|RequestInputTool|ShowContentTool|CreateAgentTool|ListAgentsTool)" runtime/agent-tools/AGENTS.md docs/rfc/0014-notes docs/specs/README.md` returns no active-tool descriptions.

**Approach:**
- Rewrite `runtime/agent-tools/AGENTS.md` around retained concrete tools and
  gateway-owned registration.
- Update RFC implementation notes so removed tools are recorded as retired, not
  current.
- Add this spec to the active specs index.

**Done when:** docs match the retained code and do not advertise retired tools.

### T4: Cleanup gates prove there are no dangling references

**Depends on:** T1-T3

**Status:** Done

**Tests:**
- `cargo check -p agent-tools -p gateway-execution -p gateway`
- UI test/typecheck command selected from existing package scripts.
- Codegraph rescan completes and targeted deleted symbols do not appear as live
  entities.

**Approach:**
- Run compile and targeted UI gates.
- Run final `rg` checks for deleted tool names and retired settings.
- Refresh codegraph after the tree is clean.

**Done when:** all gates pass or any remaining failure is documented as unrelated
existing breakage with evidence.


## Rollout

Big-bang code cleanup on the cleanup branch. No data migration or infrastructure
change is required. Rollback is a git revert of the cleanup commit.

## Risks

- A dynamically loaded consumer outside this workspace may have imported old
  public re-exports. This repo has no live caller, and the user explicitly chose
  deletion over compatibility.
- `file_tools` has overloaded history. The implementation must not remove it if
  the live gateway still uses it for `glob`.
- Some docs intentionally mention retired tools historically. Search gates should
  distinguish active descriptions from historical notes.

## Changelog

- 2026-07-09: initial deletion-first plan after codegraph and `rg` confirmed
  the legacy factory surface is unreachable from the live gateway executor.
- 2026-07-09: shipped cleanup; Rust/UI compile and targeted tests passed, and
  codegraph was refreshed against the cleaned tree.
