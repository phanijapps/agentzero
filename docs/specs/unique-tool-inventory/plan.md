# Plan: Unique Model Tool Inventory

- **Spec:** [`spec.md`](spec.md)
- **Status:** Done

> **Plan contract:** this is the implementation strategy. Unlike the spec, this
> document is allowed to change as you learn. When it changes substantially,
> note why in the changelog at the bottom.

## Approach

Encode tool-name uniqueness as an invariant over the real gateway registry builder before touching production code. Then remove only the stale second `ReadTool` registration while retaining the `GlobTool` registration and the capability-gated primary `ReadTool`. Verify actor membership, strict request validation, and the full workspace gates.

## Constraints

- `ToolRegistry` append semantics and provider-boundary strict validation remain unchanged.
- Actor capability policy remains the authority for built-in tool membership.
- The change stays independent of MCP naming and user configuration.

## Construction tests

**Integration tests:** The executor-module regression test builds the actual first-party registry for all runtime actor kinds under both file-tools settings and compares every raw name frequency to a checked-in pre-fix characterization baseline whose expected result differs only by reducing redundant `read` counts to one. The runtime request-preparation test asserts the exact duplicate-name rejection rule remains fail-closed.

**Manual verification:** Inspect the real built registry through the focused gateway-execution test and confirm ward/reviewer `read` and `glob` membership while schema preparation stays strict. A live provider call is intentionally unnecessary because the reported rejection occurs locally before network I/O.

## Design (LLD)

### Design decisions

- Enforce uniqueness in a regression test while fixing the registration site; do not turn the registry into an implicit last-write-wins map. Traces to: AC1, AC4.
- Keep the first capability-gated `ReadTool` and the legacy block's `GlobTool`; remove only its redundant `ReadTool`. Traces to: AC1, AC2, AC3, AC5.

### Interfaces & contracts

No external contract changes. The internal invariant is that each model-visible tool name identifies exactly one schema before request preparation. Traces to: AC1, AC4.

### Failure, edge cases & resilience

The regression matrix covers every actor kind with the file-tools flag on and off, including the ward/reviewer path that reproduced the failure. Characterized distinct-name baselines prevent unrelated tool-surface drift. Strict validation remains the fail-closed backstop for future built-in or MCP collisions. Traces to: AC1, AC2, AC4.

### Quality attributes (NFRs)

The fix adds no dependency, runtime process, network call, or configuration requirement. Traces to: AC5.

## Tasks

### T1: Every built-in actor registry has unique model-tool names without capability drift

**Depends on:** none

**Touches:** `gateway/gateway-execution/src/invoke/executor.rs`, `runtime/agent-runtime/src/llm/openai.rs`, `docs/specs/unique-tool-inventory/*`, `docs/specs/README.md`

**Mode:** TDD plus goal-based checks

**Tests:**

- `built_in_registry_raw_name_frequencies_match_characterized_actor_inventories`: all four runtime actor kinds and both file-tools values match their pinned pre-fix frequency maps except each redundant `read` count is reduced from two to one (AC1, AC2, AC3). `stub: true`
- `duplicate_tool_names_are_rejected_by_exact_rule_code`: provider request preparation continues to reject a deliberately duplicated schema with exactly `tool_schema_rule=duplicate_name` (AC4). Goal-based construction check; `stub: false`.
- Focused gateway-execution and agent-runtime tests plus workspace format, check, Clippy, and test gates pass (AC1-AC5). Goal-based checks; `stub: false`.

**Approach:**

- Add frequency-based test diagnostics that identify any repeated raw registry names.
- Remove only the duplicate `ReadTool` registration from the conditional file-tools block.
- Keep strict provider request validation and all capability definitions untouched.

**Done when:** The regression matrix and workspace gates pass, reviewers find no unresolved issue, and all acceptance criteria have recorded evidence.

## Rollout

This is a direct code fix with no flag, migration, infrastructure, secret, or external-system dependency. Reverting the single registration change restores the prior behavior; no data is changed.

## Risks

- A set-based assertion could erase duplicates and create a false green; the test therefore inspects raw entries and reports frequencies.
- Removing the wrong registration could hide `read` from an actor; explicit ward/reviewer membership assertions and the all-actor matrix guard against that.

## Changelog

- 2026-08-04: Initial TDD plan based on the reproduced ward-agent duplicate-name failure.
