# Plan: Distillation cleanup — move domain logic into the engram adapter

## Problem

Distillation (2,286 lines) wraps every raw store call with its own domain
logic — supersede decisions, name normalization, governance checks, relationship
canonicalization. The adapter has the CRUD methods (`supersede_fact`,
`upsert_entity`, `resolve_entity`) but they're too low-level for what
distillation needs. The adapter should own the domain policy, not just the
storage mechanics.

## Architecture

```
Current (wrong):
  distillation → checks manually → calls adapter.supersede_fact()
  distillation → normalizes name → calls adapter.upsert_entity()
  distillation → applies governance → calls adapter.upsert_relationship()

Target (right):
  distillation → calls adapter.save_distilled_fact()     (adapter handles supersede)
  distillation → calls adapter.save_governed_entity()    (adapter handles normalization)
  distillation → calls adapter.save_governed_relationship() (adapter handles canonicalization)
```

## Changes

### 1. Engram adapter: `MemoryFactStore::save_distilled_fact()`

**What moves:** `upsert_facts_with_dedup` (~149 lines from distillation) +
3 inline supersede sites (~100 lines)

```rust
// In stores/zbot-engram-adapter/src/stores/memory_facts.rs
async fn save_distilled_fact(
    &self,
    agent_id: &str,
    category: &str,
    key: &str,
    content: &str,
    confidence: f64,
) -> Result<FactDisposition, String>;

pub enum FactDisposition {
    Stored { id: String },
    Superseded { old_id: String, new_id: String },
    Unchanged,  // same content, no supersede needed
}
```

The adapter:
1. Looks up existing fact by (agent_id, category, key)
2. If same content → `Unchanged`
3. If different content → save new fact, supersede old → `Superseded`
4. If no existing → save → `Stored`

This is where supersede logic BELONGS — it's persistence policy.

### 2. Engram adapter: `KnowledgeGraphStore::save_distilled_entity()`

**What moves:** `project_distilled_graph` entity half (~200 lines) +
`normalized_graph_name` + `governed_entity_type`

```rust
// In stores/zbot-engram-adapter/src/stores/knowledge_graph.rs
async fn save_distilled_entity(
    &self,
    agent_id: &str,
    surface_name: &str,
    raw_type: Option<&str>,
    properties: Value,
) -> Result<Option<EntityId>, StoreError>;
```

The adapter:
1. Normalizes the entity name (existing `normalized_graph_name` logic)
2. Resolves existing entity by normalized name (existing `resolve_entity`)
3. Maps `raw_type` to governed `EntityType` (existing `governed_entity_type`)
4. If ungoverned type → return `None` (dropped)
5. If existing entity → bump mention, merge properties, return existing `EntityId`
6. If new → `upsert_entity`, return new `EntityId`

### 3. Engram adapter: `KnowledgeGraphStore::save_distilled_relationship()`

**What moves:** `project_distilled_graph` relationship half (~200 lines) +
`canonicalize_relationship` + `governed_relationship_type` +
`resolve_relationship_endpoint`

```rust
async fn save_distilled_relationship(
    &self,
    agent_id: &str,
    source_id: &EntityId,
    target_id: &EntityId,
    raw_type: &str,
    properties: Value,
) -> Result<Option<RelationshipId>, StoreError>;
```

The adapter:
1. Maps `raw_type` to governed `RelationshipType` (existing logic)
2. If ungoverned type → return `None` (dropped)
3. Canonicalizes direction (existing `canonicalize_relationship`: uses vs used_by)
4. Drops self-loops
5. Resolves endpoints (existing `resolve_relationship_endpoint`)
6. Calls `upsert_relationship`

### 4. Strategy emergence: stays in distillation (it's a feature)

`try_cluster_failures` + `try_emerge_strategy` (~283 lines) use the memory
store but aren't store logic — they're product features. They stay in
distillation and benefit from `save_distilled_fact()` (no more manual supersede).

### 5. Distillation after the moves

```
distillation.rs (~600 lines):
  build_transcript()          — read messages, summarize tool results
  extract_all()               — LLM call, parse response
  distill() orchestrator      — load → extract → save facts → save entities → save relationships → store episode → compile wiki
  store_episode()             — save episode + procedure
  compile_ward_wiki()         — separate LLM call for wikis
  build_llm_client()          — provider selection
  strategy emergence          — cluster failures, emerge strategies (uses save_distilled_fact)
  types                       — DistillationResponse (LLM output schema)
```

## What does NOT change

- Store trait definitions in `zbot-stores-traits` — we're adding methods to
  the ENGRAM ADAPTER's implementation, not changing the trait. Other adapters
  (SQLite) get default "unsupported" if they don't implement these.
- The LLM prompt — extraction schema stays identical.
- Session distillation behavior — same facts, entities, relationships stored.

## Tasks

| Task | What | Touches |
|---|---|---|
| T1 | `save_distilled_fact` + `FactDisposition` in engram adapter | `stores/zbot-engram-adapter/src/stores/memory_facts.rs` |
| T2 | Replace distillation's fact supersede with T1 | `gateway-execution/src/distillation.rs` |
| T3 | `save_distilled_entity` in engram adapter | `stores/zbot-engram-adapter/src/stores/knowledge_graph.rs` |
| T4 | `save_distilled_relationship` in engram adapter | same |
| T5 | Replace distillation's graph projection with T3+T4 | `gateway-execution/src/distillation.rs` |
| T6 | Delete orphaned types + clippy audit | `gateway-execution/src/distillation.rs` |

## Result

| Metric | Before | After |
|---|---|---|
| distillation.rs | 2,286 lines | ~600 lines |
| Fact supersede sites | 4 (distillation inline) | 1 (adapter method) |
| Graph governance sites | 1 (distillation inline) | 1 (adapter method) |
| Crates touched | 1 | 2 (adapter + distillation) |
| New modules | 0 | 0 (logic moves to existing files) |
