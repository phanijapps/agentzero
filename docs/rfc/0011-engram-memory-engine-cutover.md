# RFC-0011: Engram Memory Engine Cutover

- **Status:** Draft
- **Author:** zbot maintainers
- **Approver:** TBD
- **Date opened:** 2026-07-05
- **Date closed:**
- **Related:** `stores/zbot-engram-adapter/`; `docs/rfc/0012-engram-upstream-risk-reduction.md`; `docs/research/engram-zbot-capability-gap-matrix.md`; `/home/videogamer/projects/mem-alpha/docs/specs/agentzero-engram-adapter-integration/spec.md`; `/home/videogamer/projects/mem-alpha/docs/specs/zbot-engram-belief-bitemporal-cutover/spec.md`; `/home/videogamer/projects/mem-alpha/docs/guides/tutorials/integrating-engram-rust-library.md`; `docs/specs/runtime-context-control/spec.md`; `docs/specs/rig-engine-migration/spec.md`; `docs/architecture/future-state/compaction-strategy.md`

## The ask

Approve an adapter-first cutover from zbot's current memory implementation to
Engram as the backing memory framework.

The recommendation is:

- Build `zbot-engram-adapter` as the only compatibility boundary between zbot
  store traits and Engram.
- Keep zbot's gateway routes, UI wire types, settings, current conversation DB
  contract, sleep-cycle scheduling, cleanup policy, and Observatory read models
  owned by zbot.
- Use Engram for memory, knowledgebase, graph, ontology, taxonomy, belief,
  hierarchy, retrieval, consolidation framework, storage-adapter, and audit
  primitives where they fit.
- Keep zbot-specific state in adapter-owned sidecars when Engram does not have a
  generic concept for it.
- Propose upstream Engram RFCs only for generic missing ports, not for zbot
  product vocabulary or UI contracts.

The current system already has zbot store traits, gateway/UI API contracts,
sleep-cycle workers, memory settings, and a current SQLite persistence model.
The complication is that zbot now needs to simplify its memory internals while
keeping the product contract steady. The question is whether to replace the
current backing engine directly, wait for Engram to absorb every zbot concept,
or cut over through a strict adapter boundary.

Decisions requested:

1. Make `zbot-engram-adapter` the cutover boundary.
   Recommended: accept. Default if no objection by 2026-07-12: implement Engram
   behind zbot store traits and keep existing gateway/UI contracts stable.
2. Keep zbot as the sleep-cycle and cleanup owner.
   Recommended: accept. Default if no objection by 2026-07-12: zbot owns worker
   scheduling, cleanup policy, concrete jobs, model/provider choices, settings,
   manual triggers, and Observatory reporting; Engram provides framework ports
   and primitives below those jobs.
3. Preserve current DB-derived contract parity during migration.
   Recommended: accept. Default if no objection by 2026-07-12: use the current
   DB as the reference fixture source, allow synthetic fixtures where needed,
   and require dry-run/apply migration diagnostics before enabling Engram. The
   Engram-backed knowledgebase DB may have a different schema and physical
   layout; parity is measured at zbot contracts, not old table shape.
4. Gate upstream Engram changes behind adapter spikes.
   Recommended: accept. Default if no objection by 2026-07-12: open Engram RFCs
   only for generic missing capability such as memory query ports, graph
   read-model ports, procedure memory, belief vector retrieval, storage options,
   or record-time bitemporality.

## Problem & goals

zbot's memory layer has grown into a product-specific integration surface:

- Gateway HTTP endpoints and TypeScript consumers expect stable memory, graph,
  belief, hierarchy, ward, Observatory, and settings shapes.
- Runtime tools and gateway execution call zbot store traits directly.
- Sleep-cycle jobs own cleanup work: compaction, conflict resolution, decay,
  pruning, archival, synthesis, pattern extraction, belief refresh,
  contradiction propagation, hierarchy build, and reporting.
- Current persistence includes product concepts that are not necessarily Engram
  domain concepts: wards, ctx facts, skill index rows, recall logs,
  distillation runs, outbox state, UI-oriented stats, dynamic
  ontology/taxonomy choices, and migration diagnostics.

Engram now has enough memory-framework surface to be useful: memory records and
events, knowledge and graph repositories, ontology and taxonomy repositories,
retrieval fusion, belief repositories, hierarchy repositories, consolidation
run/gating concepts, SQLite adapters, and valid-time belief reads. It is still
not a direct implementation of the full zbot store-trait surface.

Goals:

- Replace the backing memory framework without breaking zbot's public gateway,
  UI, settings, or event contracts.
- Allow the backing knowledgebase DB to change shape under Engram while
  preserving zbot's API, recall, migration, and observability contracts.
- Let zbot provide dynamic ontology and taxonomy policy because zbot is a
  multi-domain tool, not a single fixed knowledge application.
- Keep zbot sleep-cycle cleanup behavior in zbot.
- Use Engram where it is generic framework code, not where zbot needs product
  policy or UI read models.
- Keep migration reversible by configuration until parity is proven.
- Make unsupported areas fail closed through capability reporting.
- Produce upstream Engram RFCs only when the gap is genuinely framework-level.
- Remove accidental tight coupling to the current zbot SQLite implementation
  after the adapter reaches parity.

Non-goals:

- No change to gateway route paths, request/response DTOs, WebSocket events, or
  UI transport types in this RFC.
- No requirement that the Engram-backed knowledgebase DB preserve the current
  zbot SQLite table layout.
- No move of sleep-cycle scheduling, operator settings, model/provider routing,
  manual triggers, or zbot cleanup policy into Engram.
- No import of zbot product terms such as wards, Observatory DTOs, or gateway
  route names into Engram domain truth.
- No hard migration that destroys the current DB before dry-run parity reports
  are accepted.
- No live bidirectional sync or automatic dual-write unless a later spec
  explicitly approves it.
- No claim of full bitemporality until both valid-time and record-time query
  behavior are implemented and tested.

## Proposal

### 1. Adapter-first architecture

`stores/zbot-engram-adapter` becomes the provider selected by the zbot
composition root when Engram mode is enabled. Its public obligation is zbot's
existing store traits, not Engram's internal shape.

The adapter should be split by reason to change:

- `factory` or `bootstrap`: construct Engram services/stores and adapter
  sidecars.
- `mapping`: memory, knowledge, belief, contradiction, hierarchy, procedure,
  wiki, episode, scope, and temporal conversions.
- `stores`: one implementation module per zbot store trait.
- `recall`: preserve zbot ranking behavior over Engram retrieval primitives.
- `observability`: preserve zbot stats, health, and Observatory read models.
- `migration`: dry-run and apply import tooling from the current zbot DB.
- `fixtures`: parity snapshots for trait calls and API journeys.
- `capabilities`: fail closed until each feature passes parity.

The existing adapter scaffold already has the right starter boundaries:
provider mode, Engram path, tenant, scope mapping, embedding mode, migration
mode, and a conservative capability report.

### 2. Sleep-cycle ownership boundary

zbot remains the runtime owner for cleanup and sleep-cycle behavior. This means
zbot owns:

- worker scheduling and intervals
- operator settings and feature flags
- model/provider selection for maintenance jobs
- manual triggers
- rollout and capability gating
- concrete cleanup jobs
- Observatory reporting and UI read models

Engram provides the framework below those jobs:

- domain contracts and repositories
- storage adapters
- retrieval/fusion primitives
- consolidation run and gating concepts
- belief and hierarchy ports
- generic audit surfaces where useful

The boundary is intentional: zbot is the product host; Engram is the memory
framework; the adapter translates between them.

### 3. Knowledgebase, ontology, and taxonomy

The knowledgebase DB is expected to change under Engram. The old zbot database
is a migration and parity reference, not the target schema. Engram may store
knowledge sources, documents, chunks, entities, relationships, ontologies, and
taxonomies using its own repository and adapter layout.

zbot supplies the knowledge policy above that storage:

- dynamic ontology selection and evolution per ward, project, domain, or
  session context
- dynamic taxonomy and controlled-vocabulary hints for broad "swiss army knife"
  use cases
- source classification, ward/project identity, and product-level visibility
  rules
- API/read-model translation for graph, memory, ward content, Observatory, and
  recall surfaces

Engram should provide generic ontology and taxonomy repositories, validation,
and graph/knowledge ports. zbot decides which ontology/taxonomy applies for a
given task and maps that policy into Engram calls through the adapter.

### 4. Migration and parity

The current zbot DB is the reference source for e2e parity. Where the current
DB lacks a case, synthetic fixtures may be generated from the same contract
shape.

The migration path is:

1. Add fixture harnesses that run the same zbot store-trait calls against the
   current provider and the Engram provider.
2. Implement the adapter feature-by-feature, keeping unsupported features
   disabled in startup capability reports.
3. Add dry-run migration that reports row counts, unsupported mappings, scope
   translations, ID translations, knowledgebase schema translations, ontology
   and taxonomy assignments, embedding handling, and validation errors.
4. Add apply migration only after dry-run reports are deterministic.
5. Enable Engram by configuration for manual testing.
6. Remove old memory-engine coupling only after parity fixtures and route/API
   journeys pass.

Provider rollout remains reversible by configuration until a later migration
spec approves an irreversible cutover.

### 5. Adapter-owned sidecars

Some zbot behavior is not Engram framework behavior. These stay in adapter or
zbot-owned sidecars unless a later Engram RFC accepts a generic abstraction:

- ctx facts and subagent handoff state
- skill index rows
- embedding byte parity and embedding cache
- recall logs, goals, and distillation lifecycle
- outbox/conversation bridge state
- Observatory aggregate read models
- migration diagnostics
- dynamic ontology/taxonomy policy cache if it is zbot-specific rather than a
  durable Engram ontology/taxonomy record
- zbot-specific procedure execution counters if Engram does not accept a
  generic procedure contract

### 6. Upstream Engram RFC gates

The adapter spike decides whether a gap is generic enough for Engram. Current
candidates are:

- storage open options and tutorial cleanup
- generic memory query/list/filter ports
- generic graph read-model/stat ports
- generic ontology/taxonomy policy hooks if adapter spikes show zbot cannot
  express dynamic multi-domain policy through existing Engram ports
- first-class procedure memory
- record-time bitemporal belief history
- belief vector retrieval
- generic consolidation audit details

If a gap can be solved without reaching into Engram internals and without
corrupting zbot semantics, keep it in the adapter.

## Options considered

The option space is MECE along the control-plane boundary: either zbot keeps the
current engine, zbot directly replaces it, zbot uses an adapter boundary, or
Engram absorbs zbot semantics before cutover.

| Option | Description | Trade-offs |
| --- | --- | --- |
| Do nothing | Keep current zbot memory implementation and defer Engram. | Lowest immediate risk, but keeps existing complexity and delays simplification. Engram improvements do not reduce zbot maintenance cost. |
| Direct replacement | Rewrite zbot to call Engram APIs directly from gateway/runtime/sleep code. | Fewer adapter layers, but high blast radius. It risks leaking Engram shapes into gateway/UI contracts and zbot product policy into Engram. |
| Adapter-first cutover | Implement Engram behind zbot store traits and preserve existing gateway/UI/sleep contracts. | Adds adapter code, but gives reversible migration, parity fixtures, and a clean framework/product boundary. This is the recommended option. |
| Upstream-first | Pause zbot migration until Engram grows every missing zbot-compatible port. | Produces cleaner Engram primitives where gaps are generic, but blocks zbot simplification and pressures Engram to absorb product-specific concepts. |

The recommended option follows ports-and-adapters prior art: keep the zbot
application contract stable and swap the backing technology through adapters.
It also follows incremental modernization prior art: replace behavior
piece-by-piece with visible parity gates instead of a large cutover.

## Risks & what would make this wrong

Pre-mortem:

- The adapter becomes a god module. Mitigation: enforce module split by store
  trait and reason to change.
- zbot product semantics leak into Engram. Mitigation: require separate Engram
  RFCs for core changes and reject zbot UI/product terms in Engram domain
  contracts.
- Ranking parity silently changes recall quality. Mitigation: preserve zbot
  ranking until before/after fixtures approve a ranking migration.
- Migration drops data or changes identity semantics. Mitigation: dry-run first,
  row-count diagnostics, ID translation reports, and reversible provider flag.
- Sleep-cycle behavior changes because Engram consolidation concepts are treated
  as zbot scheduling. Mitigation: zbot owns workers and calls Engram primitives
  through adapter/provider traits.
- Adapter sidecars become a second source of truth. Mitigation: sidecars may
  store compatibility/read-model/operational data, but canonical memory,
  knowledgebase, graph, ontology, taxonomy, belief, and hierarchy records live
  in Engram once the provider is active.

Falsifiable assumptions:

- Engram's current repositories can represent core zbot memory, knowledge,
  graph, ontology, taxonomy, belief, hierarchy, and retrieval concepts without
  schema corruption.
- zbot's public API contracts can remain stable while the backing provider
  changes.
- zbot can supply dynamic ontology and taxonomy policy at the adapter boundary
  without making Engram depend on zbot product vocabulary.
- Current DB-derived fixtures are sufficient to detect the important contract
  regressions.
- zbot does not require record-time bitemporal belief history for initial
  Engram cutover.
- Procedure behavior can initially be preserved in adapter metadata or sidecars
  without making procedure memory first-class in Engram.

Drawbacks:

- The adapter is real code and must be maintained.
- Some data may exist in sidecars until Engram grows generic ports.
- The first cutover will not delete all old storage concepts immediately.
- Strict parity slows down the migration, but it prevents hidden product
  regressions.

This RFC is wrong if zbot is willing to change its gateway/UI/settings contracts
as part of the memory migration, or if Engram intentionally becomes the zbot
product runtime rather than a framework.

## Evidence & prior art

Spike result:

- A local-source-only comparison was captured in
  `docs/research/engram-zbot-capability-gap-matrix.md`.
- zbot has store traits for memory facts, conversations, episodes, wiki,
  procedures, compaction, outbox, goals, recall logs, distillation, beliefs,
  and contradictions.
- zbot has graph store behavior with CRUD, traversal, stats, UI read models,
  vector-index health, maintenance hooks, hierarchy promotion, LCA recall, and
  hierarchy summaries.
- Engram has memory, knowledge, retrieval, belief, hierarchy, consolidation,
  ontology, taxonomy, SQLite adapter, and Node binding surfaces.
- Engram belief code now supports valid-time filtering and explicitly rejects
  record-time history.
- Current local Engram source exposes `open_with_options` on the memory,
  knowledge, belief, and hierarchy SQLite adapters and uses
  `engram-store-belief-sqlite`; the remaining gate is revision/provenance
  pinning before provider selection or migration apply.

Repo precedent:

- `docs/CONVENTIONS.md` defines RFCs as governance for significant changes and
  specs as implementation contracts.
- `docs/specs/runtime-context-control/spec.md` protects durable memory
  boundaries and warns against adding a new durable memory source by accident.
- `docs/specs/rig-engine-migration/spec.md` preserves gateway contracts, store
  traits, and durable memory behavior during a framework migration.
- `docs/architecture/future-state/compaction-strategy.md` states that durable
  memory needs one authoritative store and that sleep-cycle hygiene writes
  through existing store/repository abstractions.
- `stores/zbot-engram-adapter/src/lib.rs` already describes the intended
  division: zbot owns gateway routes, UI DTOs, settings, and sleep-cycle
  scheduling while Engram stays behind the adapter.

External prior art:

- Alistair Cockburn's ports-and-adapters architecture frames adapters as the
  technology-specific translators around application ports, keeping application
  logic from depending on external technology details:
  [Hexagonal Architecture](https://alistair.cockburn.us/hexagonal-architecture).
- AWS Prescriptive Guidance describes the same pattern as a way to reduce data
  store and UI lock-in, with application components communicating through
  technology-agnostic ports:
  [Hexagonal architecture pattern](https://docs.aws.amazon.com/prescriptive-guidance/latest/cloud-design-patterns/hexagonal-architecture.html).
- Martin Fowler's Strangler Fig guidance supports gradual, visible replacement
  over risky big-bang modernization:
  [Strangler Fig Application](https://martinfowler.com/bliki/StranglerFigApplication.html).
- The Rust RFC process treats substantial interface changes as RFC-worthy and
  separates agreement from implementation, matching this repo's governance
  model:
  [RFC 0002](https://rust-lang.github.io/rfcs/0002-rfc-process.html).

## Experiment / validation

Hypothesis: zbot can switch its backing memory provider to Engram without
changing gateway/UI/settings contracts if the cutover is implemented through
`zbot-engram-adapter` and guarded by parity fixtures.

What we measure:

- Store-trait parity for memory facts, graph, wiki, episodes, procedures,
  compaction, beliefs, contradictions, auxiliary stores, and recall behavior.
- API journey parity for `/api/memory*`, `/api/graph*`, `/api/wards*`,
  `/api/beliefs*`, `/api/contradictions*`, `/api/belief-network/*`,
  `/api/hierarchy/stats`, and settings routes.
- Migration dry-run determinism: row counts, unsupported mappings, scope
  translations, ID translations, knowledgebase schema translations, ontology
  and taxonomy mappings, embedding handling, and validation errors.
- Recall ranking deltas against accepted before/after snapshots.
- Sleep-cycle job behavior under Engram provider with zbot still owning
  scheduling and cleanup policy.

Success criteria:

- Engram provider can be selected by configuration.
- Unsupported features fail closed through capability reports.
- Current-provider and Engram-provider parity fixtures match for accepted
  contract surfaces.
- Manual testing can use a new Engram-backed DB without requiring changes to
  gateway/UI contracts.
- The Engram-backed knowledgebase DB may differ physically from the old DB while
  zbot API/read-model parity remains intact.

Failure criteria:

- The adapter must reach into private Engram SQLite internals for core behavior
  that should be a framework port.
- zbot public DTOs must change to make Engram work.
- Sleep-cycle behavior must move into Engram to preserve current behavior.
- Migration cannot produce deterministic dry-run diagnostics.
- zbot cannot express dynamic ontology/taxonomy selection without hardcoding
  zbot concepts into Engram.

## Open questions

1. Which missing read/query ports must move upstream to Engram?
   Recommended default: keep them in the adapter until a parity fixture proves
   that the adapter would need private Engram internals. Owner: zbot memory
   maintainer. Decide-by: before enabling the Engram provider flag.
2. Does initial cutover require record-time bitemporal belief history?
   Recommended default: no; valid-time behavior is enough for initial cutover,
   and record-time history becomes a separate Engram RFC only if belief parity
   proves it is required. Owner: zbot memory maintainer. Decide-by: before
   marking belief parity complete.
3. Should procedures become first-class Engram memory?
   Recommended default: preserve procedure behavior in the adapter first, then
   propose an Engram procedure-memory RFC only if the shape is reusable beyond
   zbot. Owner: zbot/Engram maintainer. Decide-by: before removing the current
   procedure store implementation.

4. How much dynamic ontology/taxonomy policy belongs in durable Engram records
   versus zbot adapter policy?
   Recommended default: durable, reusable ontology/taxonomy definitions belong
   in Engram; selection, defaulting, and task-specific policy stay in zbot until
   a generic Engram policy hook is proven necessary. Owner: zbot/Engram
   maintainer. Decide-by: before enabling knowledgebase migration apply mode.

## Follow-on artifacts

- ADR: record Engram as zbot's backing memory framework and zbot as the product
  host/sleep-cycle owner.
- Spec: `docs/specs/engram-memory-engine-cutover/` for the zbot-side
  implementation plan and parity gates.
- Spec updates: link this RFC from Engram's
  `agentzero-engram-adapter-integration` and `zbot-engram-belief-bitemporal`
  specs.
- Optional Engram RFCs: storage open options/docs, generic memory query ports,
  graph read-model/stat ports, dynamic ontology/taxonomy policy hooks,
  procedure memory, record-time bitemporality, belief vector retrieval, and
  consolidation audit details.
