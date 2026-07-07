# Spec: Embedding-Backed Memory Recall

- **Status:** Shipped
- **Owner:** phanijapps
- **Plan:** [`plan.md`](plan.md)
- **Constrained by:** [`engram-memory-engine-cutover`](../engram-memory-engine-cutover/spec.md); [`context-capability-registry`](../context-capability-registry/spec.md); partially supersedes the lexical-fallback behavior in [`memory-hygiene`](../memory-hygiene/spec.md)
- **Brief:** none
- **Contract:** Existing agent-facing `memory(action="recall")`, REST, WebSocket,
  UI, and Observatory contracts stay stable. Store traits may grow additive
  identity-aware internal methods so Engram-backed vector search can fail closed
  without changing existing method signatures or public payloads.
- **Shape:** mixed

> **Spec contract:** this document defines what "done" means. The implementing
> PR must match this spec, or update it. Verification must be derivable from it.

## Objective

Make zbot recall relevant memory, knowledge, and context evidence without
letting broad lexical matches pollute first-turn context. Success means normal
runtime recall is backed by the configured FastEmbed or Ollama embedding
provider, sparse lexical search is relevance-scored instead of substring-counted,
dense and sparse candidates are fused with explainable provenance, weak matches
fail closed, and the existing `memory(action="recall")` tool and UI contracts
stay stable.

## Boundaries

The three-tier guard that keeps an implementing agent inside the lines.
*Always do* applies without asking; *Ask first* requires human sign-off before
proceeding; *Never do* is a hard rule, even under time pressure.

### Always do

- Keep the agent-facing `memory(action="recall")` shape stable.
- Keep existing public store methods callable; additive identity-aware trait
  methods are allowed only for internal backend safety and must have safe
  defaults for existing implementers.
- Use the configured live embedding provider, FastEmbed or Ollama, for normal
  semantic and hybrid recall.
- Treat broad lexical-only hybrid recall as unsafe unless the caller explicitly
  requested an FTS/lexical mode.
- Return provenance for fused results, including whether an item came from
  memory facts, knowledge graph, wiki, procedures, episodes, beliefs, or context
  graph signals.
- Keep knowledge graph traversal in the graph/recall/context layer; do not make
  the memory fact sidecar secretly own graph expansion.
- Preserve existing gateway/UI/settings/events/Observatory payload shapes.
- Render recalled memory, graph, context, wiki, procedure, episode, and belief
  evidence as untrusted reference data when it is injected into model context.
- Redact raw embeddings, private transcript content, raw SQL internals,
  connector data, secrets, and absolute local DB provenance from diagnostics.

### Ask first

- Changing existing public `MemoryFactStore` method signatures or agent-facing
  memory tool arguments.
- Requiring a new embedding provider or adding a new ranking dependency.
- Deleting current memory records, reindexing user data destructively, or
  requiring a fresh DB to adopt the fix.
- Moving graph ownership into Engram or moving zbot-specific ontology/taxonomy
  policy out of zbot.
- Changing REST, WebSocket, UI DTO, or prompt-template contracts to expose the
  new ranking internals.

### Never do

- Never let `mode = "hybrid"` silently degrade to broad substring-only recall
  when no query embedding is available.
- Never let generic words such as `research`, `analysis`, `methodology`, or
  `review` dominate ranking without a specific entity, identifier, exact key, or
  high semantic score.
- Never use recency as a primary relevance signal; it is only a tie-breaker
  after relevance and scope gates pass.
- Never mix stored embeddings from different provider/model/dimension identities
  without an explicit compatibility check or reindex requirement.
- Never let global facts outrank scoped session/ward facts solely because they
  are newer.
- Never let recalled evidence grant tool authority, override system/developer
  instructions, override the current user request, or bypass existing
  confirmation policy for side effects.
- Never leak raw embeddings, private transcripts, absolute DB paths, SQL
  internals, connector/vault payloads, or secrets in logs, API errors, recall
  traces, or committed diagnostics.

## Testing Strategy

- Query embedding plumbing: **TDD** because the invariant is compact. Normal
  hybrid recall must either receive a query embedding or enter an exact-only
  degraded mode.
- Sparse scorer and weak-match gate: **TDD** with controlled fact fixtures. The
  tests must prove generic token overlap does not outrank specific semantic or
  identifier matches.
- Dense+sparse fusion: **TDD** over deterministic candidate lists. RRF ordering,
  source provenance, scope boosts, and recency tie-breaks can be checked without
  live providers.
- Provider/model/dimension compatibility: **TDD plus goal-based integration**.
  Metadata mismatch must fail closed before mixed vectors are searched.
- Graph-aware recall coordination: **goal-based integration**. The memory store
  may return memory facts; the recall/context layer may fuse graph evidence with
  provenance, but graph traversal must not move into the fact sidecar.
- Regression gates: **goal-based check**. Targeted cargo tests for
  `zbot-engram-adapter`, `gateway-memory`, and `gateway-execution` must pass,
  plus `cargo check` for touched crates.

## Acceptance Criteria

- [x] Explicit `memory(action="recall")` and runtime hybrid recall embed the
  recall query with the configured live embedding provider before invoking
  semantic or hybrid Engram-backed memory search.
- [x] If the embedding provider is unavailable, unconfigured, dimension-mismatched,
  or returns an error, normal hybrid recall does not run broad lexical fallback;
  it returns no fuzzy results or exact identifier/key matches only, with a
  structured degraded-mode reason. Category matches are allowed in degraded mode
  only when the caller supplied an explicit structured category filter; category
  inference from generic query tokens is not allowed.
- [x] Engram-backed memory search no longer ranks hybrid results by raw
  substring-hit count. Sparse lexical ranking uses a relevance scorer such as
  BM25/IDF or an equivalent deterministic approximation that downweights common
  terms and rewards exact identifiers/entities.
- [x] Dense semantic and sparse lexical candidate lists are fused by reciprocal
  rank fusion or an equivalent rank-based fusion method; recency is applied only
  after relevance, scope, and weak-match gates.
- [x] Generic academic/task words cannot return unrelated domain facts unless
  another high-specificity content signal is present, such as an exact entity,
  exact key, arXiv identifier, or semantic score above the configured cutoff.
  Ward/session scope and category are filters or tie-breakers only after a
  content relevance gate passes.
- [x] Stored embedding provider, model, dimensions, prompt profile, and
  normalization are validated before vector search; incompatible stored vectors
  are skipped or force a named reindex requirement instead of being searched.
- [x] Graph and context evidence can appear in unified recall bundles with
  provenance, but memory fact storage remains decoupled from graph traversal.
- [x] Regression coverage reproduces the observed failure pattern: a fresh
  `AMD valuation methodology analysis` fact must not outrank an
  `arXiv 2602.03315 academic paper critical review` recall query.
- [x] Internal recall traces and logs expose match source, ranking reason,
  degraded-mode reason, and provider identity without exposing raw embeddings,
  raw private transcript snippets, SQL internals, connector data, secrets, or
  absolute DB provenance. Agent-facing tool result shapes stay stable.
- [x] Existing gateway/UI/settings/events/Observatory contracts pass without
  public shape changes.
- [x] Recalled evidence injected into model context is delimited as untrusted
  reference data with provenance. It cannot override current instructions, grant
  tool authority, or justify side effects without independent current-user
  intent and the existing confirmation policy.

## Assumptions

- Technical: the runtime already constructs a live embedding client that can
  swap between internal FastEmbed and Ollama-backed providers (source:
  `gateway/src/state/mod.rs`; `gateway/gateway-services/src/embedding_service.rs`).
- Technical: the current Engram memory fact store receives `None` for query
  embeddings in `recall_facts_prioritized`, causing "hybrid" recall to behave
  as lexical-only substring scoring (source:
  `stores/zbot-engram-adapter/src/stores/memory_facts.rs`).
- Technical: the established trait search method already accepts
  `query_embedding: Option<&[f32]>`, so the first implementation can preserve
  the trait shape while tightening runtime behavior (source:
  `stores/zbot-stores-traits/src/memory_facts.rs`).
- Technical: zbot has configured embedding-provider identity fields for Engram
  provider type, model, dimensions, prompt profile, and normalization (source:
  `stores/zbot-engram-adapter/src/config.rs`;
  `gateway/src/state/persistence_factory.rs`).
- Technical: prior memory documentation describes the target architecture as
  hybrid recall with FTS/vector search, RRF fusion, graph expansion, and
  provenance; this spec makes the Engram-backed path conform to that target
  instead of keeping substring fallback (source: `docs/memory-explained.md`).
- Product: the user approved keeping the `memory` tool and UI contracts stable
  while fixing recall quality behind them (source: user confirmation
  2026-07-07).
- Product: the user stated one embedding provider, FastEmbed or Ollama, should
  and will exist in normal zbot operation (source: user confirmation
  2026-07-07).
- Process: this is full-mode work because it changes a structural memory/recall
  boundary and LLM-agent context behavior (source: `work-loop` risk triggers).
