# RFC-0017: Ward Layout Archetypes

- **Status:** Accepted
- **Author:** phanijapps
- **Approver:** phanijapps
- **Date opened:** 2026-07-26
- **Date closed:** 2026-07-26
- **Related:** [ADR-0002](../adr/0002-select-complete-ward-archetypes-at-creation.md), [ADR-0001](../adr/0001-use-versioned-ward-layout-contracts.md), [RFC-0016](0016-generic-ward-configuration-and-layout-resolution.md), [Dynamic Ward Layout Archetypes brief](../product/briefs/dynamic-ward-layout-archetypes.md)

## The ask

Approve a closed, local registry of task-oriented ward archetypes. When z-Bot
creates a ward, it selects one complete archetype, validates it through the
existing Ward Layout compiler, copies its exact `ward-conf.yaml` into the new
ward, and materializes its bounded starter files. From that point forward the
snapshot is immutable structural authority for that ward.

RFC-0016 made a ward's copied configuration authoritative, but its accepted
first slice deliberately shipped one universal generic template and excluded
domain profiles. The Ward Layout engine can already compile different
data-only trees; ward creation cannot yet choose among them. The question is
how to make new wards fit coding, documentation, journal, ebook, research, and
news work without allowing a model response or a later prompt to rewrite
filesystem policy.

Decisions requested:

1. Select one **complete archetype at ward creation**, then retain its exact
   snapshot. Do not merge a base and overlay, regenerate a layout per request,
   or reclassify an existing ward. Recommended: accept; it preserves the
   deterministic snapshot contract established by RFC-0016. Owner:
   phanijapps. Decide by 2026-07-26.
2. Limit selection to the closed identifiers `generic`, `coding`,
   `documentation`, `journal`, `ebook`, `research`, and `news`. Permit an
   explicit user override; otherwise accept a bounded intent recommendation
   only on the `create_new` path, with `generic` as the conservative fallback.
   Recommended: accept; model classification may suggest policy but cannot
   author it or select a path. Owner: phanijapps. Decide by 2026-07-26.
3. Store each archetype as local, user-editable, data-only
   `ward-conf.yaml`, bounded starter Markdown, and bounded `ward-agent.md`
   doctrine rendered to `AGENTS.md`.
   Prohibit executable hooks, remote templates, shell interpolation, and
   skill-owned structural rules. Recommended: accept; it reuses the existing
   validation boundary and keeps all active structure inspectable. Owner:
   phanijapps. Decide by 2026-07-26.
4. Keep existing wards snapshot-isolated. Template edits affect only future
   wards, and migration or archetype adoption is deferred to a later explicit
   workflow. Recommended: accept; automatic propagation would make existing
   plans and lint results nondeterministic. Owner: phanijapps. Decide by
   2026-07-26.
5. Limit the first release to creation, doctrine, starter documents,
   provenance, and conformance. Defer archetype-specific operations such as
   `add_book` and `create_daily_entry`. Recommended: accept; the archetype
   boundary can be proven before growing the mutation API. Owner: phanijapps.
   Decide by 2026-07-26.

## Problem & goals

The current creation path always reads
`<vault>/config/templates/ward-conf.yaml`. That is appropriate for a generic
knowledge ward but produces unnecessary documentation structure for coding
and fails to provide useful starting conventions for chronological,
publication, evidence, or news workflows. Prompting a model to invent a
`structure` map does not solve this: it creates a second, probabilistic source
of structural authority and bypasses the bounded Ward Layout language.

The desired variability is at the new-ward boundary, not at every turn. A ward
may receive many kinds of asks over its lifetime. If each ask could replace
the ward's layout, paths referenced by earlier plans could change, the stored
snapshot digest would stop describing the active contract, and conformance
results would depend on the latest classifier output.

Goals:

- Give each supported task type a useful, inspectable initial filesystem
  structure and authoring doctrine.
- Preserve a single exact `ward-conf.yaml` snapshot as the complete structural
  authority after creation.
- Make both manual and intent-assisted selection deterministic after a closed
  archetype identifier is chosen.
- Keep coding wards code-native, with OKF applying to supporting Markdown
  knowledge rather than dictating source-code organization.
- Reuse the existing bounded YAML loader, compiler, atomic publisher, digest,
  and linter rather than create a parallel templating engine.
- Fail safely: an invalid, missing, or invented identifier must not become a
  filesystem path or leave a partially created ward.
- Prove the shipped path against a fresh database and fresh vault, matching
  the user's intended acceptance test.

Non-goals:

- Reclassifying or rewriting an existing ward in response to a later ask.
- Automatically migrating wards created under the generic template.
- Supporting finance, medicine, or other domain-specific archetypes in the
  first release.
- Generating layout YAML, paths, or conformance rules from model prose.
- Executing archetype hooks, fetching remote archetypes, or allowing skills to
  own structural policy.
- Adding a layout editor UI.
- Adding archetype-specific mutation tools in the first release.

## Proposal

### Registry and on-disk contract

Seed one complete template bundle for every supported identifier under the
vault configuration directory:

```text
<vault>/config/templates/wards/
├── generic/
│   ├── ward-conf.yaml
│   ├── ward-agent.md
│   └── starters/...
├── coding/
├── documentation/
├── journal/
├── ebook/
├── research/
└── news/
```

The exact leaf names and seeding mechanics belong to the follow-on spec. The
architectural contract is:

- The registry exposes a Rust enum or equivalent closed value, not a string
  that is concatenated into a path.
- Each identifier resolves through `VaultPaths` to one locally seeded,
  canonical, confined directory. Absolute paths, `..`, separators, symlink
  escapes, aliases, environment interpolation, and remote references are
  rejected.
- Each bundle contains exactly one complete Ward Layout document. There is no
  implicit base, overlay, inheritance chain, or compiled fallback shape.
- Starter content is a bounded, declared part of that layout or its local
  bundle. It cannot execute, interpolate shell or environment values, or
  overwrite undeclared paths. The follow-on registry spec must set hard
  operator/product maxima for starter file count, tree depth, total starter
  bytes, and per-file bytes; limit exhaustion must fail before staging or
  publication.
- The existing safe YAML loader and schema/compiler rules remain mandatory.
  Custom tags or other unsupported YAML features do not gain an alternate
  deserialization path.
- Each complete bundle supplies `ward-agent.md` as the source template for the
  created ward's `AGENTS.md`. It passes through the existing doctrine loader,
  byte limit, placeholder validation/rendering, no-link checks, injection
  controls, and bounded diagnostics. The selected bundle template has sole
  precedence; it is not overlaid with a global template and cannot silently
  fall back to another archetype's doctrine. The singular
  `config/templates/ward-agent.md` transitions non-destructively into the
  `generic` bundle.

The legacy singular template becomes the seed source for the `generic`
archetype or is relocated by a clean, explicit configuration transition. It
must not remain as a hidden fallback that competes with the registry.

### Selection and trust boundary

The creation request carries one optional `WardArchetypeId`:

1. An explicit valid user override wins.
2. Otherwise, when intent analysis recommends `create_new`, it may return one
   identifier from the closed set.
3. If the recommendation is absent, low-confidence, unavailable, or invalid,
   creation uses `generic`.
4. If the registry entry for the resolved valid identifier is missing or
   invalid, creation fails closed with an actionable bounded error; it does
   not silently substitute another bundle.

The distinction between steps 3 and 4 is intentional. Classification failure
has a documented safe default. Configuration corruption must remain visible.
Model-authored free-form `WardRecommendation.structure` ceases to be a
structural input. Model output is untrusted classification data until parsed
as a closed identifier at the intent boundary.

Automatic selection uses the primary requested deliverable and lifecycle, not
incidental words or supporting work:

| Identifier | Select only when the primary ask is |
| --- | --- |
| `coding` | Creating or changing executable source, tests, project/build configuration, or a software artifact |
| `documentation` | Creating durable explanatory, reference, specification, or navigable knowledge documents without a primary executable deliverable |
| `journal` | Maintaining first-person chronological entries, recurring reflection, or a personal log |
| `ebook` | Ingesting, organizing, reading, annotating, or deriving work from long-form books and chapters |
| `research` | Building an evidence corpus and provenance-linked analysis or synthesis where timeliness is not the defining lifecycle |
| `news` | Tracking time-sensitive events through source intake, fact checking, and a dated briefing or publication lifecycle |
| `generic` | No specialized row uniquely matches, more than one row remains primary, classification is unavailable, or the caller explicitly selects generic |

There is no numeric model-confidence threshold. “Low confidence” has one
testable representation: return `generic` whenever exactly one specialized
primary row cannot be identified. The intent spec must carry positive,
negative, and ambiguous fixtures for every row. A model-invented identifier
fails parsing and is normalized to `generic` before the creation service is
called; an explicitly supplied invalid override is rejected rather than
normalized.

Automatic selection is cold-path only. If a matching ward already exists, its
`<ward>/ward-conf.yaml` is loaded and the current classifier output is ignored.
An explicit creation caller may choose `generic` or another valid identifier,
which keeps manual and test workflows deterministic.

### Creation and provenance

The existing `create_ward_from_template` service becomes archetype-aware while
retaining its validation and atomic publication sequence:

1. Resolve the closed identifier to a confined local bundle.
2. Load and compile the selected `ward-conf.yaml` using the existing limits.
3. Validate every required and starter path before creating the destination.
4. Materialize into staging, copy the exact selected YAML bytes to
   `<ward>/ward-conf.yaml`, and publish atomically.
5. Persist the selected identifier as bounded provenance metadata and compute
   the active digest from the copied snapshot.

The copied bytes, not the metadata identifier or the current registry entry,
are authoritative on reuse. Editing a registry template changes future wards
only. Errors are bounded and avoid exposing internal absolute paths; any
failure before publication leaves no visible partial ward.

### Initial archetype responsibilities

The first registry contains:

| Identifier | Primary structure |
| --- | --- |
| `generic` | Neutral notes, sources, attachments, and outputs |
| `coding` | Source, tests, project configuration, build output, and supporting technical docs |
| `documentation` | OKF-first concepts, specifications, analyses, and navigable knowledge |
| `journal` | Chronological entries, reflection prompts, topics, attachments, and editable entry templates |
| `ebook` | Library index, source-preserving book folders, chapter navigation, reading notes, concepts, and derived outputs |
| `research` | Sources and evidence, notes, analysis runs, data, and provenance-linked deliverables |
| `news` | Source intake, fact checking, topic and event tracking, drafts, and date-oriented published briefings |

All seven use the same Ward Layout schema, bounded doctrine renderer, and
construction harness. Differences live in data, starter documents, and
doctrine content. An archetype cannot add an executable behavior or its own
parser.

### Rollout and validation

Delivery is split into three independently reviewable slices:

1. Registry, explicit creation, provenance, and the `generic` and `coding`
   reference archetypes.
2. Closed intent selection and trusted creation binding.
3. The remaining five bundled archetypes and their construction fixtures.

Slices 2 and 3 may be implemented independently after Slice 1, but automatic
selection is not enabled in a release until Slice 3's all-seven construction
and clean-start gate passes. This prevents the closed classifier from
selecting a valid identifier whose shipped bundle is absent. An explicitly
selected identifier with missing or invalid local configuration continues to
fail closed rather than falling back.

The authoritative end-to-end acceptance run starts z-Bot with a fresh
database and fresh vault. It must prove seed creation, explicit and automatic
selection, exact snapshot bytes and digest, expected materialized tree,
generic fallback, invalid-configuration failure without a partial ward, and
reuse without reclassification. It must also exercise every starter resource
ceiling at and above its boundary and prove that an oversized bundle leaves no
visible partial ward. Existing database fixtures remain useful for regression
coverage but cannot substitute for this clean-start test.

## Options considered

The option spaces below are MECE along their stated control axes. They draw on
the repository's snapshot model and on established complete-template,
overlay-composition, and closed-enumeration patterns.

### When and how the effective layout is derived

Axis: the point at which variability is introduced and whether components are
composed. These options exhaust fixed, create-time complete selection,
create-time composition, and request-time generation.

| Option | Trade-off |
| --- | --- |
| Do nothing: retain one fixed generic template | Lowest implementation cost, but does not meet the task-fit outcome and keeps coding/news/journal structures generic |
| **Select one complete archetype at creation** | Preserves one inspectable snapshot and has a narrow creation seam; duplicates some declarations between bundles |
| Compose a generic base plus one or more overlays | Reduces duplicated declarations but introduces precedence, merge, deletion, and provenance rules explicitly avoided by RFC-0016 |
| Generate or switch layout per request | Maximally adaptive, but makes stored paths, plans, digests, and lint depend on current model output |

Recommendation: complete create-time selection.

### Who may choose an archetype

Axis: increasing selection authority. The options exhaust no selection,
human-only selection, bounded machine recommendation, and unconstrained model
authorship.

| Option | Trade-off |
| --- | --- |
| Do nothing: no choice | Deterministic but cannot adapt |
| User-only explicit choice | Safest classifier boundary, but prevents useful automatic setup |
| **Explicit override plus closed intent recommendation and generic fallback** | Adds bounded classifier risk while keeping policy deterministic |
| Free-form model-produced template, structure, or path | Flexible but crosses the model/filesystem trust boundary and creates a second layout language |

Recommendation: bounded recommendation with explicit override.

### Where archetype behavior lives

Axis: increasing execution capability. The options exhaust fixed compiled
behavior, declarative local bundles, prompt/skill convention, and executable
or remote extension.

| Option | Trade-off |
| --- | --- |
| Do nothing: compile all archetypes into Rust | Strong control but makes customization and iteration require a release |
| **Local declarative bundles using the existing Ward Layout contract** | User-editable and inspectable, with bounded parser and path risk |
| Skills or prompts own structure | Easy to author but probabilistic and difficult to lint as a single authority |
| Executable hooks or remotely fetched templates | Most extensible, but substantially expands code-execution and supply-chain risk |

Recommendation: local declarative bundles only.

### What happens to existing wards

Axis: propagation policy after template change. The options exhaust no
propagation, explicit propagation, automatic propagation, and per-request
switching.

| Option | Trade-off |
| --- | --- |
| **Immutable snapshots; no first-release adoption workflow** | Predictable and simplest, but users rebuild to adopt a new archetype |
| Explicit previewed adoption/migration | Useful later, but requires reconciliation and conflict policy |
| Automatic template propagation | Keeps wards current but silently changes user-owned structure |
| Per-request switching | Avoids migration state but destroys stable ward semantics |

Recommendation: immutable snapshots; consider explicit adoption separately.

### How deep the first-release operations go

Axis: increasing mutation semantics. The options exhaust creation-only,
generic typed mutation, archetype-specific typed mutation, and arbitrary
archetype execution.

| Option | Trade-off |
| --- | --- |
| Do nothing: documentation-only proposal | No implementation risk, but no usable task-fit creation |
| **Creation, doctrine, starter documents, provenance, and conformance** | Proves the boundary with a controlled API surface |
| Add generic typed content operations | Helpful but broadens scope before the registry is validated |
| Add archetype-specific or arbitrary operations | Richest UX, but couples structural selection to many new mutation contracts |

Recommendation: creation and conformance first.

## Risks & what would make this wrong

Pre-mortem:

- **A wrong classification creates a durable unsuitable ward.** Mitigation:
  explicit override, conservative `generic` fallback, closed identifiers, and
  no automatic reclassification. If user correction rates remain high, the
  automatic selector should be narrowed or disabled without changing the
  registry.
- **Archetype identifiers become path traversal.** Mitigation: represent them
  as closed values and map them to canonical confined paths; never concatenate
  model strings. Reject separators, absolute paths, `..`, and symlink escape.
- **Bundles become a second executable plugin system.** Mitigation: keep them
  data-only, local, schema-bounded, and evaluated solely by the existing
  compiler; prohibit hooks, remote resolution, and environment expansion.
- **A malformed bundle partially creates a ward.** Mitigation: compile and
  validate all paths before staging, then use the established atomic publish
  path. Enforce starter file-count, tree-depth, total-byte, and per-file-byte
  ceilings before materialization. Configuration or resource-limit failure is
  terminal and does not invoke a hidden fallback.
- **Template and ward metadata disagree.** Mitigation: copied YAML bytes and
  their digest remain authoritative; the identifier is provenance only.
- **Seven templates drift or duplicate policy.** Mitigation: one shared
  construction-test harness, deliberately small starter content, and no
  inheritance until duplication demonstrates a concrete maintenance problem.

Falsifiable assumptions:

- The current Ward Layout schema can express each initial structure without
  archetype-specific executable semantics. This is wrong if a required
  archetype needs behavior that cannot be represented as required paths,
  bounded starter documents, and doctrine.
- The creation service is the only structural snapshot seam. This is wrong if
  another production path creates wards without `WardLayoutAccess::create`.
- A generic fallback is preferable to blocking on uncertain classification.
  This is wrong if clean-start testing or user trials show that an incorrect
  generic ward is costlier than asking for confirmation.
- Complete bundles are maintainable without layering. This is wrong if common
  policy changes repeatedly require error-prone edits across all seven
  templates.

Drawbacks:

- The registry adds seed files, provenance state, test fixtures, and a new
  creation parameter.
- Complete bundles duplicate some schema and doctrine content.
- Template fixes do not repair existing wards.
- Closed identifiers make third-party archetypes a future governed extension
  rather than an immediate drop-in mechanism.
- Deferring typed operations means journal and ebook wards begin with useful
  structure but not one-command entry or book workflows.

## Evidence & prior art

Repository evidence:

- [RFC-0016](0016-generic-ward-configuration-and-layout-resolution.md)
  established exact snapshot-on-create semantics, one active structural
  authority, a data-only bounded YAML language, and failure without a compiled
  fallback. Its accepted first slice intentionally retained one generic
  template and its non-goals excluded domain profiles.
- [ADR-0001](../adr/0001-use-versioned-ward-layout-contracts.md) records the
  versioned Ward Layout contract and snapshot boundary. This RFC expands that
  decision from a singular default template to explicit create-time
  archetypes without changing per-ward authority.
- The [Dynamic Ward Layout Archetypes brief](../product/briefs/dynamic-ward-layout-archetypes.md)
  records the product cut and Codegraph evidence. The graph found one caller of
  `create_ward_from_template` through depth five, placing selection at a narrow
  service/adapter seam; it also identified `VaultPaths` as a structurally
  sensitive path boundary and `WardRecommendation` as the existing intent
  contract.

External prior art:

- [Cookiecutter 2.7 documentation](https://cookiecutter.readthedocs.io/_/downloads/en/v2.7.0/pdf/)
  describes generating projects from complete local or remote templates
  across languages. This supports complete-template selection as an
  established shape, while this proposal deliberately permits local bundles
  only.
- [Kubernetes Kustomize documentation](https://kubernetes.io/docs/tasks/manage-kubernetes-objects/kustomization/)
  documents bases and overlays as a composition alternative. That is a real
  option, but its merge and precedence semantics conflict with the desired
  single complete ward snapshot for this release.
- [JSON Schema's `enum` reference](https://json-schema.org/understanding-json-schema/reference/enum)
  defines restricting a value to a fixed set. The proposed archetype
  identifier follows this closed-value model at every untrusted boundary.

De-risk spike:

- The riskiest assumption was that the existing generic interpreter can
  compile and snapshot a materially different layout without a new engine.
  On 2026-07-26,
  `cargo test -p gateway-services ward_layout::create::tests::creates_only_required_literal_nodes_from_a_fluid_template -- --exact`
  passed (`1 passed; 0 failed`). The test exercises an alternate declared tree,
  exact snapshot copying, and lint-compatible construction. This validates the
  interpreter seam, not the still-to-be-built registry or intent handoff.

## Open questions

No unresolved research questions remain. The owner confirmed the five
recommended decision inputs on 2026-07-26, and the proposal passed its
adversarial and security review gates. Implementation discoveries that
invalidate a falsifiable assumption must return to this RFC or a superseding
decision rather than being silently resolved in code.

## Follow-on artifacts

- [ADR-0002: Select Complete Ward Archetypes at Creation](../adr/0002-select-complete-ward-archetypes-at-creation.md).
- Spec: `ward-archetype-registry-and-creation`.
- Spec: [`intent-ward-archetype-selection`](../specs/intent-ward-archetype-selection/spec.md).
- Spec: `bundled-ward-archetypes`.
