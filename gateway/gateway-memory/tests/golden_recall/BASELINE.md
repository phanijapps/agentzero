# Golden Recall — PRE-ENGRAM-MIGRATION BASELINE (P0)

Captured **before** any recall-quality phase lands (phases P1–P4 in
`docs/specs/recall-quality/analysis.md`). This is the current production
behavior: rank-only RRF fusion (k=60), MMR disabled, no recency/usage
scoring in the unified path, `intent_boost` a provable no-op.

## Contract for future phases

- **Must not regress**: presence floor stays 30/30 (retrieval finds every
  labeled item at budget 25); tag floors stay green (correction 5/5,
  avoid 2/2, stale 3/3, pattern 5/5); ward isolation holds (c24/c25);
  pinned fact survives (c15); superseded stale fact stays suppressed (c06).
- **Should improve**: `precision@5` at the production-shaped budget (10) —
  baseline **43–50%** (13–15 of 30 across runs; see the tie-variance note
  below); correction top-5 — baseline **2/5**. Weighted fusion + temporal
  lane + usage reinforcement are the levers.

## Harness notes

- **Surface measured**: the harness wires the sqlite fact store (via the
  identity-tolerant shim below) plus the real `recall_unified` fusion in
  gateway-memory. The P1 commit (`a6437ef1`) rewired the *engram adapter's*
  fact lane onto engram's weighted RRF — a different store surface this
  harness does not exercise. The numbers below therefore capture the
  pre-migration ranking behavior this suite measures; they become the
  regression floor when P2/P3 rewire the gateway-memory fusion itself.
- Deterministic: token-hash 384-dim embedder (FNV-1a), fixed-age corpus,
  no network. Corpus: 25 facts (superseded stale pair, pinned fact,
  mention/age spread, session-scoped ctx probes, cross-ward isolation
  probes), 3 procedures with track records, 3 episodes (success/partial/
  failed-with-learnings), 2 wiki articles, 2 beliefs, 5-entity KG.
- The sqlite `GatewayMemoryFactStore` does not implement
  `search_memory_facts_hybrid_with_identity` — the trait default fails
  closed when a query embedding is present; only the production engram
  adapter implements the identity gate. The harness wraps the sqlite
  store with an identity-tolerant shim delegating to the plain hybrid
  surface (equivalent to engram's behavior when identities match).
  **Harness finding worth a fix later:** the sqlite backend cannot serve
  unified recall's identity-gated path at all.
- Two budgets per case: `floor 25` asserts retrieval correctness;
  `prod 10` measures the packet the model actually sees; its top-5 is
  the precision metric. With ten fused lanes, a budget-10 packet holds
  ~one item per lane — that round-robin crowding is the ranking
  weakness the migration targets.

- **Ranking instability (harness finding)**: `precision@5` fluctuates
  between runs (13/30 and 15/30 observed) because rank-only RRF gives
  every lane's rank-1 the identical fused score (1/61) and the
  breaks those ties by `HashMap` iteration order — per-process random.
  Identical inputs, different packet order. Weighted fusion with
  preserved source scores is the fix; this metric becomes deterministic
  only after ties are broken by signal instead of hash order.

## Scorecard (2026-09-09, pre-migration)

```
[PASS] c01-research-first-correction (correction) — floor 24/25 items, prod 10 items
[PASS] c02-web-research-tools (correction) — floor 24/25 items, prod 10 items
[PASS] c03-atomic-delegation (correction) — floor 24/25 items, prod 10 items
[PASS] c04-ward-first (correction) — floor 24/25 items, prod 10 items
[PASS] c05-citations (correction) — floor 24/25 items, prod 10 items
[PASS] c06-stale-verdict-suppressed (stale) — floor 24/25 items, prod 10 items
[PASS] c07-financial-analysis-flow (pattern) — floor 24/25 items, prod 10 items
[PASS] c08-comparison-table-recipe (pattern) — floor 24/25 items, prod 10 items
[PASS] c09-ticker-metric-schema (pattern) — floor 24/25 items, prod 10 items
[PASS] c10-visualization-guide (pattern) — floor 24/25 items, prod 10 items
[PASS] c11-peer-tickers (recall) — floor 24/25 items, prod 10 items
[PASS] c12-scrape-avoid (avoid) — floor 24/25 items, prod 10 items
[PASS] c13-curl-blocked-learnings (avoid) — floor 24/25 items, prod 10 items
[PASS] c14-research-publish-procedure (pattern) — floor 24/25 items, prod 10 items
[PASS] c15-pinned-name-survives (recall) — floor 24/25 items, prod 10 items
[PASS] c16-user-location (recall) — floor 24/25 items, prod 10 items
[PASS] c17-earnings-date (recall) — floor 24/25 items, prod 10 items
[PASS] c18-revenue-mix (recall) — floor 24/25 items, prod 10 items
[PASS] c19-aapl-belief (recall) — floor 24/25 items, prod 10 items
[PASS] c20-wiki-howto (recall) — floor 24/25 items, prod 10 items
[PASS] c21-wiki-data-sources (recall) — floor 24/25 items, prod 10 items
[PASS] c22-graph-aapl-entity (recall) — floor 24/25 items, prod 10 items
[PASS] c23-graph-tool-entity (recall) — floor 24/25 items, prod 10 items
[PASS] c24-ward-isolation-hr (stale) — floor 24/25 items, prod 10 items
[PASS] c25-ward-isolation-journal (stale) — floor 24/25 items, prod 10 items
[PASS] c26-skill-lookup (recall) — floor 24/25 items, prod 10 items
[PASS] c27-agent-lookup (recall) — floor 24/25 items, prod 10 items
[PASS] c28-multi-topic (recall) — floor 24/25 items, prod 10 items
[PASS] c29-generic-market-data (recall) — floor 24/25 items, prod 10 items
[PASS] c30-msft-premium (recall) — floor 24/25 items, prod 10 items

================ GOLDEN RECALL SCORECARD ================
```


## D0 — engram-path scorecard (production path, post stem-matching + lexical-admission fixes)

Floors ALL hold on the production (engram adapter) stack:
- presence floor 30/30; correction 5/5; avoid 2/2; stale 3/3; pattern 5/5
- precision@5 **76.7%** (vs 56.7% on the sqlite measurement path)
- correction top-5 **5/5** (vs 4/5)

The sqlite-path numbers above remain as historical baseline; the engram path
is the measured production path from D0 on. Fixes made during D0 (both real
production improvements, not harness tweaks): stem matching in the sparse
lane ("scrape" → "scraping") and lexical-evidence admission parity with the
previous sqlite-backed behavior (ranking handles quality; admission filters
only zero-evidence semantic noise).
