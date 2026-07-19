# Plan: Builder Delegation Hygiene

- **Spec:** [`spec.md`](spec.md)
- **Status:** Done

> **Plan contract:** this is the implementation strategy. Unlike the spec, this
> document is allowed to change as you learn. When it changes substantially
> (a different approach, not just a re-ordering), note why in the changelog
> at the bottom.

## Approach

Add an explicit delegation mode as a small runtime contract, then make prompts
consume that contract instead of guessing from prose. The core wiring runs from
`delegate_to_agent` schema parsing into `DelegationRequest`, spawn setup,
executor initial state, and mode-specific `subagent_rules`. Then update bundled
builder/planner templates and add a guarded default-agent refresh path so
existing live agents stop carrying stale doctrine only when they are still
effectively template-managed. Keep the first implementation slice schema-light:
no DB migration, no new persisted column, no new agent class, and no change to
ward-agent capability policy.

## Constraints

- Follows RFC-0005: Builder Delegation and Ward Context Hygiene.
- Builds on `docs/specs/subagent-role-gating/`: ward agents remain full-tool
  actors and ordinary delegated executors keep implementation tools.
- Uses a warning-only `delegate_to_agent` task-length policy: 4000 characters is
  preferred, 5000 is the recommended upper range, and longer tasks should point
  at ward/spec files instead of being pasted into the tool call.
- Does not add a user-facing agent config tool allowlist or denylist.
- Does not overwrite user-customized agent files.
- Does not make `DirectArtifact` a general ward-curation path.

## Construction tests

**Integration tests:** focused gateway-execution tests for mode inference,
spawn propagation, and mode-specific rule text. Agent-service tests for guarded
template refresh.

**Manual verification:** optional daemon smoke after implementation: ask for a
self-contained single-file web artifact and confirm builder writes first without
root-doc exploration. Not required for spec completion.

## Tasks

### T1: Delegation mode type and parser are explicit

**Depends on:** none

**Touches:** `gateway/gateway-execution/src/delegation/context.rs`,
`gateway/gateway-execution/src/invoke/setup.rs`

**Tests:**
- TDD for Acceptance Criteria 1 and 3: parse all four wire values and reject or
  ignore unknown values safely.
- TDD for mode inference: step-spec task -> `StepExecutor`, exact output path
  self-contained artifact task -> `DirectArtifact`, hygiene wording ->
  `WardHygiene`, fallback builder work -> `WardBackedBuild`.

**Approach:**
- Add a `DelegationMode` enum near delegation context or invoke setup.
- Implement `as_str`, parse from string, and a focused
  `infer_delegation_mode(agent_id, task, explicit_mode)` helper.
- Keep unknown explicit strings as tool errors if parsed at the tool boundary;
  keep internal inference conservative when no explicit mode exists.

**Done when:** unit tests prove all four modes and default inference behavior.

### T2: `delegate_to_agent` accepts and forwards mode

**Depends on:** T1

**Touches:** `runtime/agent-runtime/src/tools/delegate.rs`,
`gateway/gateway-execution/src/invoke/delegation_handler.rs`,
`gateway/gateway-execution/src/delegation/context.rs`

**Tests:**
- TDD for Acceptance Criteria 1 and 2: tool schema exposes optional `mode`, tool
  execution validates it, and the handler forwards it into `DelegationRequest`.
- Regression test: existing calls without `mode` still work.

**Approach:**
- Add optional `mode` to the tool schema with enum values
  `direct_artifact`, `ward_hygiene`, `ward_backed_build`, `step_executor`.
- Parse and forward mode through the existing delegation handling path.
- Add the mode to `DelegationRequest` and `DelegationContext`; avoid DB schema
  changes by keeping persisted task/execution records unchanged.

**Done when:** a tool-call unit test can observe mode on the emitted
`DelegationRequest`.

### T3: Spawn and executor state receive mode

**Depends on:** T1, T2

**Touches:** `gateway/gateway-execution/src/delegation/spawn.rs`,
`gateway/gateway-execution/src/invoke/executor.rs`,
`gateway/gateway-execution/src/invoke/setup.rs`

**Tests:**
- TDD for Acceptance Criteria 4: child executor initial state includes
  `app:delegation_mode`.
- TDD for omitted mode: spawn uses inferred mode.
- Regression tests from subagent capability policy still pass for actor kind.

**Approach:**
- Resolve effective mode in `spawn_delegated_agent` after actor kind detection.
- Pass mode into executor construction and set it in initial state.
- Keep actor kind and delegation mode separate: actor kind controls tool
  capability; delegation mode controls execution posture.

**Done when:** tests can assert the child execution context contains the
expected delegation mode.

### T4: Executor rules become mode-specific

**Depends on:** T1, T3

**Touches:** `gateway/gateway-execution/src/invoke/setup.rs`,
`gateway/gateway-execution/tests/e2e_ward_pipeline_tests.rs`

**Tests:**
- TDD for Acceptance Criteria 5-8: each executor mode returns distinct rules.
- Goal-based text check: no executor catch-all rule says every subagent must
  read `AGENTS.md + memory-bank/core_docs.md` before all work.
- Regression test: reviewer rules remain read-only and unchanged except for
  compatible formatting if needed.

**Approach:**
- Replace `subagent_rules(SubagentRole::Executor)` with a mode-aware helper,
  for example `subagent_rules(role, mode)`.
- `DirectArtifact`: write named outputs first, verify, declare/return artifacts,
  avoid unrelated docs.
- `WardHygiene`: fill missing/empty `AGENTS.md` and memory-bank files; preserve
  non-empty files.
- `WardBackedBuild`: read supplied snapshot/relevant ward files, reuse
  primitives, update `core_docs.md` only for reusable additions.
- `StepExecutor`: follow step spec, acceptance checks, and path tables.

**Done when:** focused tests prove prompt rules match the four mode contracts.

### T5: Builder and planner templates align with mode contract

**Depends on:** T4

**Touches:** `gateway/templates/agents/builder-agent.md`,
`gateway/templates/agents/planner-agent.md`,
`gateway/templates/default_agents.json`

**Tests:**
- Goal-based check for Acceptance Criteria 9: builder template mentions all
  four modes and includes a direct-artifact fast path.
- Goal-based check for Acceptance Criteria 10: planner template does not
  mention unavailable `solution-agent` or `writer-agent`; it uses discoverable
  default agent names.
- Existing default-agent seeding tests remain green.

**Approach:**
- Rewrite the builder "Working in a ward" section so docs are read based on
  mode, not unconditionally.
- Restrict zbot self-documentation reading to explicit zbot-product questions.
- Update planner hard rules to discover/use seeded agents:
  `planner-agent`, `builder-agent`, `research-agent`, `writing-agent`,
  `reviewer-agent`, and `general-purpose` as appropriate.
- Keep template edits concise so prompt size does not grow materially.

**Done when:** grep/snapshot checks and agent seeding tests pass.

### T6: Guarded live default-agent refresh preserves customization

**Depends on:** T5

**Touches:** `gateway/gateway-services/src/agents.rs`,
`gateway/templates/agents/*.md`

**Tests:**
- TDD for Acceptance Criteria 11: an existing agent whose normalized
  `AGENTS.md` matches a known old bundled signature is backed up and refreshed.
- TDD for Acceptance Criteria 12: a customized live `AGENTS.md` is not changed.
- TDD: refresh is idempotent after the first update.

**Approach:**
- Add a small default-template refresh helper separate from
  `seed_default_agents`.
- Compare normalized content or known SHA-256 signatures for bundled old
  templates. Treat trailing whitespace-only differences as template-managed.
- Before writing new instructions, write a timestamped `.bak` beside the live
  file.
- Call the refresh helper from the same startup/default-agent path that seeds
  agents, after seeding.

**Done when:** tests cover refresh, skip, and idempotency paths.

### T7: Documentation and spec index are current

**Depends on:** T1-T6

**Touches:** `docs/specs/README.md`,
`docs/architecture/components/subagent-capability-policy/overview.md`,
`docs/architecture/components/execution-loop/data-flow.md`,
`docs/rfc/0005-builder-delegation-and-ward-context-hygiene.md`

**Tests:**
- Goal-based check: `docs/specs/README.md` includes Builder Delegation Hygiene.
- Goal-based check: component docs mention delegation mode as prompt posture,
  distinct from actor capability policy.
- Goal-based check: RFC-0005 body is filled before implementation PR approval.

**Approach:**
- Add this spec to the spec index.
- Update execution-loop/component docs only where they describe delegation
  prompt/rule flow.
- Fill RFC-0005 body from the research checkpoint before marking the spec
  Approved or implementing against it.

**Done when:** docs references resolve and describe the same mode contract as
the spec.

### T8: Regression gates pass

**Depends on:** T1-T7

**Touches:** none

**Tests:**
- `cargo test -p gateway-execution delegation`
- `cargo test -p gateway-execution e2e_ward_pipeline`
- `cargo test -p gateway-services seed_default_agents`
- `cargo check --workspace`
- `git diff --check`

**Approach:**
- Run focused tests first, then workspace check.
- Fix only failures caused by this spec.
- Document unrelated pre-existing failures with exact command output.

**Done when:** all listed gates pass or unrelated existing failures are clearly
recorded.

## Rollout

Ship as a normal behavior change with conservative defaults. Existing root
delegations without `mode` continue to work through inference. Existing live
default agents are refreshed only when their instruction files match known
default-template signatures; customized files are preserved.

## Risks

- Mode inference may misclassify ambiguous tasks. Mitigation: root can pass an
  explicit mode, and fallback for ambiguous builder work is `WardBackedBuild`,
  the safer but slightly more expensive posture.
- Template refresh could be too aggressive. Mitigation: only refresh known old
  signatures/normalized bundled content and write backups.
- Direct artifacts may skip useful ward guidance. Mitigation: direct artifact
  applies only to exact-output self-contained tasks; reusable or repo-integrated
  work falls back to `WardBackedBuild`.
- Prompt changes can interact with model behavior unpredictably. Mitigation:
  keep runtime mode metadata authoritative and test generated rule text.

## Changelog

- 2026-06-02: Implemented delegation-mode plumbing, mode-specific executor
  rules, bundled template updates, guarded live default-agent refresh, and
  component/RFC documentation.

- 2026-06-02: initial plan.
