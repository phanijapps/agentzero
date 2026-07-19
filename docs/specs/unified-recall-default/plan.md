# Plan: unified-recall-default

- **Spec:** [`spec.md`](spec.md)
- **Status:** Done

> **Plan contract:** this is the implementation strategy. Unlike the spec, this
> document is allowed to change as we learn. Material changes are recorded in
> the changelog below.

## Approach

Deliver this in six additive phases. First lock a small model-tool contract
and backend-neutral port in `agent-tools`; then adapt the existing
`MemoryRecall` aggregator in gateway code, preserving the old method as a
compatibility wrapper. Next wire the visible tool and legacy memory action into
all executor construction paths. Finally converge automatic refresh behavior,
then apply governance before durable semantic writes, wire taxonomy bootstrap
into retrieval, and finally run an ontology quality gate rather than adding
speculative ontology ranking. Each task ends with a command or manual
observable result that the user can verify before the next task begins.

## Constraints

- [`RFC-0014`](../../rfc/0014-context-capability-registry-and-context-graph.md):
  gateway/runtime capability policy remains the enforcement layer; model tool
  visibility is staged and context remains traceable.
- [`dynamic-ontology-skos-taxonomy`](../dynamic-ontology-skos-taxonomy/spec.md):
  zbot owns dynamic policy, SKOS expansion is bounded, and ontology validation
  remains advisory unless separately approved.
- [`engram-memory-engine-cutover`](../engram-memory-engine-cutover/spec.md):
  zbot talks to semantic storage through adapter/trait seams, not concrete DB
  types.
- No new provider, database schema, user setting, REST route, WebSocket event,
  or UI DTO is required for Phases 1-3.

## Construction tests

**Integration tests:** a gateway-execution registry snapshot proves actor
visibility; an in-memory/fake recall adapter proves the model tool and legacy
memory action return the same envelope; gateway-memory fixtures prove taxonomy
diagnostics and source degradation; a root/continuation/delegation fixture
proves unified semantic consistency.

**Manual verification:** run the daemon and AG-UI, submit the acceptance prompt
from AC11, and inspect the tool stream/result for `mode: unified`, source mix,
and any taxonomy diagnostics. Phase 4 is verified from the checked-in curated
retrieval fixture and report.

## Design (LLD)

### Design decisions

- Add a narrow `recall` tool instead of un-hiding broad `memory`; this retains
  RFC-0014's split-tool safety intent. This spec records the user-approved
  exception: `recall` is a bounded derived context request and carries catalog
  policy `default_visible_unified_recall_exception`. Traces to: AC2, AC6.
- Define an `agent-tools` port with plain serializable DTOs; gateway-execution
  implements it over `MemoryRecall`, preserving backend independence. Traces
  to: AC1-AC3.
- Add a `MemoryRecall` outcome/trace-returning path while preserving the
  existing `recall_unified -> Vec<ScoredItem>` API as a wrapper. Traces to:
  AC3, AC5.
- Treat the ontology phase as an evaluation gate. Taxonomy expansion and graph
  retrieval already deliver current value; ontology code waits for evidence.
  Traces to: AC12.

### Data & schema

- Add `contracts/jsonschema/unified-recall.schema.json` with `request` and
  `response` definitions. The response contains query, `mode`, bounded result
  items, fixed logical-source status map, optional taxonomy expansion, legacy
  aliases, and degradation diagnostics. It excludes embeddings and storage
  internals. The envelope trust marker applies to every returned item. Traces
  to: AC1, AC6, AC8.
- `UnifiedRecallRequest`, `UnifiedRecallItem`, `UnifiedRecallResponse`, and
  `UnifiedRecallAccess` live in `runtime/agent-tools/src/tools/recall.rs`.
  `RecallAuthorizationContext` is separately derived by a gateway-owned
  `RecallAuthorizationAccess` bridge from the authenticated execution result,
  never from ToolContext state or model arguments. These DTOs are
  semantic-store-neutral.
  Traces to: AC1-AC4.
- A gateway-only `UnifiedRecallOutcome` extends current scored-item return data
  with source status and taxonomy trace. It is an ephemeral read model, not a
  new persistence model. Traces to: AC3, AC5.

### Interfaces & contracts

- `recall` is a model-visible `Tool` with request fields `query` and optional
  `limit`; it has no mode, scope override, historical cutoff, or write field.
- The hidden compatibility call `memory(action="recall")` defaults to
  `mode="unified"`; explicit `mode="facts"` retains the prior fact-store
  path and preserves `source`, `prioritized`, `recalled`, and `reason` aliases.
  The complete historical table is: omitted mode + `as_of` → facts; explicit
  facts + `as_of` → facts; explicit unified + `as_of` → error.
- The `UnifiedRecallAccess` port returns only schema-shaped values. The gateway
  adapter maps `ScoredItem`, taxonomy candidates, and source status into that
  shape without leaking gateway-memory types into agent-tools.
- `RecallTool` obtains `RecallAuthorizationContext` from the gateway-owned
  authorization bridge. The adapter binds every source to its runtime
  tenant/workspace and agent, passes ward scope into source queries, then
  filters explicit session/ward provenance before ranking. Missing provenance
  is denied unless normalized by the source as authorized `__global__` scope.
- One `RecallOutputPolicy` at the tool boundary filters transcript-only sources,
  redacts recognised secret/path shapes, bounds serialized output to 16 KiB,
  and preserves score order while truncating/omitting lowest-ranked items. It
  returns only finite public reason codes; detailed errors remain in existing
  private tracing.
- The contract is [`contracts/jsonschema/unified-recall.schema.json`](../../../contracts/jsonschema/unified-recall.schema.json).
  Traces to: AC1-AC5.

### Component / module decomposition

- `runtime/agent-tools/src/tools/recall.rs`: model-visible read-only tool,
  serializable contract DTOs, and the host port.
- `runtime/agent-tools/src/tools/memory.rs`: optional unified-recall port and
  compatibility dispatch for `action="recall"`; exact/key-value/write actions
  remain untouched.
- `runtime/agent-tools/src/tools/mod.rs` and `src/lib.rs`: exports.
- `gateway/gateway-memory/src/recall/mod.rs`: stable outcome/trace path that
  exposes taxonomy/source diagnostics while retaining existing item-returning
  callers.
- `gateway/gateway-execution/src/invoke/unified_recall_adapter.rs`: maps
  `MemoryRecall` and optional `GoalAccess` to `UnifiedRecallAccess`.
- `gateway/gateway-execution/src/invoke/executor.rs`,
  `runner/invoke_bootstrap.rs`, `runner/core.rs`, and `delegation/spawn.rs`:
  register/wire the port for root, continuation, and delegated executors and
  move the mid-session hook to unified IDs.
- `docs/memory-explained.md` and `docs/specs/README.md`: current behavior and
  active-spec index. Traces to: AC2, AC6, AC11.

### State & control flow

1. Gateway constructs `GatewayUnifiedRecallAdapter` from the configured
   `MemoryRecall` plus optional goal adapter.
2. ExecutorBuilder registers `recall` for actors with `MemoryRead`, retains
   hidden `memory`, and passes the same adapter into it.
3. `recall` validates query/limit, obtains trusted authorization context from
   the gateway authorization bridge, then calls the port.
4. The adapter verifies its immutable tenant/workspace binding, passes agent
   and ward scope to each source, lists only scoped active goals, calls the
   traced unified recall method with an explicit provenance predicate before
   RRF/MMR, and projects safe DTOs.
5. `MemoryRecall` expands through the runtime-scope-bound SKOS taxonomy,
   collects configured source lists, normalizes authorized global provenance,
   filters candidates before fusion/reranking, and returns items plus safe
   diagnostics. An unprovable taxonomy scope returns no expansion/trace and a
   finite source status.
6. Tool output gives the model one untrusted reference bundle. Automatic
   refreshes use the same item IDs to avoid repeated injection.
7. The output policy redacts/omits unsafe material and applies the aggregate
   budget before the result reaches the model.
8. If a source is absent/fails, the outcome uses a finite reason code and
   returns remaining evidence; historical `as_of` uses only the fact store.

### Behavior & rules

- `limit` is clamped to the existing 1-20 range and query is bounded to the
  existing 500-character limit.
- Source-summary keys are stable logical names (`facts`, `graph`, `wiki`,
  `procedures`, `episodes`, `beliefs`, `hierarchy`, `goals`, `taxonomy`) rather
  than database table names.
- Taxonomy diagnostics include only selected public concept identifiers,
  labels, relation kind, and the bounded expanded query; unavailable taxonomy
  is a no-op, not an error.
- Goal titles and unfilled slot names become `GoalLite` input only when the
  existing GoalAccess adapter can list active goals. Failure to list goals is a
  named degradation and does not stop other recall.
- Mid-session deduplication uses `ScoredItem.id`, not fact-specific keys.
  Reactive micro-recall remains a targeted working-memory path in this spec.
- The source map always contains `facts`, `graph`, `wiki`, `procedures`,
  `episodes`, `beliefs`, `hierarchy`, `goals`, and `taxonomy`; each is marked
  used, empty, not configured, unavailable, or degraded. Result count equals
  result-array length and scores are descending; tests enforce both rules.
- A source gets one bounded retrieval attempt per recall; no new automatic
  retry loop is added. The existing single shorter-query embedding retry stays
  bounded and is surfaced as `embedding_unavailable` if it fails.

### Failure, edge cases & resilience

- No unified adapter: hidden legacy `memory(action="recall")` returns the
  pre-existing fact/KV fallback with `mode="facts"` and a degradation reason;
  the catalog exposes `recall` as unavailable and the model-visible tool is
  not registered.
- Query embedding unavailable: retain existing fail-closed fuzzy retrieval
  behavior and expose a safe degradation reason; do not broaden to substring
  matching.
- Individual semantic source unavailable: preserve successful source results
  and mark the unavailable source with a public reason code in diagnostics;
  log the detailed error privately.
- Taxonomy expansion cycle/empty/no match: rely on the existing bounded
  expander and report no expansion; do not synthesize terms.
- `as_of` plus explicit `mode="unified"`: return a clear tool error because
  a unified historical snapshot is not supported; never mix present graph data
  with historical facts.

### Quality attributes (NFRs)

- Safety: the complete recall envelope is marked untrusted; `recall` has no side effects
  because the envelope's required `trust_boundary` applies to the entire result
  bundle. `recall` has no side effects and is registered only through existing
  actor capability enforcement.
- Privacy: trusted authorization scope is applied before candidate ranking;
  output policy omits unauthorised/transcript-only content and redacts safe,
  known secret/path patterns before model rendering.
- Consumption: the response is at most 16 KiB after serialization. Deterministic
  highest-score-first selection drops/truncates lower-ranked content and marks
  the response `truncated`.
- Boundedness: request and result limits remain at current tool bounds; source
  diagnostics are summary-only.
- Explainability: every response names mode, source counts/status, and
  taxonomy expansion when applied.
- Portability: agent-tools DTOs depend on no concrete store, SQLite type, or
  Engram type.

### Dependencies & integration

- Reuses existing `MemoryRecall`, `RecallTaxonomyExpander`, `GoalAccess`,
  `ToolRegistry`, actor capability policy, EventBus RecallTrace, and AG-UI tool
  stream. No new dependency is needed.
- The adapter implementation remains in gateway-execution because that crate
  already depends on both agent-tools and gateway-memory; agent-tools cannot
  depend upward on gateway-memory.

## Tasks

### T1: Unified recall contract and tool port validate independently

**Phase:** 1 — contract

**Status:** Complete (2026-07-13)

**Depends on:** none

**Touches:** `contracts/jsonschema/unified-recall.schema.json`, `runtime/agent-tools/src/tools/recall.rs`, `runtime/agent-tools/src/tools/mod.rs`, `runtime/agent-tools/src/lib.rs`

**Tests:**
- TDD: valid `recall` request accepts only bounded `query` and `limit`; unknown
  and write-shaped fields are rejected. Verifies AC1, AC2.
- TDD: a fake `UnifiedRecallAccess` yields a response that serializes to the
  contract with provenance, source summary, and untrusted notice. Verifies AC1,
  AC8.
- TDD: response count equals result length, results are score-descending, all
  nine logical source keys are present, and bounded fields reject overflow.
  Verifies AC1, AC3.
- TDD: a gateway authorization-bridge-derived context is not supplied by model
  arguments. This task proves trusted context construction only; source-scope
  enforcement is verified by T3. Verifies AC4 (partial).
- TDD: hostile instruction text remains untrusted data, token-shaped secrets
  and `/home/...` paths are redacted, transcript-only candidates are omitted,
  and a 16 KiB-overflow fixture is deterministically truncated. Verifies AC8,
  AC9.
- Goal-based: the contract's `x-spec` backlink and the spec's Contract header
  agree. Verifies AC1.

**Approach:**
- Hand-author the JSON Schema contract with request/response definitions.
- Add store-neutral request/item/response DTOs, port trait, and `RecallTool`.
- Add the trusted authorization DTO and centralized output-policy helper in the
  same module; do not introduce a separate registry or storage layer.
- Export the tool without registering it yet.

**Done when:** `RecallTool` can run against a fake host, contract tests pass,
and the schema has no write operation or storage-specific field.

**Verification:** `cargo test -p agent-tools recall --locked`

### T2: Unified recall returns explainable taxonomy-aware outcomes

**Phase:** 1 — retrieval outcome

**Status:** Complete (2026-07-13)

**Depends on:** T1

**Touches:** `gateway/gateway-memory/src/recall/mod.rs`, `gateway/gateway-memory/src/recall/scored_item.rs`, `gateway/gateway-memory/src/recall/*`

**Tests:**
- TDD: current `recall_unified` compatibility callers still receive the same
  `Vec<ScoredItem>` after the outcome refactor. Verifies AC3.
- Integration: a static taxonomy expander produces a bounded expanded query and
  selected-label/relation diagnostics in the outcome. Verifies AC5.
- Integration: configured fact/graph source data plus one unavailable source
  returns real items and finite public source-status reason codes without raw
  embedding data. A database URL/path-bearing failure remains absent from the
  public outcome. Verifies AC3, AC10.

**Approach:**
- Add an outcome-returning unified-recall method containing items, safe source
  summary/status, and taxonomy trace.
- Keep `recall_unified` as a wrapper returning outcome items to avoid changing
  current automatic callers in this task.
- Reuse existing EventBus trace logic; do not add ontology ranking.
- Preserve one bounded source attempt; do not add retries, and retain the
  existing bounded embedding retry semantics only.

**Done when:** focused fixtures prove taxonomy expansion is visible in the
outcome, partial source degradation is explicit, and existing unified recall
tests stay green.

**Verification:** `cargo test -p gateway-memory recall --locked`

### T3: Gateway exposes safe unified recall and preserves legacy reads

**Phase:** 2 — model tool and compatibility

**Status:** Complete (2026-07-13)

**Depends on:** T1, T2

**Touches:** `gateway/gateway-memory/src/recall/mod.rs`, `gateway/gateway-execution/src/invoke/unified_recall_adapter.rs`, `gateway/gateway-execution/src/invoke/executor.rs`, `gateway/gateway-execution/src/runner/invoke_bootstrap.rs`, `gateway/gateway-execution/src/runner/core.rs`, `gateway/gateway-execution/src/delegation/spawn.rs`, `runtime/agent-tools/src/tools/memory.rs`

**Tests:**
- TDD: adapter maps facts, graph, taxonomy diagnostics, and active goals into
  the exact T1 response contract. Verifies AC3, AC5, AC7.
- TDD: `memory(action="recall")` defaults to unified; explicit `mode="facts"`
  and every `as_of` decision-table case preserve the specified behavior and
  legacy aliases. When `recalled` is emitted, it is JSON-identical to
  `results`. Verifies AC6.
- Integration: a scoped fake recall source presents an in-scope and a
  cross-ward/session candidate. The gateway passes an immutable visibility
  predicate into `MemoryRecall`; it excludes the latter before RRF/MMR ranking
  and strips unauthorised provenance. Cross-tenant/workspace,
  missing-provenance, and explicitly authorised `__global__` fixtures prove the
  same fail-closed rule. Completes AC4.
- TDD: the legacy `memory(action="recall")` route uses the same authorization
  binding and output policy as the visible tool; cross-scope items, transcript
  content, secret/path strings, and 16 KiB overflow are omitted/redacted or
  truncated identically. Verifies AC4, AC6, AC8, AC9.
- Integration: root, delegated executor, delegated reviewer, and ward-agent
  registries expose `recall` exactly when `MemoryRead` is allowed; `memory`
  remains hidden from model schema. The `recall` catalog entry carries
  `default_visible_unified_recall_exception`; without an adapter it remains a
  catalog-only capability with unavailable health and no model tool. Verifies
  AC2.

**Approach:**
- Implement the adapter over `MemoryRecall` and optional `GoalAccess`, parsing
  unfilled goal slots into the existing `GoalLite` type only after active goals
  are bound to agent/ward ownership.
- Add an additive scoped outcome path to `MemoryRecall`; keep the existing
  unscoped outcome method as its compatibility wrapper and apply the gateway
  authorization predicate before RRF/MMR candidate fusion. Deny missing
  provenance. Only the trusted source-query seams may assign the explicit
  `__global__` marker, and the immutable gateway binding permits it only for a
  finite, source-specific allowlist whose store queries have already enforced
  the agent/workspace boundary.
- Bind each adapter to runtime tenant/workspace and decline to construct a
  taxonomy expansion/trace if its selected taxonomy source cannot prove the
  same scope.
- Add optional port plumbing to `MemoryTool`; preserve all non-recall actions.
- Register `RecallTool` with `MemoryRead` only in every ExecutorBuilder path.
- Add catalog metadata for the new tool without changing existing API routes.
  When no adapter is configured, retain a catalog-only unavailable capability
  entry rather than a model-visible executable tool.

**Done when:** a model-visible registry snapshot contains narrow `recall`, not
broad `memory`, and legacy callers receive explicitly labeled modes.

**Verification:** `cargo test -p agent-tools memory --locked && cargo test -p gateway-execution executor --locked`

### T4: Automatic recall paths share unified semantics

**Phase:** 3 — consistency

**Status:** Complete (2026-07-13)

**Depends on:** T3

**Touches:** `gateway/gateway-execution/src/runner/core.rs`, `gateway/gateway-execution/src/runner/invoke_bootstrap.rs`, `gateway/gateway-execution/src/delegation/spawn.rs`, `gateway/gateway-execution/src/middleware/intent_analysis.rs`, `gateway/gateway-execution/src/recall/mod.rs`

**Tests:**
- Integration: bootstrap, continuation, delegation, and intent analysis call
  the traced unified path and retain their existing budgets. Verifies AC7.
- TDD: mid-session refresh accepts graph/procedure/belief IDs and does not
  reinject an already seen generic item ID. Verifies AC7.
- Integration: hostile instruction text, token-shaped secrets, private paths,
  and transcript-only candidates cannot bypass the shared output policy when
  bootstrap or continuation injects automatic recall context. Verifies AC8,
  AC9.
- Integration: an active goal changes ranking input when GoalAccess is present;
  a GoalAccess failure is visible as a degradation without losing fact/graph
  results. Verifies AC3, AC7.

**Approach:**
- Extract a gateway-execution helper that obtains active goals and invokes the
  same adapter/underlying outcome path.
- Route all existing unified sites through it, then replace the fact-only
  mid-session hook with bounded unified rendering. Apply the same T1
  `RecallOutputPolicy` before every automatic prompt rendering path.
- Leave reactive micro-recall untouched because it is a targeted context-delta
  flow, not default recall.

**Done when:** every normal automatic recall call has one semantic path, goals
are no longer always passed as an empty list, and generic-ID dedup tests pass.

**Verification:** `cargo test -p gateway-execution recall --locked && cargo test -p gateway-execution continuation --locked`

### T5: Operator verification, documentation, and ontology readiness report

**Phase:** 6 — prove value before ontology ranking

**Status:** Complete (2026-07-13)

**Depends on:** T4, T7

**Touches:** `docs/memory-explained.md`, `docs/specs/unified-recall-default/*`, `docs/specs/README.md`, `docs/architecture/future-state/*` or a focused retrieval fixture/report location

**Tests:**
- Goal-based: a checked-in curated fixture records taxonomy-plus-graph baseline
  and an ontology-aware candidate evaluation over at least 12 labelled queries.
  The candidate must improve mean nDCG@5 by at least 0.05 and lower no top-1
  relevance label. Each case contains a query, permitted scope, a frozen
  candidate-ID universe, 0/1/2 relevance grades, and baseline/candidate ranked
  IDs. The fixture carries baseline and candidate configuration fingerprints;
  a deterministic evaluator emits JSON with per-query and mean nDCG@5,
  top-1 grades, and pass/fail decision. The evaluator is test-only. Verifies
  AC12.
- Goal-based: repository check proves no ontology-aware ranking/filtering code
  was added when the gate is not passed. Verifies AC12.
- Manual QA: the AG-UI acceptance prompt produces a visible `recall` tool call,
  `mode: unified`, and a source summary. Verifies AC11.

**Approach:**
- Update the memory explanation and active spec index to describe the new
  model-visible read-only recall surface and exact historical limitation.
- Add a sanitized retrieval evaluation fixture and report template with clear
  go/no-go criteria for ontology semantics. Put any candidate ontology logic in
  test support only; a passing report creates a follow-up implementation spec,
  not production ranking code.
- Run full-mode E2E because executor construction/tool schema changes are on
  the hot path; record the manual AG-UI result in the task handoff.

**Done when:** documentation matches behavior, the user can reproduce the
AG-UI result, and the ontology report either authorizes a separate feature or
records a no-go with no speculative ranking code.

**Verification:**

```bash
cargo test -p gateway-memory ontology_retrieval_evaluation --locked
cargo check --workspace --locked
(cd apps/ui && npm test -- --run src/features/research-v2/ToolActivity.test.tsx src/features/research-v2/turns.test.ts)
(cd apps/ui && npm run build)
(cd e2e/playwright && npx playwright test full-mode/unified-recall.full.spec.ts)
```

### T6: Govern every durable semantic write

**Phase:** 4 — write-time governance

**Status:** Complete (2026-07-13)

**Depends on:** T3

**Touches:** `runtime/agent-tools/src/tools/memory.rs`, ingestion/connector
evidence adapters, session distillation, ward/wiki indexing, graph-write
adapters, and the Engram adapter bootstrap/configuration seam.

**Tests:**
- Integration: each producer records configured taxonomy classifications in
  its Engram-backed durable record.
- Integration: an ontology-invalid relationship persists with an advisory,
  finite governance finding; no raw ontology/store error reaches a tool or UI
  surface.
- Integration: a missing/confined-invalid governance definition fails startup
  or reports an explicit unavailable capability; it never silently falls back
  to ungoverned configured writes.

**Approach:**
- Use the existing Engram governance policy and confined definition paths;
  do not add a second z-Bot semantic classifier or database.
- Route evidence intake and distilled/indexed records through the same
  governance selection helper before their durable write.
- Preserve `allowUnclassified` and advisory validation semantics.

**Done when:** all durable semantic producers have one observable,
Engram-backed write-time governance path.

**Verification:** `cargo test -p zbot-engram-adapter governance --locked`

### T7: Bootstrap configured taxonomy into unified recall

**Phase:** 5 — taxonomy retrieval activation

**Status:** Complete (2026-07-13)

**Depends on:** T2, T6

**Touches:** Engram bootstrap/composition root, `gateway-memory` recall wiring,
and focused operator documentation.

**Tests:**
- Integration: the actual configured `config/governance/base-taxonomy.json`
  is bootstrapped and `MemoryRecall` emits selected labels/relations for a
  matching query.
- Integration: disabled/missing taxonomy has a finite source status and does
  not alter fact/graph retrieval.

**Approach:**
- Construct and attach `EngramTaxonomyRecallExpander` from the active provider
  configuration during runtime composition.
- Do not make ontology a query-expansion, filter, or ranking input here.

**Done when:** a running z-Bot recall result can demonstrate configured
taxonomy expansion from the local governance definitions.

**Verification:** `cargo test -p gateway-memory taxonomy --locked`

## Rollout

- **Delivery:** Phases 1-3 are additive and can ship together after T4. Existing
  automatic unified recall remains active throughout; the new on-demand tool
  adds no migration. Roll back by removing the `recall` registration and using
  the legacy hidden fact path; no data is altered.
- **Infrastructure:** none. Existing embedding, taxonomy, goal, and EventBus
  services are reused.
- **External integration:** the configured Engram adapter must already expose
  the current memory/graph/taxonomy traits. A missing capability is reported as
  degraded, not replaced with SQLite.
- **Deployment sequence:** T1 contract → T2 outcome → T3 tool wiring → T4
  automatic convergence → T6 write governance → T7 taxonomy activation → T5
  proof/report. The ontology decision does not block the shipped taxonomy-plus-
  graph recall path.

## Risks

- A model-visible recall tool can increase retrieval calls and prompt tokens;
  preserve query/limit bounds and existing small-talk skip behavior.
- Converging mid-session recall can increase repeated context; generic-ID
  deduplication and existing budget rendering must be verified.
- Source diagnostics may reveal too much implementation detail; contract tests
  must reject unsafe fields.
- Active goal boost changes ranking; use bounded fixtures and explicit source
  summaries to make the change reviewable.
- Ontology terms are currently generic and can worsen recall if treated as
  expansion/ranking truth; the phase gate prevents premature implementation.

## Changelog

- 2026-07-11: initial phased plan; taxonomy is included in the delivered
  unified path, while ontology retrieval remains quality-gated.
- 2026-07-13: expanded the plan to six phases: durable-write governance (T6)
  and runtime taxonomy activation (T7) precede the ontology quality gate (T5).
- 2026-07-13: T3 scope enforcement now uses an additive pre-fusion
  `MemoryRecall` predicate rather than adapter-only post-ranking filtering.
- 2026-07-13: completed T3. Model-visible `recall` now has an immutable
  provider-derived scope, source-specific global classifications at trusted
  query seams, bounded safe output, and legacy `memory(action="recall")`
  compatibility. T4–T7 remain pending.
- 2026-07-13: completed T4. Bootstrap, handoff, continuation, delegation,
  intent analysis, and mid-session refresh now share the provider-scoped
  adapter and output policy; refresh deduplicates source-qualified generic
  unified item IDs and renders finite source-degradation status. Retired raw
  direct goals, corrections, handoff, and procedure prompt injections so those
  data types reach automatic context only through unified recall. T5–T7 remain
  pending.
- 2026-07-13: completed T6. Governed memory, wiki/entity, belief, hierarchy,
  and relationship writes now persist the authoritative configured selections;
  model fact writes retain session, ward, and writer provenance; evidence,
  procedures, and session episodes are mirrored through the shared Engram
  provider with taxonomy concept classifications before their compatibility
  sidecar writes. Invalid configured definitions fail safely and relationship
  deduplication cannot retain stale or caller-supplied governance metadata.
  T7 and T5 remain pending.
- 2026-07-13: completed T7. Runtime bootstrap now loads confined configured
  ontology/taxonomy definitions, binds the selected taxonomy snapshot to the
  Engram provider, and constructs a scope-aware recall expander only when a
  selection exists. Session overlays select only their trusted definitions;
  stale durable concepts cannot expand after restart; unsupported overlays
  fail closed. Missing configuration reports a finite `not_configured` state
  while real fact and graph retrieval continue unchanged.
- 2026-07-13: completed T5. Added the 12-query frozen ontology evaluation
  fixture and deterministic no-go report (no production ontology ranking), a
  full-mode forced-recall browser fixture, and a compact argument-free
  completed-turn tool activity row. The AG-UI test proves the acceptance
  prompt invokes visible `recall` and renders `mode: unified` plus the source
  types; contract/adapter tests verify the bounded safe source-status values.
