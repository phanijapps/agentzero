# DRY Review — AgentZero (zbot), branch `op_clean_crap`

**Scope:** gateway/, services/, stores/, runtime/ — 260,804 lines of Rust across the workspace.
**Method:** ripgrep pattern search for repeated identifiers/bodies, then manual diff of suspect sites.

## Summary

The codebase has no god-tier copy-paste (the recent op_clean_crap waves killed those), but it has
**systemic duplication in exactly the places that rot**: math helpers, provider wiring, error
responses, store-boundary boilerplate, and LLM-JSON parsing. The most striking finding is #2 —
a factory crate exists *specifically documented* to eliminate provider-selection duplication, and
five call sites still hand-roll it. The deepest-value fix is #6 (JSON-`Value` store traits), which
would delete ~30 encode/decode sites and restore compile-time type safety across the persistence
boundary.

Estimated total: **~900–1,100 lines deleted** with zero behavior change.

---

## Top 10 Findings (ranked by impact)

### 1. Cosine similarity implemented 11 times in 11 files

**Files:**
- `stores/zbot-stores-sqlite/src/belief_store.rs` (`cosine_similarity_f64`, returns f64)
- `stores/zbot-stores-sqlite/src/memory_fact_store.rs`
- `stores/zbot-stores-sqlite/src/memory_repository.rs`
- `stores/zbot-engram-adapter/src/stores/beliefs.rs`
- `stores/zbot-engram-adapter/src/stores/knowledge_graph.rs`
- `stores/zbot-engram-adapter/src/stores/memory_facts.rs`
- `stores/zbot-engram-adapter/src/stores/sidecars.rs`
- `stores/zbot-engram-adapter/src/stores/wiki.rs`
- `gateway/gateway-memory/src/sleep/conflict_resolver.rs` (`cosine`, returns f32)
- `gateway/gateway-memory/src/sleep/pattern_extractor.rs`
- `gateway/gateway-memory/src/recall/mmr.rs`

**Evidence:** near-identical bodies; divergence already happening:
```rust
// belief_store.rs — f64 accumulation, iterator zip
fn cosine_similarity_f64(a: &[f32], b: &[f32]) -> f64 {
    if a.len() != b.len() || a.is_empty() { return 0.0; }
    let mut dot = 0f64; let mut na = 0f64; let mut nb = 0f64;
    for (x, y) in a.iter().zip(b.iter()) { ... }

// conflict_resolver.rs — f32 accumulation, index loop
fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.is_empty() || a.len() != b.len() { return 0.0; }
    let mut dot = 0.0_f32; ...
    for i in 0..a.len() { dot += a[i] * b[i]; ... }
```

**Proposed fix:** `runtime/agent-primitives/src/vec_math.rs` (agent-primitives is already the
bottom-of-graph shared crate with no deps): `pub fn cosine_f32`, `pub fn cosine_f64`,
`pub fn normalize`. Both sqlite and engram-adapter crates already depend on agent-primitives or
can (they're stores; check — if not, a `zbot-stores-domain::math` module works since every impl
already depends on domain).

**Estimated savings:** ~150 lines deleted, 11 implementations → 1; eliminates the f32/f64
precision divergence that silently affects recall ranking parity between impls.

---

### 2. Provider-selection + `LlmConfig` construction hand-rolled at 6 sites (a factory exists!)

**Files (each ~30–45 lines of the same logic):**
- `services/distillation/src/lib.rs:846` (`build_llm_client`) **and again at :963** (fallback chain, in-crate dup)
- `gateway/gateway-execution/src/ingest/extractor.rs:126` (`build_client` — comment literally says *"Mirrors `SessionDistiller::build_llm_client`"*)
- `gateway/src/memory_llm_factory.rs` (the factory itself)
- `gateway/src/http/ward_curator.rs:109`
- `gateway/gateway-execution/src/invoke/setup.rs:77` (default-or-first fallback)
- `gateway/src/state/mod.rs:1369` + `:1383` (same find-default-or-first twice in one function)

**Evidence:**
```rust
// identical in 5 files:
let provider = providers.iter().find(|p| p.is_default)
    .or_else(|| providers.first())
    .ok_or_else(|| "No suitable provider found".to_string())?;
let model = provider.default_model().to_string();
let provider_id = provider.id.clone().unwrap_or_else(|| "default".to_string());
let config = LlmConfig::new(provider.base_url.clone(), provider.api_key.clone(), model, provider_id)
    .with_temperature(..).with_max_tokens(..);
```
`memory_llm_factory.rs`'s own doc comment: *"Constructing one of these per process avoids the six
copy-pasted `build_client` methods that previously lived inside each sleep-time LLM impl."* —
the fix shipped for gateway-memory but nobody migrated the other call sites.

**Proposed fix:** promote `ProviderServiceLlmFactory` from the gateway crate into
`gateway-services` (it only needs `ProviderService`, which lives there). Add
`fn default_client(&self, temperature, max_tokens) -> Result<Arc<dyn LlmClient>>` and a
`fn for_provider(id, model, ...)`. Migrate all 6 sites (distillation keeps its target-override
logic by calling `for_provider` first, `default_client` as fallback).

**Estimated savings:** ~200 lines; single place to fix provider auth/timeout/fallback bugs.

---

### 3. Distillation: 3 identical supersede blocks + 4× 30-field `MemoryFact` literals

**File:** `services/distillation/src/lib.rs`
- Supersede+store block: **L~625 (facts), L~1255 (strategy), L~1405 (correction)** — byte-identical except variable names and the log label ("old fact" / "old strategy fact" / "old correction fact").
- `MemoryFact { ... }` literals: **L597, L1226, L1381, L2792** — 30 fields each, ~30 identical (`contradicted_by: None, created_at: now…, pinned: false, …`).
- `match self.memory_store.as_ref()` Option-matching: **9 sites**, 5 of which return the same `"no memory store wired"` error.

**Evidence** (strategy site, identical shape at the other two):
```rust
if let Some(ref existing) = existing_strategy {
    if existing.content != strategy_description && !existing.pinned {
        let supersede_res = match self.memory_store.as_ref() {
            Some(store) => store.supersede_fact(&existing.id, &strategy_fact_id, chrono::Utc::now())
                .await.map_err(DistillationError::Store),
            None => Err(DistillationError::Resource("no memory store wired".into())),
        };
        if let Err(e) = supersede_res { tracing::warn!(...); } else { tracing::debug!(...); }
    }
}
```

**Proposed fix:** two plain private helpers on `SessionDistiller`:
```rust
fn store(&self) -> Result<&dyn MemoryFactStore, DistillationError>;           // kills 9 matches
fn new_fact(&self, base: FactDraft) -> MemoryFact;                             // kills 4 literals
async fn save_with_supersede(&self, existing: Option<MemoryFact>, fact: &MemoryFact, label: &str); // kills 3 blocks
```

**Estimated savings:** ~150 lines in the crate's worst-maintained hot path; supersede policy
changes become one edit instead of three.

---

### 4. `ErrorResponse` struct defined 9 times in gateway/src/http/

**Files:** `autonomy.rs, beliefs.rs, connectors.rs, cron.rs, gateway_bus.rs, graph.rs, mcps.rs,
memory.rs, tools.rs` (+ `mod.rs` already has `HttpErrorResponse` with an extractor impl).

**Evidence:**
```rust
// tools.rs, beliefs.rs, graph.rs, memory.rs, ... — 5 identical copies:
pub struct ErrorResponse { pub error: String }
// cron.rs — diverged: adds `code: String`
```

**Proposed fix:** one `pub struct ErrorResponse { pub error: String }` in `http/mod.rs` (rename
`HttpErrorResponse` or alias); add `fn service_disabled(msg: &str)` and `fn not_found(msg: &str)`
constructors. Cron's `code` field becomes an optional second constructor or moves into `error`.

**Estimated savings:** ~60 lines; kills the risk of wire-format drift between endpoints.

---

### 5. 503-disabled-store guard repeated ~20 times across HTTP handlers

**Files:** `beliefs.rs` (3 guards + 4 tests asserting them), `graph.rs`, `memory_search.rs`,
`gateway_bus.rs` (5), `autonomy.rs` (5), `ward_content.rs` (5), `artifacts.rs`, `chat.rs`,
`commissioning.rs` — 11 files use `StatusCode::SERVICE_UNAVAILABLE` at 14+ sites.

**Evidence:**
```rust
state.belief_store.as_ref().ok_or_else(|| {
    (StatusCode::SERVICE_UNAVAILABLE,
     Json(ErrorResponse { error: BELIEF_DISABLED_MSG.to_string() }))
})?
```
The `ok_or_else` pattern appears 5× in `ward_content.rs` alone, 5× in `gateway_bus.rs`, 5× in `autonomy.rs`.

**Proposed fix (pairs with #4):** in `http/mod.rs`:
```rust
pub fn require<T>(slot: &Option<T>, msg: &str) -> Result<&T, (StatusCode, Json<ErrorResponse>)> {
    slot.as_ref().ok_or_else(|| ErrorResponse::service_disabled(msg))
}
```
**Estimated savings:** ~10 lines × ~20 sites ≈ 200 lines of guard noise; the "return 503 not 404"
policy is enforced in one place.

---

### 6. Store traits pass `serde_json::Value`, forcing decode→operate→encode at every impl

**Files:** `stores/zbot-stores-traits/src/memory_facts.rs` (e.g. `upsert_typed_fact(fact: Value)`),
and the resulting boilerplate:
- `stores/zbot-engram-adapter/src/stores/sidecars.rs` — 4 `from_value` + 2 `to_value`
- `stores/zbot-engram-adapter/src/stores/memory_facts.rs` — 1 + 3
- `stores/zbot-engram-adapter/src/stores/wiki.rs` — 1 + 2
- `stores/zbot-stores-sqlite/src/{memory_fact_store,episode_store,procedure_store,wiki_store,auxiliary_stores}.rs` — 5 + 8 total
- Every impl repeats `.map_err(|e| format!("decode Procedure: {e}"))` (sidecars.rs has it 4×).

**Evidence:**
```rust
// engram memory_facts.rs:739 and sqlite memory_fact_store.rs:735 — same ritual:
let mut typed: MemoryFact = serde_json::from_value(fact)
    .map_err(|e| format!("decode MemoryFact: {e}"))?;
```
`MemoryFact`, `Procedure`, `SessionEpisode` are **already** concrete types in
`zbot-stores-domain` — the JSON hop exists only for trait-signature compatibility.

**Proposed fix:** change trait signatures to the domain types (`async fn upsert_fact(&self, fact: MemoryFact)`),
delete every decode/encode site. Callers in distillation (`serde_json::to_value(&fact)` before
`upsert_typed_fact` at lib.rs:1285, :1436, :1780) simplify to direct calls. The conformance
harness (`zbot-stores-conformance`) already tests behavior generically, so the migration is
mechanical and guarded.

**Estimated savings:** ~32 sites × ~4 lines ≈ 130 lines, plus **compile-time type safety at the
persistence boundary** (a malformed fact currently fails at runtime; it would fail at compile).
This is the deepest fix; do it after the quick wins.

---

### 7. `recall_facts_prioritized` vs `recall_facts_prioritized_scoped` — same body, one `None`

**Files:** `stores/zbot-engram-adapter/src/stores/memory_facts.rs:411` and `:437` — **both
implemented, bodies identical** except the unscoped passes `None` for `ward_id`. The trait
(`zbot-stores-traits/src/memory_facts.rs:180`) already provides defaults for both, each falling
back to the other's sibling — a footgun for implementors.

**Evidence:** the two functions differ only in `None,` vs `ward_id,` on the 6th argument of the
same `search_memory_facts_hybrid_with_identity` call.

**Proposed fix:** in the engram impl, make the unscoped method a one-line delegation
(`self.recall_facts_prioritized_scoped(agent_id, query, None, limit, as_of).await`); longer term,
make the trait's unscoped variant default to calling scoped, and delete the unscoped override
in `stores/zbot-stores-sqlite/src/memory_fact_store.rs:288` too (~60 lines there, though that
body is richer — verify shared-shape first).

**Estimated savings:** ~25 lines now, plus removes a trait-default circularity.

---

### 8. GraphStorage: 42× double-closure + double-`map_err` SQLite ritual

**File:** `stores/zbot-stores-sqlite/src/kg/storage.rs` (5,467 lines).

**Evidence:** 42 occurrences of `.map_err(graph_to_rusqlite)` nested inside
`.map_err(GraphError::Other)`; the canonical shape (seen in `mark_pruned`, `mark_entity_archival`,
…):
```rust
self.db.with_connection(move |conn| {
    (|| -> GraphResult<()> {
        let tx = conn.unchecked_transaction().map_err(GraphError::Database)?;
        tx.execute(...).map_err(GraphError::Database)?;
        tx.commit().map_err(GraphError::Database)?;
        Ok(())
    })().map_err(graph_to_rusqlite)
}).map_err(GraphError::Other)
```

**Proposed fix:** one wrapper on GraphStorage:
```rust
fn run<R>(&self, f: impl FnOnce(&Connection) -> GraphResult<R>) -> GraphResult<R> {
    self.db.with_connection(move |c| {
        let r = f(c); let s = serde_json::to_string(&r); s.map_err(...)
    })...
}
```
(plus a `fn tx<R>(&self, f: impl FnOnce(&Transaction) -> GraphResult<R>)` variant that owns
begin/commit/rollback).

**Estimated savings:** 4–6 lines × ~40 methods ≈ **~200 lines** and the single biggest
readability win in the largest file in the repo.

---

### 9. LLM-JSON extraction: 3 independent fence-stripping/parsing stacks

**Files:**
- `services/distillation/src/lib.rs:2275` — `extract_json_from_content` + triple-fallback parse (`from_str::<DistillationResponse>` → `Vec<ExtractedFact>` → lenient `parse_distillation_from_value`)
- `gateway/gateway-execution/src/ingest/extractor.rs:167` — own prompt clause *"Do not wrap in code fences"* + own parse path
- `gateway/gateway-execution/src/middleware/intent/agent.rs` — intent JSON parse (the newest rewrite, prompt-based because `response_format` breaks Ollama)

**Evidence:** distillation alone carries ~90 lines of lenient-JSON recovery that the other two
sites will eventually re-need (documented lesson: models emit fences/commentary regardless of
prompt).

**Proposed fix:** `agent-runtime/src/llm/json.rs`: `pub fn parse_llm_json<T: DeserializeOwned>(raw: &str) -> Result<T, LlmJsonError>` — strip fences, locate first balanced `{...}`, parse, and on failure return a preview-bearing error. Distillation's lenient field-name fallback stays local (it's schema-specific); the generic part moves.

**Estimated savings:** ~60 lines now; more importantly the next structured-output feature
(wiki, procedures, corrections prompts) doesn't grow a fourth copy.

---

### 10. Test scaffolding copied: `make_state()` ×3 and mock-provider plumbing ×N

**Files:**
- `gateway/src/http/{beliefs,artifacts,customization}.rs` — 3 copies of:
```rust
fn make_state() -> (TempDir, AppState) {
    let dir = TempDir::new().expect("temp dir");
    std::fs::create_dir_all(dir.path().join("agents")).unwrap();
    std::fs::create_dir_all(dir.path().join("skills")).unwrap();
    let state = AppState::minimal(dir.path().to_path_buf());
    (dir, state)
}
```
- `runtime/agent-tools/src/tools/memory.rs` — 4 inline `async fn recall_facts_prioritized` mock impls (L1998, L2288, L2462, L3182); `ward.rs` another at L2303.

**Proposed fix:** `gateway/src/http/tests_common/mod.rs` (or `#[cfg(test)]` module in `http/mod.rs`) with `make_state()` + `make_state_with_dirs(extra: &[&str])`; a `mocks::FactStore` builder in agent-tools tests.

**Estimated savings:** ~40 lines; test-state policy changes (new required dir, new minimal field) become one edit.

---

## Quick wins (<1 day each, in dependency order)

1. **`ErrorResponse` consolidation** (#4 + #5) — 1–2 h, ~260 lines, `gateway/src/http/` only.
2. **Cosine into a shared crate** (#1) — 2 h, ~150 lines, mechanical replace.
3. **`recall_facts_prioritized` delegation** (#7) — 15 min, 25 lines, zero risk.
4. **`make_state` + test mocks** (#10) — 30 min.
5. **Distillation supersede/fact-literal helpers** (#3) — 2–3 h, ~150 lines, tests already cover all three sites.
6. **`ProviderServiceLlmFactory` promotion + 6-site migration** (#2) — half day, ~200 lines; the factory already exists and is proven.
7. **`GraphStorage::run`/`tx` wrappers** (#8) — half day, ~200 lines; do method-by-method to keep diffs reviewable.

## Bigger bets (schedule, don't sneak)

- **#6 typed store traits** — one focused PR per trait impl, conformance harness proves parity. Biggest long-term value: deletes the JSON hop *and* the decode-boilerplate class of bugs.
- **#9 shared LLM-JSON parser** — do it the next time a structured-output prompt is added; moving it speculatively without a third consumer is borderline.
