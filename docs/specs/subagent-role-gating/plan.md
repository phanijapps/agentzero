# Plan: Subagent Capability Policy

- **Spec:** [`spec.md`](spec.md)
- **Status:** Done

> **Plan contract:** this is the implementation strategy. Unlike the spec, this
> document is allowed to change as you learn. When it changes substantially
> (a different approach, not just a re-ordering), note why in the changelog at
> the bottom.

## Approach

Add capability gating at the point where z-Bot already decides which tools an
executor receives: `ExecutorBuilder::build_tool_registry`. Seed a user-visible
`reviewer-agent` through the same default-agent path that populates
`~/Documents/zbot/agents`, while an internal `RuntimeActorKind` +
`ToolCapability` policy supplies the actual boundary. The policy should classify
root, ordinary delegated executors, delegated reviewers, and `ward:<name>` ward
agents separately. The work should land in small commits: seed/load the reviewer
identity, introduce actor-aware builder state, add capability declarations,
filter first-party tools by policy, align reviewer prompt text, and add tests
for every inventory.

## Constraints

- Follow the existing agent config shape in
  `gateway/gateway-services/src/agents.rs`; do not add config policy fields
  without explicit approval.
- Preserve existing root/orchestrator behavior unless a change is needed to
  pass actor information through capability filtering.
- Preserve existing executor-subagent implementation tools.
- Preserve full first-party tool access for `ward:<name>` ward agents.
- Keep this independent from the separate Hermes gaps for durable work queues,
  remote/serverless execution, clarify implementation, peer messaging, and
  plugin lifecycle hooks.

## Construction tests

**Integration tests:** targeted `gateway-execution` tests that build or inspect
root, delegated executor, delegated reviewer, and ward-agent tool inventories
without invoking an LLM.

**Manual verification:** none required for V1 beyond inspecting that
`reviewer-agent` loads from the default/vault agent surface.

## Tasks

### T1: Reviewer agent identity loads through AgentService

**Depends on:** none

**Tests:**
- Goal-based check for Acceptance Criteria 1: `AgentService` can load
  `reviewer-agent` from a default or vault agent directory with review-focused
  instructions.
- Goal-based check for Acceptance Criteria 2: `reviewer-agent` appears in the
  available agents list used for delegation candidates.

**Approach:**
- Add a default/bundled `reviewer-agent` config and instructions in the same
  location/pattern as existing bundled/default specialists:
  `gateway/templates/default_agents.json` and
  `gateway/templates/agents/reviewer-agent.md`.
- Verify it seeds into `VaultPaths::agents_dir()` through
  `AgentService::seed_default_agents`; for the normal daemon this is
  `~/Documents/zbot/agents/reviewer-agent/`.
- Keep local user overrides under `~/Documents/zbot/agents/reviewer-agent/`
  valid without making that user directory the only repo source of truth.
- Use instructions that describe read-only review and the required structured
  `RESULT: APPROVED` / `RESULT: DEFECTS` ending.

**Done when:** `reviewer-agent` is loadable through `AgentService` in tests and
shows review-only instructions.

### T2: Actor-aware builder state replaces binary delegated policy

**Depends on:** none

**Tests:**
- TDD for Acceptance Criteria 3 and 9: root builds as `Root`; ordinary delegated
  executor builds as `DelegatedExecutor`; review tasks build as
  `DelegatedReviewer`; `ward:<name>` targets build as `WardAgent`.
- TDD for runtime context metadata: each non-root actor injects
  `app:actor_kind`; ordinary delegated actors also inject `app:is_delegated` and
  `app:subagent_role` where applicable.

**Approach:**
- Add an actor-aware field to `ExecutorBuilder`, such as
  `actor_kind: RuntimeActorKind`, defaulting to `Root`.
- Keep `with_delegated(true)` as a compatibility wrapper if useful, mapping to
  `DelegatedExecutor`.
- Add `with_subagent_role(role)` for ordinary delegation and
  `with_actor_kind(RuntimeActorKind::WardAgent)` for `ward:<name>` targets.
- Inject actor/capability state into `ExecutorConfig.initial_state`.

**Done when:** tests can observe actor state without constructing a full LLM
execution.

### T3: Delegation spawn passes actor kind to ExecutorBuilder

**Depends on:** T2

**Tests:**
- TDD for Acceptance Criteria 3: `spawn_delegated_agent` or a focused helper
  passes `detect_subagent_role(...)` into the builder.
- TDD for Acceptance Criteria 7: `ward:<name>` targets bypass ordinary
  subagent role detection and build as `WardAgent`.
- Existing role detection tests in
  `gateway/gateway-execution/tests/e2e_ward_pipeline_tests.rs` remain green.

**Approach:**
- Update `gateway/gateway-execution/src/delegation/spawn.rs` after role
  detection to call `with_subagent_role(role)` for ordinary subagents.
- Add a small helper, for example `actor_kind_for_delegation(child_agent_id,
  task)`, that returns `WardAgent` for `ward:<name>` and otherwise maps review
  signals to `DelegatedReviewer` or `DelegatedExecutor`.
- Keep rule prepending exactly once; avoid duplicating role rules in both
  spawn and setup paths.

**Done when:** a review task reaches executor construction as reviewer actor,
and a `ward:<name>` task reaches executor construction as ward-agent actor.

### T4: Capability policy is explicit and test-covered

**Depends on:** T2

**Tests:**
- TDD for Acceptance Criteria 4: every first-party tool registered in
  `ExecutorBuilder::build_tool_registry` has an explicit capability set.
- TDD for Acceptance Criteria 5: executor subagent inventory contains
  `shell`, `write_file`, `edit_file`, `respond`, context/memory tools, and no
  orchestration tools.
- TDD for Acceptance Criteria 6: reviewer subagent inventory contains read-only
  inspection/reporting tools and excludes `shell`, `write_file`, `edit_file`,
  `delegate_to_agent`, `wait_agent`, `kill_agent`, `steer_agent`, ward
  mutation, and memory mutation.
- TDD for Acceptance Criteria 7: ward-agent inventory contains the first-party
  union available to root and executor profiles, including implementation and
  orchestration/control tools when backing services are wired.
- TDD for Acceptance Criteria 8: root inventory still contains orchestration
  tools when the relevant services are wired.

**Approach:**
- Add a small internal `ToolCapability` enum and `ToolPolicy` helper near
  `gateway/gateway-execution/src/invoke/executor.rs`, or in a sibling module if
  tests need direct access.
- Register tools through a helper that takes tool name/capabilities/tool
  instance and registers only if the current actor policy allows every required
  capability.
- For reviewer read-only file access, use existing `ReadTool`, `GlobTool`,
  and `GrepTool` (`read`, `glob`, and `grep`) rather than shell.
- If `MemoryTool` or `WardTool` cannot be made read-only without adding new
  API surface, omit them from reviewer V1 and rely on recall/context already
  injected by the runtime.
- Keep optional graph/ingest/goal adapters conservative: `graph_query` is
  allowed if read-only; `ingest` and mutating `goal` operations stay executor,
  root, or ward-agent only unless proven read-only.

**Done when:** capability tests and tool-name tests fail on any accidental
capability expansion or ward-agent restriction.

### T5: Reviewer prompt rules align with enforced read-only behavior

**Depends on:** T4

**Tests:**
- Goal-based check for Acceptance Criteria 10: reviewer rules no longer say
  reviewers should run code or execute commands.
- Existing reviewer result-format test remains green.

**Approach:**
- Update `subagent_rules(SubagentRole::Reviewer)` to instruct reviewers to read
  supplied context/files, inspect outputs already produced by other agents, and
  report findings.
- Keep `RESULT: APPROVED` / `RESULT: DEFECTS` exact endings.

**Done when:** reviewer prompt text and reviewer tool inventory no longer
contradict each other.

### T6: Regression gates pass

**Depends on:** T1-T5

**Tests:**
- `cargo test -p gateway-execution role`
- `cargo test -p gateway-execution ward_agent`
- `cargo test -p gateway-execution e2e_ward_pipeline`
- `cargo test -p gateway-execution delegate`
- `cargo check --workspace`
- `git diff --check`

**Approach:**
- Run focused tests first, then workspace check.
- Fix only failures caused by this spec.

**Done when:** all listed commands pass or any unrelated pre-existing failure is
documented with exact output.

## Rollout

Ship as a normal behavior change. There is no feature flag in V1 because the
point of the change is to make reviewer safety a hard default while preserving
ward-agent authority. Users who need command-running validation from a normal
subagent should delegate to an executor agent; ward agents remain full-tool
actors by design.

## Risks

- Some existing prompts may expect reviewers to run tests directly. V1 changes
  that workflow: reviewers inspect provided outputs and source files, while
  executor/root agents run commands.
- If root delegation still sends review tasks to `code-agent`, role detection
  should still apply reviewer policy. The explicit `reviewer-agent` improves
  product clarity but must not be the only enforcement trigger.
- Read-only reviewer usefulness depends on enough context being available via
  recall, file reads, and parent-provided outputs. Follow-up work may add a
  controlled "request executor to run check" workflow.
- Ward agents with orchestration/control tools can recursively coordinate other
  agents. This is intentional for this spec, but tests must make the policy
  explicit so future hardening does not accidentally change it.

## Changelog

- 2026-06-01: initial plan.
- 2026-06-01: revised from role-specific tool lists to internal capability
  policy and added `WardAgent` as a full-tool actor.
