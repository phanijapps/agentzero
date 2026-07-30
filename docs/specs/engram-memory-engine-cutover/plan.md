# Plan: Engram Memory Engine Cutover

- **Spec:** [`spec.md`](spec.md)
- **Status:** Done

> **Plan contract:** this is the implementation strategy. Unlike the spec, this
> document is allowed to change as you learn. When it changes substantially
> (a different approach, not just a re-ordering), note why in the changelog at
> the bottom.

## Approach

Migrate by making `zbot-engram-adapter` the only place that knows both
AgentZero store traits and Engram repository/service crates. The first slice is
construction and fail-closed capability reporting: AgentZero can open a fresh
Engram SQLite backing set through explicit open options, but no feature reports
supported until the matching trait parity fixture exists. Later slices map each
store trait family to Engram or adapter sidecars, add dry-run/apply migration,
wire provider selection, and retire old direct memory-engine coupling after
parity passes.

## Constraints

- [`RFC-0011`](../../rfc/0011-engram-memory-engine-cutover.md): adapter-first
  cutover, AgentZero-owned gateway/UI/settings/sleep-cycle contracts, current
  DB as migration/parity reference.
- [`RFC-0012`](../../rfc/0012-engram-upstream-risk-reduction.md): use generic
  Engram framework ports and avoid private SQLite internals for production
  behavior unless explicitly waived.
- [`rig-engine-migration`](../rig-engine-migration/spec.md): Rig remains the
  execution engine and gateway-facing runtime facade; this spec changes the
  memory backing framework below AgentZero store contracts.
- [`runtime-context-control`](../runtime-context-control/spec.md): live context
  compaction and durable memory boundaries stay stable unless both specs are
  updated.
- RFC-0011/RFC-0012 are still Draft. The 2026-07-05 implementation waiver
  covered T1-T2; 2026-07-06 follow-up waivers authorized T3-T9 while preserving
  fail-closed capability gates. A later 2026-07-06 adversarial review tightened
  T9 so the current-SQLite semantic memory/knowledge fallback could not remain
  a terminal state; the follow-up cleanup retired it from production startup.
- Real current DB content, embeddings, private transcripts, connector data, and
  exact local DB provenance are private runtime data and must not be committed.

## Construction tests

**Integration tests:**

- Current-provider vs Engram-provider store-trait parity fixtures for each
  feature before that feature reports `Supported`.
- Dry-run migration fixtures from sanitized current-DB patterns and synthetic
  cases before apply mode is enabled.
- Existing gateway/UI route and Observatory tests after provider selection is
  wired.

Protected contract artifacts:

| Surface | Artifacts | Gate |
| --- | --- | --- |
| Memory API/UI | `gateway/src/http/memory.rs`, `gateway/src/http/memory_search.rs`, `apps/ui/src/services/transport/http.ts`, `apps/ui/src/features/memory/command-deck/*` | Existing HTTP and command-deck tests plus provider parity fixtures. |
| Graph/Observatory | `gateway/src/http/graph.rs`, `gateway/src/http/hierarchy.rs`, `gateway/src/http/belief_network.rs`, `apps/ui/src/features/observatory/*`, `apps/ui/src/features/observatory-v2/*` | Graph/Observatory/hierarchy/belief-network tests and payload snapshot checks. |
| Settings/embeddings | `gateway/src/http/settings.rs`, `gateway/src/http/embeddings.rs`, `apps/ui/src/features/settings/*` | Settings/embedding tests; no DTO rename. |
| Wards/Vault/Research ward context | `gateway/src/http/ward_content.rs`, `gateway/src/http/ward_actions.rs`, `gateway/src/http/ward_usage.rs`, `gateway/src/http/vault.rs`, `apps/ui/src/services/transport/http.ts`, `apps/ui/src/features/memory/command-deck/*`, `apps/ui/src/features/vault/*`, `apps/ui/src/features/research-v2/*` | Ward content, vault ward, memory ward rail, chat/research ward event, and transport tests; `/api/wards*` and `/api/vault/wards*` keep their shapes. |
| Sleep-cycle jobs | `gateway/gateway-memory/src/sleep/*`, `gateway/gateway-execution/src/sleep/*` | Worker tests prove unsupported capabilities do not start dependent jobs. |
| Recall traces | `gateway/gateway-memory/src/recall/*`, `apps/ui/src/features/observatory-v2/useRecallTrace.ts`, gateway events/ws protocol crates | Recall trace/event mapping tests preserve existing observability. |

**Manual verification:**

- After provider wiring, run daemon/UI or CLI against a fresh Engram DB and
  verify chat, memory write/recall, knowledge graph activity, AgentZero-owned
  sleep-cycle cleanup, session reload, and Memory/Graph/Observatory tabs.

## Design (LLD)

### Design decisions

- Adapter first: only `stores/zbot-engram-adapter` imports Engram storage crates.
  Traces to: AC1, AC2, AC18.
- Fail closed: capability reports stay unsupported until parity proves a store
  family. Traces to: AC3, AC4.
- Sidecars are explicit: zbot-specific behavior that Engram should not own
  stays adapter-owned and test-covered. Traces to: AC10.
- Runtime fallback retired: Engram is the semantic memory provider and
  `conversations.db` is the zbot-owned SQLite runtime DB. Traces to: AC13,
  AC16, AC18.

### Data & schema

- Engram owns the target memory/knowledge/belief/hierarchy physical DB layout;
  AgentZero selects Engram's single-file SQLite layout (`engram_data.db`) for
  desktop backup/debug/delete simplicity.
- Current zbot SQLite stores are reference inputs for migration and parity, not
  schema contracts Engram must preserve.
- Adapter compatibility tables may hold zbot-specific records such as ctx facts,
  skill index rows, recall logs, outbox bridge state, migration diagnostics, and
  embedding byte parity when no generic Engram concept exists. In single-file
  layout these tables are namespaced inside `engram_data.db`; in multi-file
  layout they can still use separate sidecar files.
- Exact local DB source metadata remains gitignored; committed fixtures use
  coarse provenance and allowlisted shapes only.

### Interfaces & contracts

- Public contract: existing `zbot-stores-traits`, gateway memory/graph/belief
  routes, UI DTOs, memory settings, WebSocket events, and Observatory read
  models.
- Internal contract: Engram public repository/service traits and SQLite
  `open_with_options` constructors.
- No new REST/event contract is introduced by this spec.

### Component / module decomposition

- `config`: provider mode, Engram path, tenant, scope mapping, embedding mode,
  SQLite storage layout, and migration mode.
- `scope`: translates AgentZero ward/session/partition identifiers into Engram
  `Scope` without widening.
- `bootstrap`: constructs Engram SQLite services/stores and capability reports.
- `mapping`: translates zbot domain types to Engram domain types and back.
- `stores`: implements zbot store traits by feature family.
- `migration`: dry-run/apply import tooling and diagnostics.
- `fixtures`: current-provider vs Engram-provider parity harnesses.
- `sidecar`: adapter-owned storage for zbot-specific records not accepted into
  generic Engram.

### State & control flow

1. AgentZero loads memory settings and builds user-controlled `AdapterConfig`
   fields.
2. The composition root injects the trusted zbot data root, validates the
   config, and asks `zbot-engram-adapter` for a provider.
3. Adapter bootstrap opens Engram SQLite stores through explicit open options.
4. Capability report is emitted before sleep-cycle workers or route handlers
   can rely on a feature.
5. Supported store traits route to Engram-backed implementations.
6. Unsupported store traits fail closed in `ProviderMode::Engram`; there is no
   post-cutover current-SQLite semantic memory/knowledge runtime fallback.
7. Migration dry-run reports deterministic diagnostics before apply mode writes
   a fresh Engram DB.

### Behavior & rules

- Capability `Supported` means the feature has trait parity tests, mapping
  tests, and focused failure tests.
- Scope translation trims but does not normalize or widen caller identifiers.
- Dynamic ontology/taxonomy selection remains an AgentZero policy decision.
- Adapter mapping errors return trait-compatible errors without hiding
  capability blockers.
- Sleep-cycle workers call AgentZero store traits and capability gates; they do
  not become Engram schedulers.
- Provider bootstrap and public diagnostics redact paths, SQL internals, row
  contents, transcripts, embeddings, connector/vault data, and secrets.

### Failure, edge cases & resilience

- Missing Engram path in Engram mode fails config validation before bootstrap.
- Parent directory creation is explicit in the adapter immediately before
  `SqliteOpenOptions` is built; downstream Engram `create_parent_dirs` is
  disabled.
- Configured Engram database paths are confined under the trusted zbot data
  root by resolving the root, requested parent, and any existing DB file;
  user-deserialized settings cannot set the root, and symlink escapes,
  traversal, nonexistent roots, and disallowed absolute paths are rejected
  before SQLite opens the file.
- Opening one Engram store successfully and another failing must fail the whole
  provider bootstrap; no partial provider is reported as usable.
- Apply-mode migration refuses to run when dry-run diagnostics are unavailable
  or stale.
- Unsupported feature access fails closed with a clear feature name and reason.

### Quality attributes (NFRs)

- Safety: no destructive migration until dry-run parity is accepted.
- Operability: startup capability report names unsupported feature blockers.
- Maintainability: Engram imports stay concentrated in the adapter crate.
- Performance: provider construction uses WAL and busy timeout defaults for
  file-backed SQLite.

### Dependencies & integration

- Engram crates come from the public `https://github.com/phanijapps/engram`
  repository's `main` branch; `Cargo.lock` records the resolved commit for
  reproducible builds.
- First used crates: `engram-integration`, `engram-conformance`, Engram port
  crates, and only the remaining low-level SQLite crates needed inside store
  compatibility sidecars until provider handles fully cover the parity surface.
- Lockfile churn is limited to Engram path crates and their required crypto
  stack; `typenum 1.20.1` is resolver-required by Engram's `sha2 0.11`
  dependency, while unrelated `windows-sys` and `socket2` reselections stay at
  their prior locked versions.
- Existing `zbot-stores-*` traits and domain types remain the compatibility
  surface until the cutover is complete.

## Tasks

### T1: Adapter opens an Engram backing set fail-closed

**Depends on:** none

**Touches:** `stores/zbot-engram-adapter/Cargo.toml`, `stores/zbot-engram-adapter/src/*`,
`docs/specs/engram-memory-engine-cutover/*`, `docs/specs/README.md`

**Tests:**

- TDD: config with `ProviderMode::Engram` and a path builds file-backed
  `SqliteOpenOptions` with WAL, busy timeout, foreign keys, migrations, and
  adapter-created parent directories while downstream directory creation is
  disabled. Verifies AC1, AC2.
  stub: true (`stores/zbot-engram-adapter/tests/bootstrap.rs`)
- TDD: bootstrap opens memory, knowledge, belief, and hierarchy Engram stores
  through the adapter and fails the whole provider on any construction error.
  Verifies AC1, AC3.
  stub: true (`stores/zbot-engram-adapter/tests/bootstrap.rs`)
- TDD: bootstrap capability report remains unsupported for all feature families
  until parity tasks mark them supported. Verifies AC3.
  stub: true (`stores/zbot-engram-adapter/tests/bootstrap.rs`)
- TDD: Engram DB path resolution rejects missing data roots, ambiguous roots,
  deserialized root overrides, disallowed absolute paths, traversal, and
  symlink escapes before opening SQLite, while accepting absolute paths inside
  the trusted root. Verifies AC2.
  stub: true (`stores/zbot-engram-adapter/tests/bootstrap.rs`)
- TDD: `ProviderMode::Engram` has no implicit current-SQLite fallback for an
  unsupported feature. Verifies AC3.
  stub: true (`stores/zbot-engram-adapter/tests/bootstrap.rs`)
- Goal-based: `cargo test -p zbot-engram-adapter` passes. Verifies AC1-AC3.
  stub: no stub (goal-based)

**Approach:**

- Add Engram runtime and SQLite adapter crate dependencies to
  `zbot-engram-adapter`.
- Add a `bootstrap` module that owns Engram SQLite store construction.
- Keep constructed stores behind an `EngramStores` handle without implementing
  zbot store traits yet.
- Extend tests around config, bootstrap, and capability reporting.

**Done when:** the adapter can open a fresh Engram SQLite backing set through
one public constructor and still reports every feature unsupported by default.

### T2: Store parity harness exists before feature support

**Depends on:** T1

**Touches:** `stores/zbot-engram-adapter/src/fixtures/*`,
`stores/zbot-stores-conformance/*`, `stores/zbot-engram-adapter/tests/*`

**Tests:**

- Goal-based: a fixture runner can execute the same accepted store-trait case against
  current SQLite and an Engram-backed candidate. Verifies AC4.
  artifact: `stores/zbot-engram-adapter/src/fixtures/mod.rs::tests::runner_executes_default_scope_registry_against_current_and_engram_candidates`;
  command: `cargo test -p zbot-engram-adapter --locked`.
- Goal-based: unsupported Engram feature rows cannot be marked supported unless
  the fixture registry has a passing case for that feature. Verifies AC3, AC4.
  artifact: `stores/zbot-engram-adapter/src/fixtures/mod.rs::tests::capability_support_requires_passing_current_and_engram_outcomes`;
  command: `cargo test -p zbot-engram-adapter --locked`.
- Goal-based: fixture registry entries include positive and negative scope
  cases for tenant, ward, session, and partition before any capability can flip
  to supported. Verifies AC15.
  artifact: `stores/zbot-engram-adapter/src/fixtures/mod.rs::tests::default_registry_tracks_required_scope_coverage`;
  command: `cargo test -p zbot-engram-adapter --locked`.
- Goal-based: conformance seed cases are named in
  `zbot-stores-conformance` before the adapter can track them. Verifies AC4.
  artifact: `stores/zbot-stores-conformance/src/parity.rs`;
  command: `cargo test -p zbot-stores-conformance --locked`.

**Approach:**

- Start with memory facts and belief fixtures because they have compact trait
  surfaces and Engram domain analogues.
- Reuse existing conformance patterns where available instead of creating a
  parallel testing framework.
- Keep this slice to harness metadata and fail-closed capability gating; it
  does not implement Engram-backed store traits or enable provider features.

**Done when:** every future capability flip has a required fixture path.

Tasks T7-T9 are sequencing records, not authorized execution under the current
waiver. Before any of them can move to EXECUTE, RFC-0011 and RFC-0012 must be
accepted or the user must grant a follow-up explicit waiver. Their deferred
rows are test obligations, not materialized stubs for this loop; each must get
concrete verification artifacts in the follow-up plan gate before execution.

### T3: Memory facts map to Engram memory records

**Depends on:** T2

**Status:** Done on 2026-07-06.

**Execution precondition:** satisfied by explicit 2026-07-06 follow-up waiver.

**Touches:** `stores/zbot-engram-adapter/src/mapping/*`,
`stores/zbot-engram-adapter/src/stores/*`,
`stores/zbot-engram-adapter/tests/*`

**Tests:**

- TDD: `MemoryFact` inserts map to Engram memory records without losing agent,
  ward/session scope, category, confidence, source, timestamps, or embedding
  mode; `importance` is recorded as absent because the current `MemoryFact`
  domain type has no field for it. Verifies AC6.
  artifact: `stores/zbot-engram-adapter/tests/memory_facts.rs::memory_fact_maps_to_engram_record_losslessly`;
  command: `cargo test -p zbot-engram-adapter --locked`.
- TDD: read/list/get/delete/archive calls preserve the supported
  `MemoryFactStore` semantics through Engram-backed records plus the explicit
  adapter memory-fact sidecar. Verifies AC6.
  artifact: `stores/zbot-engram-adapter/tests/memory_facts.rs::{save_count_list_get_delete_and_archive_round_trip,typed_upsert_preserves_embedding_sidecar}`;
  command: `cargo test -p zbot-engram-adapter --locked`.
- TDD: unsupported memory-adjacent methods with later-task owners return named
  unsupported errors rather than silently succeeding under the `MemoryFacts`
  capability. Verifies AC6, AC10.
  artifact: `stores/zbot-engram-adapter/tests/memory_facts.rs::later_task_memory_methods_fail_closed`;
  command: `cargo test -p zbot-engram-adapter --locked`.
- Goal-based: capability reporting can mark `MemoryFacts` supported while
  unrelated features remain unsupported and `from_config` remains fail-closed.
  Verifies AC3, AC6.
  artifact: `stores/zbot-engram-adapter/tests/memory_facts.rs::memory_fact_support_can_be_enabled_without_enabling_other_features`;
  command: `cargo test -p zbot-engram-adapter --locked`.

**Approach:**

- Implement the narrow typed methods first, then preserve JSON fallback methods
  used by older callers.
- Keep embedding bytes in sidecar mode until Engram retrieval parity is accepted.
- Use Engram `MemoryRepository` as the canonical record write/read path and an
  adapter-owned sidecar SQLite index for zbot-specific filters, JSON row shape,
  embedding bytes, and simple search until Engram exposes a generic query port.

**Done when:** memory facts can be enabled in capability reporting.

### T4: Knowledge, graph, taxonomy, and hierarchy map to Engram repositories

**Depends on:** T2

**Status:** Done on 2026-07-06.

**Execution precondition:** satisfied by explicit 2026-07-06 follow-up waiver.

**Touches:** `stores/zbot-engram-adapter/src/mapping/*`,
`stores/zbot-engram-adapter/src/stores/*`,
`stores/zbot-engram-adapter/tests/*`

**Tests:**

- TDD: zbot wiki articles map to Engram source/document/chunk records while
  preserving `WikiStore` list/get/delete/search read models and sidecar
  embeddings. Verifies AC7.
  artifact: `stores/zbot-engram-adapter/tests/knowledge_graph.rs::wiki_articles_round_trip_through_engram_knowledge`;
  command: `cargo test -p zbot-engram-adapter --locked`.
- TDD: zbot KG entities and relationships map to Engram knowledge entities and
  relationships while preserving `KnowledgeGraphStore` CRUD, search, neighbor,
  traverse, and HTTP read-model behavior. Verifies AC7, AC15.
  artifact: `stores/zbot-engram-adapter/tests/knowledge_graph.rs::graph_entities_relationships_and_read_models_round_trip`;
  command: `cargo test -p zbot-engram-adapter --locked`.
- TDD: dynamic ontology/taxonomy policy is preserved as adapter metadata and
  never hard-coded globally. Verifies AC7.
  artifact: `stores/zbot-engram-adapter/tests/knowledge_graph.rs::knowledge_mapping_preserves_dynamic_policy_metadata`;
  command: `cargo test -p zbot-engram-adapter --locked`.
- TDD: hierarchy aggregate writes preserve summary, inter-cluster relation, and
  LCA/path behavior through Engram hierarchy records plus the adapter read
  sidecar. Verifies AC7.
  artifact: `stores/zbot-engram-adapter/tests/knowledge_graph.rs::hierarchy_aggregate_summary_and_path_round_trip`;
  command: `cargo test -p zbot-engram-adapter --locked`.
- Goal-based: capability reporting can mark `Wiki`, `KnowledgeGraph`, and
  `Hierarchy` supported without enabling beliefs, recall, migration, or
  auxiliary sidecars. Verifies AC3, AC7.
  artifact: `stores/zbot-engram-adapter/tests/knowledge_graph.rs::knowledge_capabilities_can_be_enabled_independently`;
  command: `cargo test -p zbot-engram-adapter --locked`.

**Approach:**

- Use Engram knowledge and hierarchy repositories where public ports exist.
- Keep zbot-specific graph statistics/read models, alias lookup, embedding
  byte parity, wiki hybrid search, and hierarchy summary/LCA read models in an
  explicit adapter sidecar until Engram exposes generic query ports for those
  behaviors.
- T4 records one upstream Engram SQLite adapter gap: its `put_chunk` method does
  not bind the `knowledge_chunks.source_id` column required by the schema. The
  AgentZero adapter writes the same `KnowledgeChunk` contract JSON through
  Engram's public connection lock with `source_id` populated; provider cutover
  should either receive the upstream fix or keep this workaround documented.

**Done when:** knowledge graph and hierarchy can be enabled in capability
reporting without changing gateway/UI payloads.

### T5: Belief and contradiction stores map to Engram belief repositories

**Depends on:** T2, T3

**Status:** Done on 2026-07-06.

**Execution precondition:** satisfied by explicit 2026-07-06 follow-up waiver.

**Touches:** `stores/zbot-engram-adapter/src/mapping/*`,
`stores/zbot-engram-adapter/src/stores/*`,
`stores/zbot-engram-adapter/tests/*`

**Tests:**

- TDD: zbot beliefs map to Engram belief records without losing partition
  scope, subject, content, confidence, valid-time interval, source fact IDs,
  stale state, synthesizer metadata, reasoning, supersession, timestamps, or
  sidecar embedding bytes. Verifies AC8.
  artifact: `stores/zbot-engram-adapter/tests/beliefs.rs::belief_mapping_preserves_valid_time_sources_and_metadata`;
  command: `cargo test -p zbot-engram-adapter --locked`.
- TDD: `BeliefStore` upsert/get/list/search/stale/supersede/retract/source-reference
  behavior preserves zbot trait semantics through Engram belief records plus
  the adapter sidecar. Verifies AC8.
  artifact: `stores/zbot-engram-adapter/tests/beliefs.rs::belief_store_round_trip_valid_time_stale_source_and_search`;
  command: `cargo test -p zbot-engram-adapter --locked`.
- TDD: `BeliefContradictionStore` insert/list/pair-exists/resolve behavior
  preserves canonical pair ordering and idempotent duplicate handling through
  Engram contradiction records plus the adapter sidecar. Verifies AC8.
  artifact: `stores/zbot-engram-adapter/tests/beliefs.rs::contradictions_round_trip_canonicalize_and_resolve`;
  command: `cargo test -p zbot-engram-adapter --locked`.
- TDD: record-time history requests against the Engram repository return a
  named unsupported capability rather than pretending current rows are an
  audit log. Verifies AC8.
  artifact: `stores/zbot-engram-adapter/tests/beliefs.rs::record_time_history_is_explicitly_unsupported`;
  command: `cargo test -p zbot-engram-adapter --locked`.
- Goal-based: capability reporting can mark `Beliefs` and `Contradictions`
  supported without enabling recall, migration, or auxiliary sidecars. Verifies
  AC3, AC8.
  artifact: `stores/zbot-engram-adapter/tests/beliefs.rs::belief_capabilities_can_be_enabled_independently`;
  command: `cargo test -p zbot-engram-adapter --locked`.

**Approach:**

- Use Engram `BeliefRepository` for canonical belief and contradiction writes,
  lifecycle transitions, and explicit record-time rejection.
- Keep zbot-specific DTO parity, valid-time active reads, source-reference
  reads, list ordering and limits, raw little-endian embedding bytes, semantic
  search bytes, and contradiction partition join behavior in an explicit
  adapter sidecar.
- Keep record-time history disabled with a named unsupported error until a
  generic Engram bitemporal implementation or accepted sidecar lands.

**Done when:** beliefs and contradictions can be enabled in capability
reporting.

### T5b: Adapter bootstrap uses Engram's provider facade

**Depends on:** T3, T4, T5

**Status:** Done on 2026-07-06.

**Execution precondition:** satisfied by explicit 2026-07-06 follow-up waiver
after Engram implemented the Rust memory-layer tutorial/API changes.

**Touches:** `stores/zbot-engram-adapter/Cargo.toml`,
`stores/zbot-engram-adapter/src/bootstrap.rs`,
`stores/zbot-engram-adapter/src/config.rs`,
`stores/zbot-engram-adapter/src/stores/*`,
`stores/zbot-engram-adapter/tests/*`,
`docs/specs/engram-memory-engine-cutover/*`

**Tests:**

- TDD: adapter config maps to Engram `EngramConfig` with confined storage path,
  strict scope policy, dry-run migration mode, fail-closed capability policy,
  and embedding-space identity. Verifies AC1, AC2, AC3.
  artifact: `stores/zbot-engram-adapter/tests/bootstrap.rs::adapter_config_maps_to_engram_provider_config`;
  command: `cargo test -p zbot-engram-adapter --locked`.
- TDD: provider bootstrap calls Engram's `bootstrap_provider`, exposes the
  upstream capability report, and still gates AgentZero feature support through
  adapter parity capabilities. Verifies AC1, AC3, AC4.
  artifact: `stores/zbot-engram-adapter/tests/bootstrap.rs::bootstrap_uses_engram_provider_facade_and_preserves_adapter_gates`;
  command: `cargo test -p zbot-engram-adapter --locked`.
- TDD: knowledge/wiki/belief/hierarchy compatibility stores consume provider
  handles instead of constructing low-level Engram SQLite stores directly where
  the public provider now exposes the required ports. Verifies AC1, AC7, AC8.
  artifact: existing `knowledge_graph.rs` and `beliefs.rs` adapter tests;
  command: `cargo test -p zbot-engram-adapter --locked`.
- Goal-based: `cargo check -p zbot-engram-adapter --locked` and focused adapter
  tests pass without reintroducing direct bootstrap construction.

**Approach:**

- Treat `engramPath` as the confined Engram storage directory used by
  `EngramConfig::storage_path`; individual Engram DB files are created below
  that directory by upstream wiring.
- Keep AgentZero's adapter capability report separate from upstream Engram
  capability discovery: upstream says which generic Engram ports exist, adapter
  support says which zbot store-trait families have parity coverage.
- Replace direct low-level bootstrap in `bootstrap.rs` with
  `engram_conformance::bootstrap_provider`.
- Store the upstream provider in the adapter provider wrapper and hand cloned
  provider handles into compatibility store implementations.
- Keep memory repository writes on the concrete repository path only if the
  upstream `MemoryService` facade cannot expose repository-level parity methods
  cleanly in this slice; record that explicitly rather than hiding it.

**Done when:** adapter bootstrap and supported compatibility stores are wired
through the Engram provider facade without changing gateway/UI contracts.

### T5a: Recall ranking and trace parity is explicit

**Depends on:** T3, T4, T5

**Status:** Done on 2026-07-06.

**Execution precondition:** satisfied by explicit 2026-07-06 follow-up waiver
for the remaining phases.

**Touches:** `stores/zbot-engram-adapter/src/recall/*`,
`gateway/gateway-memory/src/recall/*`, `stores/zbot-engram-adapter/tests/*`

**Tests:**

- TDD: recall support reports a named retrieval-port blocker while upstream
  Engram `retrieval` remains unsupported, and `AdapterFeature::Recall` stays
  disabled. Verifies AC9.
  artifact: `stores/zbot-engram-adapter/tests/recall.rs::recall_gate_reports_retrieval_port_blocker`;
  command: `cargo test -p zbot-engram-adapter --locked`.
- TDD: recall support cannot be marked enabled unless a parity artifact records
  ordered candidates, scores, source labels, and trace fields. Verifies AC9.
  artifact: `stores/zbot-engram-adapter/tests/recall.rs::recall_gate_requires_ranking_trace_parity_artifact`;
  command: `cargo test -p zbot-engram-adapter --locked`.

**Approach:**

- Preserve zbot ranking and trace surfaces as the contract.
- Use Engram retrieval primitives when public ports exist.
- Keep candidate sidecars or waivers explicit when Engram lacks the required
  generic port.

**Done when:** recall support cannot be enabled without a ranking/trace parity
artifact.

### T6: Adapter sidecars cover zbot-only records

**Depends on:** T2

**Status:** Done on 2026-07-06.

**Execution precondition:** satisfied by explicit 2026-07-06 follow-up waiver
for the remaining phases.

**Touches:** `stores/zbot-engram-adapter/src/stores/sidecars.rs`,
`stores/zbot-engram-adapter/src/stores/*`,
`stores/zbot-engram-adapter/tests/*`

**Tests:**

- TDD: ctx facts, primitives, and skill-index rows persist through the existing
  memory sidecar rather than failing or silently no-oping in Engram mode.
  Verifies AC10.
  artifact: `stores/zbot-engram-adapter/tests/memory_sidecars.rs`;
  command: `cargo test -p zbot-engram-adapter --locked`.
- TDD: procedures, episodes, KG ingestion episodes, goals, recall logs,
  distillation runs, compaction audit rows, and outbox bridge state persist in
  adapter-owned sidecar tables with zbot trait semantics. Verifies AC10.
  artifact: `stores/zbot-engram-adapter/tests/sidecars.rs`;
  command: `cargo test -p zbot-engram-adapter --locked`.
- Goal-based: sidecar tables are created under the confined Engram storage
  directory and no sidecar API leaks into Engram crate APIs. Verifies AC10.

**Approach:**

- Implement sidecars only for product concepts that are not generic Engram
  framework concerns.
- Keep sidecar code inside the adapter crate.

**Done when:** zbot-only records have explicit durable ownership.

### T7: Migration dry-run/apply gates provider rollout

**Depends on:** T3, T4, T5, T5a, T6

**Status:** Done on 2026-07-06.

**Execution precondition:** satisfied by explicit 2026-07-06 remaining-phases
waiver.

**Touches:** `stores/zbot-engram-adapter/src/migration/*`,
`tools/*`, `docs/specs/engram-memory-engine-cutover/*`

**Tests:**

- Goal-based: dry-run migration reports deterministic diagnostics without
  writing Engram storage. Verifies AC11.
  artifact: `stores/zbot-engram-adapter/tests/migration.rs::dry_run_reports_sanitized_manifest_without_creating_engram_storage`;
  command: `cargo test -p zbot-engram-adapter --locked`.
- Goal-based: apply mode refuses to run without accepted dry-run diagnostics and
  writes only to the confined Engram path. Verifies AC13.
  artifact: `stores/zbot-engram-adapter/tests/migration.rs::apply_refuses_without_matching_accepted_manifest`,
  `stores/zbot-engram-adapter/tests/migration.rs::apply_writes_marker_under_confined_engram_path_after_acceptance`;
  command: `cargo test -p zbot-engram-adapter --locked`.
- Goal-based: apply mode rejects a stale dry-run manifest when canonical source
  DB identity, schema version, sanitized counts, adapter version, Engram
  revision, provider config, mapping config, or migration code version changes.
  Verifies AC12.
  artifact: `stores/zbot-engram-adapter/tests/migration.rs::apply_refuses_without_matching_accepted_manifest`;
  command: `cargo test -p zbot-engram-adapter --locked`.
- Goal-based: sentinel diagnostics prove public/committed errors redact paths,
  SQL internals, rows, transcripts, embeddings, connector/vault data, and
  secrets. Verifies AC14.
  artifact: `stores/zbot-engram-adapter/tests/migration.rs::dry_run_diagnostics_redact_paths_sql_and_row_content`;
  command: `cargo test -p zbot-engram-adapter --locked`.
- Goal-based: gitignored local diagnostics contain only allowlisted
  provenance/fingerprint fields and never contain secrets, private row content,
  transcript snippets, embeddings, connector/vault payloads, or API keys.
  Verifies AC14.
  artifact: `stores/zbot-engram-adapter/src/migration.rs`;
  command: `cargo clippy -p zbot-engram-adapter --all-targets --locked -- -D warnings`.
- Goal-based: committed fixtures contain no raw private DB content. Verifies
  AC9, AC10.
  artifact: synthetic SQLite fixtures only in `stores/zbot-engram-adapter/tests/migration.rs`.

**Approach:**

- Read from current SQLite providers through public repositories or approved
  migration readers.
- Emit coarse committed fixtures and gitignored exact local diagnostics.

**Done when:** apply migration is available but gated by dry-run acceptance.

### T7a: Upstream dependency checklist gates provider selection

**Depends on:** T3, T4, T5, T5a, T6, T7

**Status:** Done on 2026-07-06.

**Execution precondition:** satisfied by explicit 2026-07-06 remaining-phases
waiver.

**Touches:** `docs/specs/engram-memory-engine-cutover/*`,
`stores/zbot-engram-adapter/src/capabilities.rs`

**Tests:**

- Goal-based: an Engram dependency checklist records each required upstream
  capability as implemented, explicitly waived, or isolated in an adapter
  sidecar before any feature support flip or provider selection. Verifies AC5,
  AC9, AC18.
  artifact: `docs/specs/engram-memory-engine-cutover/dependency-checklist.md`,
  `stores/zbot-engram-adapter/tests/dependency_checklist.rs`;
  command: `cargo test -p zbot-engram-adapter --locked`.
- Goal-based: `cargo metadata`, lockfile evidence, dependency scanner evidence,
  exact Engram revision/provenance, and dirty-state policy are recorded before
  provider selection or migration apply mode. Verifies AC5.
  artifact: `docs/specs/engram-memory-engine-cutover/dependency-checklist.md`;
  command: `cargo metadata --format-version 1 --no-deps --locked`.

**Approach:**

- Track SQLite open options, read/query ports, dynamic ontology/taxonomy policy,
  memory/belief retrieval indexes, and conformance fixtures from RFC-0012.
- Keep the checklist in the spec folder until it becomes an ADR or release
  note.

**Done when:** provider selection has an auditable upstream dependency gate.

### T8: Provider selection wires through AgentZero composition roots

**Depends on:** T7, T7a

**Status:** Done on 2026-07-06.

**Execution precondition:** satisfied by explicit 2026-07-06 remaining-phases
waiver.

**Touches:** `gateway/*`, `apps/daemon/*`, `stores/*`,
`docs/specs/engram-memory-engine-cutover/*`

**Tests:**

- Goal-based integration: Engram is the default semantic memory provider and
  builds supported trait stores. Verifies AC16, AC17.
  artifact: `gateway/gateway-memory/src/lib.rs::tests::memory_provider_defaults_to_engram`,
  `gateway/src/state/persistence_factory.rs::tests::engram_provider_selection_builds_trait_bundle`,
  `gateway/src/state/mod.rs::tests::new_app_state_engram_provider_skips_sqlite_knowledge_db`;
  command: `cargo test -p gateway-memory memory_provider --locked`,
  `cargo test -p gateway persistence_factory --locked`.
- Goal-based integration: sleep-cycle workers consult capability gates and keep
  AgentZero-owned scheduling/cleanup. Verifies AC3, AC17, AC19.
  artifact: `gateway/src/state/mod.rs` routes worker construction through the
  selected trait stores and skips SQLite KG backfill in Engram mode;
  command: `cargo clippy -p gateway --all-targets --locked -- -D warnings`.
- Goal-based integration: `ProviderMode::Engram` rejects unsupported feature
  access before side effects; no current-SQLite fallback occurs unless a later
  explicit hybrid mode exists. Verifies AC3.
  artifact: `stores/zbot-engram-adapter/tests/bootstrap.rs::engram_mode_unsupported_feature_has_no_implicit_fallback`,
  `gateway/src/state/mod.rs` skips `knowledge.db` init, SQLite embedding reindex,
  and SQLite KG backfill when Engram is selected.
  command: `cargo test -p gateway new_app_state_engram_provider_skips_sqlite_knowledge_db --locked`.

**Approach:**

- Add provider construction at the existing store composition root.
- Avoid public route/DTO changes.

**Done when:** runtime composition can create a fresh Engram DB and route
semantic memory/knowledge trait stores through Engram.

### T9: Old memory-engine coupling is retired after parity

**Depends on:** T8

**Status:** Done on 2026-07-06.

**Execution precondition:** satisfied by explicit 2026-07-06 remaining-phases
waiver.

**Touches:** `stores/zbot-stores-sqlite/*`, `stores/zbot-engram-adapter/*`,
`gateway/src/state/*`, `gateway/src/http/memory.rs`, `gateway/src/http/graph.rs`,
`gateway/src/http/beliefs.rs`, `gateway/src/http/belief_network.rs`,
`gateway/src/http/hierarchy.rs`, `gateway/gateway-memory/src/recall/*`,
`gateway/gateway-memory/src/sleep/*`, `runtime/agent-tools/src/tools/memory.rs`,
`docs/specs/engram-memory-engine-cutover/*`, active architecture docs that name
the live memory provider`

**Tests:**

- Goal-based: repository search proves active memory/knowledge/belief/hierarchy
  production behavior goes through `zbot-engram-adapter` or the allowed
  non-runtime points listed below after cutover. Verifies AC18.
  artifact: repository searches listed below. Results remain in allowed
  migration/test/sidecar/legacy-store locations; gateway runtime imports only
  `zbot_engram_adapter` at the gateway composition root.
- Goal-based: targeted Rust gates pass for stores, gateway memory routes,
  services, runtime context control, and sleep-cycle jobs. Verifies AC17-AC19.
  command: `cargo check -p gateway --locked`,
  `cargo clippy -p gateway --all-targets --locked -- -D warnings`,
  `cargo clippy -p gateway-memory --all-targets --locked -- -D warnings`,
  `cargo test -p zbot-engram-adapter --locked`.
- Manual QA: fresh DB smoke covers chat, memory/knowledge activity,
  sleep-cycle cleanup, reload, and UI tabs. Verifies AC19.
  status: deferred to `docs/backlog.md#engram-fresh-db-manual-smoke`.

**Approach:**

- Remove direct old SQLite memory-engine dependencies once Engram-backed
  features are supported and provider rollout is accepted.
- Update active docs in the same change.

Allowed points after cleanup:

- Migration readers that read the old current DB as source data.
- Test fixtures comparing current provider and Engram provider.
- Adapter sidecars explicitly listed in this plan.
- SQLite conversation/execution/outbox paths that are not semantic
  memory/knowledge providers.

Repository-search checks:

- `rg -n "KnowledgeDatabase::new|MemoryRepository::new|GatewayMemoryFactStore|SqliteBeliefStore|SqliteKgStore" gateway services runtime stores -g '*.rs'`
- `rg -n "zbot_stores_sqlite::(GatewayMemoryFactStore|KnowledgeDatabase|MemoryRepository|SqliteBeliefStore)" gateway services runtime -g '*.rs'`
- `rg -n "engram_store|engram_runtime|engram_knowledge|engram_memory|engram_core" gateway services runtime -g '*.rs'` returns no production imports outside `stores/zbot-engram-adapter`.

**Done when:** the Engram provider is the memory-layer path and old direct
memory-engine coupling is gone from production startup/runtime paths, with hits
remaining only at the allowed non-runtime or non-memory SQLite points.

## Rollout

- **Delivery:** Engram is the semantic memory provider. `conversations.db`
  remains the zbot-owned SQLite runtime DB for conversation/execution/outbox
  concerns; old semantic SQLite code remains only for migration, tests, and
  legacy store crate compatibility.
- **Infrastructure:** no new external service; first provider uses local SQLite
  databases under the configured zbot data directory.
- **External-system integration:** local Engram checkout is used during
  development; a pinned upstream source is a later dependency decision.
- **Deployment sequencing:** bootstrap first, parity fixtures second, feature
  mapping third, migration tooling fourth, provider selection fifth, cleanup
  last.

## Risks

- Engram public read/query ports may not cover all zbot read models yet; the
  adapter must either keep sidecars or record upstream blockers.
- Store-trait parity may expose zbot behavior that is product-specific rather
  than generic memory-framework behavior.
- Workers could still ask for unsupported Engram retrieval/ranking ports;
  capability gates must continue to fail closed.
- Local path dependencies are acceptable for this implementation slice but need
  a pinned source decision before release/publish.

## Changelog

- 2026-07-05: initial implementation plan from RFC-0011/RFC-0012 and existing
  adapter scaffold.
- 2026-07-06: completed T2 parity harness. Store-trait seed metadata now lives
  in `zbot-stores-conformance`; `zbot-engram-adapter` has fail-closed fixture
  gates for memory facts and beliefs with explicit tenant, ward, session, and
  partition scope cases. Trusted fixture report construction remains crate/test
  internal for this slice.
- 2026-07-06: completed T3 memory fact mapping. `MemoryFact` rows now map
  loss-consciously to Engram `MemoryRecord`s; `EngramMemoryFactStore` uses
  Engram as the canonical put/get/status path and a sibling adapter sidecar for
  legacy JSON list/search filters and preserved embedding bytes.
- 2026-07-06: completed T4 wiki, knowledge graph, and hierarchy mapping.
  `EngramWikiStore` writes Engram source/document/chunk records plus a wiki
  sidecar for current read models and embeddings; `EngramKnowledgeGraphStore`
  writes Engram knowledge/hierarchy records plus a graph sidecar for aliases,
  neighbor/search/read-model parity, embeddings, and hierarchy summaries.
- 2026-07-06: completed T5 belief and contradiction mapping.
  `EngramBeliefStore` writes Engram belief/contradiction records for canonical
  lifecycle operations and keeps an adapter sidecar for zbot DTO parity, list
  ordering, embedding-byte search, source-reference lookup, and contradiction
  partition joins. Record-time history remains a named unsupported capability.
- 2026-07-06: completed T5a recall readiness gating. Recall support now reports
  a named retrieval-port blocker while upstream Engram retrieval is unsupported,
  and cannot be reported enabled without an explicit ranking/trace parity
  artifact covering ordered candidates, scores, source labels, and trace fields.
- 2026-07-06: completed T6 adapter-owned compatibility tables. Memory ctx
  facts, primitives, skill-index rows, embedding cache, strategy synthesis,
  procedures, episodes, KG ingestion episodes, goals, recall logs,
  distillation runs, compaction audit rows, and bridge outbox state stay
  adapter-owned rather than upstream Engram-owned.
- 2026-07-06: completed T7/T7a migration and dependency gates. Dry-run reports
  deterministic redacted manifests; apply writes only an accepted marker and
  refuses non-empty source rows until row import mappings exist. The dependency
  checklist records Engram capability/provenance evidence without local paths.
- 2026-07-06: completed T8/T9 provider selection and terminal cleanup. Engram
  is the default semantic memory provider, `gateway/src/state` builds
  adapter-backed trait stores, active SQLite `knowledge.db`
  initialization/reindex/backfill is skipped, and the current SQLite semantic
  memory/knowledge runtime fallback is retired.
- 2026-07-06: wired Engram's single-file SQLite layout through adapter and
  gateway memory settings. Engram mode now defaults to
  `engram/engram_data.db` for Engram core stores and adapter compatibility
  tables.
- 2026-07-08: dynamic ontology/SKOS follow-up extended migration manifests
  with path-free governance evidence: selected ontology IDs, taxonomy scheme
  IDs, validation mode, unclassified policy, SKOS expansion limits, and
  definition-content fingerprints now participate in dry-run/apply matching.
