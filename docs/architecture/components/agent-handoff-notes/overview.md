# Agent Handoff Notes

Agent Handoff Notes are a small coordination surface for root and ward-agent
orchestrators. They are not Pattern 4 peer messaging. The feature only lists
delegated agents in the current session and sends one-way notes to a running
child execution by `execution_id`.

## Purpose

Parallel work often produces useful mid-run facts: a source a builder should
use, a file path another agent owns, or a correction that prevents duplicated
work. Root already has `steer_agent`, but that tool reads as command/control.
`handoff_to_agent` gives the orchestrator a friendlier one-way handoff surface
while keeping the same steering delivery mechanism.

## Tools

| Tool | Actor access | Behavior |
|------|--------------|----------|
| `list_session_agents` | Root, ward agents | Lists delegated executions for the current `session_id` with `execution_id`, `agent_id`, status, task, timestamps, and child session ID. |
| `handoff_to_agent` | Root, ward agents | Validates the target execution belongs to the current session and is running, then queues a one-way note through `SteeringRegistry`. |

Ordinary delegated executors and reviewers do not receive these tools. That
preserves the subagent capability policy: normal subagents finish their assigned
work instead of orchestrating other agents. Ward agents keep full first-party
tool access by design.

## Data Flow

1. Root delegates work and receives child `execution_id` values.
2. Root calls `list_session_agents` if it needs the current roster.
3. Root calls `handoff_to_agent` with a concise note for one running child.
4. The tool checks execution state for the caller's current `session_id`.
5. If the target is running, the note is queued through the existing steering
   queue and is read before the child agent's next LLM call.
6. Root calls `wait_agent` when it needs completed output.

## Boundaries

- One-way only. There is no reply channel, mailbox, message ID, or blocking
  request/reply flow.
- Current session only. The handoff target must be an execution in the caller's
  current `session_id`.
- No persistence. Handoff notes are not a durable memory layer and do not add
  database schema.
- Not federation. Cross-daemon, role-addressed, or enterprise peer messaging
  remains parked in
  `docs/architecture/future-state/2026-05-11-pattern4-peer-messaging-design.md`.

## Related Components

- `gateway/gateway-execution/src/tools/handoff_to_agent.rs`
- `gateway/gateway-execution/src/tools/list_session_agents.rs`
- `gateway/gateway-execution/src/tools/steer_agent.rs`
- `gateway/gateway-execution/src/tools/wait_agent.rs`
- `docs/architecture/components/subagent-capability-policy/overview.md`
