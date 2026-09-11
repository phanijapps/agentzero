# Spec: plan_attention warm-route scoping (ward-slim deferred item)

- **Status:** Shipped
- **Owner:** phanijapps
- **Plan:** none (single-phase, light mode with live gate)
- **Constrained by:** ward-slim P3 (deferred this exact item)
- **Brief:** shard contradiction — planner-regeneration advice vs the warm route's prohibition
- **Contract:** none
- **Shape:** integration

## Objective

`<plan_attention>` instructs root: "If the plan is unavailable, re-delegate
to planner-agent to regenerate it." On the warm route (existing ward), the
Task Analysis injection says the exact opposite: "Do NOT delegate to
`planner-agent`. Do NOT plan or manage steps yourself" — the ward-agent
plans internally, so root never holds a root-owned plan there. A root that
follows the shard on the warm route violates its own routing instruction.

Scope the regeneration line to the route the per-request Task Analysis
names — the same analysis-over-shard precedence P3 established.

## Acceptance criteria

- [x] AC1 — The line reads: plan unavailable → follow the current Task
  Analysis's route; only planner-routed work regenerates via planner-agent.
  No unconditional "re-delegate to planner-agent" remains in the shard.
- [x] AC2 — The rest of `<plan_attention>` is byte-identical; no other
  shard or injection text changes.
- [x] AC3 — Post-change live runs match baselines: fast-path
  `[ward, respond]`; graph-warm `ward → delegate(ward:…)` chain to respond,
  no planner-agent delegation on the warm route.

### Live evidence (2026-08-21)

- Baselines (develop build): fast `[ward, respond]`;
  warm `ward → delegate(ward:financial-analysis) → respond`.
- After the edit (daemon rebuild 21:45): fast `[ward, respond]` identical;
  warm `ward → connector_resource → delegate(research-agent)` — ward bound,
  delegated, zero planner-agent calls on either route.

## Boundaries

### Never do

- No changes to the planner gate, warm-route injection, or any runtime code.
- No new teaching — a scoping clause only.

## Testing strategy

Shard-content grep assertions (before/after), full gates, and both live
flows re-run on the rebuilt daemon (the high-stakes rule this deferral was
gated on).
