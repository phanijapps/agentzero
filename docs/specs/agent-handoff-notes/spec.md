# Spec: Agent Handoff Notes

- **Status:** Shipped
- **Owner:** phanijapps
- **Plan:** [`plan.md`](plan.md)
- **Constrained by:** [`subagent-role-gating`](../subagent-role-gating/spec.md);
  [`Pattern 4 peer messaging northstar`][pattern4]

> **Spec contract:** this document defines what "done" means. The implementing
> PR must match this spec, or update it. Verification must be derivable from it.

## Objective

Give root and ward-agent orchestrators a lightweight way to see the delegated
agents in the current session and send one-way handoff notes to a running child
agent without implementing full Pattern 4 peer messaging. During parallel
research/build shards, the orchestrator should be able to route a concise
correction, finding, file path, or ownership note to the right running
execution by `execution_id`. The slice is complete when this works through
first-party tools, respects the existing actor capability policy, and remains
strictly one-way.

## Boundaries

The three-tier guard that keeps an implementing agent inside the lines.
*Always do* applies without asking; *Ask first* requires human sign-off
before proceeding; *Never do* is a hard rule, even under time pressure.

### Always do

- Add a `list_session_agents` first-party tool that reads the real session ID
  from tool context state and returns delegated executions for the current
  session with `execution_id`, `agent_id`, `status`, `task`, timestamps, and
  `child_session_id` when available.
- Add a `handoff_to_agent` first-party tool that takes an `execution_id` and
  concise `message`, then routes the note through the existing
  `SteeringRegistry` to the target execution.
- Validate the handoff target through execution state before steering: the
  target execution must exist, belong to the caller's current `session_id`, and
  be in a running state.
- Make `handoff_to_agent` return an explicit structured result:
  `delivered` when state validation passes and the steering queue accepts the
  note, `agent_not_running` when the target is terminal or missing from the
  steering registry, and `target_not_found` when the `execution_id` is unknown
  or outside the current session.
- Register both tools only for actors allowed to control agents under the
  existing capability policy. Root and ward-agent actors may receive the tools;
  ordinary delegated executors and reviewers must not.
- Keep `steer_agent` available for deliberate intervention and make
  `handoff_to_agent` the friendlier one-way coordination surface. The new tool
  may share the same underlying steering queue, but its description and return
  text must frame the action as a handoff note rather than command/control.
- Keep handoff payloads bounded with the same or stricter maximum size as
  `steer_agent` so agents send compact coordination notes instead of large
  artifacts.
- Update runtime prompt/tool guidance so root can use the sequence:
  `delegate_to_agent` -> `list_session_agents` when needed ->
  `handoff_to_agent` for cross-shard notes -> `wait_agent` for final results.
- Document this as "Agent Handoff Notes" in `docs/architecture/components/` so it is
  not confused with the full Pattern 4 peer messaging northstar.

### Ask first

- Adding reply semantics, request/reply waits, inboxes, message IDs, or
  blocking `wait_for_reply` behavior.
- Persisting handoff notes into `conversations.db`, `knowledge.db`, execution
  logs, or a new message table.
- Addressing targets by role, agent name, ward name, or remote daemon identity
  instead of current-session `execution_id`.
- Letting ordinary delegated executor or reviewer subagents call
  `list_session_agents` or `handoff_to_agent`.
- Changing `SteeringRegistry`, `SteeringQueue`, or steering drain timing beyond
  what is required to add the convenience wrapper.
- Renaming, removing, or changing `steer_agent`, `wait_agent`, or `kill_agent`
  behavior.

### Never do

- Never implement `PeerMessageBus`, `ReplyStore`, reply channels, federation,
  cross-daemon routing, or cross-session messaging in this spec.
- Never add database schema or migration changes for this slice.
- Never treat handoff delivery as proof the target agent read, obeyed, or
  incorporated the note; delivery only means the note was queued for the
  target's next steering drain after the tool observed the execution in a
  running state.
- Never route handoff notes to completed agents by mutating their conversation
  history or replaying their execution.
- Never expand this feature into durable work queues, durable checkpoints,
  remote execution, or Pattern 4 enterprise federation.
- Never weaken the subagent capability policy to make peer coordination easier.

## Testing Strategy

- Tool behavior: **TDD**. `handoff_to_agent` is a compact wrapper over
  `SteeringRegistry`; unit tests should prove delivered, not-running, missing
  argument, and oversized-message behavior.
- Session roster behavior: **TDD**. `list_session_agents` should be tested
  against a temporary state store so it returns only current-session delegated
  executions and serializes stable field names.
- Capability policy: **TDD**. Tool-inventory tests should prove root and
  ward-agent actors receive both tools when services are wired, while ordinary
  executor and reviewer actors do not.
- Prompt and documentation alignment: **goal-based check**. Grep or snapshot
  checks should prove tool guidance names Agent Handoff Notes as one-way and
  does not claim reply, mailbox, persistence, or federation semantics.
- Workspace compatibility: **goal-based check**. Targeted gateway-execution
  tests plus `cargo check --workspace` prove the new tools do not regress
  existing delegation, steering, waiting, or capability-policy behavior.

## Acceptance Criteria

- [x] `list_session_agents` is registered for root and ward-agent actors when
  the state service is wired, and returns delegated executions for the current
  session using the real `session_id` from context state.
- [x] `list_session_agents` output includes `execution_id`, `agent_id`,
  `status`, `task`, `started_at`, `completed_at`, and `child_session_id` using
  stable JSON field names.
- [x] `handoff_to_agent` is registered for root and ward-agent actors when the
  state service and steering registry are wired, accepts `execution_id` and
  `message`, validates the target is in the current session, and queues the
  message into the target's steering queue.
- [x] `handoff_to_agent` returns `{"status":"delivered"}` for a running target
  accepted by the steering registry, `{"status":"agent_not_running"}` for
  completed, crashed, cancelled, or registry-missing targets, and
  `{"status":"target_not_found"}` for unknown or cross-session execution IDs.
- [x] Ordinary delegated executor and reviewer actors do not receive
  `list_session_agents` or `handoff_to_agent` in their tool inventories.
- [x] Existing `steer_agent`, `wait_agent`, and `kill_agent` behavior remains
  unchanged and covered by existing tests.
- [x] Runtime guidance presents the feature as one-way handoff notes and keeps
  `wait_agent` as the way to retrieve completed results.
- [x] `docs/architecture/components/` documents where the feature sits relative to
  delegation, steering, wait/kill, ward agents, and the parked Pattern 4
  northstar.
- [x] No database schema, new persistence layer, `PeerMessageBus`,
  `ReplyStore`, federation, or reply-channel code is added.

## Assumptions

- Technical: delegated execution records already carry `id`, `session_id`,
  `agent_id`, `parent_execution_id`, `task`, `status`, timestamps, and
  `child_session_id`. (source:
  `services/execution-state/src/types.rs`;
  `services/execution-state/src/repository.rs`;
  `stores/zbot-stores-sqlite/src/schema.rs`)
- Technical: `delegate_to_agent` already returns an `execution_id` that can be
  used by `wait_agent`, `steer_agent`, or `kill_agent`. (source:
  `runtime/agent-runtime/src/tools/delegate.rs`)
- Technical: `steer_agent` already queues a bounded message through
  `SteeringRegistry` and returns `agent_not_running` when the execution is not
  registered as running. (source:
  `gateway/gateway-execution/src/tools/steer_agent.rs`)
- Technical: `wait_agent` already reads completed child-session output by
  `execution_id`, so this spec does not need reply semantics. (source:
  `gateway/gateway-execution/src/tools/wait_agent.rs`)
- Technical: actor capability policy currently distinguishes `Root`,
  `DelegatedExecutor`, `DelegatedReviewer`, and `WardAgent`; ordinary delegated
  subagents are non-orchestrating while ward agents retain full tool access.
  (source: `gateway/gateway-execution/src/invoke/executor.rs`;
  `docs/specs/subagent-role-gating/spec.md`)
- Technical: the Pattern 4 design is explicitly a northstar and includes
  `PeerMessageBus`, reply storage, federation, and request/reply behavior that
  this spec excludes. (source:
  `docs/architecture/future-state/2026-05-11-pattern4-peer-messaging-design.md`)
- Product: the desired slice is an ease-of-life fragment of Pattern 4, not the
  full Pattern 4 implementation. (source: user confirmation 2026-06-04)
- Process: active feature specs live under `docs/specs/<feature>/` with
  `spec.md`, `plan.md`, and an entry in `docs/specs/README.md`. (source:
  `docs/specs/README.md`; `docs/specs/subagent-role-gating/spec.md`)

[pattern4]: ../../../docs/architecture/future-state/2026-05-11-pattern4-peer-messaging-design.md
