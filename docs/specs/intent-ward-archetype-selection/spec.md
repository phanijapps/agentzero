# Spec: Intent Ward Archetype Selection

- **Status:** Draft
- **Owner:** phanijapps
- **Plan:** [`plan.md`](plan.md)
- **Constrained by:** [RFC-0017](../../rfc/0017-ward-layout-archetypes.md), [ADR-0002](../../adr/0002-select-complete-ward-archetypes-at-creation.md)
- **Brief:** `dynamic-ward-layout-archetypes`
- **Contract:** none
- **Shape:** integration

Product provenance: [Dynamic Ward Layout Archetypes brief](../../product/briefs/dynamic-ward-layout-archetypes.md).

> **Spec contract:** this document defines what "done" means. The implementing
> PR must match this spec, or update it. Verification must be derivable from it.

## Objective

When intent analysis recommends creating a new ward, carry one closed
archetype recommendation through trusted runtime state into ward creation.
Explicit user choice wins; ambiguous, unavailable, or invalid classifier
output becomes `generic`; and any request routed to an existing ward ignores
archetype classification and preserves that ward's snapshot.

## Boundaries

### Always do

- Parse model output as `WardArchetypeId` at the intent boundary and pass only
  the typed value through trusted root tool state.
- Classify by the primary requested deliverable/lifecycle in RFC-0017 and use
  `generic` whenever exactly one specialized row is not established.
- Keep explicit override precedence and existing-ward reuse behavior covered
  by integration and clean-start E2E tests.

### Ask first

- Changing the selection matrix, ambiguity rule, or explicit-override
  precedence.
- Adding a confidence score, user confirmation flow, UI selector, or telemetry
  retention beyond existing intent/ward records.
- Preserving `WardRecommendation.structure` for any consumer after its
  structural authority is removed.

### Never do

- Never forward model-authored YAML, paths, starter content, free-form
  structure, or unknown identifiers to ward creation.
- Never reclassify, migrate, rewrite, or compare a selected archetype against
  an existing ward during reuse.
- Never add a parallel intent service, layout router, top-level crate, or
  prompt-owned filesystem policy.

## Testing Strategy

- **Closed selection, precedence, and ambiguity rules — TDD:** table-driven
  fixtures make the trust-boundary behavior concrete.
- **Intent-to-tool handoff — goal-based integration tests:** prove the parsed
  enum, action, and override reach the existing adapter without string/path
  re-interpretation.
- **Existing ward immutability — goal-based integration tests:** use two
  conflicting asks and compare snapshot bytes/digest before and after reuse.
- **Automatic clean-start journey — goal-based E2E:** a fresh database and
  vault prove prompt schema, intent parsing, trusted state, seeding, and
  creation together.

TDD stub handoff: the closed schema and precedence matrix are concrete Rust
tests in `intent_analysis.rs`. Per `new-spec`, no red stub is committed during
spec authoring; `work-loop` PLAN must materialize and compile the named T1/T2
stubs before EXECUTE.

## Acceptance Criteria

- [ ] `WardRecommendation` exposes an optional closed `archetype` field and no
  model-visible or runtime-consumed free-form `structure` field.
- [ ] The intent prompt/schema uses the RFC-0017 selection matrix and requires
  `generic` when no specialized row or multiple specialized rows are primary.
- [ ] Positive, negative, and ambiguous fixtures cover every archetype:
  executable software → `coding`; durable explanatory knowledge →
  `documentation`; first-person chronology → `journal`; long-form
  book/chapter work → `ebook`; provenance-led non-news synthesis →
  `research`; time-sensitive fact-checked briefing → `news`; mixed/unknown →
  `generic`.
- [ ] A valid explicit creation override takes precedence over the intent
  recommendation; an invalid explicit override is rejected with a bounded
  error and is never silently normalized.
- [ ] Missing intent analysis, classifier failure, a missing archetype field,
  or a model-invented/unparseable value becomes `generic` before calling the
  creation service.
- [ ] Automatic archetype state is populated only when
  `WardAction::CreateNew`; `UseExisting` clears/ignores it even if serialized
  input contains an archetype.
- [ ] The root execution/tool state carries only the typed selected identifier
  and its selection source (`explicit`, `intent`, or `fallback`); delegated
  prompts cannot mutate that value or supply a replacement path.
- [ ] Creating through the `ward` tool passes the trusted value exactly once
  to `WardLayoutAccess::create`; retry or continuation cannot silently select
  a different archetype for the same creation attempt.
- [ ] Given an existing coding ward, a later news ask routed to that ward
  leaves its snapshot bytes, digest, materialized tree, and provenance
  unchanged.
- [ ] Intent formatting may explain the selected archetype but never renders
  free-form structure as authorized paths or instructions.
- [ ] Loading an upgraded vault cannot expose the legacy model-facing
  `structure` response contract: an unchanged previously seeded prompt
  migrates to the new default, while a custom prompt preserves unrelated user
  prose but has legacy structure-contract lines removed before model use and
  receives the authoritative closed contract.
- [ ] With a fresh database and vault, an unambiguous coding ask creates a
  coding ward, an ambiguous coding-and-news ask creates a generic ward, and a
  subsequent conflicting ask that reuses either ward does not reclassify it.
- [ ] Automatic selection cannot be enabled in a shipped build until the
  all-seven bundled-archetype construction and clean-start gate is green;
  explicit selection of a missing or invalid configured bundle still fails
  closed.

## Assumptions

- Technical: `WardRecommendation` and `WardAction` are the current structured
  intent contract (source:
  `gateway/gateway-execution/src/middleware/intent_analysis.rs`).
- Technical: the `ward` tool and `WardLayoutAccess` are the existing creation
  handoff and can accept a typed optional value without a new external
  contract (source: `runtime/agent-tools/src/tools/ward.rs`, user confirmation
  2026-07-26).
- Process: selection follows RFC-0017 and ADR-0002 and cannot restore
  prompt-owned layout authority removed by RFC-0016 (source: cited governance
  documents).
- Product: automatic selection occurs only for new wards; explicit choice
  wins; `generic` is the ambiguity and failure fallback (source: user
  confirmation 2026-07-26).
- Product: the authoritative acceptance run uses a fresh database (source:
  user confirmation 2026-07-26).
