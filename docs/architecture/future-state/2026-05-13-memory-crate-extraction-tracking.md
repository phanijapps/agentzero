# Memory Crate Extraction — Change Tracking

**Purpose:** Catalog of every memory-related code change made across Phases 1–3 of the Reflective Memory roadmap, so a future refactor to extract memory into its own crate has a complete starting inventory. No code changes yet — this is a tracking document only.

**Author trail:** Built up across the work in sessions starting `dd8b51a8-e020-4ada-bb4f-e59ccacbb115` and continuing in `2be03b02-1859-4ed2-8680-3a36172e9215` / `d0db5589-286d-4cfd-99ed-c9b9a697e67f`. Last updated 2026-05-13.

Last updated: 2026-05-13. **Extraction status: complete (Phases A–F + MEM-005 all shipped).**

---

## 1. Scope: What "Memory" Means Today

The memory subsystem currently lives across three crates:

| Crate | Role |
|-------|------|
| `stores/zbot-stores-traits` | `MemoryFactStore`, `CompactionStore`, `EpisodeStore`, `KnowledgeGraphStore` traits + `MemoryFact` re-exports |
| `stores/zbot-stores-sqlite` | SQLite implementations of all above traits |
| `stores/zbot-stores-domain` | `MemoryFact`, `StrategyFactInsert`, `StrategyFactMatch` types |
| `gateway/gateway-execution/src/sleep/` | All sleep-time memory components (Compactor, Synthesizer, PatternExtractor, Pruner, OrphanArchiver, **CorrectionsAbstractor**, **ConflictResolver**, **HandoffWriter**) |
| `gateway/gateway-memory` | **COMPLETE — Phases A through F.** The full memory subsystem: config types, all sleep-component engines + abstraction traits, recall pipeline, `MemoryLlmFactory` + 6 production `Llm*` impls, `HandoffWriter` struct, `SleepOps` + `SleepTimeWorker`, and the `MemoryServices::new(MemoryServicesConfig)` factory. Gateway constructs services in 41 lines (was 104). |
| `gateway/gateway-execution/src/recall/` | Recall scoring + retrieval pipeline |
| `gateway/gateway-execution/src/runner/invoke_bootstrap.rs` | Session-start memory injection (handoff, corrections, goals, targeted recall) |
| `gateway/gateway-services/src/recall_config.rs` | `RecallConfig` |
| `gateway/gateway-services/src/settings.rs` | `MemorySettings` (new) inside `ExecutionSettings` |
| `gateway/src/state/mod.rs` | Construction + wiring of all memory components |

A future `zero-memory` crate would naturally absorb most of `gateway/gateway-execution/src/sleep/` and `recall/`, plus `RecallConfig` and `MemorySettings`. The store traits already live in `zbot-stores-*` and don't need to move.

---

## 2. New Components Added in Phases 1–3

These are the *new* memory components built across the three phases. All live in `gateway/gateway-execution/src/sleep/`.

### Phase 1 — Session Handoff + Targeted Recall + Always-Inject

| Component | File | Responsibility |
|-----------|------|----------------|
| `HandoffWriter` | `sleep/handoff_writer.rs` | LLM-summarizes a session at end, writes a `handoff.latest` + `handoff.{session_id}` fact to the memory store under the `__handoff__` sentinel agent |
| `read_handoff_block` | `sleep/handoff_writer.rs` | Session-start reader. Ward-scoped (returns `None` if stored handoff's `ward_id` doesn't match current session's ward). Returns formatted `## Last Session` block |
| `LlmHandoffWriter` | `sleep/handoff_writer.rs` | Production LLM impl wired via `ProviderService` |
| `format_corrections_block` | `runner/invoke_bootstrap.rs` | Always-injects active correction facts at session start (independent of recall) |
| `format_goals_block` | `runner/invoke_bootstrap.rs` | Injects `state=="active"` goals from `goal_adapter.list_active()` |
| Targeted recall pass | `runner/invoke_bootstrap.rs` | Second `recall_unified` call using `handoff.latest.summary` as query, formatted as `## Context from Last Session` |

### Phase 2 — Pattern Abstraction

| Component | File | Responsibility |
|-----------|------|----------------|
| `CorrectionsAbstractor` | `sleep/corrections_abstractor.rs` | Promotes 3+ correction facts to a single `schema` category fact via LLM abstraction. In-memory `Mutex<Option<Instant>>` throttle |
| `AbstractionLlm` trait | `sleep/corrections_abstractor.rs` | LLM judge interface (mockable) |
| `LlmCorrectionsAbstractor` | `sleep/corrections_abstractor.rs` | Production LLM impl |

### Phase 3 — Conflict Resolution

| Component | File | Responsibility |
|-----------|------|----------------|
| `ConflictResolver` | `sleep/conflict_resolver.rs` | Scans schema-fact pairs by embedding cosine ≥ 0.85, LLM-judges contradictions, calls `supersede_fact(loser, winner)`. In-memory throttle |
| `ConflictJudgeLlm` trait | `sleep/conflict_resolver.rs` | LLM judge interface |
| `LlmConflictJudge` | `sleep/conflict_resolver.rs` | Production LLM impl |
| `cosine` helper | `sleep/conflict_resolver.rs` | f32 cosine similarity (defensive against empty / mismatched lengths) |
| `pick_winner` helper | `sleep/conflict_resolver.rs` | Higher-confidence wins, tie-break by `updated_at` |

### Phase 4 — Belief Network Foundation

| Component | File | Responsibility |
|-----------|------|----------------|
| `decay_entity_confidence` / `decay_relationship_confidence` | `stores/zbot-stores/src/knowledge_graph.rs` (trait), `stores/zbot-stores-sqlite/src/knowledge_graph.rs` (impl) | Batch-apply temporal decay to KG `confidence` columns based on `last_seen_at` age, floor at `min_confidence`, transaction-wrapped |
| `DecayEngine::decay_kg_confidence` | `gateway/gateway-execution/src/sleep/decay.rs` | Orchestrates both store calls, enabled-guard, returns `KgDecayStats` |
| `evidence` TEXT column | `stores/zbot-stores-sqlite/src/knowledge_schema.rs` | Schema-only — preparatory for future contradiction-provenance work. No code populates it yet. |
| `KgDecayConfig` | `gateway/gateway-services/src/recall_config.rs` | Configurable half-lives + floor + skip-recent guard |
| `KnowledgeGraphStore::find_kg_nodes_by_episode_ids` + `apply_entity_confidence_multiplier` + `apply_relationship_confidence_multiplier` | trait + SQLite impl | MEM-001 Part A — locate kg_entities and kg_relationships whose `source_episode_ids` blob contains any of a given set, then multiplicatively decay their `confidence` floored at `min_floor`. Comma-flanked token match (no substring false positives), agent-scoped, archival excluded. |
| `MemoryFactStore::list_contradicted_fact_episode_ids` | trait + SQLite impl | Returns distinct `source_episode_id` values from `memory_facts` rows where `contradicted_by IS NOT NULL` and `updated_at > since`. Lookback bounded by the engine's `contradiction_lookback_hours` (default 24h). |
| `DecayEngine::propagate_fact_contradictions` + `ContradictionPropagationConfig` | `gateway-memory/src/sleep/decay.rs` | MEM-001 Part A orchestrator. Chains the three trait calls above. Builder-style `with_contradiction_propagation(fact_store, config)` so the engine stays a no-op when unwired or disabled. Wired into the sleep cycle in `worker.rs` immediately after `decay_kg_confidence`. Default config: `enabled=true`, `decay_factor=0.9`, `min_floor=0.05`, `lookback_hours=24`. |
| `EntityNameEmbeddingHit.confidence` + `GraphTraversalConfig.min_kg_confidence` + recall step-4 weighting | `stores/zbot-stores-domain/src/kg_ops.rs` + `stores/zbot-stores-sqlite/src/kg/storage.rs` + `gateway/gateway-memory/src/lib.rs` + `gateway/gateway-memory/src/recall/mod.rs` | MEM-001 Part B-1 — recall step-4 (graph-ANN over entity name index) now joins `kg_entities.confidence` and (a) filters hits below `min_kg_confidence` (default 0.1) and (b) multiplies the final score by entity confidence. Closes the loop with Part A: when the sleep cycle decays a contradicted entity's confidence, recall actually downweights it. Filter applies to seed_ids too, so the hierarchy LCA surface (step 5c) stays consistent with what recall shows. |
| `KnowledgeGraphStore::traverse_weighted` + `WeightedTraversalHit` + recall step 4b | `stores/zbot-stores/src/knowledge_graph.rs` + `stores/zbot-stores-sqlite/src/{kg/storage.rs,knowledge_graph.rs}` + `gateway/gateway-memory/src/recall/mod.rs` | MEM-001 Part B-2 — confidence-aware BFS traversal in recall. Recursive CTE walks `kg_relationships` outward from the top-3 graph-ANN seeds (bounded by `max_hops` + `max_graph_facts`), filters edges below `min_kg_confidence`, and accumulates an `edge_confidence_product` along each path. Recall scores each hit as `hop_decay^hop × edge_confidence_product × entity_confidence` and emits ScoredItems with `provenance.source = "kg_traversal"` (distinct from step-4's `kg_name_index` seeds). Shortest-hop-wins dedup; ties go to higher edge-confidence product. Agent-scoped, archival rows + edges excluded. **MEM-001 fully closed.** |
| `CachedLlmClient` helper + production Llm* impls converted | `gateway/gateway-memory/src/llm_factory.rs` + 9 sleep/recall LLM impls | MEM-003 — every Llm* impl (`LlmConflictJudge`, `LlmCorrectionsAbstractor`, `LlmSynthesizer`, `LlmBeliefSynthesizer`, `LlmContradictionJudge`, `LlmPatternExtractor`, `LlmPairwiseVerifier`, `LlmAggregateEntity`, `LlmQueryGate`) now embeds a `CachedLlmClient` instead of re-building per call. `tokio::sync::OnceCell` so concurrent first-call paths coalesce to one build. Errors are not memoized — transient factory failures retry on next call. Net effect: one `build_client` per process per (impl, config) tuple. |
| Bulk decay UPDATE + `exp` SQLite UDF | `stores/zbot-stores-sqlite/src/{knowledge_db.rs,knowledge_graph.rs}` | MEM-004 — `decay_kg_table` replaced with a single `UPDATE … SET confidence = MAX(?, confidence * exp(-? * (julianday('now') - julianday(last_seen_at)))) WHERE …`. rusqlite's `bundled` feature doesn't ship `SQLITE_ENABLE_MATH_FUNCTIONS`, so a tiny Rust-side `exp(x)` scalar UDF is registered on every pooled connection via `create_scalar_function` (deterministic, no I/O). One SQL round-trip per decay pass regardless of row count. Tested at 1/90/180 days against the canonical exp formula. Cargo.toml gains the `functions` feature on rusqlite. |

### Phase 6 — Hierarchical Memory (HiRAG + LeanRAG)

Multi-layer aggregation on top of `kg_entities`. Sleep cycle K-means-clusters layer-N entities, LLM-synthesises a layer-N+1 aggregate per cluster, writes LeanRAG-style inter-cluster relations when connectivity λ > τ. Recall walks `parent_cluster_id` from top seed entities to their lowest common ancestor and emits each path entity (and the inter-cluster edges between path entities) as new `ItemKind::HierEntity` / `ItemKind::HierRelation` ScoredItems. Disabled by default behind `execution.memory.hierarchy.enabled`. Design at `docs/architecture/future-state/` and the saved-memory entry `project_hierarchical_memory_plan.md`.

| Component | File | Responsibility |
|-----------|------|----------------|
| Schema v31 — `kg_entities.layer` + `parent_cluster_id`; `kg_relationships.layer` + `is_inter_cluster` | `stores/zbot-stores-sqlite/src/knowledge_schema.rs` + `migrations/v31_kg_hierarchy_columns.sql` | Columns + 4 supporting indexes. PRAGMA-guarded ALTER mirrors v29/v30. |
| `kmeans_cosine` + `cluster_sparsity` + `should_stop_layering` | `gateway/gateway-memory/src/sleep/clustering.rs` | Pure-Rust K-means with K-means++ init, cosine distance, HiRAG cluster-sparsity stop rule. Seeded LCG, no `rand` dep. |
| `KnowledgeGraphStore::list_entities_with_embeddings_at_layer` | trait + SQLite impl | Builder's candidate-pool fetch. Joins `kg_entities × kg_name_index`, dual-matches agent_id against `__global__`. |
| `KnowledgeGraphStore::connectivity_strength` | trait + SQLite impl | Counts cross-cluster edges in both directions. Gates inter-cluster relation synthesis at λ > τ. |
| `KnowledgeGraphStore::promote_cluster_to_aggregate` | trait + SQLite impl | Atomic INSERT aggregate + UPDATE member `parent_cluster_id` + optional `kg_name_index` row. |
| `KnowledgeGraphStore::write_inter_cluster_relation` | trait + SQLite impl | INSERT into `kg_relationships` with `is_inter_cluster = 1`, layer-tagged. |
| `KnowledgeGraphStore::compute_lca_path` | trait + SQLite impl | Walks `parent_cluster_id` from seed entities up to LCA. `MAX_LCA_WALK = 16` cap protects against corrupt cycles. |
| `KnowledgeGraphStore::list_inter_cluster_relations` | trait + SQLite impl | Returns edges where both endpoints are in a given id set. Used by H-4 recall step 5c to surface the "lean" edges along the LCA path. |
| `HierarchyBuilder` + `AggregateEntityLlm` trait | `gateway-memory/src/sleep/hierarchy_builder.rs` | Sleep-cycle orchestrator: cluster → synthesise → write aggregates + inter-cluster relations. Singleton short-circuit (no LLM). Internal throttle mirrors `BeliefSynthesizer`. |
| `LlmAggregateEntity` | `gateway-memory/src/sleep/llm_aggregate_entity.rs` | Production trait impl via `MemoryLlmFactory`. Two methods: `synthesize_aggregate(members)` and `synthesize_relation(agg_a, agg_b, λ)`. JSON-strict prompts. |
| `HierarchySettings` | `gateway-memory/src/lib.rs` (inside `MemorySettings`) | `enabled`, `intervalHours`, `maxLayers`, `clusterTargetSize`, `interClusterRelationThreshold`, `llmBudgetPerCycle`. Disabled by default. |
| `ItemKind::HierEntity` + `ItemKind::HierRelation` + adapters | `gateway-memory/src/recall/scored_item.rs` + `recall/adapters.rs` | LCA path entities + inter-cluster edges as ScoredItems. Render under `## Topical Map` heading in the consumer formatter (`gateway-execution::recall::format_scored_items`). |
| Recall step 5c | `gateway-memory/src/recall/mod.rs` | After graph-ANN, walks LCA, fetches inter-cluster relations among path entities, emits both with `pattern`-slot category weight (`0.9`, conservative). |

**Dual-match agent_id pattern.** Real-data smoke surfaced that base entities live under `agent_id = '__global__'` (cross-agent dedup convention) while the daemon's HierarchyBuilder is configured for `agent_id = 'root'`. All H-3/H-4 SQL filters now do `(agent_id = ?1 OR agent_id = '__global__')`, matching the idiom in `search_entities_by_name_embedding`.

**Singleton aggregate naming.** First production run showed singletons surfacing as `[topic L1] entity_root_<uuid>` — useless. Fixed by looking up the member's `name` via `get_entity` before promoting, plus embedding the description so singletons participate in higher-layer clustering.

---

## 3. Recall Pipeline Changes

`gateway/gateway-execution/src/recall/mod.rs` was modified across all three phases.

| Change | Commit | What it does |
|--------|--------|--------------|
| Real similarity scores extracted from `search_memory_facts_hybrid` instead of synthesizing 0.5 | `158d9d77` | Phase 1 root-cause fix — recall was returning every fact with score 0.5 because the hybrid-search JSON's `score` field was being ignored |
| `min_score` threshold filter (default 0.3) | `c51b9066` + `158d9d77` | Suppresses noise; phase 1 |
| Schema category weight 1.6 added to `RecallConfig` | `cd3421f3` | Phase 2; schemas rank above corrections in recall |
| Superseded-fact retain filter in `recall_facts` legacy path | `2d1c7a80` | Phase 3; placed before sort so we don't waste work scoring then dropping |
| Superseded-fact filter inside `filter_map` in `recall_unified` | `2d1c7a80` | Phase 3; short-circuits before `fact_to_item` |

---

## 4. Settings Additions

`gateway/gateway-services/src/settings.rs` gained a `MemorySettings` struct nested inside `ExecutionSettings`. It now exposes multiple recall-pipeline and sleep-cycle knobs:

```rust
pub struct MemorySettings {
    pub corrections_abstractor_interval_hours: u32,  // default 24, Phase 2
    pub conflict_resolver_interval_hours: u32,       // default 24, Phase 3
    pub query_gate: Option<QueryGateConfig>,          // Phase 5 (Self-RAG, opt-in)
    pub mmr: Option<MmrConfig>,                       // MMR diversity rerank
    pub rerank: Option<RerankConfig>,                 // cross-encoder rerank, opt-in
    pub intent_router: Option<IntentRouterConfig>,    // kNN intent classifier
    pub kg_decay: Option<KgDecayConfig>,              // Phase 4 foundation
}
```

Also added: `pub use settings::MemorySettings` re-export from `gateway/gateway-services/src/lib.rs`.

The HTTP layer `UpdateExecutionSettingsRequest` in `gateway/src/http/settings.rs` also gained `pub memory: Option<MemorySettings>` for the settings update endpoint.

**For extraction:** `MemorySettings` belongs in the future `zero-memory` crate. The wrapper `ExecutionSettings.memory` field can stay where it is — `gateway-services` would just depend on `zero-memory` for the inner type.

---

## 5. Store-Trait Additions

These trait method additions happened during Phase 3 and represent a real architectural fact: embeddings live in a separate `memory_facts_index` sqlite-vec table since v22 schema, so `get_facts_by_category` returns `embedding: None` always.

| Method | File | Phase | Why |
|--------|------|-------|-----|
| `MemoryFactStore::get_fact_embedding(fact_id) -> Result<Option<Vec<f32>>>` | `stores/zbot-stores-traits/src/memory_facts.rs` | Phase 3 | Hydrate embeddings on-demand for `ConflictResolver` pair scoring |
| SQLite impl: one-line delegation to `memory_repo.get_fact_embedding` | `stores/zbot-stores-sqlite/src/memory_fact_store.rs` | Phase 3 | (`memory_repository.rs:1121` already had the method) |

**For extraction:** these trait additions stay in `zbot-stores-traits` — they're part of the store contract, not the memory crate proper.

---

## 6. Wiring Points (Construction Sites)

The future `zero-memory` crate would export factory functions or builder types. Today, the gateway constructs every memory component manually in `gateway/src/state/mod.rs` between lines ~767–845. The exact construction block has grown to:

1. `Compactor::new(kg_store, compaction_store, embedding_client)`
2. `DecayEngine::new(kg_store, DecayConfig::default())`
3. `Pruner::new(kg_store, compaction_store)`
4. `Synthesizer::new(kg_store, episode_store, memory_store, compaction_store, llm_synth, embedder)`
5. `PatternExtractor::new(episode_store, conversation_store, procedure_store, compaction_store, llm_pattern)`
6. `OrphanArchiver::new(kg_store, compaction_store)`
7. `CorrectionsAbstractor::new(memory_store, compaction_store, llm_abstractor, interval)` *(Phase 2)*
8. `ConflictResolver::new(memory_store, compaction_store, llm_judge, interval)` *(Phase 3)*
9. All assembled into `SleepOps { ... }` literal
10. Passed to `SleepTimeWorker::start_with_ops(...)` with hardcoded `Duration::from_secs(60 * 60)` overall interval

**For extraction:** This block is the natural seam. A `MemoryServices::new(stores, llm_clients, settings)` factory would replace the ~80 lines of imperative construction.

Also: the `SleepOps` struct and `SleepTimeWorker` itself live in `gateway/gateway-execution/src/sleep/worker.rs`. Both should move to the memory crate.

---

## 7. Dependencies the Memory Crate Would Need

Today's memory code reaches into:

| Dep | Used by | Required? |
|-----|---------|-----------|
| `agent_runtime::llm::{ChatMessage, LlmClient, LlmConfig}` | Every `Llm*` production impl | Yes — needs to stay or move to a `zero-memory-llm` adapter |
| `gateway_services::ProviderService` | Every `Llm*` production impl (resolves the default provider) | Awkward — `ProviderService` should arguably not be a memory dep. Could be replaced by an `LlmClientFactory` trait passed in |
| `gateway_services::VaultPaths` | Test harnesses only | Tests-only; not a runtime dep |
| `agent_tools::GoalSummary` | `format_goals_block` in `invoke_bootstrap.rs` | Yes — goals are conceptually orthogonal to memory but injected alongside |
| `zbot_stores`, `zbot_stores_traits`, `zbot_stores_domain`, `zbot_stores_sqlite` | All store ops | Yes — stays in stores crates |
| `chrono`, `serde`, `async_trait`, `tracing`, `uuid` | Pervasive | Standard, no friction |

**Blocker:** the `ProviderService` dependency is the main awkwardness. A clean extraction would introduce a `MemoryLlmFactory` trait in `zero-memory` that the gateway implements by wrapping `ProviderService`. This keeps the memory crate provider-agnostic.

---

## 8. Complete Commit Inventory (Phase 1–3)

Listed in reverse-chronological order. All on branch `feat/parallel-delegation-aggregation`.

### Phase 6 — Hierarchical Memory (HiRAG + LeanRAG, 2026-05-16 → 17)

- `654db462` feat(memory): schema v31 — hierarchical-memory columns on kg_entities + kg_relationships (PR #170)
- `26c351f9..58069947` H-3 builder, 7 commits squash-merged into `8949af1a` (PR #172, *which was orphan-history merged then retargeted as #172/develop*) — K-means clustering + stop rule, connectivity λ query, aggregate writer, LayerN fetch, orchestrator + LLM trait, sleep-cycle wiring + `HierarchySettings`, demo test
- `d4e90b66` feat(memory): `LlmAggregateEntity` + wire `HierarchyBuilder` into sleep cycle (PR #173) — production LLM adapter + `services.rs` reads `hierarchy.enabled` + `state/mod.rs` plumbs settings
- `95df5461` feat(memory): LCA-bounded recall integration (PR #174) — `compute_lca_path` trait + `ItemKind::HierEntity` + recall step 5c
- `5f1cd723` feat(memory): inter-cluster relations in recall + doc update (PR #175) — `list_inter_cluster_relations` + `ItemKind::HierRelation` + dual-match agent_id fix + singleton display-name fix + observatory-side info log on every cycle
- (in flight on `feat/mem-001-contradiction-propagation`) feat(memory): MEM-001 Part A — contradiction propagation from `memory_facts.contradicted_by` down to KG entities and relationships sharing the same source episodes. New trait methods (`find_kg_nodes_by_episode_ids`, `apply_entity_confidence_multiplier`, `apply_relationship_confidence_multiplier`, `list_contradicted_fact_episode_ids`), `ContradictionPropagationConfig`/`ContradictionPropagationStats`, builder-style `DecayEngine::with_contradiction_propagation`, wired into the sleep cycle right after `decay_kg_confidence`. Token-flanked match (`,<id>,`) — no substring false positives. Tests: 6 SQLite trait tests + 3 engine tests + worker cycle unchanged (3 existing tests still green). MEM-001 Part B re-filed because the backlog premised it on a graph-traversal block that never landed in `recall/mod.rs` — see backlog update for the corrected scope.
- (PR #176, merged) feat(observatory+memory): hierarchy pill + slideover + `hierarchy_summary` trait method + `GET /api/hierarchy/stats` + prompt-side aggregate-name dedup (Option B). `AggregateEntityLlm::synthesize_aggregate` now takes `prior_names: &[String]` so the orchestrator can pass already-used names back to the LLM mid-cycle as an avoid-list. Visual surface uses the app's standard `belief-card` + `meta-chip` idioms instead of inline styles. Tests added: 4 `hierarchy_summary` SQL tests, 2 prompt-shape tests, 1 orchestrator accumulator test, 9 React panel tests.

**Behaviour change:** with `execution.memory.hierarchy.enabled = true`, the sleep cycle clusters layer-N entities, LLM-synthesises aggregates at layer N+1, writes inter-cluster relations gated by λ > τ. Recall walks the LCA path from top-N seed entities and emits `HierEntity` + `HierRelation` ScoredItems under a `## Topical Map` heading. Flag-off (default) is byte-for-byte unchanged from pre-H-3.

**First production run** (2026-05-17, ward = `__global__`, 693 layer-0 entities, `clusterTargetSize=20`, `intervalHours=0`): 30 layer-1 aggregates created, 11 inter-cluster relations. LLM-synthesised names like `agentic-system-components`, `multi-agent system cluster`, `agent-orchestration-cluster`. Relation types like `strongly-correlates-with`, `shares-context-with`. *Same-cycle name collisions* (multiple clusters synthesising the same generic label) were observed in this run and motivated Option B (PR #176).

### Phase E + F — Worker move, MemoryServices factory, gateway wiring collapse (2026-05-13)
- `20d59f9b` refactor(gateway): collapse memory construction into MemoryServices::new — Phase F
- `2b894611` feat(gateway-memory): add MemoryServices factory — Phase E.2
- `3c4e9973` refactor(gateway-memory): move SleepOps + SleepTimeWorker to gateway-memory — Phase E.1

**Effect:** `gateway/src/state/mod.rs` construction block reduced from 104 lines to 41 lines (63-line reduction). All memory subsystem construction now lives in `gateway-memory::MemoryServices::new(MemoryServicesConfig)`. Gateway retains only policy (interval hours, agent_id) — wiring is owned by the factory.

**Extraction complete.** A future memory-feature developer touches only `gateway-memory`.

### Phase D — LLM factory abstraction + 6 LLM impls migrated (2026-05-13)
- `f8adf7b1` refactor(handoff): move HandoffWriter struct via ConversationStore trait — MEM-005
- `87b46052` refactor(gateway-memory): move LlmPairwiseVerifier + use MemoryLlmFactory
- `d014de6a` refactor(gateway-memory): move LlmHandoffWriter + use MemoryLlmFactory
- `137d17c7` refactor(gateway-memory): move LlmConflictJudge + use MemoryLlmFactory
- `0f0806df` refactor(gateway-memory): move LlmCorrectionsAbstractor + use MemoryLlmFactory
- `c442cd3a` refactor(gateway-memory): move LlmPatternExtractor + use MemoryLlmFactory
- `9d0b6eb7` refactor(gateway-memory): move LlmSynthesizer + use MemoryLlmFactory
- `e9099d99` feat(gateway-memory): add MemoryLlmFactory trait + ProviderServiceLlmFactory impl
- `cb926b41` refactor(gateway-memory): move parse_llm_json into gateway-memory::util

**Per-impl LlmClientConfig** preserved exactly: Synthesizer (0.0, 512), PatternExtractor (0.0, 1024), CorrectionsAbstractor (0.0, 512), ConflictJudge (0.0, 256), HandoffWriter (0.2, 256), PairwiseVerifier (0.0, 128).

**MEM-005** bonus: HandoffWriter struct also moved via a one-method extension to `ConversationStore` trait (`get_session_messages`) + hoisting `Message` POD type into `zbot-stores-domain`.

### Phase C — Recall module moved to gateway-memory (2026-05-13)
- `f1c3be31` refactor(gateway-memory): move recall module from gateway-execution — Phase C

### Phase B — Sleep components moved to gateway-memory (2026-05-13)
- `bba92b87` refactor(gateway-memory): move HandoffWriter from gateway-execution
- `4ae55c8c` refactor(gateway-memory): move PatternExtractor from gateway-execution
- `4793aa9d` refactor(gateway-memory): move Synthesizer from gateway-execution
- `1e2e3062` refactor(gateway-memory): move Compactor from gateway-execution
- `dec59f88` refactor(gateway-memory): move ConflictResolver from gateway-execution
- `1a89ac97` refactor(gateway-memory): move CorrectionsAbstractor from gateway-execution
- `312a8ea5` refactor(gateway-memory): move OrphanArchiver from gateway-execution
- `89ad46ac` refactor(gateway-memory): move Pruner from gateway-execution
- `59a660a1` refactor(gateway-memory): move DecayEngine from gateway-execution
- `aa98a131` chore(gateway-memory): scaffold sleep submodule + add deps for Phase B

**Note:** `HandoffWriter` struct itself stayed in gateway-execution (concrete-type dep would cycle through zbot-stores-sqlite). Trait + read_handoff_block + supporting types + 5 of 10 tests moved. Full struct move tracked as MEM-005.

### Phase A — Memory crate extraction begins (2026-05-13)
- `a1e96a74` feat(gateway-memory): extract config types — Phase A of memory crate extraction

### Phase 5 — Bi-Temporal Memory + Self-RAG Query Gate (2026-05-15)

This phase completes the truth-interval model for facts/entities/relationships and adds an opt-in retrieval-gate ahead of hybrid search. Design doc: `docs/architecture/future-state/2026-05-15-bitemporal-wiring-design.md`. Survey-borrow context: arxiv 2602.05665 (Yang et al., graph memory survey — Graphiti bi-temporal pattern) + arxiv 2603.07670 (Du, memory mechanisms survey — Self-RAG retrieval gate).

**Recall pre-fix (prerequisite for bi-temporal):**
- `25da415b` fix(memory): distinguish bi-temporal `valid_until` from supersession in recall (PR #142) — `apply_class_aware_penalty` now checks `superseded_by.is_some()` instead of `valid_until.is_some()`. Without this, populating `valid_until` for genuine time-bounded facts would silently penalize them as if superseded.

**Bi-temporal wiring (4 PRs landed as separate phases):**
- `6913ff3f` feat(memory): populate `valid_from` on fact creation (PR #143, Phase 1) — `save_fact` trait gained `valid_from: Option<DateTime<Utc>>` parameter, default `Utc::now()`. v25 migration backfills legacy rows: `UPDATE memory_facts SET valid_from = created_at WHERE valid_from IS NULL`.
- `fca6abc8` feat(memory): point-in-time recall via `as_of` parameter (PR #144, Phase 2) — `recall_facts_prioritized` and `search_memory_facts_hybrid` (plus `_typed` variant) gained `as_of: Option<DateTime<Utc>>`. Default `None` = `Utc::now()`. Filter: `(valid_from IS NULL OR valid_from <= ?as_of) AND (valid_until IS NULL OR valid_until > ?as_of)`. Agent-callable `memory recall` tool exposes `as_of` as optional ISO-8601 param. Behavioral change: facts with past `valid_until` are now correctly excluded from default "now" queries.
- `faf450fe` feat(memory): align `kg_relationships` with bi-temporal schema (PR #145, Phase 3) — added `valid_from` + `valid_until` to `kg_relationships`. v26 migration backfills from legacy `valid_at`/`invalidated_at`. Legacy columns kept (gradual deprecation). `kg/storage.rs::store_relationship` now mirrors `first_seen_at` into `valid_from`.
- `5c9cb278` feat(memory): bi-temporal conflict transition semantics (PR #146, Phase 4) — `supersede_fact(old_id, new_id, transition_time)`. ConflictResolver passes `winner.created_at` so the loser's `valid_until` reflects when the world changed, not when the resolver ran. Continuous timeline, no gap, no overlap.

**Self-RAG retrieval gate (open):**
- PR #147 (feat/recall-self-rag-query-gate) — new `gateway/gateway-memory/src/recall/query_gate.rs` with `QueryGate` / `LlmQueryGate` / `RetrievalDecision::{Skip, Direct, Split}`. Wired into `MemoryRecall::recall` before hybrid search. Opt-in via `execution.memory.queryGate.enabled = true`. Failure-safe — any LLM error falls back to `Direct(raw_input)`. Goes through existing `MemoryLlmFactory`.

**Recall double-boost fix (back-merged):**
- `25da415b`'s sibling — PR #139 / cherry-pick #141 — removed `* 1.5` pre-boost on corrections in `recall_facts_prioritized` (corrections were getting 2.25× total instead of 1.5×, crowding out user-facts). Also bumped agent-callable memory tool recall default limit from 5 → 15.

### Phase 4 Foundation — KG Confidence Decay (2026-05-13)
- `e3a606a4` feat(sleep): wire KG confidence decay into sleep worker
- `8e7e22ee` feat(sleep): add decay_kg_confidence to DecayEngine
- `d6c11834` feat(kg): add decay_entity_confidence + decay_relationship_confidence store methods
- `c5927dc5` feat(recall): add KgDecayConfig to RecallConfig
- `207e3ed7` feat(kg): add evidence column to kg_entities and kg_relationships

### Phase 3 — Conflict Resolution (2026-05-13)
- `93206722` feat(gateway): wire ConflictResolver into sleep worker
- `ae969b87` feat(sleep): wire ConflictResolver into SleepOps and cycle loop
- `50b1a35d` chore(sleep): export ConflictResolver from sleep mod
- `9ff7002d` feat(sleep): add ConflictResolver — supersede contradicting schema facts *(also added `MemoryFactStore::get_fact_embedding` trait method)*
- `9c9a447c` feat(settings): add conflict_resolver_interval_hours to MemorySettings
- `2d1c7a80` fix(recall): exclude superseded facts from results

### Phase 2 — Pattern Abstraction (2026-05-12 → 13)
- `34d6058e` feat(memory): configurable corrections_abstractor_interval_hours in settings.json *(introduced `MemorySettings` struct)*
- `9b51842e` feat(sleep): wire CorrectionsAbstractor into SleepOps and cycle loop
- `11c1c939` feat(sleep): add CorrectionsAbstractor — distill correction facts into schema facts
- `cd3421f3` feat(recall): add schema category weight (1.6) to RecallConfig

### Phase 1 — Reflective Recall + Handoff (2026-05-10 → 12)
- `9adb8b62` feat(handoff): targeted recall from last session topics
- `158d9d77` fix(recall): use real similarity scores + suppress results below min_score
- `c51b9066` feat(recall): add min_score threshold to RecallConfig (default 0.3)
- `354e5ea9` feat(handoff): inject active goals at session start
- `9d0891c8` feat(handoff): always-inject active corrections at session start
- `c0a9850b` feat(handoff): include tool-call names in LLM summary prompt
- `9c67f2a1` fix(handoff): ward-scoping + real correction_count
- `f807185b` refactor(handoff): store handoff in fact DB instead of flat file
- `b1bf766c` feat(handoff): inject ## Last Session block + wire LlmHandoffWriter
- `e4495dc0` feat(handoff): wire HandoffWriter through execution pipeline
- `3c29221e` refactor(handoff): eliminate double-parse of completed_at in read_handoff_block
- `4868ef75` feat(handoff): export HandoffWriter types from sleep/mod.rs
- `57f057e9` feat(handoff): implement read_handoff_block + 3 tests
- `e4ce967b` fix(handoff): prefix unused stub params with underscore
- `779eaaea` style(handoff): apply cargo fmt to handoff_writer.rs
- `160b2232` feat(handoff): add HandoffWriter with 4 passing unit tests

---

## 9. Plan Documents (Specs)

Each phase has a written plan that describes the *intent* of the change set. These are the authoritative specs to consult during extraction:

- `docs/superpowers/plans/2026-05-12-reflective-memory-phase1-completion.md`
- `docs/superpowers/plans/2026-05-12-recall-min-score-threshold.md`
- `docs/superpowers/plans/2026-05-12-pattern-abstraction.md` *(Phase 2)*
- `docs/superpowers/plans/2026-05-13-conflict-resolution.md` *(Phase 3)*
- `docs/architecture/future-state/2026-05-15-bitemporal-wiring-design.md` *(Phase 5 — bi-temporal wiring across all 4 sub-phases)*

---

## 10. Suggested Extraction Order (when you're ready)

1. ✅ **DONE (commit `a1e96a74`)** — **First**: Move `RecallConfig` + `MemorySettings` types into the new `zero-memory` crate. Keep references in `gateway-services` as re-exports. Low risk, no logic changes.
2. ✅ **DONE (Phase B, 9 commits)** — **Second**: Move the sleep-time components (`Compactor`, `Synthesizer`, `PatternExtractor`, `Pruner`, `OrphanArchiver`, `CorrectionsAbstractor`, `ConflictResolver`, `HandoffWriter`) — they're self-contained behind their trait-object inputs.
3. ✅ **DONE (Phase D + MEM-005, 9 commits)** — **Third**: Introduce `MemoryLlmFactory` trait, replace direct `ProviderService` dependency. This is the hardest decoupling because every `Llm*` production impl has the same `build_client` shape — there's an opportunity to DRY this into one shared helper.
4. ✅ **DONE (Phase C, 1 commit)** — **Fourth**: Move `recall/mod.rs` + `RecallConfig` into `zero-memory`. Caller in `invoke_bootstrap.rs` stays in gateway because it composes recall with goals + handoff + corrections (all of which are memory) but also with the orchestrator runtime (not memory).
5. ✅ **DONE (Phase E)** — Move `SleepOps` + `SleepTimeWorker` into `gateway-memory`. Replace the imperative construction block in `state/mod.rs` with a `MemoryServices::new(MemoryServicesConfig { ... })` factory call.

   ✅ **DONE (Phase F)** — `gateway/src/state/mod.rs` construction block collapsed from 104 lines to 41 lines via `MemoryServices::new(...)`. All 14 config fields accept trait objects (no concrete-type leaks into gateway).

After extraction, the `gateway` crate should only need:
- `zero-memory::MemoryServices` (factory)
- `zero-memory::SleepTimeWorker` (started in state/mod.rs)
- `zero-memory::RecallEngine` (called from invoke_bootstrap.rs)

Plus a few injected traits for cross-cutting concerns: `MemoryLlmFactory`, the existing store traits.

**Phase 4 foundation already in `zero-memory` candidates:** the new `KgDecayConfig`, `DecayEngine::decay_kg_confidence`, and `KgDecayStats` all live in the same files as their Phase 1–3 siblings, so the extraction migration touches the same crates.
