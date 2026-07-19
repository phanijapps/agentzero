# Plan: Observatory scale-safe summaries

- **Spec:** [`spec.md`](spec.md)
- **Status:** Executing

## Approach

The initial graph payload is already bounded, so the fix stays on the server
summary path. Replace Engram's JSON-materializing aggregate counters and
hierarchy summary with SQL aggregation over its sidecar columns. In the gateway,
return the documented empty hierarchy response before touching the store when
the feature is disabled. No UI contract, visual cap, or stored data changes.

## Constraints

- Keep `KnowledgeGraphStore` trait and existing `/api/graph/stats` and
  `/api/hierarchy/stats` DTOs compatible.
- Retain the durable relationship deduplication behavior; counting rows is
  correct because writes canonicalize a relationship before it is stored.
- Use the existing SQLite sidecar schema; add no dependency or table-data
  migration. Add only compatible indexes for enabled hierarchy summaries.

## Construction tests

**Integration tests:** Engram knowledge graph tests write invalid JSON payloads
directly into scalar-valid sidecar rows, then verify count and hierarchy reads.

**Manual verification:** start the daemon with hierarchy disabled, open
`/observatory`, then enable hierarchy and verify its health strip still reports
the same fields for existing graph data.

## Design (LLD)

### Design decisions

Use SQL `COUNT(*)`, `GROUP BY`, and `json_extract(properties_json, ...)` for
the small aggregate fields rather than deserialize `entity_json` and
`relationship_json`. This preserves the source-of-truth data and avoids a
materialized object graph. Traces to: AC 1-3.

### Data & schema

`kg_entities` already stores `agent_id`, `pruned`, `layer`, `mention_count`,
and `properties_json`; `kg_relationships` already stores `agent_id`,
`archived`, and `is_inter_cluster`. The change reads these existing columns and
adds compatible indexes for active layer and inter-cluster lookups; there is no
table-data migration. Traces to: AC 1, AC 3.

### Interfaces & contracts

No public schema changes. `GET /api/graph/stats` and `GET
/api/hierarchy/stats` return their current JSON structures. Traces to: AC 1-3.

### Failure, edge cases & resilience

Malformed full JSON payloads do not affect summary reads because summaries use
scalar columns. Store errors retain the existing empty-summary fallback. A
disabled hierarchy returns before acquiring the graph store. Traces to: AC 1-3.

### Quality attributes (NFRs)

The full graph JSON payload is no longer deserialized to serve Observatory
summary requests. Thus request work is bounded by SQLite aggregate operations,
not by the memory allocation and JSON decoding cost of 100,000 graph records.
Traces to: AC 1-4.

## Tasks

### T1: Aggregate counts avoid graph-payload deserialization

**Depends on:** none

**Touches:** `stores/zbot-engram-adapter/src/stores/knowledge_graph.rs`, `stores/zbot-engram-adapter/tests/knowledge_graph.rs`

**Tests:**
- Insert scalar-valid malformed entity and relationship JSON, then assert both
  aggregate counters return the correct result. Verifies AC 1.

**Approach:**
- Replace materialized entity and relationship count paths with active-row SQL
  `COUNT(*)` queries.

**Done when:** the new regression test passes and graph count responses do not
decode graph JSON.

### T2: Hierarchy summaries are opt-in and aggregate-only

**Depends on:** T1

**Touches:** `stores/zbot-engram-adapter/src/stores/knowledge_graph.rs`, `gateway/src/http/hierarchy.rs`, `stores/zbot-engram-adapter/tests/knowledge_graph.rs`

**Tests:**
- Insert scalar-valid malformed hierarchy rows, then assert layer, aggregate,
  and inter-cluster summaries remain correct. Verifies AC 3.
- Gateway compilation and manual disabled-hierarchy request verify the store is
  not read when the feature is off. Verifies AC 2.

**Approach:**
- Use sidecar layer, inter-cluster, and properties JSON scalar expressions.
- Short-circuit the gateway response when hierarchy is disabled.

**Done when:** enabled summaries remain compatible and disabled hierarchy does
not acquire a graph summary.

### T3: Verify the production path remains bounded

**Depends on:** T1, T2

**Touches:** `docs/specs/observatory-scale/*`

**Tests:**
- Existing Observatory hook tests assert the 200 / 500 fetch caps. Verifies AC 4.

**Approach:**
- Run focused Rust and UI tests plus formatting/type checks; make no visual-cap
  increase.

**Done when:** focused test suites and mechanical gates pass.

## Rollout

**Delivery:** normal deploy; no flag or migration. The change is backwards
compatible and can be rolled back as code only. Existing databases already
contain every column used by the queries.

## Risks

- Legacy direct database writes could create physical duplicate relationships;
  normal adapter writes canonicalize them. This patch preserves that established
  invariant rather than changing deduplication on the read path.
- `properties_json` must remain valid JSON for hierarchy aggregates. It is
  adapter-owned and valid for stored entities; malformed full payloads are
  explicitly covered without relying on them.

## Changelog

- 2026-07-17: initial plan after tracing the Observatory mount requests and
  Engram sidecar read paths.
