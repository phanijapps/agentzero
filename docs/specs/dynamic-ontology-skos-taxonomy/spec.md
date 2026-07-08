# Spec: Dynamic Ontology and SKOS Taxonomy

- **Status:** Implementing
- **Owner:** phanijapps
- **Plan:** [`plan.md`](plan.md)
- **Constrained by:** [`RFC-0011`](../../rfc/0011-engram-memory-engine-cutover.md); [`RFC-0012`](../../rfc/0012-engram-upstream-risk-reduction.md); [`engram-memory-engine-cutover`](../engram-memory-engine-cutover/spec.md)
- **Brief:** none
- **Contract:** none; preserves existing gateway routes, UI DTOs, WebSocket events, store traits, memory settings, migration diagnostics, and Observatory read models while adding internal governance config and adapter behavior.
- **Shape:** mixed

> **Spec contract:** this document defines what "done" means. The implementing
> PR must match this spec, or update it. Verification must be derivable from it.

## Objective

Add zbot-owned dynamic ontology governance and durable SKOS-style taxonomy
support on top of the Engram memory layer. Success means zbot can select and
evolve ontology classes/properties per ward, project, session, source, or task
without forcing one global schema, while SKOS concept schemes provide stable
controlled vocabulary for classification, recall expansion, validation hints,
and graph navigation. Existing gateway, UI, event, store-trait, migration, and
Observatory contracts keep their current shapes; when no ontology or taxonomy
definition is configured, current memory and graph behavior is preserved.

## Boundaries

The three-tier guard that keeps an implementing agent inside the lines.
*Always do* applies without asking; *Ask first* requires human sign-off before
proceeding; *Never do* is a hard rule, even under time pressure.

### Always do

- Keep dynamic ontology selection, evolution policy, and defaults in zbot, not
  in Engram.
- Store durable SKOS-style concept schemes, concepts, labels, and direct
  broader/narrower/related relations through Engram taxonomy repositories.
- Treat ontology validation as advisory by default: record findings and traces,
  but preserve writes unless an explicit future policy says otherwise.
- Preserve `EntityType::Custom`, `RelationshipType::Custom`, and unclassified
  records so zbot remains useful across arbitrary domains.
- Keep selected ontology IDs, concept scheme IDs, validation mode, and
  unclassified-record policy visible in migration dry-run/apply manifests.
- Confine all configured governance definition paths under the trusted zbot
  config/data root before reading or bootstrapping them.
- Keep Engram generic: use its ontology/taxonomy ports, but do not push zbot
  route names, ward DTOs, UI models, or product defaults into Engram.

### Ask first

- Enabling write-rejecting ontology or taxonomy validation.
- Adding a UI editor or review workflow for ontology/taxonomy changes.
- Importing external RDF/OWL/SHACL/SKOS files beyond local zbot-owned
  definitions.
- Changing public gateway, UI, WebSocket, store-trait, or settings payload
  shapes instead of using additive read-model fields or internal config.
- Replacing the existing graph, hierarchy, recall, or Observatory contracts with
  Engram-native DTOs.

### Never do

- Never make one global ontology mandatory for all zbot work.
- Never use SKOS taxonomy as a substitute for ontology classes/properties, or
  ontology classes as a substitute for SKOS concepts.
- Never auto-merge model-discovered ontology terms or taxonomy concepts into
  active durable definitions without an explicit governed policy.
- Never silently drop unknown entity types, relationship predicates, concepts,
  labels, or source evidence because they do not match the active definitions.
- Never expose raw transcripts, embeddings, absolute paths, SQL internals,
  connector/vault data, or secrets in validation findings, traces, API errors,
  committed fixtures, or migration diagnostics.
- Never make Engram depend on zbot-specific ontology or taxonomy defaults.

## Testing Strategy

- Governance config parsing, defaulting, path confinement, and selector
  precedence: **TDD**, because the invariants are compact and misconfiguration
  can affect every write.
- Built-in dynamic ontology and SKOS taxonomy bootstrap: **TDD plus goal-based
  integration**, proving definitions are deterministic, scoped, idempotent, and
  stored in Engram's configured SQLite layout.
- Knowledge entity/chunk mapping and relationship validation: **TDD**, because
  each zbot type, custom escape hatch, concept reference, and advisory finding
  has a direct expected output.
- Recall expansion through SKOS labels and relations: **goal-based integration**
  with bounded fixtures, because the behavior is only meaningful across
  taxonomy storage, query construction, ranking input, and trace output.
- Migration and provider rollout: **goal-based integration** against synthetic
  and sanitized current-DB fixtures, proving manifests include governance
  choices and no public contract changes.
- UI/Observatory visibility: **goal-based integration plus manual QA** only for
  additive health/findings surfaces that reuse existing Observatory patterns.

## Acceptance Criteria

- [ ] With no configured ontology or taxonomy files, Engram-backed memory,
  knowledge graph, recall, migration dry-run, and Observatory behavior match
  the current supported zbot behavior except for explicitly additive diagnostics.
- [ ] A built-in zbot base ontology is generated or loaded deterministically
  from the current entity and relationship vocabulary, including `Custom`
  escape hatches that remain allowed and traceable.
- [ ] Local governance config supports scoped selection by ward, project,
  session, source, and task, with ordered overlays, versioned ontology IDs,
  versioned SKOS concept scheme IDs, validation mode, and
  allow-unclassified policy.
- [ ] Startup bootstraps active ontology and SKOS taxonomy definitions into
  Engram through public ontology/taxonomy repositories, idempotently, under the
  trusted data root and configured SQLite storage layout. Local definition file
  import beyond the built-in definitions is deferred:
  `dynamic-ontology-local-definition-import`.
- [ ] Knowledge entity and knowledge chunk writes attach Engram `OntologyRef`
  and `ConceptRef` values when the active policy can classify them, while
  preserving legacy `ontology_id` and `taxonomy_id` metadata for compatibility.
- [ ] Relationship writes run advisory ontology validation for active ontologies
  and persist sanitized findings for unknown predicates, missing classes, and
  domain/range mismatches without blocking writes by default.
- [ ] Taxonomy lookups use SKOS-compatible `prefLabel`, `altLabel`,
  `broader`, `narrower`, and `related` semantics for classification and recall
  expansion, bounded by configured depth, fan-out, and total candidate limits.
- [ ] Recall traces identify which concept labels and SKOS relations expanded a
  query, and expansion cannot cross tenant, ward, workspace, session, or
  visibility boundaries.
- [ ] Model- or extraction-discovered ontology terms and taxonomy concepts can
  be recorded as proposed changes or findings, but cannot become active
  definitions unless an explicit governed merge policy is added later. The
  durable proposed-change queue is deferred:
  `dynamic-ontology-proposed-term-queue`.
- [ ] Migration dry-run/apply manifests include ontology selection, taxonomy
  scheme selection, validation mode, unclassified-record policy, and definition
  fingerprints; apply mode fails if these differ from the accepted dry-run.
- [ ] Existing gateway/UI/settings/events/Observatory route and payload shapes
  remain compatible; any governance health or findings output is additive,
  sanitized, and covered by focused tests.
- [ ] Tests prove custom entity/relationship values, unclassified records,
  empty taxonomy schemes, deprecated concepts, cyclic SKOS relations, duplicate
  labels, and missing ontology definitions degrade predictably.

## Assumptions

- Technical: zbot currently has fixed Rust `EntityType` and `RelationshipType`
  vocabularies plus `Custom` escape hatches, not durable ontology files
  (source: `services/knowledge-graph/src/types.rs`).
- Technical: the Engram adapter currently preserves `ontology_id` and
  `taxonomy_id` metadata but does not populate Engram `concept_refs` for
  entities or chunks (source:
  `stores/zbot-engram-adapter/src/mapping/knowledge.rs`).
- Technical: Engram provides durable ontology and SKOS-aligned taxonomy
  domain/repository primitives, including concept schemes, concepts,
  broader/narrower/related relations, and advisory ontology validation (source:
  `/home/videogamer/projects/mem-alpha/core/domain/src/taxonomy.rs`;
  `/home/videogamer/projects/mem-alpha/core/knowledge/src/ontology.rs`).
- Technical: SKOS is suitable for taxonomies and controlled vocabularies with
  concept schemes, labels, informal hierarchies, associations, and mapping
  between schemes (source: W3C SKOS Primer, https://www.w3.org/TR/skos-primer/).
- Process: concrete feature work belongs under `docs/specs/<feature>/` with
  `spec.md` and `plan.md`, and specs cite constraining RFCs/ADRs upward
  (source: `docs/CONVENTIONS.md`).
- Product: zbot owns dynamic ontology selection/evolution, while taxonomy
  should be SKOS-style and durable (source: user confirmation 2026-07-06).
