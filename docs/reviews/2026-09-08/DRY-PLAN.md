# Plan: DRY bonus winners — 4 consolidations, ~1,100 lines

## Fix 1: Cosine similarity ×11 → `agent-primitives::vec_math`

**New module:** `runtime/agent-primitives/src/vec_math.rs`

```rust
pub fn cosine_f32(a: &[f32], b: &[f32]) -> f32   // f32 accumulation (gateway-memory sites)
pub fn cosine_f64(a: &[f32], b: &[f32]) -> f64   // f64 accumulation (stores sites)
pub fn normalize(v: &mut Vec<f32>)               // if duplicated (check mmr.rs)
```

**Replace in 11 files** (delete local fn, import, keep call sites compiling):

| File | Local fn |
|---|---|
| stores/zbot-stores-sqlite/src/belief_store.rs | `cosine_similarity_f64` |
| stores/zbot-stores-sqlite/src/memory_fact_store.rs | same |
| stores/zbot-stores-sqlite/src/memory_repository.rs | same |
| stores/zbot-engram-adapter/src/stores/beliefs.rs | same |
| stores/zbot-engram-adapter/src/stores/knowledge_graph.rs | same |
| stores/zbot-engram-adapter/src/stores/memory_facts.rs | same |
| stores/zbot-engram-adapter/src/stores/sidecars.rs | same |
| stores/zbot-engram-adapter/src/stores/wiki.rs | same |
| gateway/gateway-memory/src/sleep/conflict_resolver.rs | `cosine` (f32) |
| gateway/gateway-memory/src/sleep/pattern_extractor.rs | same |
| gateway/gateway-memory/src/recall/mmr.rs | same |

Both precision variants kept: f64 for stores (matches existing behavior),
f32 for gateway-memory. No behavior change — same accumulation order.

**Check deps:** agent-primitives is bottom-of-graph. sqlite + engram-adapter +
gateway-memory all already depend on it (verify; if a store crate doesn't,
add the dep — it's pure functions, no transitive deps).

## Fix 2: `ErrorResponse` ×9 → 1 + `require()` helper

**In `gateway/src/http/mod.rs`:**

```rust
// existing HttpErrorResponse has an extractor; rename or alias to ErrorResponse
pub struct ErrorResponse { pub error: String }

impl ErrorResponse {
    pub fn service_disabled(msg: impl Into<String>) -> (StatusCode, Json<Self>) { ... }
}

// 503-store-guard helper — kills ~20 ok_or_else blocks
pub fn require<T>(slot: &Option<T>, msg: &str) -> Result<&T, (StatusCode, Json<ErrorResponse>)> {
    slot.as_ref().ok_or_else(|| ErrorResponse::service_disabled(msg))
}
```

**Migrate:** autonomy.rs, beliefs.rs, connectors.rs, cron.rs, gateway_bus.rs,
graph.rs, mcps.rs, memory.rs, tools.rs — delete local structs, use shared one.
Cron's diverged `code` field: fold into `error` string (`"{code}: {msg}"`).

## Fix 3: Provider factory promotion + 6-site migration

**Move** `gateway/src/memory_llm_factory.rs` → `gateway/gateway-services/src/llm_factory.rs`
(it only needs ProviderService which lives there).

**Add to factory:**
```rust
pub fn default_client(&self, temperature: f64, max_tokens: u32)
    -> Result<Arc<dyn LlmClient>, String>;   // default-or-first
pub fn for_provider(&self, id: &str, model: &str, temperature: f64, max_tokens: u32)
    -> Result<Arc<dyn LlmClient>, String>;   // explicit target
```

**Migrate 6 sites:**
1. `services/distillation/src/lib.rs:846` + `:963` (two in-crate dups → for_provider with default_client fallback)
2. `gateway/gateway-execution/src/ingest/extractor.rs:126` (comment admits it mirrors distillation)
3. `gateway/gateway-execution/src/invoke/setup.rs:77`
4. `gateway/src/http/ward_curator.rs:109`
5. `gateway/src/state/mod.rs:1369/:1383` (twice in one fn)

Dependency check: gateway-execution already depends on gateway-services? Yes
(distillation depends on both). gateway depends on gateway-services. ✓

## Fix 4: Typed store-trait write paths (kill JSON-Value hop)

**Scope: WRITE paths only.** Read paths (`list_by_ward`, `get_article`, …)
return Value straight to HTTP Json() — that's the wire format; leave them.

**Trait signature changes** (`stores/zbot-stores-traits/src/`):

| Trait | Before | After |
|---|---|---|
| memory_facts.rs | `upsert_typed_fact(fact: Value, …)` | `upsert_typed_fact(fact: MemoryFact, …)` |
| procedures.rs | `upsert_procedure(procedure: Value, …)` | `upsert_procedure(procedure: Procedure, …)` |
| wiki.rs | `upsert_article(article: Value, …)` | `upsert_article(article: WikiArticle, …)` |
| episodes.rs | `insert_episode(episode: Value, …)` | `insert_episode(episode: SessionEpisode, …)` |
| sidecars.rs | check + same pattern | typed |

**Impls simplify** (delete `from_value` + map_err):
- engram-adapter: sidecars.rs (4), memory_facts.rs (1), wiki.rs (1)
- sqlite: memory_fact_store, episode_store, procedure_store, wiki_store, auxiliary_stores

**Callers simplify** (delete `to_value`):
- services/distillation/src/lib.rs:1285, :1436, :1780 (+ episodes/procedures if present)
- gateway ingest, ward curator, anywhere writing these rows

**Domain types already exist** in zbot-stores-domain: MemoryFact, Procedure,
WikiArticle, SessionEpisode. Traits crate depends on domain (verify; add dep).

**Conformance harness** (`zbot-stores-conformance`) already tests these paths
generically — it compiles against the new signatures and proves parity.

## Execution order

1. Fix 1 (cosine) — mechanical, no deps risk
2. Fix 2 (ErrorResponse) — gateway/src/http only
3. Fix 3 (factory) — move + migrate
4. Fix 4 (typed traits) — biggest; separate commit for easy revert

Each fix: cargo check → tests → clippy → fmt. Commit per fix.
