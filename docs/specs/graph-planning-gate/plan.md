# Plan: Graph Planning Gate

- **Spec:** [`spec.md`](spec.md)
- **Status:** Done

> **Plan contract:** this is the implementation strategy. Unlike the spec, this
> document is allowed to change as implementation reveals new facts.

## Approach

Turn the graph-planning prompt instruction into a narrow runtime invariant.
Bootstrap will place an intent-derived planner task into root executor state
only for cold graph requests. The existing ward tool consumes that state after
a successful root `create` or `use`, adds the actual active ward, and emits the
same sequential delegation action the normal delegate tool uses. The delegate
procedure, terminal-response, and checklist tools reject an attempt to skip
this transition. The executor boundary applies the same restriction to MCP
tools, which are not registered in the normal built-in tool registry. This
keeps the existing planner and continuation machinery; it does not introduce
a second scheduler.

## Constraints

- Reuse `ExecutorBuilder::with_initial_state`, `ToolContext` state, and the
  existing `DelegateAction` event path.
- Keep the state scoped to one root executor invocation; no database migration
  or new event/API is in scope.
- Preserve the warm graduated-ward route and the existing plan artifact format.
- Do not stage or alter unrelated Observatory performance changes.

## Construction tests

**Integration tests:** focused bootstrap routing test from graph intent to
initial gate state, plus existing executor action-event coverage.

**Manual verification:** submit a new Research graph request; inspect
`agent_executions` for root → planner-agent before any step executor, then
verify `session_plans` becomes available through the established planner flow.

## Design (LLD)

### State & control flow

`IntentOutcome` carries an optional planner task only when intent chose graph
and no graduated existing ward was bound. Bootstrap serializes it under a
private root context key. Before ward entry, `DelegateTool` and `UpdatePlanTool`
read that key and return a redirect rather than applying a side effect;
`RunProcedureTool` and `RespondTool` do likewise. On a successful root
`ward(create|use)`, `WardTool` claims the transition, marks it started, and
emits one sequential, waiting `DelegateAction` to `planner-agent`. The task
appends the ward name from the successful tool operation, so it wins over a
provisional or unassigned recommendation. The executor emits the existing
action event and stops for delegation; gateway spawning and callback
continuation remain unchanged. Traces to AC1–AC3.

The executor also checks the gate before built-in or MCP dispatch. Thus a
model-visible MCP identifier such as `blender_mcp__execute_code` returns the
same redirect before the MCP client is resolved or called. Once ward entry
starts planner-agent, the gate phase changes and normal specialist MCP access
is unchanged. Traces to AC1.

### Failure, edge cases & resilience

The gate is no-op for ward listing/info, delegated agents, simple/Quick Chat,
and graduated ward-agent routes. A duplicate ward call cannot emit a second
planner action because the tool context claims the planning transition first.
It intentionally is not durable across a daemon restart: session recovery is
outside this narrow direct-bypass correction. Traces to AC2 and AC4.

## Tasks

### T1: Represent and enforce a cold-graph planning gate

**Depends on:** none

**Touches:** `runtime/agent-tools/src/tools/{guards.rs,execution/update_plan.rs}`, `runtime/agent-runtime/src/tools/delegate.rs`

**Tests:**

- TDD: cold-gate root delegation to `builder-agent` returns a redirect and
  creates no `DelegateAction`. Covers AC1.
- TDD: cold-gate `update_plan` returns a redirect and leaves `app:plan`
  unset. Covers AC1.
- TDD: cold-gate procedure and response calls return redirects without
  dispatching a step or setting a terminal response action. Covers AC1.
- TDD: an MCP-shaped external tool identifier redirects before MCP dispatch.
  Covers AC1.

**Approach:**

- Add a small serializable gate payload and shared state readers next to the
  existing placeholder guards.
- Apply that guard before the normal delegate/checklist side effects.

**Done when:** no root tool can manufacture worker execution or a root plan
before planning begins.

### T2: Start planner-agent from the successful ward transition

**Depends on:** T1

**Touches:** `runtime/agent-tools/src/tools/ward.rs` and its focused tests

**Tests:**

- TDD: root ward creation with a cold gate emits exactly one sequential,
  waiting planner action whose task names the actual ward. Covers AC2.
- TDD: ward list/info and delegated ward operations do not emit a planner
  action. Covers AC2 and AC4.

**Approach:**

- Reuse the existing tool-context event action path; do not call gateway
  services from the ward tool.
- Claim and mark the gate before assigning the action to make repeated calls
  harmless.

**Done when:** any valid ward establishment deterministically stops the root
for planner delegation.

### T3: Install the gate from graph intent bootstrap

**Depends on:** T1-T2

**Touches:** `gateway/gateway-execution/src/{middleware/intent_analysis.rs,runner/invoke_bootstrap.rs}` and focused tests

**Tests:**

- TDD: graph `create_new` and unassigned cold routing put the planner payload
  in root state. Covers AC1–AC3.
- TDD: simple and graduated existing-ward graph routing omit it. Covers AC4.

**Approach:**

- Extract the existing rich planner-task formatting once, reuse it for prompt
  guidance and the runtime gate, and append the ward at transition time.
- Install state only for root cold graph execution.

**Done when:** the runtime invariant is driven by structured intent output,
not only prompt compliance.

### T4: Verify and document the regression correction

**Depends on:** T1-T3

**Touches:** `docs/specs/graph-planning-gate/*`, `docs/specs/README.md`

**Tests:**

- Goal-based: run focused agent-tools, agent-runtime, and gateway-execution
  tests, `cargo fmt --check`, `cargo check` for touched crates, and
  `git diff --check`.

**Approach:**

- Record test evidence and mark satisfied acceptance criteria.
- Add the active spec index row after verification.

**Done when:** the regression is covered, constraints are met, and repository
checks are clean.

## Rollout

This is an immediate, reversible behavior correction with no migration. New
cold graph requests receive the gate after deployment; completed or crashed
historical sessions are not altered. Revert the small runtime state transition
to roll back.

## Risks

- An over-broad gate could interrupt Quick Chat or mature ward-agent work;
  bootstrap tests must prove routing exclusions.
- A planner task based on the recommendation rather than the successful ward
  would revive the workspace mismatch; ward tests inspect the task text.
- A duplicate action could spawn two planners; the transition claim must be
  tested.

## Changelog

- 2026-07-17: Initial plan for the cold graph planning-bypass correction.
- 2026-07-17: Implemented the executor-context gate, ward-triggered planner
  action, and focused regression coverage; package tests and gateway check pass.
- 2026-07-17: Extended the executor boundary to redirect MCP-shaped tool calls
  before external dispatch while cold graph work awaits ward entry.
