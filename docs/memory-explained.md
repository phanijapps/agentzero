# How zbot Remembers

## 1. High-level overview

### What zbot's memory actually is

A chat transcript is not memory. A transcript captures what was said in order; it doesn't capture what the agent has *learned*. zbot's memory is the second layer — the part that survives sessions and gets re-read every time a new conversation opens. It is what makes the agent more useful in week two than it was in week one.

The metaphor that holds together across the whole subsystem is **daily log vs. reference book**. Every conversation is a page in the log: exhaustive but inert. What gets distilled from those pages — corrections, schemas, user facts, relationships, beliefs — is the reference book the agent actually consults. The log is large and grows monotonically; the reference book stays small, opinionated, and re-read constantly.

### What ships today

The reference book is split across four layers, each implemented and live:

- **Fragments** — flat notes (`memory_facts`) tagged with a category, content, confidence, embedding, and a lifecycle status (`active`, `contradicted`, `superseded`, `archived`).
- **Knowledge graph** — entities (`kg_entities`) and typed relationships (`kg_relationships`), each with their own confidence and bi-temporal interval. Provenance links fragments to the conversations they came from, and to the entities those conversations produced.
- **Belief Network** — synthesized stances (`kg_beliefs`) built from one or more facts about the same subject, plus first-class contradiction records (`kg_belief_contradictions`) when two beliefs disagree.
- **Hierarchical memory** *(opt-in via `execution.memory.hierarchy.enabled`)* — a multi-layer hierarchy built on top of `kg_entities`. The sleep cycle clusters layer-N entities, synthesises an aggregate entity for each cluster at layer N+1, and writes inter-cluster relations between aggregates whose underlying connectivity is high enough. Recall walks `parent_cluster_id` from the top seed entities up to their lowest common ancestor and surfaces the path as a *topical map* alongside the raw facts. HiRAG/LeanRAG-inspired.

A **bi-temporal model** runs through all four: every row records when a fact became true *in the world* (`valid_from`) and when it stopped being true (`valid_until`), separately from when it was written to the database. Point-in-time recall ("what did I believe in February?") falls out of that for free.

A **hybrid recall pipeline** decides what the agent reads next: FTS5 keyword search + sqlite-vec cosine similarity, blended via reciprocal rank fusion, optionally pre-filtered by a Self-RAG retrieval gate, optionally diversity-reranked by MMR, optionally cross-encoder-reranked by `fastembed-rs`, then category-weighted and ward-boosted.

An **hourly sleep cycle** runs maintenance: compaction, synthesis, pattern extraction, corrections abstraction, conflict resolution, confidence decay, orphan archival, pruning, belief synthesis, contradiction detection, event-driven belief propagation, and (opt-in) hierarchical aggregation.

### The four moves the agent makes

| Move | What happens | When |
|---|---|---|
| **Capture** | New facts written to `memory_facts`; entities + relationships into the graph; embeddings indexed; FTS5 triggers fire | During and after conversations |
| **Distill** | Facts → beliefs (B-1 synthesis); 3+ corrections → schemas; recurring tool sequences → patterns; near-duplicate entities → merges | Sleep cycle, default hourly |
| **Retrieve** | Hybrid search + RRF + rescore + graph expansion + belief surfacing | Session bootstrap, mid-session every 5 turns, explicit `memory(action="recall")` |
| **Maintain** | Decay, supersede, retract, mark stale, archive, prune; propagate confidence changes through the belief network | Sleep cycle + inline on fact lifecycle events |

The rest of this document describes how each layer is built, how the agent interacts with it, and what knobs the operator has to tune it.

---

## 2. Memory Architecture and how Agents interact

This section is the agent's-eye view: what enters the prompt, what the agent can ask for, and what gets written back.

### What the agent reads at session start

When a session starts, the bootstrap reads memory in a specific order. The order matters — the LLM weights earlier context more strongly, so the most authoritative material goes first:

1. **`## Last Session`** — the handoff fragment from the previous session in the same ward, formatted as a compact summary. Ward-scoped on purpose: a maritime-tracking handoff shouldn't bleed into a finance session.
2. **`## Active Goals`** — every goal with `state == "active"`, pulled directly from the goal tracker. Always injected; no recall filter.
3. **`## Active Corrections`** — every `correction`-category fragment for this agent, fetched unconditionally. Recall can miss; corrections must not.
4. **`## Active Beliefs`** *(when Belief Network is enabled)* — high-confidence beliefs surfaced by the recall pipeline for the user's query, formatted as a separate block ahead of raw facts.
5. **`## Context from Last Session`** — a separate recall pass using the handoff summary as the query string. Captures topical context from last time even when the user's first message is short or generic.
6. **`## Recalled Context`** — the standard recall pass against the actual user message, blending facts, wiki, procedures, and graph-expanded entities.

### What the agent sees mid-session

Every `mid_session_recall.every_n_turns = 5` turns, recall re-runs with the latest exchange as the query. Fragments that clear `min_novelty_score = 0.3` get injected. This catches topic drift without re-injecting everything the agent already saw.

When the session ends, a handoff writer summarises the conversation via LLM (with tool-call names included, so the next session knows what was *attempted*, not just what was said) and writes a fresh `handoff.latest` fragment. The loop closes.

### What the agent can ask for explicitly: the `memory` tool

A single `memory` tool is exposed to every agent. Its `action` parameter routes to one of:

| Action | Purpose |
|---|---|
| `get` | Look up a fragment by exact `key` |
| `set` | Write or update a fragment by `key` (durable; bypasses session-only state) |
| `save_fact` | Write a category-tagged fact with confidence, source episode, and optional embedding |
| `recall` | Run the hybrid search pipeline against a query string; optional `as_of` for point-in-time, optional `ward` filter |
| `belief` | Look up a belief by subject (returns content + confidence + source facts) — Belief Network only |
| `contradictions` | List unresolved contradictions, optionally filtered by `belief_id` — Belief Network only |

`as_of` accepts an ISO-8601 timestamp. When omitted, recall uses `Utc::now()` — so default queries correctly exclude facts whose bi-temporal interval has ended.

### Write paths

Three paths write to memory:

- **Implicit (post-session distillation)** — when a session ends, a distiller walks the transcript and writes any new fragments, entities, and relationships. This is the bulk path; the agent doesn't have to do anything for it to happen.
- **Explicit (agent-driven)** — the agent can call `memory(action="set"|"save_fact")` mid-session to durably store something it wants to remember. Used sparingly; most useful for facts the user gave directly ("call me Phani").
- **Human-curated (UI)** — the `/memory` tab in the desktop UI lets a human inspect, edit, supersede, or delete fragments and resolve contradictions. The `/observatory` tab visualises the knowledge graph and surfaces health metrics (belief counts, contradiction counts, distillation progress).

### Ward scoping

Every fact and entity carries a `partition_id` (the ward). Recall applies a `ward_affinity_boost = 1.3` multiplier when a candidate's ward matches the active session's ward. Global-scope rows (no partition) always surface. This is what keeps maritime-tracking conversations from pulling finance memory and vice versa.

### Two views, one provenance chain

Fragments answer *"what should I remember about this topic?"* The graph answers *"what is connected to what?"* They are linked by **provenance**: when a fragment is written, the `source_episode_ids` column ties it back to the conversation it came from, and the same episode IDs are recorded on the entities that conversation produced. Recall can start from a matched fragment, walk to the entities it referenced, and reach related entities (and their fragments) by graph traversal — even when the user's query didn't name any of them.

The Belief Network sits a layer above: beliefs are aggregates of facts about the same subject. They have their own embedding and surface in recall alongside facts, with their own category weight and their own prompt block (`## Active Beliefs`).

---

## 3. Technical Architecture

### Storage layout

Two SQLite databases live under the agent's data directory (`$XDG_DATA_HOME/agentzero/` on Linux, equivalent paths on macOS / Windows):

**`knowledge.db`** — the live memory store. Schema version **30**. Tables:

| Table | Purpose |
|---|---|
| `memory_facts` | Flat fragments (content + metadata, no embeddings on row) |
| `memory_facts_fts` | FTS5 virtual table over fragment content, kept in sync by triggers |
| `memory_facts_index` | sqlite-vec virtual table holding fact embeddings, joined by `fact_id` |
| `kg_entities` | Knowledge graph nodes, with confidence and lifecycle columns |
| `kg_relationships` | Typed edges with confidence and bi-temporal interval |
| `kg_episodes`, `session_episodes` | Provenance from conversations to entities and fragments |
| `kg_beliefs` | Synthesized belief stances, with on-row embedding (since v30) |
| `kg_belief_contradictions` | First-class contradiction records between belief pairs |
| `compaction_log` | Audit trail for KG merges |
| `goals`, `goal_progress` | Session-level goal tracker |

**`conversations.db`** — the daily log: chat history, separate file, separate concerns. The memory subsystem never touches it directly — it goes through the `ConversationStore` trait, which decouples memory from any specific chat backend.

### Schema migration history

| Version | What it added |
|---|---|
| v23 | Wiki FTS index |
| v24 | Global-scope backfill (`partition_id IS NULL` semantics) |
| v25 | `memory_facts.valid_from` backfill |
| v26 | `kg_relationships` bi-temporal columns |
| v27 | `kg_beliefs` table |
| v28 | `kg_belief_contradictions` table |
| v29 | `kg_beliefs.stale` flag (for B-3 propagation) |
| v30 | `kg_beliefs.embedding` (for B-4 recall integration) |
| v31 | `kg_entities.layer` + `parent_cluster_id`; `kg_relationships.layer` + `is_inter_cluster` (for hierarchical memory H-3/H-4) |

Missing migrations are applied on startup, in order. Failures abort startup loudly rather than half-migrating.

### Fragment categories

Every fragment has a category. The category controls how aggressively recall promotes it:

| Category | Weight | What it is |
|---|---|---|
| `schema` | 1.6 | Higher-level principle distilled from multiple corrections |
| `correction` | 1.5 | User told the agent to do X instead of Y |
| `belief` | 1.5 | Synthesized stance from one or more facts (Belief Network) |
| `strategy` | 1.4 | Cross-session approach the agent has accumulated |
| `user` | 1.3 | Durable facts about the user (role, preferences, expertise) |
| `instruction` | 1.2 | Durable directives that aren't corrections |
| `domain` | 1.0 | General domain knowledge |
| `pattern` | 0.9 | Procedural patterns abstracted from tool-call sequences |
| `ward` | 0.8 | Scope-specific metadata |
| `skill` / `agent` | 0.7 | Index metadata, mostly lookup |
| `handoff` | special | Session summary written at session end |

Schemas outrank corrections because they're distilled. Skill and agent indices sit at the bottom because they exist for lookup, not behavior shaping. Beliefs sit at the correction level (1.5) — conservative until empirical validation shows they deserve higher.

### Lifecycle states

Every fragment is in one of four states:

- **active** — visible to recall
- **contradicted** — `contradicted_by` is set; still visible, but penalized (`× 0.7`)
- **superseded** — `superseded_by` is set; hidden from recall, kept for audit
- **archived** — soft-deleted (`epistemic_class = 'archival'` on the KG side)

### Knowledge graph

An **entity** in `kg_entities` carries: `id`, `agent_id`, `name`, `normalized_name`, `normalized_hash` (identity), `entity_type` (`person`, `project`, `concept`, etc.), `confidence` (starts at 0.8, decays), `mention_count` and `access_count`, `first_seen_at` / `last_seen_at` / `last_accessed_at`, `epistemic_class` (`current` / `archival`), `source_episode_ids`, and an `evidence` JSON column reserved for contradiction-propagation records.

A **relationship** in `kg_relationships` carries `source_entity_id`, `target_entity_id`, `relationship_type`, and the same confidence and lifecycle columns. Edges have their own `last_seen_at` — they're only refreshed when the relationship actually shows up in a new conversation.

**Confidence decays.** Background pass applies exponential decay based on `last_seen_at` with a configurable half-life (default `entity_half_life_days = 90`, `relationship_half_life_days = 90`). Floored at `min_confidence = 0.01`. Rows touched within `skip_recent_hours = 24` are skipped. Orphan entities (no relationships, old `last_seen_at`) are flagged and archived. Archived rows are never deleted — they're just hidden from recall.

**Contradiction propagation (MEM-001 Part A).** Time-decay isn't the only confidence dial. When a `memory_facts.contradicted_by` is set, the entities and relationships extracted from the same source episode become suspect too. Each sleep cycle lists distinct `source_episode_id` values from contradicted facts within the lookback window (`lookback_hours = 24` by default), finds the kg_entities and kg_relationships whose `source_episode_ids` blob contains any of those episode ids (comma-flanked token match — no substring false positives), and applies `confidence = MAX(min_floor, confidence × decay_factor)`. Defaults: `decay_factor = 0.9`, `min_floor = 0.05`. The graph reacts to fact-level corrections immediately rather than waiting for the 90-day half-life to do the work.

**Graph-ANN recall surface (MEM-001 Part B-1).** The recall pipeline runs an ANN query over `kg_name_index` against the user-query embedding and surfaces the top entities as `ItemKind::GraphNode` ScoredItems. Hits below `min_kg_confidence = 0.1` are dropped (low-confidence noise — including entities decayed by Part A's contradiction propagation), and the score of surviving hits is multiplied by `kg_entities.confidence` so a 0.9-confidence entity at cosine 0.8 outranks a 0.4-confidence entity at the same cosine. The filter applies to the seed-id set fed to the hierarchy LCA surface too, so the topical map stays consistent with what recall shows.

**Relationship-hop traversal (MEM-001 Part B-2).** After the graph-ANN seeds are scored, recall walks `kg_relationships` outward from the top three seeds via a recursive CTE bounded by `max_hops = 2`. Edges whose `confidence < min_kg_confidence` are skipped at the CTE level — contradicted or decayed edges don't propagate noise to recall. Each reachable entity gets a `kg_traversal`-sourced ScoredItem with `score = hop_decay^hop × edge_confidence_product × entity_confidence`. Dedupe is shortest-hop-wins; ties go to the path with the highest edge-confidence product. Capped at `max_graph_facts = 5` additions per recall, agent-scoped, archival rows + edges excluded. The whole step is gated by `graph_traversal.enabled` (default `true`) — set to `false` to skip the traversal pass entirely.

### Bi-temporal model

Every fragment and relationship records a **truth-interval**, not a single timestamp:

- `valid_from` — when the fact became true **in the world**
- `valid_until` — when it stopped being true (`NULL` = still true)

Separate from `created_at` (when the row was written). The distinction matters when the world changes and the agent has to keep history straight.

**Worked example: employment history.** User says "I work at Anthropic" on 2026-01-15 → fact A written with `valid_from=2026-01-15, valid_until=NULL`. User says "I just started at OpenAI" on 2026-04-01 → fact B written. ConflictResolver picks B as the winner and updates fact A with `valid_until = B.created_at = 2026-04-01`. No gap, no overlap.

Months later the user asks: *"What was my role at Anthropic again?"* The recall pass runs with `as_of = some_date_in_february`. SQL filter:

```sql
WHERE valid_from <= ?as_of AND (valid_until IS NULL OR valid_until > ?as_of)
```

Fact A passes (`2026-01-15 ≤ 2026-02-15 < 2026-04-01`). Fact B doesn't. The agent answers from A. Without bi-temporal modeling this query would have either returned both facts (confusing) or just B (wrong).

### The Belief Network — all phases live

Facts are atomic. **Beliefs** are aggregates: synthesized stances about the same subject, built from one or more facts. The Belief Network ships in six phases, all of which are now live on the development branch.

#### Phase B-1 — Belief synthesis

A sleep-time worker (`BeliefSynthesizer`) groups facts by `(partition_id, subject)` and produces a single belief per group. Each belief captures `content` (one sentence), `confidence` (derived from constituent facts with 90-day recency weighting), `source_fact_ids` (queryable provenance), `valid_from` / `valid_until` (matching the underlying facts), and `reasoning` (only set when synthesis needed an LLM call).

**Single-fact short-circuit.** Real-data audit found 95%+ of subjects have only one fact backing them. For those, the belief is just a wrapper — `content = fact.content`, `confidence = fact.confidence × recency_weight`, zero LLM calls. Only multi-fact subjects invoke the synthesizer LLM. Across 709 facts in production, that's ~15 LLM calls per cycle, not 709.

Recency weight: `weight(t) = 1 / (1 + age_days(t) / 90)`. A 3-month-old fact contributes at half weight; a year-old fact at about 20%.

#### Phase B-2 — Belief contradictions

A second worker (`BeliefContradictionDetector`) examines pairs of beliefs within the same **topical neighborhood** (defined by subject prefix, default depth 1 — `user.dietary.vegetarian` and `user.dietary.beef` are both in the `user` neighborhood). For each unevaluated pair, an LLM judge returns one of:

| Decision | Effect |
|---|---|
| `logical_contradiction` | Row written to `kg_belief_contradictions` with `contradiction_type = "logical"` |
| `tension` | Row written with `contradiction_type = "tension"` (compatible facets, contextual conflict) |
| `compatible` | No row written; logged at debug |
| `duplicate` | No row written in B-2; logged at info (auto-merge is future work) |

The judge's reasoning is stored on the row (`judge_reasoning`) so a human can audit why a pair was classified that way. Budget cap: `contradictionBudgetPerCycle = 20` LLM calls per cycle, largest-neighborhood-first. Pairs that already have a row are skipped.

#### Phase B-3 — Confidence propagation

**Event-driven, not polling.** When a source fact loses confidence — superseded, contradicted, or decayed — beliefs built on it must respond. Two fact-lifecycle paths now call `BeliefPropagator::propagate_invalidation(fact_id, transition_time)` immediately:

| Trigger | When it fires |
|---|---|
| `ConflictResolver::supersede_fact` | Always — every supersession propagates |
| `DecayEngine::propagate_fact_confidence_drops` | When confidence crossed below `factConfidenceDropThreshold` (default 0.3), or a single-cycle drop exceeded that value |

The propagator queries beliefs whose `source_fact_ids` JSON array contains the invalidated fact:

| Belief shape | Action |
|---|---|
| Single-source (only this fact) | **Retract** — set `valid_until = transition_time`. Default recall stops returning it; `as_of` queries still surface it for pre-retraction timestamps. |
| Multi-source | **Mark stale** — set `stale = 1`. The next BeliefSynthesizer cycle re-derives from remaining valid sources and clears the flag. |

Cascade depth is capped at 3 hops; effective depth is 1 today since beliefs only reference facts, not other beliefs. Failure mode: errors log a warning and continue — supersession of a fact must never be blocked by belief bookkeeping.

#### Phase B-4 — Recall integration

Beliefs are searchable. When `BeliefSynthesizer` produces a belief, it embeds `belief.content` and writes the vector to `kg_beliefs.embedding` (added in schema v30). Embeddings live on the row, not in a separate vec0 table — belief count is bounded (current ~15 multi-fact subjects in production; even 100× that is small) and in-memory cosine is sub-millisecond.

The recall pipeline gains a step 5b that fetches beliefs in parallel with the existing fact/wiki/procedure/graph searches, filters superseded / past-interval / NULL-embedding rows, and returns top-K scored beliefs. They merge into the unified result set and compete in the same rescore step as everything else with category weight 1.5.

When beliefs surface, the gateway formatter groups them under `## Active Beliefs`, separate from `## Recalled Context`. The heading lives in `gateway-execution`, not `gateway-memory` — presentation belongs at the consumer layer.

#### Phase B-5 — /memory UI

The `/memory` tab in the desktop UI has three sub-tabs: **Facts** (the existing fragment browser), **Beliefs** (new — list, detail drawer, source-fact links, confidence badge), and **Contradictions** (new — list, two-belief preview, resolver drawer with "accept A / accept B / both true (tension) / merge" actions).

Filters: unresolved-only checkbox on the Contradictions tab; subject prefix filter on Beliefs. Empty/disabled states render explicit "enable in Settings → Advanced → Memory" copy when the Belief Network feature flag is off.

#### Phase B-6 — /observatory UI

The `/observatory` tab gained:

- **Belief Network totals** in the bottom status strip — total beliefs, total contradictions, unresolved-contradiction count with warning color.
- **Belief Network details slideover** (triggered by an "↗ details" button in the status strip) — three worker stat cards (BeliefSynthesizer, BeliefContradictionDetector, BeliefPropagator) showing last-run cycle metrics, an activity feed grouped by event type (synthesis / contradiction-found / propagation-fired), and a propagation chain visualizer.

The cards collapse into the status strip by default so the page stays clean. The detail surface stays available for power users.

### Hierarchical memory — opt-in topical layering

The Belief Network distills facts along the **subject** axis. Hierarchical memory does the same for entities along the **abstraction** axis: clusters of layer-N entities get summarised into a single layer-N+1 aggregate entity, recursively, until the cluster-sparsity score stops growing. Inspired by HiRAG's recursive aggregation and LeanRAG's "lean" inter-cluster relations.

Disabled by default. Flip `execution.memory.hierarchy.enabled = true` to turn it on. With the flag off, the rest of memory behaves byte-for-byte the same as before — no aggregates get written, no recall surface changes, no LLM is called.

#### Storage shape

The hierarchical layer lives **inside** `kg_entities` and `kg_relationships`, not in separate tables. Four schema-v31 columns carry the hierarchy:

| Column | Table | Meaning |
|---|---|---|
| `layer` | `kg_entities` | `0` for base entities. `>0` for aggregates synthesised by the builder. |
| `parent_cluster_id` | `kg_entities` | Soft FK back into `kg_entities`. Points at the layer-N+1 aggregate this entity was clustered into. `NULL` for top-of-hierarchy rows. |
| `layer` | `kg_relationships` | `0` for base-extracted edges. `>0` for inter-cluster edges synthesised between aggregates. |
| `is_inter_cluster` | `kg_relationships` | `1` when the edge was synthesised by the builder between two aggregates whose underlying connectivity exceeded the λ threshold. `0` for ordinary edges. |

Aggregate entities are full `kg_entities` rows — they have a `name` + `properties` JSON (with `{"aggregate": true, "description": "...", "member_count": N}`) + an embedding in `kg_name_index`. They obey the same decay, supersession, and bi-temporal lifecycle as every other entity, so a stale aggregate fades out like a stale base entity.

#### The builder — `HierarchyBuilder` sleep worker

A new sleep-cycle worker runs after the Compactor (so it doesn't cluster near-duplicate noise). Per agent, it loops:

```
layer = 0
loop while layer < max_layers (default 4):
    pool = entities with embeddings at this layer
    if pool < cluster_target_size (default 20): stop (PoolTooSmall)
    k = max(2, pool.len() / target_size)
    labels = kmeans_cosine(embeddings, k, seed) [K-means++ init, cosine metric]
    if labels collapse to one cluster: stop (SingleCluster)
    sparsity = cluster_sparsity(labels)
    if |sparsity - prev_sparsity| <= 0.05: stop (Converged)
    for each cluster:
        if single-member: promote without LLM (singleton short-circuit)
        else: LLM → {name, description}; embed description; write aggregate
              update each member's parent_cluster_id
    for each (cluster_i, cluster_j) pair:
        λ = connectivity_strength(i, j)  -- count of cross-cluster edges
        if λ > inter_cluster_relation_threshold (default 3):
            LLM → relation_type ("encompasses" / "differs-from" / ...)
            write kg_relationships row with is_inter_cluster=1
    layer += 1
```

Cost is bounded three ways:

1. **Singleton short-circuit.** Most clusters in real graphs are single-member because cluster sizes vary. Those promote directly with `name = member.name`, no LLM call. The optimisation is load-bearing — without it, a 1k-entity ward would burn ~50 LLM calls per layer.
2. **Per-cycle budget cap** (`llm_budget_per_cycle = 50`). Once exhausted, the cycle exits with `BudgetExhausted` and resumes next time.
3. **24-hour throttle** (`intervalHours = 24`). The cycle is idempotent — re-running before the throttle elapses is a no-op.

The LLM trait (`AggregateEntityLlm`) has two methods, mirroring `BeliefSynthesisLlm`:

- `synthesize_aggregate(members) → {name, description}`
- `synthesize_relation(agg_a, agg_b, λ) → relation_type`

Production wiring goes through `LlmAggregateEntity` (the gateway's `MemoryLlmFactory` thin wrapper). Both calls use temperature 0 so re-runs over the same clusters produce the same names.

#### The recall side — LCA-bounded topical map

At recall time, after the existing graph-ANN step returns its top-N seed entities (matched to the user's query via `kg_name_index` cosine), a new step 5c walks `parent_cluster_id` from each seed upward to find the **lowest common ancestor**:

```
seeds = top-N entities from graph ANN (already in the pipeline)
LCA = deepest entity present in every seed's ancestry chain
path = union of (seed → LCA) chains, deduplicated, seeds excluded
inter_cluster_edges = relationships(is_inter_cluster=1, layer ∈ path layers,
                                    both endpoints ∈ path)
```

Path entities surface as `ItemKind::HierEntity` items with content `[topic L{N}] {id}`. Inter-cluster relations surface as `ItemKind::HierRelation` items with content `[edge L{N}] {src} —[type]→ {tgt}`. Both go through the same RRF fusion + MMR rerank + min-score path as everything else, with `pattern`-slot category weight (`0.9` — conservative until empirical validation).

Three ways the LCA walk can degenerate, each handled cleanly:

- **No hierarchy built yet** — `parent_cluster_id` is `NULL` everywhere, no LCA, recall gets zero `HierEntity` items. The pipeline is byte-for-byte unchanged from pre-H-4.
- **Seeds share no common ancestor** — they belong to different parts of the hierarchy. `LcaPath { lca: None, path_entities: [], max_layer: 0 }`. Same behaviour as "no hierarchy".
- **Corrupt parent-pointer cycle** — `MAX_LCA_WALK = 16` cap bails out rather than looping.

The consumer formatter in `gateway-execution::recall::format_scored_items` renders the hierarchical items under their own headings (`## Topical Map` for `HierEntity`, etc.), separate from `## Recalled Context` and `## Active Beliefs`. The agent sees the abstraction chain distinct from the raw facts.

#### Settings — `execution.memory.hierarchy.*`

| Knob | Default | What it does |
|---|---|---|
| `enabled` | `false` | Master switch. When off, no sleep-cycle work, no recall surface change. |
| `intervalHours` | `24` | Builder cycle cadence. |
| `maxLayers` | `4` | Hard cap on layers built per cycle. |
| `clusterTargetSize` | `20` | K-means target — `k ≈ n / target`. |
| `interClusterRelationThreshold` | `3` | LeanRAG's λ > τ gate. Cluster pairs with fewer than this many underlying edges don't get an inter-cluster relation. |
| `llmBudgetPerCycle` | `50` | Per-cycle ceiling on aggregate + relation LLM calls. |

### Recall pipeline — end to end

```
1. Embed the query (degrades to keyword-only if no embedding client)
2. (Optional) Query gate (Self-RAG):
     decision = Skip | Direct(query) | Split([q1, q2, q3])
3. Hybrid fact search per (sub)query:
     - FTS5 BM25-ranked match against memory_facts_fts.content
     - sqlite-vec cosine against memory_facts_index
     - Blend via reciprocal rank fusion (vector_weight=0.7, bm25_weight=0.3)
4. Wiki search
5. Procedural pattern search
5a. Graph traversal expansion (max_hops=2, hop_decay=0.6, cap=5)
5b. (If Belief Network enabled) belief_store.search_beliefs()
5c. (If Hierarchical memory enabled) compute_lca_path over top seed entities,
    plus inter-cluster relations along the LCA path
6. Merge candidates into unified result set
7. Rescore with category weights, contradiction penalty, supersession penalty,
   ward affinity boost
8. (Optional) MMR diversity rerank (lambda=0.6, candidate_pool=30)
9. (Optional) Cross-encoder rerank (fastembed-rs, BGE-reranker-base by default)
10. (Optional) Intent router applies per-intent category-weight profile
11. Min-score filter (min_score=0.3), sort by final score, truncate to max_facts=10
12. Bi-temporal point-in-time filter (valid_from ≤ as_of AND valid_until > as_of)
```

#### The query gate (Self-RAG)

When `execution.memory.queryGate.enabled = true`, an LLM call before hybrid search classifies the input:

| Decision | What recall does |
|---|---|
| `Skip` | Small talk or self-contained. Skip hybrid search. Corrections still always-inject. |
| `Direct(query)` | Clean single-topic question. Use `query` as the retrieval query. |
| `Split([q1, q2, q3])` | Multi-topic input. Run hybrid search per subquery and dedup-merge by `fact_id`. |

Failure-safe: any LLM error, timeout, or malformed JSON falls back to `Direct(raw_input)`. Adds ~200-800ms when enabled.

#### Rescore steps

Run in order on every candidate:

1. **Category weight multiplier** — see table above.
2. **Contradiction penalty** — if `contradicted_by` is set, multiply by 0.7.
3. **Supersession penalty (class-aware)** — if `superseded_by` is set: `current` → 0.1×, `archival` → 0.3×, `convention`/`procedural` → no penalty, unknown → 0.3×.
4. **Ward affinity boost** — matching ward gets ×1.3.
5. **Supersession filter** — fragments with `superseded_by` set are dropped entirely.
6. **Min-score threshold** — drop anything below `min_score = 0.3`.
7. **Sort by final score, truncate** — `max_facts = 10` rows survive; `max_recall_tokens = 3000` cap on formatted block size.

### MMR diversity rerank — shipped

Maximal Marginal Relevance reranks the candidate pool to balance relevance against redundancy: `score(d) = λ × relevance(d) − (1−λ) × max_similarity(d, already_selected)`. Defaults: `lambda = 0.6`, `candidate_pool = 30`. Enabled by default. Where this helps: when a belief and one of its source facts both surface, MMR can demote the duplicate; when three near-identical facts about the same subject all match, MMR keeps the top one and bumps a different topic into the top-K.

### Cross-encoder rerank — opt-in

`execution.memory.rerank.enabled = true` loads a local cross-encoder model via `fastembed-rs` (default `BAAI/bge-reranker-base`, ~280MB). Higher quality top-K than MMR alone; adds latency and disk. The reranker runs after MMR — MMR diversifies, cross-encoder rescores the diversified set.

### Intent router — opt-in

`execution.memory.intentRouter.enabled = true` activates a kNN-based intent classifier that picks per-intent category-weight profiles. The classifier is seeded by `assets/intent_exemplars.json` (default exemplar bank covering coding, scheduling, factual lookup, personal-info, etc.) and adjustable from settings. `k = 5` nearest neighbours, `confidence_threshold = 0.55` to apply a profile. When disabled, default category weights are used universally.

### Sleep cycle workers

The sleep cycle runs every `cycle_interval_hours = 1` by default. Each worker is independent and self-throttles:

| Worker | Purpose | Throttle |
|---|---|---|
| `Compactor` | Find near-duplicate KG entities, LLM-judge merges | per-cycle budget |
| `Synthesizer` | Cross-session entity co-occurrence → `strategy` fragments | per-cycle |
| `PatternExtractor` | Recurring tool-call sequences → procedural `pattern` fragments | per-cycle |
| `CorrectionsAbstractor` | 3+ thematically similar corrections → `schema` fragment | `corrections_abstractor_interval_hours = 24` |
| `ConflictResolver` | LLM-judge contradicting `schema` pairs (cosine ≥ 0.85); set `superseded_by` + `valid_until` on loser | `conflict_resolver_interval_hours = 24` |
| `DecayEngine` | Exponential decay of entity/relationship `confidence` based on `last_seen_at`; plus **contradiction propagation** (MEM-001 Part A) — multiplicative `× decay_factor` on kg_entities and kg_relationships sharing episodes with newly-contradicted facts | hourly |
| `OrphanArchiver` | Entities with no relationships + old `last_seen_at` → `epistemic_class = 'archival'` | hourly |
| `Pruner` | Soft-delete entities flagged by decay below threshold | hourly |
| `BeliefSynthesizer` | Group facts by subject → belief rows (single-fact short-circuit, else LLM) | `beliefNetwork.intervalHours = 24` |
| `BeliefContradictionDetector` | LLM-judge belief pairs within topical neighborhoods | `intervalHours = 24`, `contradictionBudgetPerCycle = 20` |
| `BeliefPropagator` | Retract or mark-stale beliefs when source facts lose confidence | **event-driven** (not on cycle) |
| `HierarchyBuilder` | K-means-cluster entities, LLM-synthesise layer-N+1 aggregates, write inter-cluster relations gated by λ > τ | `hierarchy.intervalHours = 24`, opt-in |
| `Verifier` | Sanity-check fragment/entity links; flag orphan provenance | hourly |
| `BeliefNetworkActivity` | Persist per-cycle metrics for the /observatory activity feed | every belief cycle |

LLM-using jobs each have their own throttle so the hourly cycle doesn't burn tokens on jobs without new material.

### HTTP API surface

| Endpoint | Method | Purpose |
|---|---|---|
| `/api/beliefs/{agent_id}` | GET | List beliefs (paginated, optional `partition_id` filter) |
| `/api/beliefs/{agent_id}/{belief_id}` | GET | Belief detail with source facts |
| `/api/contradictions/{agent_id}` | GET | List contradictions (paginated, optional `partition_id` filter) |
| `/api/contradictions/{contradiction_id}/resolve` | POST | Resolve with `accept_a` / `accept_b` / `tension` / `merge` |
| `/api/belief-network/stats` | GET | Worker stats + totals |
| `/api/belief-network/activity` | GET | Activity feed (last N cycles) |
| `/api/distillation/status` | GET | Session-distillation counters (for the observatory status strip) |

All return `503` when the Belief Network is disabled at settings level. The UI surfaces this as "enable in Settings → Advanced → Memory" copy instead of a generic error.

---

## 4. Remaining details

### Configurability

Two files own every knob. Both are optional — missing keys, missing files, and corrupted files all fall through to compiled defaults.

**`settings.json` → `execution.memory`** controls background-job throttles and opt-in recall-pipeline features:

```json
{
  "execution": {
    "memory": {
      "correctionsAbstractorIntervalHours": 24,
      "conflictResolverIntervalHours": 24,

      "queryGate": {
        "enabled": false,
        "maxSubqueries": 4,
        "maxSubqueryLen": 200,
        "timeoutMs": 3000
      },

      "beliefNetwork": {
        "enabled": false,
        "intervalHours": 24,
        "neighborhoodPrefixDepth": 1,
        "contradictionBudgetPerCycle": 20,
        "factConfidenceDropThreshold": 0.3
      },

      "mmr": {
        "enabled": true,
        "lambda": 0.6,
        "candidate_pool": 30
      },

      "rerank": {
        "enabled": false,
        "model_id": "BAAI/bge-reranker-base",
        "candidate_pool": 20,
        "top_k_after": 10,
        "score_threshold": 0.0
      },

      "intentRouter": {
        "enabled": false,
        "k": 5,
        "confidence_threshold": 0.55
      },

      "hierarchy": {
        "enabled": false,
        "intervalHours": 24,
        "maxLayers": 4,
        "clusterTargetSize": 20,
        "interClusterRelationThreshold": 3,
        "llmBudgetPerCycle": 50
      },

      "kgDecay": null
    }
  }
}
```

| Block | Default | What it does |
|---|---|---|
| `correctionsAbstractorIntervalHours` | `24` | Throttle for `CorrectionsAbstractor` (3+ similar corrections → `schema`). `0` = every cycle. |
| `conflictResolverIntervalHours` | `24` | Throttle for `ConflictResolver` (LLM-judge contradicting schemas). `0` = every cycle. |
| `queryGate` | disabled | Self-RAG retrieval gate. Failure-safe. ~200-800ms when enabled. |
| `beliefNetwork` | disabled | B-1 synthesis + B-2 contradictions + B-3 propagation + B-4 recall surfacing. Queryable via `memory(action="belief"\|"contradictions")`. |
| `mmr` | **enabled** | Diversity rerank. `lambda=0.6` balances relevance vs diversity. |
| `rerank` | disabled | Cross-encoder rerank via `fastembed-rs`. ~280MB model download on first enable. |
| `intentRouter` | disabled | kNN intent classifier picking per-intent category-weight profiles. |
| `hierarchy` | disabled | H-3 builder + H-4 LCA recall. K-means clusters entities, LLM synthesises aggregate entities at higher layers, inter-cluster relations gated by λ > τ. Recall surfaces the LCA topical map alongside facts. |
| `kgDecay` | `null` | KG decay tuning override; `null` = compiled defaults. |

**`config/recall_config.json`** controls the recall and rescore pipeline. Headline knobs:

| Knob | Default | What it controls |
|---|---|---|
| `category_weights` | (table above) | Rescore step 1 multipliers |
| `min_score` | `0.3` | Noise floor |
| `contradiction_penalty` | `0.7` | Rescore step 2 |
| `ward_affinity_boost` | `1.3` | Matching-ward boost |
| `vector_weight` / `bm25_weight` | `0.7` / `0.3` | RRF blend |
| `max_facts` | `10` | Recall top-K |
| `max_recall_tokens` | `3000` | Hard ceiling on recall block size |
| `graph_traversal.max_hops` | `2` | Expansion depth |
| `graph_traversal.hop_decay` | `0.6` | Per-hop relevance decay |
| `graph_traversal.max_graph_facts` | `5` | Cap on graph-expansion additions |
| `kg_decay.entity_half_life_days` | `90` | Entity confidence half-life |
| `kg_decay.relationship_half_life_days` | `90` | Relationship confidence half-life |
| `kg_decay.min_confidence` | `0.01` | Confidence floor |
| `kg_decay.skip_recent_hours` | `24` | Skip rows touched recently |
| `mid_session_recall.every_n_turns` | `5` | Mid-session recall cadence |
| `mid_session_recall.min_novelty_score` | `0.3` | Score floor for mid-session injection |

A missing file falls back to compiled defaults. A partial file deep-merges with defaults — user values win per key, missing keys keep their default. A corrupted file logs a warning and uses defaults.

### Failure modes and graceful degradation

The subsystem is designed so that any single subsystem can fail without blocking the rest:

- **No embedding client wired** → recall degrades to keyword-only (FTS5 BM25). No crashes.
- **Embedding client transient failure on belief synthesis** → belief is written with `embedding = NULL`. Visible to exact-lookup queries, invisible to semantic recall. Logged for observability.
- **Query gate LLM error / timeout / malformed JSON** → fall back to `Direct(raw_input)`. System always retrieves something.
- **BeliefPropagator error** → log warning, continue. Fact supersession never blocked by belief bookkeeping.
- **Contradiction detector budget exhausted mid-cycle** → exit cleanly, resume next cycle. No re-evaluation of pairs already classified.
- **Sleep worker crash** → isolated to that worker. Other workers continue their cycle. Logged with backtrace.
- **Migration failure on startup** → abort startup loudly. Half-migrated DB is worse than no startup.

### Closed-loop walkthrough

One concrete trace, following a correction through every stage:

1. **The correction lands.** User says *"use sentence case in commit titles."* A fragment is written to `memory_facts`:
   ```
   key: correction.5b2a
   category: correction
   content: "use sentence case in commit titles"
   confidence: 0.85
   ```
   The embedding is inserted into `memory_facts_index` keyed by `fact_id`. FTS5 triggers index the content.

2. **The graph catches it.** Entities ("commit titles," "git workflow") are extracted into `kg_entities`. Relationships are written. `source_episode_ids` link both fragment and entities to the conversation.

3. **Next session opens.** Bootstrap reads in order: handoff → goals → corrections → handoff-recall → user-recall. The new correction shows up in `## Active Corrections` unconditionally.

4. **Mid-session match.** Hours later, the user asks *"can you write the commit message for this?"* Hybrid search returns `correction.5b2a` near the top. RRF blends FTS5 + vector. Category weight 1.5 applied. Above `min_score = 0.3`. Top-K includes it.

5. **The correction accumulates.** Over weeks, three more corrections about commit hygiene arrive. The CorrectionsAbstractor cycle (24-hour throttle) sees four corrections share a theme, asks an LLM to distill, gets back:
   ```
   key: schema.corrections.a3f8b2c0
   category: schema
   content: "Always use sentence case in commit titles and keep subject lines under 72 characters."
   confidence: 0.92
   ```
   The raw corrections aren't deleted. The schema (weight 1.6) outranks them (weight 1.5).

6. **A conflict appears.** Months later, a different schema is written — *"Use Title Case for commit titles."* The ConflictResolver finds the pair by embedding cosine ≥ 0.85, LLM-judges, picks the winner. The loser gets `superseded_by = winner.id` AND `valid_until = winner.created_at`. Default recall filters it out; `as_of` queries pre-transition still see it.

7. **The belief layer reflects it.** BeliefSynthesizer next cycle sees facts with `subject = "git.commit.title_case"`. Single-fact short-circuit: belief written with the winner's content as-is. Confidence inherited from the source fact's current decayed value. When the loser was superseded, BeliefPropagator immediately marked any belief sourcing only from the loser as `valid_until = now` (retracted) or `stale = 1` (multi-source, re-synthesised next cycle).

The reference book gets smaller, sharper, and more consistent with each cycle. Raw feedback becomes principles; principles get arbitrated against each other; stale principles disappear from current recall without losing their place in history.

### What's still loose

| Area | Status |
|---|---|
| Belief category weight at 1.5 | Conservative until validation corpus measures recall quality. May rise to 1.7-1.8 if beliefs prove reliable. |
| Edge-weighted graph traversal | Today every edge counts equally regardless of confidence. Schema and decay machinery are already in place. |
| Auto-merge of `duplicate` belief pairs (B-2) | Detected but not yet merged. Future phase will canonicalise subject names. |
| Belief-derived-from-belief | Schema allows cascade depth 3 but effective depth is 1 today; reserved for future synthesis topology. |
| Cross-ward belief routing | Beliefs are partition-scoped. A user fact ("vegetarian") could plausibly be ward-global; needs design. |
| Hierarchical KG (HiRAG / LeanRAG) | Planned. See `docs/architecture/future-state/` once design lands. |
| Validation corpus | 10-run validation corpus across 4 tiers exists (`docs/memory-validation.md`). Continuous expansion would let weights be tuned empirically rather than heuristically. |

### Where to look next

- **Design doc** — `docs/architecture/future-state/2026-05-15-belief-network-design.md` (Belief Network B-1 through B-6)
- **Tracking doc** — `docs/architecture/future-state/2026-05-13-memory-crate-extraction-tracking.md` (running log of memory-subsystem changes; paired with this file)
- **Slide deck** — `docs/memory-slides.html` (the visual companion to this doc)
- **Validation corpus** — `docs/memory-validation.md` (recall test scenarios across 4 tiers)
- **Source** — `gateway/gateway-memory/src/` (memory subsystem), `stores/zbot-stores-sqlite/src/knowledge_schema.rs` (schema), `gateway/src/http/beliefs.rs` (HTTP surface), `apps/ui/src/features/memory/` and `apps/ui/src/features/observatory/` (UI)

### Recap

- **Two views of memory** — flat fragments hold content; the knowledge graph holds shape. Both have confidence and a lifecycle. Beliefs sit a layer above, synthesised from fact groups.
- **The agent reads in order** — handoff → goals → corrections → beliefs → handoff-recall → user-recall. Mid-session, recall fires every 5 turns.
- **The agent writes via three paths** — implicit (distillation), explicit (`memory` tool), or human-curated (UI).
- **Recall is hybrid** — FTS5 + sqlite-vec cosine, RRF-blended, rescored by category/contradiction/supersession/ward, optionally gated/MMR'd/reranked.
- **The sleep cycle keeps the reference book honest** — compaction, synthesis, abstraction, conflict resolution, decay, belief synthesis, contradiction detection, propagation.
- **Bi-temporal model** — every row records when it became true and when it stopped, separately from when it was written. Point-in-time recall falls out for free.

The daily log records everything. The reference book is what the agent reads.
