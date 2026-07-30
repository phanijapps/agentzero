# Spec: Fluid Ward Context Integration

- **Status:** Archived
- **Owner:** phanijapps
- **Plan:** [`plan.md`](plan.md)
- **Constrained by:** [ADR-0001](../../adr/0001-use-versioned-ward-layout-contracts.md), [RFC-0016](../../rfc/0016-generic-ward-configuration-and-layout-resolution.md), [`ward-layout-configuration`](../ward-layout-configuration/spec.md)
- **Brief:** none
- **Contract:** none; consumes the normalized ward snapshot
- **Shape:** integration

## Objective

Inject the active generic ward template into intent, planning, skills,
delegation, continuation, and ward reuse so models use what the user declared
instead of remembered paths. Planning and spec skills remain generic: they use
a declaration when present and do not require one when absent.

## Boundaries

### Always do

- Carry one snapshot digest and bounded normalized YAML projection through the
  whole ward-backed execution.
- Delimit the projection as untrusted data and resolve declared paths through
  the shared ward-layout service.
- Project only strict interpreter primitives, arbitrary safe role IDs, and
  ward-relative resolved values; exclude unknown metadata and scalar prose.
- Re-run the shared linter before ward-backed planning and after writes.

### Ask first

- Adding a mandatory artifact role or changing context projection semantics.

### Never do

- Teach concrete ward paths or mandatory spec/plan/task concepts in prompts.
- Keep `WardRecommendation.structure`, `ward-designer`, `ward_hygiene`, or
  legacy layout compatibility.
- Add UI, repair/approval services, or unrelated mutation APIs.

## Testing Strategy

- Context serialization and digest propagation use unit/integration tests.
- Prompt/skill behavior uses snapshot tests with incompatible templates.
- One E2E uses a template with no spec, plan, or tasks.

## Acceptance Criteria

- [ ] Intent analysis selects ward/concept/action only and never proposes a
  filesystem structure.
- [ ] Ward activation lints the snapshot and injects a bounded normalized
  typed-rule projection plus digest before planning; raw YAML, unknown metadata,
  instruction-like prose, secrets, provider values, file bodies, and absolute
  host paths never enter prompts, logs, or session state.
- [ ] Spec and planning skills use declared roles when present and remain
  safe when roles are absent or renamed: spec creation returns a bounded
  `role_not_declared` nudge and writes nothing, while planning uses the existing
  ephemeral session plan without creating a ward plan/task file.
- [ ] Delegation, continuation, and ward reuse carry the same digest and
  re-resolve after snapshot drift.
- [ ] Invalid snapshots/lint failures return the shared bounded nudge; no prompt
  invents a fallback path or repair.
- [ ] `ward-designer`, `ward_hygiene`, and active legacy path doctrine are
  removed with no compatibility behavior.

## Assumptions

- Product: a ward may be a knowledge workspace without specs or plans (source:
  user correction 2026-07-19).
- Technical: runtime consumers can store a generic normalized projection and
  digest without understanding role names (source: existing session/context
  structures).
