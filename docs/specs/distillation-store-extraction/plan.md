# Plan: Move store logic from distillation into the stores

## Problem

Distillation (2,286 prod lines) contains ~1,400 lines of store logic that
belongs in the store traits: fact supersede/dedup, graph projection with
normalization/governance, entity resolution. The stores already have the right
methods (`save_fact`, `upsert_entity`, `upsert_relationship`,
`get_entity_by_normalized_name`) but distillation wraps them with its own logic
instead of the stores owning it.

## What moves where

### 1. MemoryFactStore: `save_distilled_fact()` — eliminates ~250 lines from distillation

**Currently in distillation (`upsert_facts_with_dedup`, ~149 lines + 3 supersede sites):**
- Look up existing fact by key
- Compare content (skip if identical)
- Check confidence threshold (`verify_fact_confidence`)
- Call `supersede_fact()` if replacing
- Call `save_fact()` if new
- Handle corrections (supersede with correction fact ID)

**Moves to `MemoryFactStore` trait method:**
```rust
/// Save a distilled fact with dedup and supersede semantics.
/// Returns Disposition indicating what happened.
async fn save_distilled_fact(
    &self,
    fact: DistilledFact,
) -> Result<FactDisposition, String>;

pub enum FactDisposition {
    Stored(FactId),
    Superseded { old_id: FactId, new_id: FactId },
    SkippedUnchanged,
    SkippedLowConfidence,
}
```

**Why the store:** supersede/dedup is persistence policy — it's about how facts
relate in storage, not about what the LLM extracted. Every consumer that saves
facts (distillation, memory_write tool, corrections abstractor) needs the same
logic.

### 2. KnowledgeGraphStore: `upsert_distilled_entity()` and `upsert_distilled_relationship()` — eliminates ~550 lines

**Currently in distillation (`project_distilled_graph` + helpers, ~550 lines):**
- Normalize entity/relationship names
- Resolve entities by normalized name (find existing or create new)
- Apply governed entity types (map raw LLM output to valid `EntityType`)
- Apply governed relationship types (same for `RelationshipType`)
- Canonicalize relationships (direction, self-loops, entity type inference)
- Resolve relationship endpoints (entity A → entity B vs dangling)
- Call `upsert_entity()` / `upsert_relationship()`

**Moves to `KnowledgeGraphStore` trait methods:**
```rust
/// Save an extracted entity with normalization and governance.
/// Returns the resolved EntityId (existing or new).
async fn upsert_distilled_entity(
    &self,
    agent_id: &str,
    surface_name: &str,
    raw_type: Option<&str>,
    properties: Value,
) -> Result<EntityId, StoreError>;

/// Save an extracted relationship with canonicalization and governance.
/// Returns Some(RelationshipId) if the relationship was stored,
/// None if it was rejected (self-loop, ungoverned type, dangling endpoint).
async fn upsert_distilled_relationship(
    &self,
    agent_id: &str,
    source: &EntityId,
    target: &EntityId,
    raw_type: &str,
    properties: Value,
) -> Result<Option<RelationshipId>, StoreError>;
```

**Why the store:** normalization, governance, and canonicalization are data
integrity rules. They apply to ALL graph writes, not just distillation. The
governance rules (which entity types are valid, which relationship types are
allowed) are store-level invariants.

### 3. Strategy emergence extraction — eliminates ~283 lines

**Currently in distillation (`try_cluster_failures`, `try_emerge_strategy`):**
- Reads recent session memories
- Clusters failure patterns by tool/action
- Scores clusters by frequency
- Writes strategy memories

**Moves to:** its own module `gateway-execution/src/strategy_emergence.rs`
(or stays in distillation/ if the directory split already happened).

**Why separate:** strategy emergence is a FEATURE (product capability), not
distillation infrastructure. It happens to read/write memories but its logic
is "detect patterns and write strategies" not "extract from transcript."

### 4. What stays in distillation

After the moves, distillation is:
- **Types** (~200 lines) — the extraction response schema, shrinks because
  graph/fact types move with the store logic
- **Transcript building** (~119 lines) — reading session messages, summarizing
  tool results for the LLM prompt
- **LLM extraction** (~323 lines) — prompt construction, provider call, JSON parsing
- **Provider selection** (~78 lines) — pick the right LLM
- **Wiki compilation** (~54 lines) — separate LLM call for ward wikis
- **Orchestrator** (~81 lines) — load transcript → LLM → parse → call stores

**Target: ~800 lines (from 2,286).**

## Task breakdown

| Task | What | Depends on |
|---|---|---|
| T1 | Add `save_distilled_fact` + `FactDisposition` to MemoryFactStore trait; implement in engram adapter + sqlite | none |
| T2 | Replace `upsert_facts_with_dedup` in distillation with store calls | T1 |
| T3 | Add `upsert_distilled_entity` + `upsert_distilled_relationship` to KnowledgeGraphStore trait; implement in engram adapter + sqlite | none |
| T4 | Replace `project_distilled_graph` + canonicalize + governance in distillation with store calls | T3 |
| T5 | Extract strategy emergence to its own module | none |
| T6 | Audit remaining distillation types; delete orphans | T2, T4 |

## Constraints

- Each task compiles independently; tree stays green between tasks.
- The store trait additions have default implementations that return
  "unsupported" so existing adapters don't break.
- Distillation tests that test supersede/dedup/governance logic move with the
  code (they become store tests, not distillation tests).
- The LLM prompt shape doesn't change — the extraction schema stays the same.
- No new crates.

## Testing

- T1/T3: unit tests on the new store methods (supersede semantics, governance
  rules, normalization) — these ARE the business logic tests
- T2/T4: integration test: distillation calls the new store methods, verify
  facts/entities/relationships end up in the store with correct dispositions
- T5: existing strategy emergence tests move with the code
- T6: clippy grep audit — zero dead types
