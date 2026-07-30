# Spec: Engram Memory Engine Cutover

- **Status:** Shipped
- **Owner:** phanijapps
- **Plan:** [`plan.md`](plan.md)
- **Constrained by:** [`RFC-0011`](../../rfc/0011-engram-memory-engine-cutover.md); [`RFC-0012`](../../rfc/0012-engram-upstream-risk-reduction.md); [`rig-engine-migration`](../rig-engine-migration/spec.md); [`runtime-context-control`](../runtime-context-control/spec.md)
- **Brief:** none
- **Contract:** none; preserves existing AgentZero store traits, gateway routes, UI DTOs, WebSocket events, memory settings, and Observatory read models rather than introducing a new public contract.
- **Shape:** mixed
- **Implementation waiver:** user authorized the Engram memory-layer implementation on 2026-07-05 while RFC-0011 and RFC-0012 remain Draft; that waiver covered T1-T2 adapter construction and parity-harness preparation. On 2026-07-06 the user explicitly authorized T3 memory-fact mapping/support work, T4 knowledge/graph/hierarchy mapping/support work, T5 belief/contradiction mapping/support work, T5a recall readiness gating, T6 adapter-owned sidecars, and then the remaining T7-T9 migration/provider-selection/cleanup phases.

> **Spec contract:** this document defines what "done" means. The implementing
> PR must match this spec, or update it. Verification must be derivable from it.

## Objective

Switch AgentZero's durable memory, knowledge, graph, belief, hierarchy, and
recall backing framework to Engram through `stores/zbot-engram-adapter` while
preserving AgentZero's public product contract. Success means gateway routes,
UI DTOs, settings files, sleep-cycle jobs, `conversations.db`, and Observatory
surfaces keep their existing shapes while Engram is the runtime semantic
memory provider, the old SQLite semantic memory/knowledge fallback is retired,
and migration/parity tooling can still use the old DB shape as reference data.

## Boundaries

The three-tier guard that keeps an implementing agent inside the lines.
*Always do* applies without asking; *Ask first* requires human sign-off before
proceeding; *Never do* is a hard rule, even under time pressure.

### Always do

- Keep `zbot-engram-adapter` as the only compatibility boundary between
  AgentZero store traits and Engram crates.
- Keep gateway routes, UI DTOs, WebSocket event shapes, memory settings, sleep
  workers, cleanup policy, and Observatory read models owned by AgentZero.
- Treat the current zbot SQLite DB as migration/parity reference data, not as
  the target Engram schema.
- Keep unsupported adapter features fail-closed in startup capability reports
  until parity fixtures prove the feature.
- In `ProviderMode::Engram`, reject unsupported feature access before side
  effects; hybrid fallback requires a separate explicit mode and tests.
- Use Engram public services/repositories/open options; record any temporary
  private-helper dependency as a blocker or waiver before provider cutover.
- Preserve dynamic ontology/taxonomy selection in AgentZero policy and pass the
  selected policy into Engram through the adapter.
- Confine configured Engram database paths to a trusted zbot data root injected
  by the composition layer, not deserialized from user settings, by resolving
  the root, database parent, and any existing DB file before opening SQLite.

### Ask first

- Changing public gateway memory, graph, belief, hierarchy, ward, Observatory,
  or settings payload shapes to match Engram internals.
- Replacing AgentZero sleep-cycle scheduling or cleanup ownership with Engram
  orchestration.
- Enabling apply-mode migration or Engram provider selection without dry-run
  diagnostics and parity fixtures.
- Reintroducing a current-SQLite semantic memory/knowledge runtime fallback.
- Introducing live dual-write, bidirectional sync, or destructive data
  migration.

### Never do

- Never make Engram depend on zbot product vocabulary such as gateway route
  names, ward DTOs, Observatory DTOs, or UI read models.
- Never require the Engram-backed knowledgebase DB to preserve the old zbot
  table layout.
- Never use `conversations.db` as durable semantic memory or as the target
  knowledgebase.
- Never commit real current DB content, raw private transcripts, embeddings,
  connector data, vault data, API keys, or exact local DB provenance.
- Never silently widen scope when translating AgentZero ward/session/partition
  identifiers into Engram `Scope`.
- Never mark an adapter feature supported until the matching store-trait parity
  fixture and focused adapter tests pass.
- Never expose raw absolute paths, SQL internals, private row contents,
  transcript snippets, embeddings, connector/vault data, or secrets in public
  capability reports, API errors, UI errors, or committed diagnostics.

## Testing Strategy

- Adapter configuration, scope mapping, feature gating, and store construction:
  **TDD** because each has compact invariants and failure modes.
- Store-trait compatibility for memory facts, wiki/knowledge graph, beliefs,
  contradictions, hierarchy, procedures, episodes, outbox, and auxiliary
  stores: **TDD plus conformance tests** against the existing trait surfaces.
- Migration tooling: **goal-based integration** using sanitized current-DB
  reference data and synthetic fixtures; committed artifacts must be
  positive-allowlist sanitized.
- Provider rollout and gateway/UI behavior: **goal-based integration plus manual
  QA**. Existing routes and UI read models are verified through current tests;
  final manual smoke uses a fresh Engram DB created by the user.
- Cleanup of old memory-engine coupling: **goal-based repository check** proving
  active production code no longer reaches through old SQLite internals for
  memory/knowledge behavior once Engram parity is accepted.

Protected public contract matrix:

| Surface | Protected artifacts | Verification |
| --- | --- | --- |
| Memory HTTP/API | `gateway/src/http/memory.rs`, `gateway/src/http/memory_search.rs`, `apps/ui/src/services/transport/http.ts`, `apps/ui/src/features/memory/command-deck/*` | Existing HTTP tests plus memory command-deck tests continue to pass; no path or DTO rename without spec update. |
| Graph, hierarchy, and Observatory | `gateway/src/http/graph.rs`, `gateway/src/http/hierarchy.rs`, `gateway/src/http/belief_network.rs`, `apps/ui/src/features/observatory/*`, `apps/ui/src/features/observatory-v2/*` | Existing Observatory, graph hook, hierarchy, and belief-network tests continue to pass; Graph/Observatory routes keep the same payload shapes. |
| Beliefs and contradictions | `gateway/src/http/beliefs.rs`, `apps/ui/src/features/observatory/belief-network/*`, `apps/ui/src/features/memory/command-deck/types.beliefs.ts` | Existing belief HTTP and UI tests continue to pass; unsupported record-time behavior fails closed rather than shape-shifting responses. |
| Settings and embeddings | `gateway/src/http/settings.rs`, `gateway/src/http/embeddings.rs`, `apps/ui/src/features/settings/*` | Existing settings/embedding tests continue to pass; Engram provider config is additive and does not rename current settings. |
| Wards, Vault, and Research ward context | `gateway/src/http/ward_content.rs`, `gateway/src/http/ward_actions.rs`, `gateway/src/http/ward_usage.rs`, `gateway/src/http/vault.rs`, `apps/ui/src/services/transport/http.ts`, `apps/ui/src/features/memory/command-deck/*`, `apps/ui/src/features/vault/*`, `apps/ui/src/features/research-v2/*` | Existing ward content, vault ward, memory ward rail, chat/research ward event, and transport tests continue to pass; `/api/wards*` and `/api/vault/wards*` keep their route and payload shapes. |
| Sleep-cycle cleanup | `gateway/gateway-memory/src/sleep/*`, `gateway/gateway-execution/src/sleep/*` | Worker tests prove unsupported Engram capabilities do not start dependent cleanup jobs in Engram mode. |
| Runtime events and recall observability | `gateway/gateway-events`, `gateway/gateway-ws-protocol`, `gateway/gateway-memory/src/recall/*`, `apps/ui/src/features/observatory-v2/useRecallTrace.ts` | Existing event mapping and recall trace tests continue to pass; Engram recall preserves zbot trace/log surfaces. |

## Acceptance Criteria

- [x] `zbot-engram-adapter` depends on local Engram crates through one
  manifest boundary and constructs Engram storage through the public provider
  facade (`EngramConfig` + `bootstrap_provider`) rather than direct low-level
  SQLite store bootstrap.
- [x] Engram mode requires a trusted, composition-injected zbot data root and a
  configured Engram DB path; user-deserialized settings cannot redefine the
  data root, relative DB paths resolve under the data root, absolute DB paths
  are accepted only when they canonicalize under that root, symlink escapes and
  ambiguous roots are rejected, and only the confined path is passed to
  `SqliteOpenOptions`.
- [x] Engram provider startup validates config, opens only through the adapter
  bootstrap path, reports unsupported features fail-closed, rejects unsupported
  feature access before side effects, and never starts sleep-cycle jobs for
  unsupported capabilities.
- [x] Store-trait parity fixtures can run the same accepted calls against the
  current SQLite provider and the Engram provider for every feature the adapter
  reports as supported.
- [ ] Engram dependency source is pinned before release/publish. (deferred: engram-pinned-source-before-release)
  The final pin must use the sanctioned dependency mechanism and include exact
  source revision/provenance, dirty-state policy, `cargo metadata` evidence,
  lockfile evidence, and dependency scanner evidence; mutable local path
  dependencies remain a local implementation waiver only.
- [x] Memory fact read/write/list/recall behavior is backed by Engram memory
  records or an explicitly documented adapter sidecar while preserving
  `MemoryFactStore` semantics.
- [x] Wiki, knowledge graph, ontology, taxonomy, and hierarchy read/write
  behavior is backed by Engram knowledge and hierarchy repositories while
  preserving existing gateway/API read models.
- [x] Belief and contradiction behavior is backed by Engram belief repositories
  such that valid-time `as_of` filters, stale mark/clear/list, source-reference
  round trips, contradiction list/resolve, and active-belief reads have parity;
  record-time history queries return a named unsupported capability until a
  generic Engram implementation or adapter sidecar is accepted.
- [x] Recall preserves zbot ranking and observability contracts while producing
  candidates from Engram retrieval primitives where Engram has public ports, or
  records an explicit retrieval-port waiver/sidecar decision before capability
  support is enabled.
- [x] Procedures, episodes, ctx facts, skill index rows, recall logs, goals,
  distillation records, outbox state, and migration diagnostics either map to
  generic Engram concepts or stay in adapter-owned sidecars with tests.
- [x] Dry-run migration reports row counts, unsupported mappings, scope/ID
  translations, knowledgebase schema translation, ontology/taxonomy choices,
  embedding handling, validation failures, and capability blockers without
  writing Engram storage.
- [x] Dry-run artifacts carry a gitignored fingerprint over canonical source DB
  identity, schema version, sanitized counts, adapter version, Engram revision,
  provider config, scope/ontology/taxonomy mapping config, and migration code
  version; apply mode fails unless the accepted manifest matches exactly.
- [x] Apply migration is disabled until dry-run reports are deterministic and
  accepted; apply mode writes only to the confined configured Engram DB path.
- [x] Public and committed diagnostics use redacted error codes/messages and
  positive-allowlist fields; raw local paths, SQL internals, private row data,
  transcript snippets, embeddings, connector/vault data, and secrets are never
  written to public, committed, or gitignored diagnostics. Gitignored local
  diagnostics may contain only positive-allowlist provenance and fingerprints
  required to bind dry-run to apply.
- [x] Scope tests prove tenant, ward, session, and partition identity survive
  read/write/search/migration translation and that no query returns neighboring
  scope data.
- [x] Terminal cleanup removes the current SQLite semantic memory/knowledge
  fallback from production startup; `conversations.db` remains the zbot-owned
  SQLite runtime DB for conversation/execution/outbox concerns.
- [x] Existing gateway/UI/settings/events/Observatory contracts pass without
  public shape changes when the Engram provider is selected for supported
  features.
- [x] Old memory-engine coupling is removed after parity: active
  memory/knowledge/belief/hierarchy behavior flows through
  `zbot-engram-adapter` rather than directly through old SQLite repositories.
  The only allowed SQLite remnants are conversation/execution/outbox concerns,
  migration readers, and tests.
- [ ] Fresh DB manual smoke passes. (deferred: engram-fresh-db-manual-smoke)
  The run must start the daemon/UI or CLI, invoke an agent, record
  memory/knowledge activity, run sleep-cycle cleanup owned by AgentZero, reload
  the session, and show Memory, Graph, and Observatory tabs.

## Assumptions

- Technical: the current workspace already contains `stores/zbot-engram-adapter`
  and includes it in the root Cargo workspace (source: `Cargo.toml`;
  `stores/zbot-engram-adapter/src/lib.rs`).
- Technical: the public Engram repository at
  `https://github.com/phanijapps/engram` provides SQLite open options and
  SQLite stores for memory, knowledge, belief, and hierarchy (source: the
  Engram adapter manifests and the revision recorded in `Cargo.lock`).
- Technical: Rig is now the execution framework, but the Rig migration spec
  intentionally left memory/knowledge ownership in AgentZero; this spec changes
  that backing framework through a separate adapter boundary (source:
  `docs/specs/rig-engine-migration/spec.md`).
- Technical: AgentZero store traits remain the product compatibility surface
  for memory-layer migration (source: `stores/zbot-stores-traits/src/*`;
  `stores/zbot-stores/src/*`).
- Process: significant concrete work after accepted RFC direction gets its own
  spec under `docs/specs/` and should preserve linked RFC constraints (source:
  `docs/CONVENTIONS.md`; `docs/rfc/0011-engram-memory-engine-cutover.md`).
- Product: AgentZero keeps gateway/UI/settings/current conversation DB
  contract, sleep-cycle scheduling, cleanup, and Observatory behavior while
  Engram provides reusable memory framework primitives (source: user direction
  in this thread and `docs/rfc/0011-engram-memory-engine-cutover.md`).
