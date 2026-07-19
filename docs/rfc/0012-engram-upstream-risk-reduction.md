# RFC-0012: Engram Upstream Risk Reduction

- **Status:** Draft
- **Author:** zbot maintainers
- **Approver:** TBD
- **Date opened:** 2026-07-05
- **Date closed:**
- **Related:** `docs/rfc/0011-engram-memory-engine-cutover.md`; `docs/research/engram-zbot-capability-gap-matrix.md`; `/home/videogamer/projects/mem-alpha/docs/rfcs/`; `/home/videogamer/projects/mem-alpha/docs/specs/agentzero-engram-adapter-integration/spec.md`

## The ask

Approve a small set of generic upstream Engram implementation dependencies that
make the AgentZero memory-engine cutover less risky.

This RFC is about what Engram should expose as a reusable framework. It does
not define AgentZero adapter internals, gateway routes, UI contracts, sleep
workers, product settings, migration UI, or zbot-specific read models. Those
remain AgentZero work under RFC-0011 and follow-on specs.

Recommended upstream Engram dependencies:

1. **SQLite open options.**
   Engram SQLite adapters expose explicit open options for file/in-memory paths,
   WAL mode, busy timeout, foreign keys, migrations, and parent-directory
   creation.
2. **Scoped read/query ports.**
   Engram exposes storage-neutral read/query/list/stat ports for memory,
   knowledge, graph, ontology, taxonomy, and belief records so AgentZero does
   not depend on private SQLite helper methods.
3. **Dynamic ontology/taxonomy selection.**
   Engram accepts caller-supplied ontology/taxonomy selection and validation
   policy at ingestion, retrieval, validation, and consolidation boundaries.
4. **Durable memory and belief retrieval indexes.**
   Engram provides `RetrievalIndex` implementations for memory and belief
   records so AgentZero recall can compose them through Engram's existing
   retrieval seam.
5. **Cross-adapter conformance fixtures.**
   Engram ships reusable fixtures for the generic ports above so SQLite and
   future backends can prove behavior without AgentZero-specific tests.

Decisions requested:

1. Track these as upstream Engram dependencies for the AgentZero cutover.
   Recommended: accept. Default if no objection by 2026-07-12: AgentZero can
   build adapter spikes against Engram, but production cutover gates on the
   relevant upstream dependency being implemented or explicitly waived.
2. Treat only generic Engram framework work as upstream.
   Recommended: accept. Default if no objection by 2026-07-12: any
   zbot-specific compatibility behavior stays in AgentZero.
3. Let implemented upstream slices close independently.
   Recommended: accept. Default if no objection by 2026-07-12: if Engram has
   already shipped a dependency, AgentZero records it as satisfied and does not
   duplicate it.

## Problem & goals

The AgentZero migration should not force Engram to become a zbot product
runtime. At the same time, AgentZero should not cut over by reaching into
private Engram SQLite details or by reimplementing generic memory-framework
behavior locally.

The risk is the middle layer: if Engram lacks generic public ports for common
integration work, the AgentZero adapter either becomes too large or depends on
adapter internals that will break as Engram evolves.

Goals:

- Keep Engram work generic and reusable by other hosts.
- Keep zbot-specific contracts and compatibility code in AgentZero.
- Reduce AgentZero migration risk before enabling an Engram-backed provider.
- Preserve Engram's freedom to change its physical database layout.
- Avoid relying on private SQLite adapter helpers for generic behavior.
- Give AgentZero a clear dependency checklist for cutover planning.

Non-goals:

- No AgentZero HTTP routes, DTOs, UI read models, or settings in Engram.
- No zbot sleep-cycle scheduling, cleanup policy, manual triggers, or
  Observatory behavior in Engram.
- No requirement that Engram preserve the old AgentZero knowledgebase schema.
- No product-specific ontology/taxonomy defaults in Engram.
- No record-time bitemporal history unless a later generic Engram RFC accepts
  it.

## Proposal

### 1. SQLite open options

Engram should provide one explicit construction model across SQLite adapters:

- path is file-backed or in-memory
- parent directory creation is explicit
- WAL/journal mode is explicit
- busy timeout is explicit
- foreign-key behavior is explicit
- migration execution is explicit

Current local Engram evidence: `SqliteOpenOptions`, `SqlitePath`, and
`SqliteJournalMode` exist in `engram-runtime`, and memory, knowledge, belief,
and hierarchy SQLite adapters expose `open_with_options`. A narrow verification
passed with:

```bash
cargo check -p engram-runtime -p engram-store-sql \
  -p engram-store-knowledge-sqlite \
  -p engram-store-belief-sqlite \
  -p engram-store-hierarchy-sqlite
```

AgentZero should treat this dependency as satisfied once it pins an Engram
revision containing that implementation.

### 2. Scoped read/query ports

Engram should expose generic read/query ports rather than forcing hosts to use
store-specific helper methods.

Expected generic surfaces:

- memory record list/count/query by scope, status, kind, valid time,
  confidence, source, and policy visibility
- knowledge source/document/chunk/entity/relationship list/count/query by
  scope, source, graph, kind, concept, and pagination
- graph list/neighbors/stats surfaces that do not expose SQL table shape
- ontology and taxonomy list/query surfaces
- belief and contradiction list/query surfaces by scope, subject, status,
  valid time, stale state, source reference, and resolution state

AgentZero adapter code may temporarily spike against available Engram helper
methods, but production cutover should prefer public ports or an explicit
waiver.

### 3. Dynamic ontology/taxonomy selection

AgentZero is multi-domain. It cannot rely on one fixed ontology or taxonomy.
Engram should therefore accept caller-supplied knowledge governance selection:

- ontology IDs
- concept scheme IDs
- validation mode
- allow or reject unclassified concepts/entities
- record validation findings when writes are affected

Engram should store durable ontology/taxonomy definitions and validate records.
AgentZero chooses which definitions apply for a ward, project, session, source,
or task and passes that choice through the adapter.

### 4. Durable memory and belief retrieval indexes

Engram already has a retrieval composition seam. AgentZero should not build a
separate generic retrieval framework for memory and belief records.

Expected Engram work:

- memory `RetrievalIndex` returns `RetrievalTargetType::Memory`
- belief `RetrievalIndex` returns `RetrievalTargetType::Belief`
- both respect scope, policy, lifecycle state, valid-time filters, confidence,
  and request limits
- vector, keyword, cue-anchor, or hybrid mechanics stay adapter-private

AgentZero can preserve its product ranking contract on top, but the generic
candidate production should come from Engram where possible.

### 5. Cross-adapter conformance fixtures

Engram should extend reusable fixture runners beyond memory-service write and
retrieval examples.

Expected fixture coverage:

- read/query ports
- graph read summaries
- ontology/taxonomy selection and validation
- memory retrieval index behavior
- belief retrieval index behavior
- SQLite open-options behavior

AgentZero can still run its own parity fixtures against zbot contracts. Engram
fixtures prove Engram's generic behavior across adapters.

## Options considered

The option space is MECE along where the risk-reduction work lives.

| Option | Description | Trade-offs |
| --- | --- | --- |
| Do nothing | AgentZero implements everything locally or uses available Engram internals. | Fastest initially, but creates a large adapter and fragile dependence on Engram internals. |
| Push zbot compatibility into Engram | Engram implements AgentZero store traits, DTOs, and product behaviors. | Shortens AgentZero adapter code, but pollutes Engram and makes it less reusable. Rejected. |
| Track generic Engram dependencies | Engram implements reusable framework ports; AgentZero owns zbot-specific mapping and contracts. | More coordination, but keeps the boundary clean and reduces long-term risk. Recommended. |
| Block all AgentZero work until Engram is complete | Wait for every upstream dependency before writing adapter code. | Safest for churn, but too slow. Adapter spikes can proceed while cutover gates remain strict. |

## Risks & what would make this wrong

Pre-mortem:

- Engram ports become too broad and turn into god traits. Mitigation: request
  focused read/query ports by concern.
- AgentZero treats upstream dependencies as a reason to delay all adapter work.
  Mitigation: allow adapter spikes but gate production cutover.
- Engram accepts zbot-specific concepts by accident. Mitigation: reject
  AgentZero route names, DTOs, UI models, schedulers, and product categories in
  upstream Engram work.
- AgentZero overstates an upstream dependency as required when an adapter-side
  sidecar is cleaner. Mitigation: every upstream request must be generic and
  useful to another host.

Falsifiable assumptions:

- AgentZero can map dynamic ontology/taxonomy policy into Engram without
  Engram owning product defaults.
- Generic read/query ports are enough to avoid private SQLite dependencies for
  production cutover.
- Memory and belief retrieval indexes can feed zbot recall without changing
  zbot's public contract.
- Cross-adapter Engram fixtures reduce cutover risk beyond AgentZero-only
  parity tests.

Drawbacks:

- This adds coordination with Engram release timing.
- Some AgentZero adapter code may be temporary while waiting on upstream ports.
- The cutover has more explicit gates, so it may feel slower than direct
  implementation.

## Evidence & prior art

Repo evidence:

- RFC-0011 defines AgentZero's adapter-first Engram cutover and keeps gateway,
  UI, settings, sleep-cycle, and migration contracts in AgentZero.
- `docs/research/engram-zbot-capability-gap-matrix.md` identifies generic
  upstream candidates and separates them from zbot-specific adapter work.
- `stores/zbot-engram-adapter` already has provider mode, scope mapping,
  embedding mode, migration mode, and fail-closed capability reporting.
- Local Engram code shows SQLite open options have been implemented in the
  current checkout; AgentZero should pin a revision before relying on it.

External prior art:

- Ports-and-adapters architecture supports keeping framework ports generic while
  hosts own product adapters.
- Strangler-style migrations reduce risk by replacing backing behavior behind
  stable contracts rather than rewriting public surfaces first.

## Experiment / validation

Hypothesis: AgentZero can reduce Engram cutover risk by tracking generic
upstream dependencies separately from zbot-specific adapter work.

What we measure:

- Adapter code does not call private Engram SQLite internals for production
  behavior unless a waiver is recorded.
- Engram dependencies are either implemented, explicitly waived, or mapped to
  adapter sidecars before enabling provider cutover.
- AgentZero parity fixtures pass with current provider and Engram provider.

Success criteria:

- SQLite open options dependency is pinned to an Engram revision.
- Generic read/query dependency is implemented or waived before production
  provider cutover.
- Dynamic ontology/taxonomy selection has a documented path before
  knowledgebase migration apply mode.
- Memory/belief retrieval indexes are implemented or AgentZero records why its
  recall adapter can safely own that behavior.
- Engram conformance fixtures exist for any generic behavior AgentZero relies
  on across backends.

## Open questions

1. Which upstream dependencies are hard blockers for the first manual Engram DB
   test?
   Recommended default: only SQLite open options is a blocker for first manual
   DB construction; read/query and retrieval ports are blockers for production
   cutover. Owner: zbot maintainer. Decide-by: before first Engram-provider
   manual run.
2. Should procedure memory be an upstream Engram dependency?
   Recommended default: no for the first pass. Keep procedures in AgentZero
   adapter/sidecar until a generic procedure-memory shape is proven. Owner:
   zbot maintainer. Decide-by: before procedure parity work starts.
3. Should record-time bitemporality be an upstream Engram dependency?
   Recommended default: no for the first pass. Valid-time behavior is enough
   unless parity fixtures prove otherwise. Owner: zbot maintainer. Decide-by:
   before belief parity is marked complete.

## Follow-on artifacts

- AgentZero spec: `docs/specs/engram-memory-engine-cutover/`.
- AgentZero ADR after acceptance: Engram as backing framework, AgentZero as
  product host and sleep-cycle owner.
- Engram upstream specs/RFCs as needed for generic read/query ports,
  ontology/taxonomy selection, memory/belief retrieval indexes, and
  conformance fixtures.
