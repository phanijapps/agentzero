# Plan: Planner Template Handoff

- **Spec:** [`spec.md`](spec.md)
- **Status:** Done

## Approach

Close the handoff at the trusted executor boundary: after the automatic planner
child is bound to the selected ward, its executor independently reloads the
canonical snapshot through `GatewayWardLayoutAccess` and injects the existing
normalized context. Tighten the bundled planner/spec/plan skill contracts so a
concrete refinement run consumes declared roles, while absent-role and
sparse-scaffold behavior remain intact. Give only that exact planner identity
read-only lint authority when its host-loaded packet matches the child session
and selected ward; all mutating template actions remain root-only.

## Constraints

- Reuse `WardLayoutState.context`; do not transport template authority through model-supplied `DelegateAction.context` or sanitize the packet a second time.
- Preserve RFC-0016's generic logical-role model and fail-closed absent-role behavior.
- Preserve initial ward creation semantics and all warm/simple routing behavior.
- Add no dependency or new module boundary.
- Do not broaden delegated access to Ward search, dry-run, or concept mutation.

## Construction tests

**Integration tests:** existing `agent-tools`, `agent-runtime`, and `gateway-execution` ward/planning suites.

**Manual verification:** inspect the captured planner executor instruction for a fixture whose normalized parent scope is optional and repeatable and contains `spec.md`, `plan.md`, and repeatable Markdown tasks with no task index. Confirm the instruction contains the delimited adapter context, digest, selected ward, and persistence directive without raw YAML or absolute template paths.

## Design (LLD)

### Interfaces & contracts

`GatewayWardLayoutAccess::state` already returns `WardLayoutState { context,
packet }`. `ExecutorBuilder` is the trusted boundary that knows the host-loaded
agent identity and host-bound ward, so it loads planner template authority
there. This satisfies AC1–AC2 without widening `DelegateAction` or accepting
model-supplied template context.

### State & control flow

Cold graph intent stores the planner goal in the planning gate. Successful ward
entry consumes the gate and emits one sequential planner delegate. Spawn binds
the planner child to the selected ward; executor construction reloads that
ward's normalized snapshot, fails closed if it is unavailable, and injects the
delimited data context. Planner output then resumes the root through the
existing continuation path. This satisfies AC1–AC4 and AC7–AC9.

### Failure, edge cases & resilience

An unavailable or invalid normalized context stops planner construction
with a bounded error and cannot cause raw-YAML or ephemeral execution fallback.
If the snapshot becomes unavailable after Ward entry but before child executor
construction, the failed planner is recorded and its parent continuation is
suppressed and the paused root execution is terminated with a bounded
`planner_startup_failed` lifecycle error. The session is terminalized as
crashed and can be reactivated by a later user retry, so neither a fresh
ungated root executor nor a stranded running turn remains. This handling does
not depend on the root having requested continuation before the child fails.
A valid template that genuinely lacks persistent roles remains valid and uses
the ephemeral plan. This satisfies AC4–AC7.

### Dependencies & integration

The change reuses `agent-tools` ward state, gateway ward-layout normalization,
and embedded planner skills. There are no external dependencies or deployment
ordering requirements.

## Tasks

### T1: Planner executor carries the selected normalized template

**Depends on:** none

**Touches:** `gateway/gateway-execution/src/invoke/executor.rs`

**Tests:**

`stub: true`

- `gateway/gateway-execution/src/invoke/executor.rs::planner_executor_receives_selected_template_packet_and_prompt` proves the injected template suffix equals `GatewayWardLayoutAccess::state(...).context` byte-for-byte and carries the digest/ward without raw YAML or absolute paths (AC1–AC2, AC8–AC9).
- `gateway/gateway-execution/src/invoke/executor.rs::planner_executor_fails_closed_when_selected_ward_is_missing` proves a selected ward with no loadable snapshot stops construction before model execution (AC6).
- `gateway/gateway-execution/src/invoke/executor.rs::planner_executor_fails_closed_when_selected_template_is_invalid` proves invalid context stops construction before model execution (AC6).

**Approach:**
- Extend the existing executor template-isolation tests and confirm both stubs fail on the root-only load condition.
- Allow only root and the exact bundled `planner-agent` identity to load template context; keep all other delegated executors isolated.

**Done when:** both red executor tests pass without changing `DelegateAction`'s schema or generic delegated-executor isolation.

### T2: Planner contracts persist applicable declared roles

**Depends on:** T1

**Touches:** `gateway/templates/agents/planner-agent.md`, `gateway/templates/skills/spec-builder/SKILL.md`, `gateway/templates/skills/plan-composer/SKILL.md`, `gateway/gateway-execution/tests/e2e_ward_pipeline_tests.rs`

**Tests:**

`stub: true`

- `gateway/gateway-execution/tests/e2e_ward_pipeline_tests.rs::planner_contract_requires_declared_refinement_artifacts_before_returning_steps` pins the exact optional-repeatable parent with `spec.md`, `plan.md`, repeatable Markdown tasks, no task index, selected ward/digest, lint-before-return, and absent-role degradation (AC3–AC5, AC8).
- Manual QA through a real planner invocation bound to `history-library`: request the `discovery-of-india` refinement, assert `spec.md` and `plan.md`, no placeholder task or task index, successful structured ward lint, returned output containing the selected ward/digest, and no refinement artifacts outside the selected ward. This is the observable persistence/confinement check for AC3 and AC7 and must be recorded before completion.

**Approach:**
- Tighten the generic planner and skill language around applicability versus absence.
- Keep all path choices resolved from the injected template rather than naming `.zbot/specs` in prompts.

**Done when:** embedded-template contract tests pin declared-role persistence, lint-before-return, and absent-role degradation.

### T2a: Planner can verify its selected ward without gaining template mutation authority

**Depends on:** T1

**Touches:** `runtime/agent-tools/src/tools/ward.rs`

**Tests:**

`stub: true`

- `runtime/agent-tools/src/tools/ward.rs::planner_with_matching_host_packet_can_lint_selected_ward` proves the exact planner may run read-only lint with a session/ward/context-bound packet (AC7).
- `runtime/agent-tools/src/tools/ward.rs::template_actions_reject_delegated_context` continues proving an ordinary delegate is rejected, and the planner remains rejected from non-lint template actions (AC7).

**Approach:**
- Extend the existing template-context validator with an action-scoped planner-lint allowance keyed by exact actor and agent identity.
- Retain all existing packet freshness checks and root-only authorization for search, dry-run, and create-concept.

**Done when:** live planner lint returns a structured report instead of `root_required`, while ordinary delegates and planner mutation attempts remain denied.

### T3: Cross-path gates prove sparse creation and execution parity

**Depends on:** T1, T2, T2a

**Touches:** no additional production files

**Tests:**
- Goal-based: `cargo test -p agent-tools -p agent-runtime -p gateway-execution -- --test-threads=1` (AC5–AC9).
- Goal-based: clippy with warnings denied, formatting check, spec-status lint when available, and `git diff --check`.

**Approach:**
- Run focused red/green tests first, then the complete affected-crate gates.
- Confirm the fresh-ward exact-root tests still prove optional directories remain lazy.

**Done when:** all affected-crate tests and mechanical gates are green.

## Rollout

Ships with the daemon rebuild. No migration or irreversible state change is
required; rollback is the code/template revert. Existing sessions are not
retroactively replanned.

## Risks

- Duplicating sanitization could create drift; reuse the adapter context verbatim.
- Over-broad prompt language could force artifacts in templates that do not declare them; keep applicability conditional on normalized roles.
- Automatic planner tasks are visible to models, so the untrusted-data delimiter must remain intact.

## Verification Evidence

- Live repair on session `sess-859d12e9-97de-5784-a622-3ea239a9ef54` delegated to planner execution `exec-f5cd595e-3545-4fe7-b866-1f06cee7e87c` and persisted only `history-library/.zbot/specs/discovery-of-india/spec.md` and `plan.md`; a vault-wide path check found no matching refinement artifacts in any sibling ward.
- Live read-only verification on planner execution `exec-f462ae8c-219e-40aa-bb70-48d2eef7f04e` returned ward `history-library`, template digest `ccd1377d42921db7e57d1f3d207c7bf337e72dc98ab2505d0edb0336f4a72226`, `ok: true`, `data.valid: true`, no findings, and no file changes.
- Fixture replays prove a template with no persistent planning roles remains ephemeral with no invented `.zbot` tree, while a spec-only template writes its declared `spec.md` and does not invent a fallback `plan.md`.
- The planner startup-race regression simulates `planner_template_unavailable` immediately after delegation registration and before any continuation request; it proves the failure callback is retained while pending delegation reaches zero, continuation remains disabled, the root/session become terminal, and a bounded root lifecycle error is published.
- Mechanical gates passed: formatting, affected-crate tests (including 427 `agent-runtime` tests, 588 `gateway-execution` unit tests, 14 Ward pipeline tests, and the cold-boot integration test), clippy with warnings denied, and `git diff --check`.

## Changelog

- 2026-08-18: initial plan based on session `sess-859d12e9-97de-5784-a622-3ea239a9ef54` and RFC-0016.
- 2026-08-18: implemented host-bound planner template reload, scoped planner lint authorization, declared-role persistence contracts, and live repair/verification.
- 2026-08-18: added absent-role/no-fallback fixture replays and closed the planner-construction race that could otherwise wake an ungated root continuation.
