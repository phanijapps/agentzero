# Spec: Subagent Capability Policy

- **Status:** Done
- **Owner:** phanijapps
- **Plan:** [`plan.md`](plan.md)
- **Constrained by:** none

> **Spec contract:** this document defines what "done" means. The implementing
> PR must match this spec, or update it. Verification must be derivable from it.

## Objective

Make agent execution roles enforceable through an internal capability policy
instead of prompt text or ad-hoc tool lists. z-Bot should support an explicit
`reviewer-agent` under the vault agent directory, route review work to a
read-only reviewer profile, keep executor subagents able to implement work, and
preserve ward-as-agent as a full-tool actor. The policy must distinguish root,
delegated executor, delegated reviewer, and ward-agent execution so future tools
cannot accidentally cross role boundaries.

## Boundaries

The three-tier guard that keeps an implementing agent inside the lines.
*Always do* applies without asking; *Ask first* requires human sign-off
before proceeding; *Never do* is a hard rule, even under time pressure.

### Always do

- Create or seed an explicit `reviewer-agent` identity through the bundled
  default-agent templates (`gateway/templates/default_agents.json` and
  `gateway/templates/agents/reviewer-agent.md`) so it lands in the vault agent
  surface (`~/Documents/zbot/agents/reviewer-agent/` in a normal daemon
  install) and root agents can delegate review work to a named reviewer.
- Enforce actor capability policy in Rust at executor/tool-registry construction
  time; reviewer safety must not depend only on `AGENTS.md` instructions or
  `agents/*/config.yaml`.
- Add an internal actor kind model with at least these profiles:
  `Root`, `DelegatedExecutor`, `DelegatedReviewer`, and `WardAgent`.
- Add an internal capability model for first-party tools, with capabilities at
  least covering filesystem read/write, shell/process execution, memory
  read/write, ward read/write, graph read, ingest/write, goal mutation,
  response, skill/MCP listing/loading, multimodal analysis, and agent
  orchestration/control.
- Treat ordinary delegated executor and reviewer agents as
  non-orchestrating: they may finish their assigned work, but may not spawn,
  steer, wait for, kill, clarify with, or message other agents.
- Keep executor subagents capable of implementation work: `shell`,
  `write_file`, and `edit_file` remain available to executor-role subagents.
- Keep reviewer subagents read-only in V1: reviewers can inspect files and
  context and respond with findings, but cannot mutate project state or run
  arbitrary commands.
- Treat `ward:<name>` warm ward agents as `WardAgent`, not as ordinary delegated
  reviewers/executors. Ward agents must retain full first-party tool access for
  the execution context, including implementation tools and orchestration/control
  tools when the required services are wired.
- Inject actor state into executor context so future tools can enforce the same
  policy consistently (`app:actor_kind`, `app:is_delegated`,
  `app:subagent_role` where applicable, and explicit capability flags if
  useful).

### Ask first

- Adding a user-facing `allowedTools`, `deniedTools`, or equivalent tool policy
  field to `agents/*/config.yaml`.
- Allowing reviewer agents to run any shell command, even apparently read-only
  commands.
- Adding a third `Leaf` enum variant or changing existing persisted execution
  records to store a new role field.
- Changing root/orchestrator tool availability beyond what is required to make
  the same registry pass through capability filtering.
- Restricting `ward:<name>` ward agents below full first-party tool access.
- Making `clarify` or peer `message_agent` tools part of this implementation
  slice.
- Applying the policy to external MCP/plugin tools beyond conservative current
  behavior unless the implementation can classify them without schema changes.

### Never do

- Never rely on a reviewer-agent prompt alone as the security boundary.
- Never make reviewer agents capable of `shell`, `write_file`, `edit_file`,
  ward mutation, or memory mutation in V1.
- Never let delegated subagents register `delegate_to_agent`, `wait_agent`,
  `kill_agent`, `steer_agent`, future `clarify`, or future `message_agent`.
- Never remove implementation tools from executor subagents as part of this
  reviewer hardening.
- Never collapse `ward:<name>` agents into the ordinary delegated reviewer or
  delegated executor policy.
- Never introduce remote/serverless execution, durable work queues, or plugin
  lifecycle hooks in this spec.

## Testing Strategy

- Capability policy and tool inventory: **TDD**. Tool-to-capability mappings and
  actor-to-capability policies are compact invariants and should be pinned with
  unit tests before registry refactoring.
- Delegation role propagation: **TDD**. Tests should prove a review task
  reaches `ExecutorBuilder` as reviewer actor and receives the reviewer
  inventory, while implementation tasks receive executor inventory.
- Ward-agent preservation: **TDD**. Tests should prove `ward:<name>` targets are
  classified as `WardAgent` and receive the union of root/orchestration and
  implementation tools that are wired for the execution context.
- Reviewer prompt alignment: **goal-based check**. Grep or unit tests should
  prove reviewer rules no longer instruct reviewers to run code when `shell` is
  absent.
- Agent surface seeding: **goal-based check**. A file existence/config-load
  check should prove `reviewer-agent` can be loaded from the normal agent
  service path and appears in delegation candidates.
- Workspace compatibility: **goal-based check**. `cargo check --workspace` and
  targeted gateway-execution tests prove the registry refactor does not break
  root execution, ward-agent warm paths, or existing executor subagents.

## Acceptance Criteria

- [x] A `reviewer-agent` identity exists in the default/vault agent surface and
  loads through `AgentService` with review-focused instructions. Existing
  installs can also create the same folder directly under
  `~/Documents/zbot/agents/reviewer-agent/`, but bundled seeding is the repo
  source of truth.
- [x] Review-like tasks can be delegated to the reviewer agent without relying
  on ad-hoc prompt wording in the root agent.
- [x] `ExecutorBuilder` has an actor-aware construction path that distinguishes
  root, delegated executor, delegated reviewer, and ward-agent execution.
- [x] First-party tools are registered through an internal capability policy,
  not only through hand-maintained root/subagent tool lists.
- [x] Delegated executor subagents have implementation tools including
  `shell`, `write_file`, and `edit_file`, and do not have orchestration tools.
- [x] Delegated reviewer subagents have read-only inspection/reporting tools and
  include `read`, `glob`, and `grep`; they do not have `shell`, `write_file`,
  `edit_file`, ward mutation, memory
  mutation, or orchestration tools.
- [x] `ward:<name>` ward agents retain full first-party tool access for their
  execution context, including `shell`, `write_file`, `edit_file`, ward/memory
  tools, and orchestration/control tools when their backing services are wired.
- [x] Root/orchestrator agents keep the existing orchestration tools required
  for delegation, steering, waiting, killing, planning, and final response.
- [x] The runtime context for tools exposes actor/capability metadata
  that future tools can use to reject out-of-role calls.
- [x] Reviewer prompt rules match the enforced capability set and do not ask
  reviewers to run commands.
- [x] Existing role-detection tests still pass, and new tool-inventory tests
  cover root, executor, reviewer, and ward-agent inventories.

## Assumptions

- Technical: `SubagentRole` currently has only `Executor` and `Reviewer`, and
  it only drives prompt rules. (source:
  `gateway/gateway-execution/src/invoke/setup.rs`)
- Technical: delegated agents currently get one shared subagent tool inventory
  with mutation-capable tools. (source:
  `gateway/gateway-execution/src/invoke/executor.rs`)
- Technical: warm ward routing uses `ward:<name>` targets after a graduation
  gate in intent bootstrap, and delegated spawn resolves those targets to the
  ward's own execution context. (source:
  `gateway/gateway-execution/src/runner/invoke_bootstrap.rs`,
  `gateway/gateway-execution/src/delegation/spawn.rs`)
- Technical: agent config currently supports identity, provider/model,
  instructions, skills, and MCPs, but not a core Rust tool allowlist. (source:
  `gateway/gateway-services/src/agents.rs`)
- Product: reviewer identity should be explicit as `reviewer-agent` in the
  normal vault/default agent surface, including the live
  `~/Documents/zbot/agents` location. (source: user confirmation 2026-06-01)
- Product: V1 should not add a third `Leaf` enum variant; delegated subagents
  are leaf-for-orchestration while `Executor`/`Reviewer` decide mutation rights.
  (source: user confirmation 2026-06-01)
- Product: future `clarify` and `message_agent` should be reserved for
  root/orchestrator roles, but not implemented in this spec. (source: user
  confirmation 2026-06-01)
- Product: ward-as-agent execution should retain access to all first-party tools
  rather than inheriting reviewer/executor subagent restrictions. (source: user
  confirmation 2026-06-01)
- Process: active specs use Objective, Boundaries, Testing Strategy, Acceptance
  Criteria, and Assumptions as the implementation contract. (source:
  `docs/specs/runtime-context-control/spec.md`,
  `docs/specs/durable-ward-memory/spec.md`)
