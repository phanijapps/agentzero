# Plan: Dynamic Ontology and SKOS Taxonomy

- **Spec:** [`spec.md`](spec.md)
- **Status:** Executing

> **Plan contract:** this is the implementation strategy. Unlike the spec, this
> document is allowed to change as you learn. When it changes substantially
> (a different approach, not just a re-ordering), note why in the changelog
> at the bottom.

## Approach

Implement this as a zbot governance layer over Engram, not as an Engram fork or
an Engram product vocabulary. Add a small policy/config module in
`stores/zbot-engram-adapter`, feed it from gateway composition, bootstrap
versioned ontology and SKOS taxonomy definitions through Engram public
repositories, then use that policy during knowledge mapping, advisory
validation, recall expansion, migration manifesting, and additive Observatory
health. The riskiest part is preventing taxonomy expansion and validation from
changing existing recall/graph behavior when no definitions are configured, so
the first and last tasks both prove no-definition parity.

## Constraints

- Follows [`RFC-0011`](../../rfc/0011-engram-memory-engine-cutover.md) and
  [`RFC-0012`](../../rfc/0012-engram-upstream-risk-reduction.md): zbot owns
  dynamic ontology/taxonomy policy; Engram stays generic.
- Builds on [`engram-memory-engine-cutover`](../engram-memory-engine-cutover/spec.md):
  the adapter remains the compatibility boundary and public gateway/UI/store
  contracts stay stable.
- Uses W3C SKOS-compatible taxonomy concepts and relations for controlled
  vocabulary semantics; zbot does not need to expose RDF as its config format in
  the first implementation.
- Engram ontology validation is advisory unless a later accepted policy enables
  write rejection.

## Construction tests

**Integration tests:** one no-definition parity test across Engram provider
startup, knowledge entity write/read, relationship write/read, recall, and
migration dry-run; one configured-governance integration test proving bootstrap,
mapping, validation findings, and recall expansion work together.

**Manual verification:** after implementation, start with a fresh Engram DB,
load a tiny project-scoped ontology and SKOS scheme, write graph facts for one
ward, run recall using an alternate label, and confirm Memory, Graph, and
Observatory surfaces still load with additive governance health only.

## Design (LLD)

### Design decisions

- Dynamic ontology and SKOS taxonomy are separate layers. Ontology governs
  entity classes, relationship properties, and validation findings; SKOS
  taxonomy governs concepts, labels, relations, classification, and recall
  expansion. Traces to: AC2, AC5, AC6, AC7.
- Advisory is the default validation mode. Findings are observable and
  sanitized, but writes continue unless a future approved policy changes that.
  Traces to: AC6, AC9, AC11.
- Built-in definitions are a base, not a global cage. Scoped overlays can add
  or deprecate terms per ward/project/session/source/task. Traces to: AC2,
  AC3, AC8.

### Data & schema

- Add zbot governance config types under
  `stores/zbot-engram-adapter/src/governance/`:
  `GovernancePolicy`, `OntologySelection`, `TaxonomySelection`,
  `ValidationMode`, `AllowUnclassifiedPolicy`, `SkosExpansionPolicy`, and
  `DefinitionFingerprint`.
- Load repository-provided base definitions and optional local definitions from
  trusted config-root children, initially shaped as zbot-owned JSON documents
  for ontology definitions and SKOS concept schemes.
- Persist active definitions into Engram's ontology/taxonomy repositories.
  Adapter sidecars are allowed only for zbot-specific policy cache,
  fingerprints, and sanitized findings not supported by Engram ports.
- Include governance fingerprints in migration manifests so dry-run/apply binds
  to the same ontology/taxonomy choices.

### Interfaces & contracts

- No new public REST, WebSocket, or store-trait contract is introduced in the
  first slice.
- Internal config is a contract for operators and tests. It must be versioned,
  path-confined, and round-trip tested before provider startup consumes it.
- Any Observatory or settings output is additive to existing read models and
  must be sanitized. A future public editor/review API needs a separate spec.

### Component / module decomposition

- `stores/zbot-engram-adapter/src/governance/`: policy types, config loader,
  built-in definitions, selector, bootstrapper, validators, taxonomy expansion,
  fingerprints, and tests.
- `stores/zbot-engram-adapter/src/mapping/knowledge.rs`: attach
  `OntologyRef`/`ConceptRef`, preserve compatibility metadata, and keep custom
  types unblocked.
- `stores/zbot-engram-adapter/src/migration.rs`: add governance manifest fields
  and blockers/warnings.
- `gateway/gateway-memory/src/lib.rs` and
  `gateway/src/state/persistence_factory.rs`: pass trusted config/data roots
  and governance settings into the adapter.
- `gateway/gateway-memory/src/recall/*`: consume bounded taxonomy expansion
  candidates and emit trace details without changing external recall shape.
- Existing Observatory/gateway read-model modules: expose additive governance
  health only if needed by AC11.

### State & control flow

1. Gateway composition resolves trusted roots and provider config.
2. Adapter loads built-in definitions, then overlays local definitions in
   deterministic order.
3. Adapter chooses active ontology/taxonomy policy for the write/search scope.
4. Startup bootstraps active definitions into Engram idempotently.
5. Knowledge writes attach refs, run advisory validation, and record sanitized
   findings.
6. Recall expands query terms through SKOS labels and direct relations under
   budget, then passes expansion evidence into existing ranking/trace paths.
7. Migration dry-run records the same policy and fingerprints before apply can
   be accepted.

### Behavior & rules

- Selector precedence is specific before general:
  task > source > session > project > ward > default.
- Missing definitions degrade to current behavior with a diagnostic, not a
  startup failure, unless the provider config explicitly requires governance.
- Unknown ontology terms and unclassified concepts are preserved. They may
  produce findings or proposed changes, but never mutate active definitions by
  default.
- SKOS expansion follows only direct `prefLabel`, `altLabel`, `broader`,
  `narrower`, and `related` signals in the first slice. Transitive expansion is
  bounded and opt-in per policy.

### Failure, edge cases & resilience

- Invalid config: fail provider startup with sanitized path-free diagnostics.
- Unsupported Engram ontology/taxonomy capability: report unsupported
  governance capabilities and continue no-definition behavior only when config
  does not require governance.
- Duplicate labels: keep deterministic winner selection and emit a warning.
- Cyclic broader/narrower relations: reject the configured definition or record
  a validation error before bootstrap; never recurse unbounded.
- Deprecated concepts: never use for new classification unless explicitly
  mapped; preserve existing records that reference them.

### Quality attributes (NFRs)

- Startup bootstrap is idempotent and proportional to changed definitions, not
  to all memory/knowledge records.
- Taxonomy expansion has explicit depth, fan-out, and total candidate caps, with
  defaults small enough to keep recall predictable.
- Public/additive diagnostics are positive-allowlist sanitized and path-free.
- Scope isolation remains strict across tenant, ward, workspace, session, and
  visibility boundaries.

### Dependencies & integration

- Engram crates consumed by the existing adapter must expose public ontology
  and taxonomy repositories used by this feature.
- Gateway composition must provide trusted config/data roots; user settings do
  not directly define unconstrained filesystem roots.
- Existing zbot store traits, gateway routes, UI DTOs, events, and Observatory
  read models remain the compatibility target.

## Tasks

### T1: Governance config and selector contract round-trip

**Depends on:** none

**Status:** Done on 2026-07-08.

**Touches:** `stores/zbot-engram-adapter/src/config.rs`, `stores/zbot-engram-adapter/src/governance/**`, `gateway/gateway-memory/src/lib.rs`, `gateway/src/state/persistence_factory.rs`

**Tests:**
- TDD: config serde defaults preserve no-definition behavior from AC1.
- TDD: path confinement rejects governance definition paths outside the trusted
  config root from AC3.
- TDD: selector precedence resolves task > source > session > project > ward >
  default from AC3.

**Approach:**
- Add governance policy/domain types in the adapter.
- Add trusted config-root plumbing from gateway composition to adapter config.
- Keep all fields optional by default so current provider configs behave the
  same until definitions are supplied.

**Done when:** adapter config tests prove governance config can be absent,
loaded, rejected, and selected deterministically without opening Engram.

### T2: Built-in dynamic ontology and starter SKOS scheme exist

**Depends on:** T1

**Status:** Done on 2026-07-08.

**Touches:** `stores/zbot-engram-adapter/src/governance/**`, `services/knowledge-graph/src/types.rs`

**Tests:**
- TDD: every current `EntityType` and `RelationshipType` has a deterministic
  base ontology class/property mapping from AC2.
- TDD: `Custom` entity and relationship values remain allowed and produce
  traceable unclassified outcomes from AC2 and AC12.
- TDD: starter SKOS scheme validates labels and direct relations from AC7.

**Approach:**
- Create built-in base definitions from the current zbot vocabulary.
- Keep the base ontology dynamic by allowing overlay replacement/addition per
  selector.
- Seed only minimal SKOS concepts needed for existing zbot vocabulary; richer
  domain concepts come from local overlays.

**Done when:** base definitions are deterministic, versioned, and covered by
tests that fail when the Rust vocabulary changes without governance updates.

### T3: Definitions bootstrap into Engram idempotently

**Depends on:** T1, T2

**Touches:** `stores/zbot-engram-adapter/src/governance/**`, `stores/zbot-engram-adapter/src/lib.rs`, `stores/zbot-engram-adapter/src/config.rs`

**Tests:**
- Integration: bootstrapping twice produces the same ontology/taxonomy records
  and fingerprints from AC4.
- Integration: single-file SQLite layout stores definitions without creating
  extra DB files from AC4.
- TDD: unsupported Engram ontology/taxonomy capability reports a named
  unsupported governance feature from AC1 and AC4.

**Approach:**
- Add a bootstrapper that calls Engram public `OntologyRepository` and
  `TaxonomyRepository` ports.
- Store governance fingerprints in adapter-owned compatibility state only where
  Engram lacks a generic record.
- Make bootstrap run during provider construction after path validation and
  before supported capabilities are advertised.

**Done when:** provider startup can bootstrap active definitions through Engram
or report sanitized unsupported diagnostics before any governed write occurs.

### T4: Knowledge mapping attaches ontology and SKOS references

**Depends on:** T1-T3

**Touches:** `stores/zbot-engram-adapter/src/mapping/knowledge.rs`, `stores/zbot-engram-adapter/src/stores/knowledge_graph.rs`, `stores/zbot-engram-adapter/src/stores/wiki.rs`

**Tests:**
- TDD: classified entities/chunks carry `ConceptRef` values and selected
  ontology metadata from AC5.
- TDD: legacy `ontology_id` and `taxonomy_id` metadata round-trips from AC5.
- TDD: unclassified/custom records are preserved and produce bounded findings,
  not write failures, from AC5 and AC12.

**Approach:**
- Inject governance policy into knowledge/wiki mapping paths.
- Classify entity types, aliases, chunk concepts, and optional metadata hints
  against the active SKOS scheme.
- Preserve existing zbot metadata and read-model reconstruction semantics.

**Done when:** Engram knowledge records contain typed refs where possible and
existing zbot entity/wiki round-trips still pass.

### T5: Advisory ontology validation findings are persisted and observable

**Depends on:** T3, T4

**Touches:** `stores/zbot-engram-adapter/src/governance/**`, `stores/zbot-engram-adapter/src/stores/knowledge_graph.rs`, `stores/zbot-engram-adapter/src/sidecars.rs`

**Tests:**
- TDD: unknown predicates, missing classes, and domain/range mismatches produce
  sanitized advisory findings from AC6.
- TDD: default advisory mode never blocks writes from AC6.
- TDD: findings contain no raw transcript, path, SQL, embedding, connector, or
  secret values from AC11.

**Approach:**
- Use Engram `validate_graph` where available.
- Persist sanitized findings in Engram if a public finding surface exists; use
  adapter-owned sidecar tables in `engram_data.db` only for zbot read-model
  compatibility gaps.
- Add a small query surface for additive governance health if Observatory needs
  it.

**Done when:** invalid governed graph writes succeed by default, findings are
recorded, and public/additive output is sanitized.

### T6: SKOS taxonomy expansion feeds recall under budget

**Depends on:** T3, T4

**Touches:** `stores/zbot-engram-adapter/src/governance/**`, `gateway/gateway-memory/src/recall/**`

**Tests:**
- Integration: `prefLabel` and `altLabel` expand recall input and appear in
  trace details from AC7 and AC8.
- Integration: `broader`, `narrower`, and `related` expansion respects depth,
  fan-out, total candidate, scope, and visibility limits from AC7 and AC8.
- TDD: cyclic relations and deprecated concepts do not produce unbounded or
  preferred expansions from AC12.

**Approach:**
- Add a taxonomy expansion query helper over the active concept scheme.
- Feed expansion candidates into the existing recall path as explainable query
  cues, not as a replacement for zbot ranking.
- Emit additive trace metadata using existing recall observability structures.

**Done when:** recall can find a fixture through an alternate SKOS label while
the trace proves expansion stayed scoped and bounded.

### T7: Migration manifests bind governance choices

**Depends on:** T1-T4

**Touches:** `stores/zbot-engram-adapter/src/migration.rs`, `docs/specs/engram-memory-engine-cutover/plan.md`

**Tests:**
- Goal-based integration: dry-run manifests include ontology/taxonomy IDs,
  validation mode, allow-unclassified policy, and definition fingerprints from
  AC10.
- TDD: apply mode rejects mismatched governance fingerprints from AC10.
- TDD: diagnostics remain path-free and positive-allowlist sanitized from AC11.

**Approach:**
- Extend manifest fields and fingerprint input with governance policy evidence.
- Include sanitized warnings for source records that cannot be classified.
- Keep apply disabled unless dry-run/apply governance fingerprints match.

**Done when:** governance changes between dry-run and apply are detected before
Engram storage is mutated.

### T8: Additive capability and Observatory health

**Depends on:** T3, T5, T6

**Touches:** `stores/zbot-engram-adapter/src/capabilities.rs`, `gateway/src/http/graph.rs`, `gateway/src/http/memory.rs`, `gateway/src/http/memory_search.rs`, `gateway/src/http/settings.rs`, `apps/ui/src/features/observatory/**`, `apps/ui/src/features/observatory-v2/**`

**Tests:**
- Goal-based integration: existing gateway/UI/settings/events/Observatory tests
  pass without shape changes from AC11.
- TDD: governance capability reports expose only support state, counts, IDs,
  versions, and sanitized finding codes from AC11.
- Manual QA: Observatory still loads on a fresh DB and can show additive
  governance health if implemented.

**Approach:**
- Add governance capability flags and optional health/finding summaries.
- Reuse existing read-model surfaces where possible; avoid a new public editor
  or mutation API.
- Update UI only if backend tests prove an additive health surface is needed.

**Done when:** supported/unsupported governance state is visible enough for
operators without changing existing public contract shapes.

### T9: Parity, cleanup, and operator docs

**Depends on:** T1-T8, T10

**Touches:** `stores/zbot-engram-adapter/**`, `gateway/**`, `docs/specs/dynamic-ontology-skos-taxonomy/**`, `docs/guides/**`

**Tests:**
- Cross-cutting integration: no-definition parity test from AC1 remains green
  after all tasks.
- Cross-cutting integration: configured-governance fixture covers bootstrap,
  mapping, validation, recall expansion, migration manifesting, and additive
  health from AC1-AC12.
- Goal-based check: `rg` confirms no zbot-specific ontology/taxonomy defaults
  were added to Engram path dependencies from AC11.

**Approach:**
- Remove temporary governance shims that are no longer needed.
- Document the local definition format, selector precedence, safe defaults, and
  rollback path.
- Record any Engram upstream gaps as explicit follow-up RFC/spec items rather
  than leaving adapter placeholders.

**Done when:** tests and docs prove the feature can be enabled, disabled, and
operated without public contract drift or hidden Engram product coupling.

### T10: Deduplicate knowledge graph connections

**Depends on:** T4, T5

**Touches:** `stores/zbot-engram-adapter/src/stores/knowledge_graph.rs`, `stores/zbot-engram-adapter/src/governance/**`, `docs/specs/dynamic-ontology-skos-taxonomy/**`

**Tests:**
- TDD: duplicate relationships with the same source, target, normalized
  predicate, scope, and visibility collapse into one durable connection while
  preserving mention counts or evidence metadata.
- TDD: relationships that differ by source, target, predicate, scope, or
  visibility are not merged.
- Goal-based integration: Graph/Observatory read models no longer show
  duplicate node connections after the governed mapping path runs.

**Approach:**
- Add a post-governance normalization pass after ontology validation and before
  final graph read-model exposure.
- Use normalized relationship keys rather than display labels so aliases and
  casing do not create parallel edges.
- Keep dedup at the end of the feature so classification and validation can
  contribute stable predicate IDs first.

**Done when:** governed graph output is free of duplicate connections without
  dropping distinct evidence or crossing scope boundaries.

## Rollout

- **Delivery:** ship disabled-by-default for local overlay definitions. The
  built-in no-definition/default policy preserves current behavior.
- **Infrastructure:** no new service or top-level database. Governance records
  live in the configured Engram SQLite layout, with adapter sidecars only inside
  the same confined storage root if needed.
- **External-system integration:** requires the Engram revision consumed by
  zbot to expose durable ontology/taxonomy repository behavior used by T3-T6.
- **Deployment sequencing:** land config/selectors first, then bootstrap, then
  mapping/validation, then recall expansion, then migration/Observatory/docs.
  Keep write-rejecting validation out of this rollout. Deduplicate graph
  connections after governed predicate normalization exists.
- **Rollback:** remove local governance config or switch provider config back to
  no-definition/default behavior. Existing unclassified records remain readable.

## Risks

- Taxonomy expansion could change recall quality or cost. Mitigation: bounded
  expansion, trace output, and no-definition parity tests.
- Dynamic ontology overlays could create hard-to-debug classification drift.
  Mitigation: deterministic selector precedence, versioned IDs, fingerprints,
  and migration manifest binding.
- Engram public ports may not expose every list/query/finding surface zbot
  wants. Mitigation: adapter sidecars are allowed only for zbot read-model gaps
  and must be named in tests/docs.
- Users may expect SKOS to behave like a full ontology. Mitigation: keep the
  spec and docs explicit: SKOS governs concepts and labels; ontology governs
  classes/properties/validation.

## Changelog

- 2026-07-06: initial plan.
- 2026-07-08: added T10 for end-of-feature knowledge graph connection
  deduplication after governed predicate normalization.
