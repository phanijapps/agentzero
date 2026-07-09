# Plan: Agent Handoff Notes

- **Spec:** [`spec.md`](spec.md)
- **Status:** Done

> **Plan contract:** this is the implementation strategy. Unlike the spec, this
> document is allowed to change as you learn. When it changes substantially
> (a different approach, not just a re-ordering), note why in the changelog at
> the bottom.

## Approach

Implement the feature as two gateway-execution tools that reuse existing
runtime infrastructure. `list_session_agents` should query execution state for
the current session and serialize a compact roster. `handoff_to_agent` should
validate the target execution with `StateService` against the caller's
context-state `session_id`, then share the steering delivery path used by
`steer_agent` with a narrower one-way coordination contract. Registration
should flow through the existing `ToolCapability::AgentControl` gate so root
and ward-agent actors can use the tools while ordinary delegated subagents
remain non-orchestrating. The riskiest part is avoiding accidental Pattern 4
scope creep: no reply bus, persistence, target discovery outside the current
session, or database migration belongs in this implementation.

## Constraints

- Follow the subagent capability policy: root and ward-agent actors may
  orchestrate/control agents; ordinary delegated executors and reviewers may
  not.
- Preserve existing `steer_agent`, `wait_agent`, `kill_agent`, and
  `delegate_to_agent` tool names and behavior.
- Use the real session ID from tool context state for session-scoped queries;
  do not rely on legacy conversation ID surfaces.
- Validate `handoff_to_agent` targets against execution state before steering;
  `SteeringRegistry` alone is not session-aware.
- Keep the feature one-way and current-session only.
- Add no database schema, durable queue, reply channel, or federation code.

## Construction tests

**Integration tests:** targeted `gateway-execution` tests that build tool
inventories for root, ward-agent, delegated executor, and delegated reviewer
actors; tool unit tests with temporary state/registry fixtures.

**Manual verification:** optional local daemon smoke test: start a session,
delegate two child agents, call `list_session_agents`, send a handoff to one
running child, and confirm the tool returns `delivered` or
`agent_not_running` truthfully.

## Tasks

### T1: Session roster tool returns current-session child executions

**Depends on:** none

**Touches:** `gateway/gateway-execution/src/tools/list_session_agents.rs`,
`gateway/gateway-execution/src/tools/mod.rs`,
`gateway/gateway-execution/src/invoke/executor.rs`

**Tests:**
- TDD for Acceptance Criteria 1 and 2: with a temporary state store containing
  executions from two sessions, `list_session_agents` returns only delegated
  executions from the context-state `session_id`.
- TDD for stable JSON shape: each row serializes `execution_id`, `agent_id`,
  `status`, `task`, `started_at`, `completed_at`, and `child_session_id`.
- TDD for missing context: missing `session_id` returns a structured tool error
  rather than falling back to the legacy conversation ID.

**Approach:**
- Add `ListSessionAgentsTool` under `gateway/gateway-execution/src/tools/`.
- Inject `Arc<StateService<DatabaseManager>>` into the tool independently of
  the result-bus wiring used by `WaitAgentTool`; this tool depends on state,
  not on `wait_agent` availability.
- Query `StateService::list_executions` or the narrowest existing repository
  API that can filter by current session.
- Filter out root executions in memory by requiring non-root
  `delegation_type`, not by attempting an `IS NOT NULL` query through
  `ExecutionFilter.parent_execution_id`.
- Register the tool only when `state_service` is available and the actor policy
  allows `ToolCapability::AgentControl`.

**Done when:** the roster tool tests pass and the root/ward-agent inventories
include `list_session_agents` while ordinary subagent inventories do not.

### T2: Handoff tool queues one-way notes through steering

**Depends on:** none

**Touches:** `gateway/gateway-execution/src/tools/handoff_to_agent.rs`,
`gateway/gateway-execution/src/tools/mod.rs`,
`gateway/gateway-execution/src/invoke/executor.rs`,
`gateway/gateway-execution/src/runner/invoke_bootstrap.rs`

**Tests:**
- TDD for Acceptance Criteria 3 and 4: a current-session running execution with
  a registered steering handle receives the handoff text and the tool returns
  `status: delivered`.
- TDD for terminal statuses: completed, crashed, and cancelled current-session
  executions return `status: agent_not_running` before steering.
- TDD for registry race behavior: a current-session running execution missing
  from `SteeringRegistry` returns `status: agent_not_running`.
- TDD for session safety: unknown execution IDs and executions from another
  session return `status: target_not_found` and do not call `steer`.
- TDD for validation: missing `execution_id`, missing `message`, and oversized
  messages fail with clear tool errors.

**Approach:**
- Add `HandoffToAgentTool` beside `SteerAgentTool`.
- Inject both `Arc<StateService<DatabaseManager>>` and
  `Arc<SteeringRegistry>` into the tool.
- Read context-state `session_id`; reject missing context instead of falling
  back to the legacy conversation ID.
- Check `StateService::get_execution(execution_id)` and require the execution
  to belong to the current session and have running status before steering.
- Reuse the same maximum message length as `steer_agent` unless a stricter
  constant is justified.
- Prefix or frame the injected steering content so the target sees it as a
  handoff note, for example `[Handoff note from orchestrator] ...`.
- Return structured JSON distinct from `steer_agent` but backed by the same
  `SteerResult`.
- Register the tool only when `state_service` and `steering_registry` are
  available and the actor policy allows `ToolCapability::AgentControl`.

**Done when:** handoff tool tests prove delivery and not-running behavior
without changing `steer_agent` tests.

### T3: Capability inventories and root guidance recognize handoff notes

**Depends on:** T1, T2

**Touches:** `gateway/gateway-execution/src/invoke/executor.rs`,
`gateway/gateway-execution/src/runner/invoke_bootstrap.rs`,
`gateway/templates/shards/*.md`

**Tests:**
- TDD for Acceptance Criteria 5: root and ward-agent inventories include both
  new tools when backing services are wired; delegated executor and reviewer
  inventories exclude both.
- Goal-based check for Acceptance Criteria 7: prompt/tool guidance mentions
  one-way handoff notes and does not describe replies, mailboxes,
  persistence, or federation.
- Existing capability-policy tests remain green.

**Approach:**
- Add both tool names to the root-orchestrator bootstrap tool list when the
  required backing services are wired, mirroring how `steer_agent` and
  `wait_agent` are exposed.
- Pass `state_service` into `ExecutorBuilder` independently from
  `agent_result_bus` so `list_session_agents` and `handoff_to_agent` do not
  disappear when `wait_agent`/`kill_agent` are not wired.
- Keep capability gating centralized in `build_tool_registry`; do not add
  special-case role checks outside the existing actor policy.
- Update only the smallest prompt shard needed to teach root when to use
  `list_session_agents`, `handoff_to_agent`, and `wait_agent`.

**Done when:** tool inventory tests fail on accidental subagent exposure or
missing root/ward exposure.

### T4: Component documentation and spec index separate the slice from Pattern 4

**Depends on:** T1, T2

**Touches:** `docs/architecture/components/agent-handoff-notes.md`,
`docs/architecture/components/index.md`, `docs/specs/README.md`

**Tests:**
- Goal-based check for Acceptance Criteria 8 and 9: docs contain
  "one-way", "current session", and "not Pattern 4", and do not mention a
  reply bus, federation, or persistence as implemented behavior.
- Goal-based check for the spec index: `docs/specs/README.md` links to
  `agent-handoff-notes/spec.md`.

**Approach:**
- Add a concise component doc covering purpose, actors, data flow, tool
  surfaces, and boundaries.
- Link the doc from `docs/architecture/components/index.md`.
- Add the spec to `docs/specs/README.md` with Draft status.
- Point readers to the Pattern 4 future-state document for the parked
  northstar.

**Done when:** component docs clearly explain the feature without implying the
full peer-messaging design has shipped.

### T5: Regression gates pass

**Depends on:** T1-T4

**Touches:** none

**Tests:**
- `cargo test -p gateway-execution handoff`
- `cargo test -p gateway-execution list_session_agents`
- `cargo test -p gateway-execution handoff_tools`
- `cargo test -p gateway-execution steer_agent`
- `cargo test -p gateway-execution wait_agent`
- `cargo test -p gateway-execution kill_agent`
- `cargo check --workspace`
- The following command must return no persistence, migration, reply-bus, or
  federation additions for this spec:
  ```bash
  git diff --name-only -- stores services gateway runtime \
    | rg 'schema|migration|PeerMessageBus|ReplyStore|federation|reply'
  ```
- The following command should find only future-state/spec boundary text, not
  implementation code:
  ```bash
  rg "PeerMessageBus|ReplyStore|wait_for_reply|federation|reply channel" \
    gateway runtime services stores docs/specs/agent-handoff-notes \
    docs/architecture/components
  ```
- `git diff --check`

**Approach:**
- Run focused tests first, then the workspace check.
- Fix only failures caused by this feature.
- If a listed test name differs after implementation, update this plan in the
  same PR with the exact replacement command.

**Done when:** all listed gates pass or unrelated pre-existing failures are
documented with exact output.

## Rollout

Ship as a normal root/ward-agent tool addition. There is no feature flag
because ordinary delegated subagents remain blocked by the existing capability
policy, and the tool only queues notes through existing steering. Users can
continue using `steer_agent` directly for stronger intervention language.

## Risks

- Agents may overuse handoff notes as chatty coordination instead of waiting
  for final results. Mitigate with a small message limit and prompt guidance
  that reserves handoffs for concrete corrections, findings, file paths, or
  ownership notes.
- Delivery timing still follows steering drain timing. A note queued during a
  long LLM/tool turn may not be read immediately.
- `agent_not_running` may surprise root after fast child completion. Root
  should use `wait_agent` to retrieve the completed result instead of trying to
  mutate the completed child session.
- Tool naming could blur with `steer_agent`. Keep descriptions distinct:
  `handoff_to_agent` is a collaborative note; `steer_agent` is direct
  intervention.

## Changelog

- 2026-06-04: initial plan.
