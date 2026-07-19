# Plan: Embedding-Backed Memory Recall

- **Spec:** [`spec.md`](spec.md)
- **Status:** Done

> **Plan contract:** this is the implementation strategy. Unlike the spec, this
> document is allowed to change as you learn. When it changes substantially
> (a different approach, not just a re-ordering), note why in the changelog at
> the bottom.

## Approach

Fix the failing path at the boundary where explicit `memory(action="recall")`
and runtime recall reach `MemoryFactStore`: make normal hybrid recall receive a
query embedding from the already-wired live provider, make the Engram sidecar
refuse broad lexical-only hybrid results, and replace substring ranking with a
small deterministic sparse scorer plus RRF fusion. Keep graph-aware recall in
the recall/context layer, not in the memory fact sidecar. The first code slice
should be small enough to validate with adapter and gateway-memory tests before
broader context-graph recall work lands.

## Constraints

- [`engram-memory-engine-cutover`](../engram-memory-engine-cutover/spec.md):
  Engram is the semantic memory provider, but zbot keeps gateway/UI/settings
  and sleep-cycle contracts.
- [`context-capability-registry`](../context-capability-registry/spec.md):
  unified context packets may include memory, graph, skills, and tool evidence
  with actor-aware provenance.
- [`memory-hygiene`](../memory-hygiene/spec.md): older lexical fallback remains
  acceptable only for explicit lexical/FTS searches and exact-key degraded
  recall; this spec supersedes broad fuzzy lexical fallback for normal hybrid
  recall.
- Existing dirty worktree contains unrelated changes. Implementation must avoid
  reverting or reshaping those changes.
- No new dependency unless the user approves it; implement BM25/IDF and RRF
  locally unless a later review proves a dependency is worth the cost.

Tempted to add a new recall service layer; declining for this slice because the
existing trait and `MemoryRecall` boundary can carry the fix. The original
first-slice assumption kept every trait method unchanged; adversarial review
found that identity safety across facts, wiki, procedures, episodes, graph,
beliefs, and strategy synthesis requires additive identity-aware trait methods.
Those methods have safe defaults and do not change existing method signatures or
public REST/WebSocket/UI payloads. Tempted to expose ranking knobs in settings;
declining until behavior is correct and tested. Tempted to move graph traversal
into the Engram adapter; declining because graph ownership belongs to
recall/context coordination.

## Construction tests

**Integration tests:**

- `cargo test -p zbot-engram-adapter --locked memory_facts`
- `cargo test -p gateway-memory --locked recall`
- `cargo test -p gateway-execution --locked memory`
- `cargo check -p gateway --locked`

**Manual verification:**

- After code gates pass, rerun the bad-session reproduction query against a
  local test DB or synthetic fixture and confirm the AMD methodology fact no
  longer outranks the arXiv paper-review query.

## Design (LLD)

### Design decisions

- Normal hybrid recall must be embedding-backed. If no query embedding is
  available, the implementation can use exact-key/entity matches but cannot
  return broad fuzzy lexical results. Traces to: AC1, AC2, AC5.
- Sparse relevance should be deterministic and local. BM25/IDF or an equivalent
  scorer is enough for this slice; a cross-encoder reranker is explicitly out
  of scope. Traces to: AC3, AC4.
- Fusion should operate on ranks, not raw score scales. RRF avoids calibrating
  cosine scores against sparse scores. Traces to: AC4.
- Memory and graph stay separate stores. Unified recall can merge their
  evidence, but storage adapters do not traverse each other. Traces to: AC7.

### Data & schema

- No user data migration is required for the first slice.
- Existing embedding sidecar JSON remains the source of stored fact vectors.
- Stored vector compatibility is checked against provider/model/dimension
  metadata where available. Missing metadata is treated as a blocker for vector
  search until reindex or compatibility is proven.
- Regression fixtures use synthetic facts only; no current private DB rows are
  committed.

### Interfaces & contracts

- Agent-facing `memory(action="recall")` arguments and result envelope stay
  stable.
- `MemoryFactStore::search_memory_facts_hybrid` keeps its current signature in
- Additive identity-aware store methods are allowed for internal backend safety,
  including memory facts, wiki, procedures, episodes, knowledge graph, beliefs,
  and strategy synthesis. Existing method signatures remain callable and
  backends that do not override the identity-aware variants retain safe default
  behavior.
- A new constructor or builder may be added for Engram memory stores so the
  composition root can pass the live embedding client without changing the
  trait.
- Recalled evidence injected into prompt/context packets is marked as
  untrusted reference material. It can inform the answer, but it cannot grant
  tool authority or override system/developer/current-user instructions.

### Component / module decomposition

- `runtime/agent-tools/src/tools/memory.rs`: explicit memory recall caller.
- `gateway/gateway-memory/src/recall/mod.rs`: runtime unified recall caller and
  embedding behavior.
- `gateway/src/state/persistence_factory.rs`: Engram store construction and
  live embedding-client injection.
- `stores/zbot-engram-adapter/src/stores/memory_facts.rs`: sidecar sparse
  scoring, vector scoring, weak-match gates, and provenance.
- `stores/zbot-engram-adapter/tests/memory_facts.rs`: synthetic failure-pattern
  and degraded-mode coverage.

### State & control flow

1. Runtime constructs a live embedding client from FastEmbed or Ollama config.
2. The composition root passes that client to recall services and the Engram
   memory fact store.
3. `memory(action="recall")` calls `recall_facts_prioritized`.
4. The Engram store embeds the query, validates provider/vector compatibility,
   runs dense and sparse candidate generation, fuses results, and applies
   weak-match gates.
5. If embedding is unavailable or invalid, the store returns exact
   identifier/key degraded results or no fuzzy results with a reason.
6. Unified recall can additionally merge knowledge graph/context evidence above
   the storage layer.

### Behavior & rules

- `mode = "semantic"` requires a query embedding.
- `mode = "hybrid"` requires a query embedding for fuzzy results; exact-key or
  exact-identifier lexical matches may still return with degraded provenance.
- `mode = "fts"` remains explicitly lexical for UI/admin search, but it must
  use sparse relevance scoring and generic-term suppression.
- Scope boost is not a relevance bypass: scoped facts can outrank global facts
  only after they pass relevance gates.
- Category is not a relevance bypass: degraded category recall requires an
  explicit structured caller filter, not category inference from generic tokens.
- Recency is a final tie-breaker only.

### Failure, edge cases & resilience

- Embedding provider errors are logged with provider identity and a degraded
  reason, not raw query vectors.
- Dimension mismatch returns no vector candidates and records a named blocker.
- Empty queries return no fuzzy results.
- Generic-only queries return no fuzzy results unless an exact key, exact
  identifier, or explicit structured filter matches.
- Incompatible stored vectors are skipped; they do not poison the whole recall.

### Quality attributes (NFRs)

- Safety: no broad lexical fallback in normal hybrid recall.
- Performance: candidate scoring remains bounded by requested limit and an
  internal fetch cap; no full graph traversal inside the memory fact store.
- Operability: recall logs/traces expose degraded-mode reasons and match source.
- Maintainability: ranking helpers stay local and testable without a new
  dependency.

### Dependencies & integration

- Reuse `agent_runtime::llm::embedding::EmbeddingClient`.
- Reuse the gateway live embedding client so FastEmbed/Ollama hot-swaps are
  honored.
- No additional crate dependency in the first slice.

## Tasks

### T1: Spec and pre-execute review are clean

**Depends on:** none

**Touches:** `docs/specs/embedding-backed-memory-recall/*`, `docs/specs/README.md`

**Tests:**

- Goal-based check: adversarial spec review returns `Clean - ready to commit.`
  or every finding is applied before EXECUTE. Verifies plan readiness.
- Goal-based check: security spec-stage review for LLM-agent and degraded-mode
  boundaries returns clean or findings are applied before EXECUTE. Verifies AC2,
  AC9.
- Goal-based check: `python .codex/skills/work-loop/scripts/loop-cohort.py check docs/specs/embedding-backed-memory-recall --phase plan` passes after approval.

**Approach:**

- Draft the spec and plan from the approved contract.
- Initialize loop state.
- Run pre-execute adversarial and security review.
- Apply review findings before code edits.

**Done when:** the spec directory is review-clean and the work-loop plan gate is
approved.

### T2: Engram memory recall receives live query embeddings

**Depends on:** T1

**Touches:** `gateway/src/state/persistence_factory.rs`,
`gateway/src/state/mod.rs`, `stores/zbot-engram-adapter/src/stores/memory_facts.rs`,
`stores/zbot-engram-adapter/tests/memory_facts.rs`

**Tests:**

- TDD: `recall_facts_prioritized` on an Engram memory store without an embedder
  returns exact-only/empty degraded recall with a structured reason instead of
  broad fuzzy lexical results. Verifies AC2, AC9.
  artifact: `stores/zbot-engram-adapter/tests/memory_facts.rs::recall_prioritized_without_embedder_returns_structured_degraded_reason`;
  command: `cargo test -p zbot-engram-adapter --locked recall_prioritized_without_embedder_returns_structured_degraded_reason`;
  implemented: true.
- TDD: an embedding provider error causes degraded exact-only or empty results,
  not broad fuzzy lexical results. Verifies AC2.
  artifact: `stores/zbot-engram-adapter/tests/memory_facts.rs::recall_embedding_error_degrades_without_fuzzy_lexical_results`;
  command: `cargo test -p zbot-engram-adapter --locked recall_embedding_error_degrades_without_fuzzy_lexical_results`;
  implemented: true.
- TDD: gateway composition passes the live embedding client into the Engram
  memory store used by `memory(action="recall")`. Verifies AC1.
  artifact: `gateway/src/state/persistence_factory.rs::tests::engram_bundle_passes_live_embedding_client_to_memory_store`;
  command: `cargo test -p gateway --locked engram_bundle_passes_live_embedding_client_to_memory_store`;
  implemented: true.
- TDD: unified recall embeds the actual query before calling
  `search_memory_facts_hybrid`. Verifies AC1.
  artifact: `gateway/gateway-memory/src/recall/mod.rs::tests::run_hybrid_search_embeds_query_before_store_search`;
  command: `cargo test -p gateway-memory --locked run_hybrid_search_embeds_query_before_store_search`;
  implemented: true.
- TDD: the memory tool path reaches an embedding-backed fact store and does not
  bypass it with KV fallback when the store exists. Verifies AC1.
  artifact: `runtime/agent-tools/src/tools/memory.rs::tests::memory_tool_recall_uses_embedding_backed_fact_store`;
  command: `cargo test -p agent-tools --locked memory_tool_recall_uses_embedding_backed_fact_store`;
  implemented: true.
- Goal-based check: `cargo test -p zbot-engram-adapter --locked memory_facts`.

**Approach:**

- Add an Engram memory-store constructor that accepts
  `Arc<dyn EmbeddingClient>`.
- Pass the live embedding client from gateway composition into the Engram store.
- Keep existing constructor paths for tests and migration helpers, but make
  fuzzy hybrid recall fail closed when no embedder is configured.

**Done when:** explicit memory-tool recall in Engram mode has an embedding-backed
path and no broad fuzzy fallback when embedding is unavailable.

### T3: Sparse scoring and RRF fusion replace substring hybrid ranking

**Depends on:** T2

**Touches:** `stores/zbot-engram-adapter/src/stores/memory_facts.rs`,
`stores/zbot-engram-adapter/tests/memory_facts.rs`

**Tests:**

- TDD: generic token overlap alone does not return unrelated facts in hybrid
  mode. Verifies AC3, AC5.
  artifact: `stores/zbot-engram-adapter/tests/memory_facts.rs::generic_academic_tokens_do_not_return_unrelated_domain_fact`;
  command: `cargo test -p zbot-engram-adapter --locked generic_academic_tokens_do_not_return_unrelated_domain_fact`;
  implemented: true.
- TDD: generic token overlap plus same ward/category still does not pass without
  exact entity/key/identifier or semantic-threshold evidence. Verifies AC5.
  artifact: `stores/zbot-engram-adapter/tests/memory_facts.rs::scope_and_category_do_not_bypass_content_relevance_gate`;
  command: `cargo test -p zbot-engram-adapter --locked scope_and_category_do_not_bypass_content_relevance_gate`;
  implemented: true.
- TDD: dense semantic hit plus specific lexical hit fuses ahead of a newer
  generic lexical hit. Verifies AC4, AC8.
  artifact: `stores/zbot-engram-adapter/tests/memory_facts.rs::rrf_fuses_dense_and_specific_sparse_ahead_of_newer_generic_hit`;
  command: `cargo test -p zbot-engram-adapter --locked rrf_fuses_dense_and_specific_sparse_ahead_of_newer_generic_hit`;
  implemented: true.
- TDD: exact identifiers such as `2602.03315` or exact keys remain recallable
  in degraded mode. Verifies AC2, AC5.
  artifact: `stores/zbot-engram-adapter/tests/memory_facts.rs::degraded_recall_allows_exact_identifier_not_generic_category_inference`;
  command: `cargo test -p zbot-engram-adapter --locked degraded_recall_allows_exact_identifier_not_generic_category_inference`;
  implemented: true.
- TDD: mismatched provider, model, dimension, prompt profile, normalization, or
  missing metadata skips vector candidates or returns a named reindex blocker.
  Verifies AC6.
  artifact: `stores/zbot-engram-adapter/tests/memory_facts.rs::embedding_identity_mismatch_skips_vectors_with_reindex_blocker`;
  command: `cargo test -p zbot-engram-adapter --locked embedding_identity_mismatch_skips_vectors_with_reindex_blocker`;
  implemented: true.
- TDD: live embedding model drift at the same dimension degrades before vector
  search. Verifies AC6.
  artifact: `stores/zbot-engram-adapter/tests/memory_facts.rs::live_embedding_identity_mismatch_degrades_before_vector_search`;
  command: `cargo test -p zbot-engram-adapter --locked live_embedding_identity_mismatch_degrades_before_vector_search`;
  implemented: true.
- Goal-based check: `cargo test -p zbot-engram-adapter --locked memory_facts`.

**Approach:**

- Tokenize into specific and generic terms.
- Build sparse ranks from IDF-weighted exact token/entity matches.
- Build dense ranks from cosine similarity when embedding dimensions match.
- Fuse ranked lists using RRF and apply weak-match gates before recency
  tie-breaks.

**Done when:** the AMD/arXiv regression cannot reproduce through the Engram
memory fact store.

### T4: Unified recall preserves graph/context boundaries

**Depends on:** T3

**Touches:** `gateway/gateway-memory/src/recall/mod.rs`,
`gateway/gateway-memory/src/recall/*`, `gateway/gateway-execution/src/invoke/micro_recall.rs`

**Tests:**

- Goal-based integration: unified recall can include memory fact and graph/context
  evidence with provenance without requiring memory fact storage to traverse
  graph edges. Verifies AC7.
  artifact: `gateway/gateway-memory/src/recall/mod.rs::tests::unified_recall_keeps_graph_evidence_above_memory_store`;
  command: `cargo test -p gateway-memory --locked unified_recall_keeps_graph_evidence_above_memory_store`;
  implemented: true.
- TDD: no raw embeddings or private DB internals are serialized in recall trace
  items. Verifies AC9.
  artifact: `gateway/gateway-memory/src/recall/mod.rs::tests::recall_trace_redacts_embedding_and_db_internals`;
  command: `cargo test -p gateway-memory --locked recall_trace_redacts_embedding_and_db_internals`;
  implemented: true.
- TDD: recalled evidence injected into context is delimited as untrusted
  reference data and cannot carry executable instructions or tool authority.
  Verifies AC11.
  artifact: `gateway/gateway-execution/src/invoke/working_memory_middleware.rs::tests::recalled_evidence_is_rendered_as_untrusted_reference_data`;
  command: `cargo test -p gateway-execution --locked recalled_evidence_is_rendered_as_untrusted_reference_data`;
  implemented: true.
- Goal-based check: `cargo test -p gateway-memory --locked recall`.

**Approach:**

- Keep graph retrieval in recall adapters/context graph code.
- Normalize provenance labels across memory, graph, wiki, procedure, episode,
  belief, and context signals.
- Do not change the public memory tool response unless the spec is updated.

**Done when:** graph-aware recall is available through unified context assembly,
not hidden inside memory fact storage.

### T5: Runtime gates and regression coverage are green

**Depends on:** T2-T4

**Touches:** `gateway/gateway-execution/tests/*`, `gateway/tests/*`,
`docs/specs/embedding-backed-memory-recall/*`

**Tests:**

- Goal-based check: targeted gateway-execution tests around memory tool recall
  pass. Verifies AC1, AC2.
- Goal-based check: targeted gateway/gateway-memory tests around recall traces
  pass. Verifies AC7, AC9, AC10, AC11.
- Goal-based check: public gateway/UI/settings/events/Observatory surfaces are
  untouched unless this spec is updated. Command:
  `git diff --name-only -- apps/ui gateway/gateway-events gateway/gateway-ws-protocol gateway/src/http/settings.rs gateway/src/http/memory.rs gateway/src/http/memory_search.rs`.
  Expected output: no files from those protected surfaces, or an explicit spec
  amendment plus contract tests before merge. Verifies AC10.
- Goal-based check: existing public-route tests that already cover memory search
  and gateway settings remain green. Command:
  `cargo test -p gateway --test memory_unified_search --locked` and
  `cargo test -p gateway --test api_tests --locked`.
- Goal-based check: `cargo check -p gateway --locked` passes.

**Approach:**

- Add or update tests at the runtime boundary that caused the bad session.
- Run narrow crate gates first, then `cargo check -p gateway --locked`.
- Mark acceptance criteria complete only after matching tests pass.

**Done when:** all targeted gates pass and the spec ACs implemented in this
slice are checked.

## Rollout

- **Delivery:** normal code path, no feature flag. The change makes current
  hybrid recall stricter and safer.
- **Infrastructure:** no new infrastructure.
- **External-system integration:** depends on the already-configured FastEmbed
  or Ollama embedding backend.
- **Deployment sequencing:** land query embedding/fail-closed behavior before
  ranking changes so unsafe fallback is removed first.

## Risks

- Strict fail-closed behavior may return fewer memories for generic queries.
  This is intentional; exact-key and scoped queries should cover deliberate
  lookup, and context graph recall can add broader evidence later.
- Existing tests from `memory-hygiene` may expect lexical fallback. Update those
  tests only where they conflict with the new safer contract; preserve explicit
  `mode = "fts"` behavior.
- Provider hot-swap can make stored vectors incompatible with the current query
  vector. The implementation must validate dimensions and provider identity
  before vector search.
- Retrieved memory can contain stale or malicious instructions. Context
  injection must delimit recall as untrusted reference data and retain existing
  tool-confirmation rules.
- Broad worktree dirt can hide unrelated failures. Use targeted gates and report
  unrelated breakage separately.

## Changelog

- 2026-07-07: initial approved plan from the observed Engram recall pollution
  root cause and research-backed hybrid retrieval contract.
- 2026-07-07: tightened pre-execute review findings: no tool-result contract
  drift, exact-only degraded recall, scope/category as filters only, concrete
  test artifacts, provider-identity tests, public-surface gates, and untrusted
  recalled-evidence handling.
- 2026-07-07: shipped after post-gates adversarial/security fixes: normalized
  RRF-scale fact scores before runtime filtering, centralized untrusted recall
  prompt boundaries, validated live embedding identity, bounded agent recall
  query/limit/candidate work, and returned structured degraded reasons.
