# Spec: Ward Slim P4 — Tool Audience Split

- **Status:** Shipped
- **Owner:** phanijapps
- **Plan:** none (single-phase, light-plus mode: tool-surface change on the orchestrator path with a live smoke)
- **Constrained by:** [ward-slim P1+P2](../ward-slim/spec.md) (merged), [ward-slim P3](../ward-slim-p3/spec.md) (#254)
- **Brief:** ward-slim program, final phase with payload payoff
- **Contract:** none
- **Shape:** service

## Objective

The ward tool advertised the same 8 actions to every actor, inviting calls
the guard layer rejects. Split the model-facing surface to mirror what
`validate_template_context` actually permits per actor (review-corrected —
the original 5-vs-8 premise inverted two actions):

| Action | Root | Delegated planner | Ward-agent / other subagents |
|---|---|---|---|
| use / create / list / info / search | ✓ | ✓ | ✓ |
| `dry_run` / `create_concept` | ✓ (plan-composer drives them from root-owned setup steps) | ✗ (`root_required`) | ✗ |
| `lint` | ✗ hidden (planner owns conformance) | ✓ (read-only post-write lint) | ✗ |

Runtime `execute()` stays permissive for all actions on every instance —
the guards remain the enforcement layer; this change is declaration-only.

## Acceptance criteria

- [x] AC1 — `WardTool` carries a `WardAudience` (`Full`/`Root`/`Planner`/
  `Subagent`) with per-action visibility mirroring the guard table above;
  constructors `for_root`/`for_planner`/`for_subagent`; `description()` and
  `parameters_schema()` render exactly the visible set (root: concept
  actions present, no `lint`; planner: `lint` present, no concept actions;
  subagent: lifecycle only).
- [x] AC2 — executor registration derives the audience from the actor
  (Root → root set; others → lifecycle-only) with an explicit override for
  the delegated planner spawn (`ward_audience_for_child`, discriminator
  `child_agent_id == "planner-agent"` as at spawn.rs:267/326/852; pinned by
  a spawn-side unit test).
- [x] AC3 — `execute()` behavior is unchanged for every action on every
  instance (guards still gate template actions; a hallucinated root
  `lint` call still runs the same code path).
- [x] AC4 — tests pin each audience's surface (description + schema
  content per constructor) and the registration choice (planner vs default actor);
  existing ward suite + e2e ward pipeline stay green.
- [x] AC5 — live smoke on the rebuilt daemon: fast-path run still enters
  the ward first (`ward → … → respond`), planner path unaffected (covered
  by e2e + the graph gate tests). Smoke of record: 2026-08-21, WS probe
  "Compare Microsoft and Apple…" — first tool `ward`, `present_surface` ok,
  surface created, `respond` (daemon build 13:36; pre-review-audience build
  — the corrected audiences change only the declaration, re-verified by
  gates).

## Boundaries

### Never do

- No changes to `execute()` arms, guards, gate, or delegation.
- No new tool, state key, or schema field beyond the action filter.
- No new runtime rejection of template actions on the root instance
  (declaration-only split; enforcement stays with
  `validate_template_context`).

### Always do

- Keep `new()`'s signature unchanged (existing callers: tests + planner).

## Testing strategy

TDD in `ward.rs` (surface tests per constructor, red-first for the
root-facing set), executor registration test, gates: fmt, clippy,
`cargo test -p agent-tools -p gateway-execution`, e2e ward binary,
workspace check, one live fast-path probe.
