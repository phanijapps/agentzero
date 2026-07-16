# Spec: Session Plan Monitoring

- **Status:** Shipped
- **Owner:** phanijapps
- **Plan:** [`plan.md`](plan.md)
- **Constrained by:** none
- **Brief:** none
- **Contract:** [`contracts/openapi/mission-control.yaml`](../../../contracts/openapi/mission-control.yaml)
- **Shape:** mixed

> **Spec contract:** this document defines what "done" means. The implementing
> PR must match this spec, or update it. Verification must be derivable from it.

## Objective

Make a running session's latest structured plan available in Mission Control as
**Current plan**. Research presents that same plan once, as a Plan checklist;
it must no longer show a second, misleading Open loops rendering of identical
steps. Reserve Mission Control's durable `autonomy_items` workflow for
cross-session decisions and follow-ups, labelled **Decision threads**.

## Boundaries

### Always do

- Persist the latest validated `update_plan` payload for its session, including
  the trusted producing execution, source-event ordering value, explanation,
  and accepted update time.
- Expose the optional current-plan snapshot additively through the existing
  selected-session Mission Control token-detail response and show it only for
  the selected session.
- Keep a session's plan separate from semantic memory and from durable decision
  threads; a plan update never creates or transitions an autonomy item.
- Render only the existing native Plan checklist for Research and make legacy
  sessions without a saved plan display a clear empty state in Mission Control.

### Ask first

- Showing a session plan outside Research and Mission Control.
- Retaining plan revision history instead of only the latest snapshot.
- Adding plan edits, approvals, or autonomy actions to the Monitoring view.

### Never do

- Infer a durable decision thread from a plan step or silently change an
  autonomy item's state.
- Store plan snapshots in Engram or use them as recall/semantic-memory input.
- Add a dependency, a new top-level code directory, or a second plan endpoint.

## Testing Strategy

- **TDD:** plan snapshot validation, ordering, ownership, deletion, and Mission
  Control selected-session projection have compact data invariants and receive
  repository/service tests.
- **TDD:** the stream processor must persist an accepted `update_plan` before
  publishing its single Plan surface; tests exercise the observable persisted
  snapshot and surface descriptor.
- **Visual / manual QA:** Mission Control session selection shows a populated
  plan or its empty state, and Research shows one Plan checklist. Component
  tests assert the rendered headings and steps; a manual running-session check
  confirms the regular refresh displays a new update.
- **Security check:** rejected model payloads leave the prior snapshot intact,
  do not create a Research surface, perform no retry, and emit diagnostics
  containing only bounded reason codes and trusted correlation identifiers.
- **Goal-based check:** the additive REST contract is checked against the
  checked-in OpenAPI file and `cargo check -p daemon` verifies the full path.

## Acceptance Criteria

- [x] Given a valid `update_plan`, when its stream event is processed, the
  latest bounded snapshot is saved for that session with its trusted producing
  execution, durably issued per-session acceptance sequence, explanation, and
  accepted update time; only the latest server-accepted update replaces it.
- [x] Given an invalid, oversized, repository-stale, or cross-session plan update, the
  previous snapshot is unchanged. The gateway does not clone or broadcast its
  model-owned raw event, and neither persists nor projects a Research Plan
  surface for that rejected update.
- [x] Given a session without a saved plan, when Mission Control loads that
  selected session's existing token-detail response, its `current_plan` field is
  absent or null.
- [x] Given a session with a saved plan, when it is selected in Mission Control,
  the user sees a **Current plan** checklist with each step and status; a
  session without one shows **No plan recorded for this session.**
- [x] Given any `update_plan`, when Research receives the associated agent
  surface, it renders one **Plan** checklist and no agent-surface **Open loops**
  duplicate.
- [x] Given no durable autonomy items, when Mission Control loads, the separate
  panel is labelled **Decision threads** and reports no active decision threads;
  ordinary running-session plans do not affect its count.
- [x] The checked-in Mission Control OpenAPI contract describes the additive
  selected-session `current_plan` field and bounded plan-step shape, and no new
  endpoint is introduced.

## Assumptions

- Technical: `update_plan` emits `ActionPlanUpdate`, while the existing agent
  surface duplicates that payload under both `plan` and `open_loops` (source:
  `gateway/gateway-execution/src/invoke/stream_event_processor.rs`).
- Technical: Mission Control already refreshes its bounded existing session
  summary endpoint while any session is running (source:
  `apps/ui/src/features/mission-control/MissionControlPage.tsx`).
- Technical: `autonomy_items` is a separate durable store and the active local
  database currently has no rows (source: read-only SQLite query on
  `~/Documents/zbot/data/conversations.db` 2026-07-14).
- Product: Research uses Plan only; Monitoring presents a persisted Current
  plan; durable Open Loops is renamed Decision threads (source: user
  confirmation 2026-07-14).
- Security: only model-provided plan step text, status, and optional explanation
  are accepted as input. Session/execution ownership, source-event ordering, and
  persistence timestamps are derived by the server from the stream context and
  runtime event, never from the model payload. The execution-state database
  durably and atomically issues a session-scoped monotonic sequence in the same
  transaction as an accepted snapshot, so raw wall-clock time cannot define
  plan-update order and a restart cannot reset it.
- Process: active specs use the project metadata, acceptance-criteria, and
  contract conventions (source: `docs/CONVENTIONS.md` §4).
