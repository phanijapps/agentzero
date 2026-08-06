# Spec: Observatory scale-safe summaries

- **Status:** Shipped
- **Owner:** phanijapps
- **Plan:** [`plan.md`](plan.md)
- **Constrained by:** none
- **Brief:** none
- **Contract:** none — existing HTTP response shapes remain unchanged
- **Shape:** service

> **Spec contract:** this document defines what "done" means. The implementing
> PR must match this spec, or update it. Verification must be derivable from it.

## Objective

Keep the Observatory responsive when the Engram-backed knowledge graph grows
past 100,000 entities and relationships. The page must retain its existing
bounded graph preview while its health and hierarchy summaries query scalar
database columns instead of loading and deserializing the whole graph.

## Boundaries

### Always do

- Preserve the existing `/api/graph/stats` and `/api/hierarchy/stats` response shapes.
- Keep the existing 200-entity / 500-relationship visualization cap.
- Query only active (`pruned = 0` / `archived = 0`) graph records.

### Ask first

- Raising the browser graph cap or replacing the SVG visualization.
- Introducing a cache, background job, or new Observatory API endpoint.
- Changing graph relationship deduplication semantics.

### Never do

- Send all graph nodes or relationships to the browser by default.
- Decode every graph JSON payload merely to calculate counts or hierarchy health.
- Modify user graph data as part of a read-path performance fix.

## Testing Strategy

- **TDD:** Engram integration tests insert malformed graph payloads and assert
  aggregate counts and enabled hierarchy summaries still work. This proves the
  read paths use indexed scalar columns rather than deserialize graph JSON.
- **Goal-based check:** targeted Engram tests, gateway compilation, formatting,
  and the existing UI Observatory tests keep the response and rendering path
  compatible.
- **Manual QA:** load `/observatory` with hierarchy disabled and enabled; the
  initial page still shows the bounded preview and status bar without a full
  graph read.

## Acceptance Criteria

- [x] Aggregate graph counts return the active entity and relationship totals
  without deserializing entity or relationship JSON payloads.
- [x] Given hierarchy is disabled, when `/api/hierarchy/stats` is requested,
  the response remains disabled with an empty summary and does not query the
  knowledge graph store.
- [x] Given hierarchy is enabled, hierarchy layer counts, inter-cluster count,
  and top aggregates are derived from scalar sidecar columns and keep the
  current response shape.
- [x] The default Observatory remains a bounded 200-node / 500-edge preview;
  it does not render a 100,000-node SVG graph.

## Assumptions

- Product: 100,000 refers to the persisted graph cardinality, not to a demand
  to render 100,000 SVG nodes (source: user request 2026-07-17; current UI at
  `apps/ui/src/features/observatory/graph-hooks.ts`).
- Technical: Engram is the selected memory provider and persists graph summary
  columns in `kg_entities` and `kg_relationships`; hierarchy aggregates always
  occupy layers above zero (source:
  `stores/zbot-engram-adapter/src/stores/knowledge_graph.rs`).
- Technical: the active screen is `/observatory`; its health bar requests both
  graph and hierarchy summaries at mount (source: `apps/ui/src/App.tsx` and
  `apps/ui/src/features/observatory/LearningHealthBar.tsx`).
- Process: existing REST DTOs are compatibility surfaces and this change must
  remain additive internally (source: `docs/specs/engram-memory-engine-cutover/plan.md`).
