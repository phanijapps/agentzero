# Spec: Ward Slim P3 — Prompt-Teaching Dedup

- **Status:** Shipped
- **Owner:** phanijapps
- **Plan:** none (single-phase, full mode)
- **Constrained by:** [ward-slim P1+P2](../ward-slim/spec.md), [fast-path-ward-note](../fast-path-ward-note/spec.md), the orchestrator-context high-stakes rule (live multi-step verification before merge)
- **Brief:** ward-slim review (2026-08-20); contradiction found post-#252
- **Contract:** none
- **Shape:** integration

## Mode

Full (orchestrator hot path — every root system prompt changes). Loop-engine
scripts absent; spine manual. Branch `refactor/ward-slim-p3`, stacked on
`fix/agent-feedback-loops` (#253) — merge order #253 → this.

## Objective

The when-to-plan decision is stated ~7× across prompts while the runtime gate
already enforces it mechanically. Deduplicate to one statement per surface,
and remove the shard-level ward prohibition that now contradicts the
fast-path ward note shipped in #252/#253. Target: ~60 prompt lines (~700
tokens) off every root turn, 4 drift surfaces gone, zero behavior change in
either flow.

### Evidence

- `<fast_path_override>` (first_turn_protocol.md:5-7) says "Do not enter a
  ward…" — the per-request ward note now MANDATES ward entry for
  use_existing. Direct contradiction, resolved per-request only because the
  injection declares itself an override.
- `<first_actions>` (first_turn_protocol.md:18-24) restates the cold-graph
  recipe (ward → planner → steps) that the planning gate enforces
  mechanically (`planning_gate_blocks_tool` blocks every non-ward tool) and
  the injection teaches per-request.
- `planning_autonomy.md:12` restates the approach if/else ("plan-driven →
  planner-agent; ad-hoc → skip") that the classifier decides and the
  injection renders.
- Graph injection (intent_analysis.rs) carries "Do NOT delegate to a worker,
  planner, or ward-agent manually…" while the gate blocks `delegate_to_agent`
  mechanically during AwaitingWard.

### Shrink of record (AC5)

first_turn_protocol.md 5,507 → 5,269 B (−238), planning_autonomy.md 5,503 → 5,457 B (−46),
plus the cold-graph injection's gate-covered "Do NOT" sentence and the
fast-path `ward` prohibition removed per-request. ~110 prompt tokens off
every root turn; the larger original estimate (~700) assumed cutting
`<plan_attention>`/`<delegation_binding>`, which stayed (separate concerns).

### Baselines (live, WS harness, daemon at 10:06 build)

- fast-path ("rule of 72, no files"): tools `[ward, respond]`,
  ward=financial-analysis.
- graph-warm ("EV battery makers comparison page"): ward=financial-analysis,
  delegations `[ward:financial-analysis, web-researcher, writing-agent]`,
  full tool flow to `respond`.
- Post-change (daemon 10:16 build): fast-path `[ward, respond]` identical;
  graph-warm (solar-inverter variant) ward=financial-analysis, delegations
  `[ward:financial-analysis, web-researcher, ward:financial-analysis]`,
  surface created, flow to `respond` — same shape, no regressions.

## Acceptance criteria

- [x] AC1 — first_turn_protocol.md carries ONE `<task_entry>` block replacing
  `<fast_path_override>` + `<first_actions>`; it names the three routes
  (simple→direct, graph-cold→ward entry with auto-planner,
  graph-warm→per-analysis delegation) without re-teaching the planner recipe
  and without an UNCONDITIONAL ward prohibition (the Simple bullet carries
  the analysis-conditioned default; the Ward note overrides it by name).
- [x] AC2 — planning_autonomy.md no longer restates the planner-vs-direct
  if/else; its agent-table and delegation rules stay.
- [x] AC3 — intent injection: the graph branch drops the "Do NOT delegate…"
  sentence (gate-enforced); the fast-path prohibition drops `ward` from its
  list (the Ward note owns ward on that path); both paragraphs otherwise
  unchanged apart from one added pointer clause ("ward entry is governed
  by the Ward note below when one is shown"). The Ward note's own "one
  exception to the `ward` prohibition above" clause was dropped with it —
  the prohibition it referenced no longer exists.
- [x] AC4 — post-change live runs match baselines: fast-path still
  `[ward?, respond]`-shaped with no planner; graph-warm still enters the
  ward and delegates to the ward-agent; cold-graph gate behavior untouched
  (ward-only tools while awaiting).
- [x] AC5 — prompt-assembly + injection tests updated to the new wording;
  no test asserts the removed text anywhere; shard byte-shrink recorded in
  the spec.

## Boundaries

### Always do

- Keep per-request Task Analysis instructions authoritative over every
  shard default (the `<task_entry>` header states the precedence).

### Ask first

- Touching `<plan_attention>` — its planner-regeneration line still
  contradicts the warm route (pre-existing, same contradiction class;
  deferred as `ward-slim-p3-plan-attention-warm-scope`).

### Never do

- No runtime-semantics changes: gate, delegation, ward tool, intent
  classifier prompt untouched.
- Do not touch `<agent_loop>`, `<plan_attention>`,
  `<new_user_request_after_completion>`, `<delegation_binding>` — separate
  concerns, not planner-teaching.
- No vault-side prompt overrides (config/agent-prompts) — repo templates
  only; note the vault copy caveat in the PR.

## Deferred

- `ward-slim-p3-plan-attention-warm-scope` — scope
  `<plan_attention>`'s "re-delegate to planner-agent to regenerate" line to
  cold/planned work so it stops contradicting the warm route (high-stakes
  block; needs its own live verification).

Link note: `../ward-slim/spec.md` resolves on `develop` (merged via #251);
this branch forked before that merge, so the link is dead on the fork point
only.

## Testing strategy

Prompt-assembly tests (gateway-templates), injection renderer tests
(intent_analysis), e2e ward pipeline binary, then BOTH live flows re-run via
the WS harness against the rebuilt daemon and diffed against the baselines.
Gates: fmt, clippy, `cargo test -p gateway-execution -p gateway-templates`,
`cargo check --workspace`, UI suite untouched.
