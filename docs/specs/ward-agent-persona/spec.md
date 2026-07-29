# Spec: Ward Agent Persona

- **Status:** Shipped
- **Owner:** phanijapps
- **Plan:** [`plan.md`](plan.md)
- **Constrained by:** user confirmation 2026-07-20
- **Brief:** none
- **Contract:** none
- **Shape:** integration

> **Mode:** light (no risk trigger fired).
>
> **Spec contract:** this document defines what "done" means. The implementing
> change must match this spec, or update it.

## Objective

Make every newly scaffolded ward a recognizable persistent agent. Its
user-editable `AGENTS.md` defines a ward-specific identity, persona, scope,
operating principles, knowledge-navigation behavior, workflow, bounded
self-maintenance, and handoff contract. Synthesized `ward:<name>` agents use
that document as their durable doctrine across sessions.

## Boundaries

### Always do

- Derive the visible agent identity from the safe canonical ward identifier.
- Keep the persona and operating contract independent of any concrete ward layout.
- Preserve `AGENTS.md` as user-owned doctrine loaded by the synthesized Ward agent.
- Limit automatic creation to new wards; legacy wards require rebuild or an
  explicit user-approved replacement.

### Ask first

- Adding model-generated persona fields to the Ward tool interface.
- Automatically rewriting an existing customized `AGENTS.md`.

### Never do

- Hardcode domain personas or artifact paths in Rust.
- Change template propagation, concept creation, lint enforcement, or routing in this pass.
- Add a new module, dependency, persistence field, or compatibility layer.

## Testing Strategy

- **TDD:** scaffolding tests assert the complete persona contract and prove it
  remains layout-neutral.
- **TDD:** synthesis tests prove Ward doctrine is preserved in the generated
  agent instruction and empty doctrine remains safe.

## Acceptance Criteria

- [x] A newly scaffolded ward receives an `AGENTS.md` with Identity, Persona,
  Purpose and Scope, Operating Principles, Knowledge Navigation, Workflow,
  Self-Maintenance, and Handoff sections.
- [x] The identity contains a readable ward-specific display name and persistent
  Ward-agent role without embedding domain-specific Rust rules.
- [x] Persona instructions require evidence-aware judgment, explicit uncertainty,
  reuse of existing knowledge, and template-directed filesystem behavior.
- [x] Self-maintenance proposes durable doctrine changes in the handoff and
  edits `AGENTS.md` only with explicit user direction; it forbids per-run
  details and deletion or rewriting of existing persona text.
- [x] Synthesized `ward:<name>` agents load the complete `AGENTS.md` as doctrine.
- [x] Existing customized Ward doctrine is never automatically overwritten.

## Assumptions

- Technical: Ward scaffolding owns initial `AGENTS.md` content in
  `gateway/gateway-services/src/ward_layout/create.rs` (source: repository inspection 2026-07-20).
- Technical: synthesized Ward agents load `wards/<ward>/AGENTS.md` as doctrine in
  `gateway/gateway-execution/src/invoke/setup.rs` (source: repository inspection 2026-07-20).
- Product: each ward is a persistent self-agent with a persona (source: user confirmation 2026-07-20).
- Process: this is a light single-purpose work-loop and must not expand into the
  pending OKF execution fixes (source: user confirmation 2026-07-20).
