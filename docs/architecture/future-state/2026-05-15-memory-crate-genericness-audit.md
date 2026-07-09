# Memory Crate Genericness Audit

**Date:** 2026-05-15
**Status:** Audit complete — not for implementation yet; recommendations need user prioritization
**Goal:** Identify concepts in `gateway/gateway-memory` that couple it to zbot specifics, so a future "memory crate" extraction can ship as a generic, embeddable layer.
**Related:**
- `[[project_memory_crate_extraction]]` — long-term extraction tracking doc
- `[[2026-05-13-memory-crate-extraction-tracking]]` — sibling tracking
- `[[2026-05-15-bitemporal-wiring-design]]` — adjacent design touching same files

---

## Summary

The crate has **23 distinct leak sites** across 9 modules. Of these:

- **6 rename** issues (the dominant family — "ward" terminology, `category=="ward"`, magic `__global__` ward sentinel, `ward_affinity_boost`, `HANDOFF_WARD`, `ward_wiki` provenance source)
- **9 configurable** issues (hardcoded category names baked into code paths: `"correction"`, `"schema"`, `"strategy"`, `"skill"`, `"agent"`, plus hardcoded class names `"archival" | "current" | "convention" | "procedural"`, the `MIN_CORRECTIONS_TO_ABSTRACT=3` literal, the `0.92` compaction cosine, and `MAX_SCHEMA_FACTS_PER_CYCLE=50`)
- **5 move-to-gateway** issues (the entire `HandoffWriter` + `## Last Session` block, `HANDOFF_AGENT_SENTINEL`, the `format_recall_failure_message` mentioning `memory(action="recall", ...)`, the `format_scored_items` heading `## Recalled Context`, and the `read_handoff_block` formatted prompt)
- **3 leave** items (the `agent_id` parameter on every operation; bi-temporal column names; the LLM JSON-parsing util)

**Top recommendations:**
1. Rename `ward_id` → `scope_id` (or `partition_id`) across the public API surface. This is the largest single coupling and a mechanical sed.
2. Move `HandoffWriter` and `read_handoff_block` out of the memory crate. They format zbot-specific chat prompts — pure presentation belongs in `gateway-execution` / orchestrator.
3. Make `RecallConfig.category_weights` and the `epistemic_class` policy in `apply_class_aware_penalty` data-driven via config tables rather than hardcoded match arms.

The core retrieval and sleep-cycle math (RRF fusion, temporal decay, hybrid scoring, supersession penalty math, KG compaction) is already generic and only needs surface-level cleanup.

---

## Methodology

Files read end-to-end:
- `gateway/gateway-memory/src/lib.rs` (config types, public exports)
- `gateway/gateway-memory/src/services.rs` (composition factory)
- `gateway/gateway-memory/src/llm_factory.rs`
- `gateway/gateway-memory/src/util.rs`
- `gateway/gateway-memory/src/recall/mod.rs`
- `gateway/gateway-memory/src/recall/adapters.rs`
- `gateway/gateway-memory/src/recall/previous_episodes.rs`
- `gateway/gateway-memory/src/recall/scored_item.rs`
- `gateway/gateway-memory/src/sleep/mod.rs`
- `gateway/gateway-memory/src/sleep/worker.rs`
- `gateway/gateway-memory/src/sleep/compactor.rs`
- `gateway/gateway-memory/src/sleep/conflict_resolver.rs`
- `gateway/gateway-memory/src/sleep/corrections_abstractor.rs`
- `gateway/gateway-memory/src/sleep/decay.rs`
- `gateway/gateway-memory/src/sleep/handoff_writer.rs`
- `gateway/gateway-memory/src/sleep/orphan_archiver.rs`
- `gateway/gateway-memory/src/sleep/pattern_extractor.rs`
- `gateway/gateway-memory/src/sleep/pruner.rs`
- `gateway/gateway-memory/src/sleep/synthesizer.rs`
- `gateway/gateway-memory/src/sleep/verifier.rs`
- `gateway/gateway-memory/Cargo.toml`

Patterns searched: `ward`, `__global__`, `__handoff__`, `__pruned__`, `orphan-archive`, hardcoded category literals (`"correction"`, `"schema"`, `"strategy"`, `"user"`, `"skill"`, `"agent"`), epistemic-class literals (`"archival"`, `"current"`, `"convention"`, `"procedural"`), Markdown headings (`## `), and any `gateway_services::` / `gateway-services` references.

---

## Findings by Category

### Rename

Concepts that are generic but named with zbot terminology.

| # | File:Line | Quote | Suggestion | Effort |
|---|-----------|-------|------------|--------|
| R1 | `lib.rs:195` | `pub ward_affinity_boost: f64,` | `scope_affinity_boost` (or `partition_affinity_boost`) | Cross-cut: ~½ day. Touches `RecallConfig`, every test asserting it (5+), and every external config JSON. Public-API break. |
| R2 | `lib.rs:225`, `lib.rs:393` | `("ward".to_string(), 0.8),` — `ward` as a recall **category weight** | Rename the category itself if generic; OR drop from defaults and let consumers add. The category-as-a-fact-kind is fine; just the name is zbot-y. | ~½ hr config-only |
| R3 | `recall/mod.rs:243` | `if sf.fact.key.starts_with(&ward_prefix) \|\| sf.fact.category == "ward" {` | Same as R2 — the special-case treatment of `"ward"` category in the affinity-boost path. Couples scoring to a specific zbot category name. | ~1 hr — fold into configurable category weights so no name is hardcoded in code. |
| R4 | `recall/mod.rs:240` | `if !current_ward.is_empty() && current_ward != "scratch" {` | The `"scratch"` literal is a zbot UX concept (the "no ward selected" placeholder). Generic name: a `None` or empty-string sentinel handled at the call site. | ~½ hr — drop the `"scratch"` check; callers already control `ward_id: Option<&str>`. |
| R5 | `recall/adapters.rs:41` | `source: "ward_wiki".to_string(),` | `Provenance.source` is a free-form string but `"ward_wiki"` is zbot vocabulary. `"wiki"` or `"scoped_wiki"` is generic. | ~10 min |
| R6 | `recall/previous_episodes.rs:32-35` | `pub async fn fetch(&self, ward_id: &str)` calling `fetch_recent_successful_by_ward(ward_id, 3)` | Trait method on `EpisodeStore` carries the `ward` name too. Coupled to `zbot-stores-traits`. Rename to `scope_id` / `partition_id` everywhere. | Cross-cut with R1: ~1 day total once all trait surfaces touched. |

The R1/R6 cluster is the largest single coupling — `ward_id` appears in `MemoryFact`, `ScoredItem.Provenance`, `WikiArticle`, `Procedure`, `SessionEpisode`, `HandoffEntry`, plus every trait method and every test. Most of those types live in `zbot-stores-domain` / `zbot-stores-traits`, so this is a multi-crate rename, not gateway-memory-only.

### Configurable

Concepts that hardcode zbot-specific values that should be config.

| # | File:Line | Quote | Suggestion | Effort |
|---|-----------|-------|------------|--------|
| C1 | `recall/mod.rs:189` | `.get_facts_by_category(agent_id, "correction", 10)` — recall explicitly fetches a fixed category | The whole "always pull recent corrections at recall time" is a zbot policy. Make the categories-to-always-pull a config list with defaults. | ~½ day — extract into `RecallConfig.always_include_categories: Vec<CategorySpec>` |
| C2 | `recall/mod.rs:197` | `let corrections: Vec<_> = all_corrections.into_iter().take(5).collect();` then `score: fact.confidence * 1.5` | Magic `1.5` boost and magic `take(5)` cap. Already covered by `category_weights["correction"]=1.5` — this is a double-apply that bakes in the policy. | ~1 hr — kill the second apply; rely on category_weights only. |
| C3 | `recall/mod.rs:254` | `if sf.fact.category == "skill" \|\| sf.fact.category == "agent" { continue; }` — decay exemption hardcoded for two specific categories | Configurable as `temporal_decay.exempt_categories: Vec<String>` | ~½ hr |
| C4 | `recall/mod.rs:498-525` (`apply_class_aware_penalty`) | match arms `"archival" => 0.3`, `"current" => 0.1`, `"convention" \| "procedural" => no-op`, `_ => 0.3` | The epistemic-class system has FOUR baked-in class names + magic multipliers. Generic: `RecallConfig.epistemic_class_supersession_penalty: HashMap<String, f64>` with a "default" key for unknowns. | ~½ day. Touches the recall scoring path + 7 unit tests. |
| C5 | `sleep/corrections_abstractor.rs:22-24` | `MIN_CORRECTIONS_TO_ABSTRACT: usize = 3;` `MAX_CORRECTIONS_PER_CALL: usize = 20;` `MIN_CONFIDENCE: f64 = 0.7;` | These three magic numbers govern when "promote correction → schema" fires. Should be a config struct. | ~1 hr |
| C6 | `sleep/corrections_abstractor.rs:101, 137` | `get_facts_by_category(agent_id, "correction", ...)` then `save_fact(agent_id, "schema", ...)` | The "correction" source and "schema" sink categories are hardcoded. Pattern: an abstraction step that distills facts from one category into another. Make source/sink configurable. | ~1 hr. Same change applies to the `key = format!("schema.corrections.{}", short_hash(...))` on line 131. |
| C7 | `sleep/conflict_resolver.rs:18-21, 90` | `MAX_SCHEMA_FACTS_PER_CYCLE: 50`, `MAX_LLM_CALLS_PER_CYCLE: 10`, `MIN_SIMILARITY: 0.85`, `MIN_CONFIDENCE: 0.7`, plus `.get_facts_by_category(agent_id, "schema", ...)` | The resolver only ever runs against the `"schema"` category. Both the category and the budgets are hardcoded. Make a `ConflictResolverConfig { source_categories: Vec<String>, max_facts_per_cycle, max_llm_calls, min_similarity, min_confidence }`. | ~1 hr |
| C8 | `sleep/compactor.rs:19-21, 42-48` | `DEFAULT_COSINE_THRESHOLD: f32 = 0.92;` `DEFAULT_PER_TYPE_LIMIT: 50;` `DEFAULT_TYPES: &[EntityType] = &[Person, Organization, Location, Event, Concept];` | Tunables already configurable via `with_cosine_threshold` / `with_per_type_limit` (good). But the `DEFAULT_TYPES` list bakes the zbot KG ontology. A generic compactor should accept the type list as a parameter. | ~½ hr — promote to constructor parameter. |
| C9 | `sleep/synthesizer.rs:27-35` | `CANDIDATE_LIMIT`, `MAX_LLM_CALLS_PER_CYCLE`, `MIN_CONFIDENCE`, `DEDUP_COSINE_THRESHOLD`, `LOOKBACK_DAYS` all `const` | Same pattern — wrap in a `SynthesizerConfig` struct. Also hardcodes target category as `"strategy"` (`key = format!("strategy.synthesis...")` line 250) and the synth slug schema. | ~1 hr |

### Move-to-gateway

Concepts that should not live in a generic memory crate.

| # | File:Line | Quote | Suggestion | Effort |
|---|-----------|-------|------------|--------|
| M1 | `sleep/handoff_writer.rs:25` | `pub const HANDOFF_AGENT_SENTINEL: &str = "__handoff__";` and `HANDOFF_WARD: &str = "__global__";` and `HANDOFF_CATEGORY: &str = "handoff";` | Magic sentinels that say "this row is a session handoff, written as a row in the user's facts table." The whole "store a chat-session summary as a fact row keyed `handoff.latest`" is a zbot orchestrator pattern. Doesn't belong in a generic memory crate. | ~1 day — move `handoff_writer.rs` to `gateway-execution`. The crate-level re-exports (`pub use sleep::handoff_writer::*`) drop. |
| M2 | `sleep/handoff_writer.rs:93-107` | `"## Last Session  ({date} · ward: {ward} · {turns} turns)\n{summary}\n\nCorrections active: {corrections} · Goals: {goals}\nFull context: memory(action=get_fact, key=handoff.{sid})\nLast intent:  memory(action=get_fact, key={intent_key})"` | Hardcoded LLM-prompt format. References a `memory(action=get_fact, ...)` tool that only exists in zbot's agent runtime. Pure presentation layer. | Same as M1 — moves out with the writer. |
| M3 | `sleep/handoff_writer.rs:161-171` | The summarization prompt itself: `"Summarize this conversation in 3-5 sentences. Cover: - What was accomplished - What was left incomplete or in progress - What the user seemed most focused on or interested in next..."` plus `Ward: {ward}` | The prompt assumes a chat-style multi-turn conversation between user and assistant. Generic memory layer should not embed conversation templates. | Same as M1 |
| M4 | `recall/mod.rs:456-461` (`format_recall_failure_message`) | `format!("[Memory retrieval failed: {}. You can call memory(action=\"recall\", query=...) manually if you need past context.]", err)` | Mentions a specific tool name `memory(action="recall", query=...)` that lives in zbot's agent-tools crate. A generic memory layer shouldn't know what its consumers' tools are called. | ~½ hr — return the error, let the caller format the user-visible string. |
| M5 | `recall/mod.rs:469-487` (`format_scored_items`) | `lines.push("## Recalled Context".to_string());` and `format!("- [{}] {}", tag, item.content)` | Hardcoded Markdown chat-prompt format. Generic recall should return `Vec<ScoredItem>` and let presentation happen at the boundary. | ~½ hr — move the formatter to whoever injects into prompts (gateway-execution). |

### Leave (acceptable)

Concepts that look zbot-specific but on inspection are either load-bearing for the memory model itself or already generic.

- **`agent_id` parameter pervasive across the API.** Every memory operation takes `agent_id: &str`. This is a generic *tenant key*, not a zbot concept. Calling it `tenant_id` or `namespace_id` could be cleaner, but the multi-tenant model itself is a sound generic primitive. **Leave** — rename is bikeshedding; the abstraction is fine.

- **`__global__` ward sentinel.** Appears at `corrections_abstractor.rs:156`, `pattern_extractor.rs:42` (`PROC_WARD`), `handoff_writer.rs:28` (`HANDOFF_WARD`), and in test fixtures. Looks like a zbot magic string. On close reading, it's a convention for "this fact has no scope" — equivalent to `NULL` but the schema column is `NOT NULL`. The sentinel is awkward but it's a **schema constraint**, not a leak in this crate. Move would have to touch the SQLite migration first. **Leave for now**; revisit when the schema is rebuilt during extraction. Capture as a follow-up.

- **Bi-temporal column names (`valid_from`, `valid_until`, `superseded_by`, `epistemic_class`).** These match the Graphiti / temporal-RDF literature; not zbot-specific. **Leave.**

- **`parse_llm_json` / `strip_code_fence` (`util.rs`).** Generic LLM-output postprocessing. Already perfectly portable. **Leave.**

- **RRF math (`scored_item.rs:rrf_merge`), temporal decay function (`recall/mod.rs:temporal_decay`), `apply_class_aware_penalty` arithmetic.** All pure functions with no zbot semantics. The only leak is in their *config* (covered under Configurable). **Leave.**

- **`Compactor`, `Pruner`, `DecayEngine`, `OrphanArchiver`.** Operate on `Arc<dyn KnowledgeGraphStore>` / `Arc<dyn CompactionStore>` — backend-agnostic. The orphan threshold (`MIN_AGE_HOURS = 24`, `ARCHIVE_LIMIT = 100`, `MIN_CONFIDENCE_FOR_KEEP = 0.5`) are zbot-tuned but not zbot-coupled. Already covered under Configurable C8. **Leave** as classes.

- **`MemoryLlmFactory` trait (`llm_factory.rs`).** Abstracts LLM client construction. The crate-level decision to inject a factory rather than depend directly on `gateway-services::ProviderService` is exactly what a generic crate should do. **Leave** — this is already correctly generic.

- **`gateway_services::VaultPaths` in test-only setup code** (12+ test files). Test fixtures only; not in the production path. **Leave** — could be replaced with a vault-path stub when extracting, but is not a production leak.

---

## Recommended Priority Order

Ranked by `genericness_impact / effort`:

1. **M4 + M5 (drop in-crate prompt formatters)** — ~1 hour total. Removes two of the most embarrassing leaks (the tool-name reference and the `## Recalled Context` heading) with minimum risk. No schema change, no API break beyond moving two pub-fns out.

2. **M1 + M2 + M3 (move `HandoffWriter`)** — ~1 day. Single biggest concept that does NOT belong here. Touching `services.rs` factory imports and one `pub use` in `lib.rs`. Frees the crate from `agent-runtime::ChatMessage` formatting concerns for session handoffs.

3. **C4 (`apply_class_aware_penalty` → configurable)** — ~½ day. The four-class match arm is the most opinionated piece of recall scoring in the crate. Making it data-driven also unblocks consumers who want different fact-class ontologies (e.g., a graph-RAG layer that doesn't have `"archival"`/`"current"`).

4. **C8 + C9 (compactor/synthesizer config structs)** — ~2 hours. Mechanical; promotes magic constants to a per-component config. Worth doing the same day as C5 + C7 since the pattern is identical.

5. **R1 + R6 (ward → scope rename across trait surface)** — ~1 day. Biggest single rename but cross-cuts `zbot-stores-traits` and `zbot-stores-domain` too. Defer until the bi-temporal wiring (PR #142 etc.) settles to avoid merge conflicts.

6. **C1 + C2 (recall "always pull corrections" → configurable)** — ~½ day. Removes the most coupled bit of the recall pipeline. Less urgent because it doesn't break anything, just makes the policy explicit.

7. **R2 + R3 + R5 (`"ward"` category, `ward_wiki` source, etc.)** — ~1 hour. Cosmetic but cheap. Bundle with R1.

8. **R4 (drop `"scratch"` literal in recall)** — ~½ hr. Trivial.

9. **C3 (skill/agent decay exemption → config)** — ~½ hr. Trivial.

10. **C5 + C6 + C7 (correction/conflict tunables)** — ~3 hours. Same pattern, do together.

Total estimated effort if all done: **~4-5 engineer days**, spread across at least three PRs to keep blast radius manageable.

---

## Out of Scope

- **`zbot-stores-domain` field renames** (e.g., `MemoryFact.ward_id`). Several leaks (R1, R5, R6) depend on these. Audit flagged but the primary fix belongs in the stores crates, not in `gateway-memory`.
- **`zbot-stores-traits` method signatures** (e.g., `fetch_recent_successful_by_ward`, `get_session_ward_id`). Same — these are trait-level renames that touch every backend.
- **Schema-level rename** of `kg_entities.ward_id` / `memory_facts.ward_id` columns. Migration work, not crate work.
- **Tests as a coupling source.** Test fixtures use `gateway_services::VaultPaths` and `zbot-stores-sqlite` types. Production code does not. Acceptable for now; will need adapter test harness during extraction.
- **`HANDOFF_MAX_AGE_DAYS = 7`** — even though it's a zbot-tuned default, it ships with the (to-be-moved) handoff writer; no separate action.
- **`tokio` runtime dependency.** The sleep worker uses `tokio::spawn`. Acceptable; a generic crate can require a tokio runtime in the same way Axum / Reqwest do.
- **`chrono` for timestamps.** Already a workspace standard; not a zbot-specific dependency.
- **Bi-temporal wiring gaps** — covered in the parallel `2026-05-15-bitemporal-wiring-design.md`.

---

## Decision Log

- **2026-05-15:** Did not propose renaming `agent_id` → `tenant_id` despite considering it. The current name is unambiguous in the AI-agent-systems domain that this crate targets; `tenant_id` would be cleaner generically but is a larger churn for marginal clarity. Leave for an explicit later decision.
- **2026-05-15:** Decided `__global__` is a schema concern, not a `gateway-memory` concern. The sentinel exists because the schema column is `NOT NULL` — fixing requires altering the schema. Listed under "Leave" for now.
- **2026-05-15:** Treated `category_weights["ward"] = 0.8` (R2) as a rename leak rather than configurable because the **default category exists at all** is what couples the crate. Removing the default key cleanly removes the coupling.
- **2026-05-15:** The `HandoffWriter` block is the most prompt-coupled piece of the entire crate. It survived the gateway-memory extraction because moving it required threading `agent-runtime::ChatMessage` somewhere else. That's a one-time refactor cost; it should not be a permanent residency reason.
