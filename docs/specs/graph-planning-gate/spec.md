# Spec: Graph Planning Gate

- **Status:** Shipped
- **Owner:** phanijapps
- **Plan:** [`plan.md`](plan.md)
- **Constrained by:** none
- **Brief:** none
- **Contract:** none
- **Shape:** service

> **Spec contract:** this document defines what "done" means. The implementing
> PR must match this spec, or update it. Verification must be derivable from it.

## Objective

For a non-Quick-Chat request whose intent analysis selects the graph approach
but does not route it to a graduated ward-agent, the root must establish the
ward and then deterministically start `planner-agent` before any worker agent
delegation, procedure execution, terminal response, root-owned checklist, or
MCP tool execution can represent execution. The planner receives the
actual selected ward and intent context; after it finishes, the established
step-execution continuation remains unchanged.

## Boundaries

### Always do

- Create an in-memory planning gate only for cold graph execution: a new,
  ungraduated, or unassigned ward that must be established by the root.
- Trigger one sequential `planner-agent` delegation immediately after the
  root successfully uses or creates that ward, including the final active ward
  in its task.
- Reject root `delegate_to_agent`, `run_procedure`, `respond`, and
  `update_plan` calls while the gate is awaiting ward establishment.
- Reject every non-`ward` tool at the executor boundary while the gate is
  awaiting ward establishment, including externally supplied MCP tools.
- Preserve the graduated existing-ward route, Quick Chat, simple intent, and
  normal post-planning step delegation exactly as they behave today.

### Ask first

- Persisting the planning-gate phase across daemon restarts or adding a
  session/database schema.
- Changing planner-agent's plan artifact format, its configuration, or the
  established root continuation protocol.
- Adding a generic scheduler, a new public API/event, dependency, or a new
  top-level module.

### Never do

- Never allow a cold graph request to delegate directly to a builder,
  researcher, writer, or ward agent before `planner-agent` starts.
- Never use intent text alone as the planner's active workspace; pass the ward
  actually accepted by the ward tool.
- Never apply this gate to Quick Chat or a graduated ward-agent route.

## Testing Strategy

- Planning-gate state and tool enforcement: **TDD**. Focused tool tests prove
  a cold graph gate redirects root-created checklists and direct worker
  delegation, while allowing no bypass before ward setup.
- MCP enforcement: **TDD**. An MCP-shaped `{server}__{tool}` call is redirected
  before MCP dispatch, proving a configured side-effecting server cannot
  bypass planner startup.
- Ward transition: **TDD**. A ward-tool test proves a successful root
  `create`/`use` emits exactly one sequential planner delegation with the
  selected ward; listing or delegated ward operations do not do so.
- Bootstrap routing: **TDD**. Gateway tests prove the gate is installed only
  for graph intent without a graduated existing ward, and is absent for simple
  and graduated-ward paths.
- Regression checks: **goal-based check**. Run the focused package tests,
  formatting, and `cargo check` for the touched workspace members.

## Acceptance Criteria

- [x] Given a cold graph intent, when the root has not entered a ward, direct
  worker delegation, procedure execution, terminal response, and `update_plan`
  or MCP call are redirected to ward establishment and cannot create
  executable work, end the request, publish a root checklist, or call an
  external server.
- [x] Given that root successfully creates or enters the selected ward, the
  runtime emits one non-parallel, waiting `planner-agent` delegation containing
  the actual ward and the original intent context.
- [x] A successful graph session follows the durable order observed in
  `sess-147b4c3c-58e3-4925-9aa1-7c7a2fb3ec87`: root → planner-agent → numbered
  step executors; it cannot follow the bypass seen in
  `sess-d00058fe-8aa1-4915-9448-1036db1da15d`: root → builder-agent.
- [x] Quick Chat, simple intent, and graph requests routed to a graduated
  existing ward-agent do not receive the planning gate.
- [x] Focused tests, formatting, and stated Rust checks pass without staging
  or changing the unrelated Observatory worktree edits.

## Assumptions

- Technical: executor initial state reaches both legacy and Rig tool contexts,
  and tool-set delegation actions become runner delegation events (source:
  `gateway/gateway-execution/src/invoke/executor.rs`,
  `runtime/agent-runtime/src/{executor.rs,rig_adapter/engine.rs}`).
- Technical: the current direct-delegation guard only detects placeholder
  specs, so a fresh graph ward has no enforced planner transition (source:
  `runtime/agent-runtime/src/tools/delegate.rs`).
- Product: Research graph work must establish its ward, run `planner-agent`,
  and then run plan steps (source: user confirmation 2026-07-17; durable
  `sess-147b4c3c-58e3-4925-9aa1-7c7a2fb3ec87` history).
- Process: the feature contract, plan, construction tests, and acceptance
  criteria belong under one `docs/specs/<feature>/` directory (source:
  `docs/CONVENTIONS.md §4`).
