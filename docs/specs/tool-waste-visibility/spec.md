# Spec: Tool Waste Visibility

- **Status:** Shipped
- **Owner:** phanijapps
- **Plan:** [`plan.md`](plan.md)
- **Constrained by:** none

> **Spec contract:** this document defines what "done" means. The implementing
> PR must match this spec, or update it. Verification must be derivable from it.

## Objective

Make tool waste visible and easier for agents to recover from during session
wastewalks. A blocked tool attempt must be persisted as a warning with a
machine-readable `blocked_by_hook` reason instead of a successful
`Tool completed` row; invalid tool arguments must tell the agent the expected
argument shape; the planner-agent's mandatory planning skills must resolve to
installed skills; and tool-result logs should carry execution duration where
the runtime can measure it. The goal is not a new dashboard. The goal is that
the existing logs and session traces make preventable tool waste measurable.

## Boundaries

The three-tier guard that keeps an implementing agent inside the lines.
*Always do* applies without asking; *Ask first* requires human sign-off
before proceeding; *Never do* is a hard rule, even under time pressure.

### Always do

- Preserve existing tool-call and tool-result event ordering.
- Keep blocked hook attempts visible to the model as tool results so provider
  tool-call protocols remain valid.
- Keep the existing `execution_logs` schema; use existing `duration_ms` and
  structured metadata fields.
- Keep fixes scoped to runtime tool-event emission, gateway logging,
  tool-schema error text, planner templates, and focused tests.

### Ask first

- Adding a new wastewalk dashboard, API endpoint, or persistence table.
- Changing hook policy, allow/deny rules, or which tool invocations are blocked.
- Renaming public tool names or removing compatibility for existing skill names.

### Never do

- Never treat a blocked hook attempt as a successful tool completion in logs.
- Never hide a blocked hook result from the assistant/model transcript.
- Never make this pass depend on a new runtime crate, service boundary, or
  top-level module.
- Never broaden this into session scoring, retention, archival, or cleanup.

## Testing Strategy

- Blocked hook logging: **TDD**. Unit tests around runtime event emission and
  gateway event logging should prove blocked hooks carry an error/reason and
  persist as warning tool results.
- Tool schema hints: **TDD**. Tool unit tests should prove common invalid
  `memory`, `shell`, and `load_skill` invocations return actionable expected
  argument shapes.
- Planner skill names: **TDD plus goal-based check**. Template tests or direct
  content checks should prove the planner template references installed skills,
  and skill loading should accept legacy planner aliases if practical.
- Duration propagation: **TDD where the event boundary allows it, goal-based
  otherwise**. Tests should prove tool-result log metadata can carry measured
  duration without changing the database schema.
- Regression gates: **goal-based check**. Run focused Rust tests for
  `agent-runtime`, `agent-tools`, `gateway-execution`, and `api-logs` where
  touched, plus `cargo fmt --check` and targeted clippy if practical.

## Acceptance Criteria

- [x] Runtime blocked hook events emit a non-empty error/reason instead of
  `error: None`.
- [x] Gateway execution logging persists blocked hook results at warning level
  with metadata that includes `blocked_by_hook` or an equivalent structured
  reason.
- [x] Existing logs no longer need string matching on `[blocked by hook]` to
  identify blocked tool waste.
- [x] Invalid `memory` calls that omit `action` explain the expected argument
  shape.
- [x] Invalid `shell` calls that omit `command` explain the expected argument
  shape.
- [x] Missing `load_skill` errors include a recovery hint and the available
  planner skill names where relevant.
- [x] Planner-agent instructions reference installed planner skills or supported
  aliases, so the first mandatory planning skill load no longer fails.
- [x] Tool-result logging can persist a measured `duration_ms` for runtime tool
  execution results where timing is available.
- [x] Focused tests and formatting/lint gates for touched crates pass or any
  blocker is documented with exact command output.

## Assumptions

- Technical: blocked tool calls currently emit `ToolResult { result:
  "[blocked by hook]", error: None }`, so they persist as successful tool
  results (source: `runtime/agent-runtime/src/executor.rs`).
- Technical: gateway log persistence marks tool result level from
  `error.is_some()`, so blocked hooks with `error: None` become `info` /
  `Tool completed` (source:
  `gateway/gateway-execution/src/invoke/event_logging.rs`).
- Technical: `duration_ms` exists in the execution log schema and API service,
  but the stream-event logging path does not pass timing into
  `log_tool_result` (source: `services/api-logs/src/service.rs`;
  `gateway/gateway-execution/src/invoke/event_logging.rs`).
- Technical: planner-agent template requires `planning-highlevel` and
  `planning-decompose`, but installed template skills include `spec-builder`
  and `plan-composer` instead (source:
  `gateway/templates/agents/planner-agent.md`; `gateway/templates/skills/`
  probe).
- Process: no repo-level `docs/CONVENTIONS.md` or `docs/CHARTER.md` exists;
  active specs are tracked in `docs/specs/README.md` (source:
  `test -f docs/CONVENTIONS.md`, `test -f docs/CHARTER.md`,
  `docs/specs/README.md`).
- Product: scope is limited to making waste visible and self-correcting:
  blocked hook results warn, invalid tool-argument errors include expected
  schema, planner skill names stop causing first-step failures, and tool
  durations are recorded where feasible. No new wastewalk UI/dashboard in this
  pass (source: user confirmation 2026-06-09).
