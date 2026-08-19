# Spec: Planner Template Handoff

- **Status:** Shipped
- **Owner:** phanijapps
- **Plan:** [`plan.md`](plan.md)
- **Constrained by:** [ADR-0003](../../adr/0003-use-filesystem-authoritative-llm-wiki-wards.md), [RFC-0016](../../rfc/0016-generic-ward-configuration-and-layout-resolution.md), [RFC-0018](../../rfc/0018-filesystem-authoritative-llm-wiki-wards.md)
- **Brief:** none
- **Discovery:** none
- **Contract:** none
- **Shape:** integration

> **Spec contract:** this document defines what "done" means. The implementing
> PR must match this spec, or update it. Verification must be derivable from it.

## Objective

Cold graph planning independently reloads the validated Active Ward Template
for the exact ward selected by the successful `ward(create|use)` transition.
When that template declares an optional repeatable refinement parent containing
specification, plan, and repeatable task-file roles and intent supplies a
concrete refinement slug, `planner-agent` writes the required specification and
plan artifacts before it hands execution steps back to the root. Optional task
files are materialized only when the plan actually decomposes into tasks.
Templates without those roles retain the ephemeral session-plan behavior.

## Boundaries

### Always do

- Pass only the bounded, normalized, delimiter-safe template context produced by the ward-layout adapter.
- Fail closed with a bounded nudge and no planner execution when the selected template is unavailable or invalid.
- Permit only the exact bundled `planner-agent`, with a matching host-loaded ward packet, to run the read-only Ward lint action from delegated execution; keep every other template action root-only.
- Treat a concrete cold-graph planning invocation as a refinement run and use every applicable declared persistent planning role.
- Keep `role_not_declared` limited to roles genuinely absent from the active normalized template.

### Ask first

- Changing which ward-layout nodes are required during initial ward creation.
- Changing the public Ward Layout YAML schema or role-resolution semantics.
- Generating specification content in the host instead of through `planner-agent`.

### Never do

- Inject raw ward YAML, absolute template paths, unknown metadata, or instruction-like template prose into the planner prompt.
- Invent `.zbot/specs` or any fallback path when the active template lacks matching roles.
- Eagerly materialize optional or repeatable wildcard nodes during empty ward creation.

## Testing Strategy

- **TDD, unit surface:** automatic ward-triggered planner delegation carries the normalized template context and an explicit declared-role materialization contract.
- **TDD, prompt-contract surface:** planner instructions distinguish a declared applicable role from an absent role and require persistence for a concrete refinement run.
- **Goal-based integration gate:** existing ward-layout, graph-planning, delegation, and executor suites remain green, proving sparse ward creation and warm/simple behavior are unchanged.
- **Manual QA, real planner surface:** a graph refinement proves the selected ward alone receives required `spec.md` and `plan.md`, no placeholder task or task index is created when no task decomposition is needed, structured lint succeeds, and returned output carries the selected ward and digest. Optional task-file creation remains separately pinned by the prompt contract and is not required for a refinement with no task decomposition.
- **Security review:** the untrusted-layout boundary remains bounded and encoded through the existing adapter-produced context.

## Acceptance Criteria

- [x] A successful cold-graph `ward(create|use)` starts `planner-agent` with the exact normalized Active Ward Template context returned for the selected ward.
- [x] The planner executor independently loads the selected ward snapshot and includes the adapter-produced context byte-for-byte as a delimited untrusted-data block containing the template digest, with unknown metadata, instruction-like template prose, raw YAML, and absolute host template paths absent.
- [x] Given a normalized optional repeatable refinement parent containing required `spec.md` and `plan.md` roles plus optional repeatable Markdown task rules, and a concrete refinement slug, the planner persists the required artifacts at resolved ward-relative paths before execution steps are returned; task files are created only for real task decomposition and no task index is required.
- [x] `role_not_declared` is emitted only for an applicable artifact whose role is absent from the normalized template; user omission of the word “spec” is not sufficient.
- [x] Templates without persistent planning roles continue using only the ephemeral session plan and do not invent fallback paths.
- [x] An unavailable or invalid selected template fails closed before planner model execution, artifact persistence, or execution-step return.
- [x] Planner writes remain confined to the ward selected by the host-bound child session; planner output names the selected ward and template digest, and the exact bundled planner can run read-only post-write lint with its matching host-loaded packet before steps are returned.
- [x] Fresh ward creation continues materializing only required literal nodes; optional planning directories remain lazy.
- [x] Legacy and Rig execution paths both preserve the same planner handoff behavior, and existing graph-planning tests remain green.

## Assumptions

- Technical: `GatewayWardLayoutAccess` is the source of the bounded, escaped normalized template context and the planner reloads it from the host-bound child ward rather than model-supplied delegation context (source: `gateway/gateway-execution/src/invoke/ward_layout_adapter.rs`).
- Technical: `WardTool` loads the selected ward layout before constructing the automatic planner action (source: `runtime/agent-tools/src/tools/ward.rs`).
- Product: declared persistent planning roles are instantiated for a concrete graph refinement run (source: user confirmation 2026-08-18).
- Process: optional ward-layout nodes remain absent during empty ward creation and materialize on demand (source: `docs/specs/bundled-ward-archetypes/spec.md`).
- Process: normalized template projection remains untrusted data rather than instructions (source: ADR-0003 and RFC-0016).
