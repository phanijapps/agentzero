# Brief: Dynamic ward layout archetypes

- **Slug:** `dynamic-ward-layout-archetypes`
- **Received:** 2026-07-26
- **Owner:** phanijapps
- **Shape:** A (outcome brief; no story list)

## Outcome

When a request needs a new ward, z-Bot creates that ward from a layout suited
to the kind of work instead of applying one universal documentation-heavy
`ward-conf.yaml`. Generic, coding, documentation/knowledge, journal, ebook,
research, and news work start with useful, distinct structures while every
created ward retains one inspectable and deterministic layout snapshot.

## Feasibility finding

**Feasible, with a medium-sized architecture change.**

The generic Ward Layout interpreter already compiles arbitrary data-only
layouts, materializes their required tree, copies the selected YAML into the
new ward, and binds runtime context to the copied snapshot digest. The missing
capability is selection: creation is hard-wired to the single
`config/templates/ward-conf.yaml`, the `ward` tool accepts no layout
archetype, and intent analysis emits a free-form `structure` map rather than a
trusted layout identifier.

The safe implementation is an allowlisted, versioned archetype registry. Intent
analysis may select an archetype identifier for a **new** ward, but it must
never generate or edit layout YAML. Ward creation validates the selected
template, atomically copies its exact bytes to `ward-conf.yaml`, and thereafter
the ward snapshot remains the sole authority.

Changing the active layout on every ask is not compatible with the current
snapshot/digest model: one ward could change meaning between turns, invalidate
plans and writes, and make lint results nondeterministic. Existing wards should
continue to use their own snapshot regardless of later asks.

### Codegraph validation

An isolated Codegraph scan on 2026-07-26 indexed 1,343 files, 15,645 entities,
and 21,063 extracted relationships. The resulting call graph contains 7,804
nodes and 14,083 edges across 1,805 detected communities; its largest community
contains 72 symbols.

The graph strengthens the feasibility assessment:

- `create_ward_from_template` has exactly one discovered caller through depth
  five: the `create` implementation in
  `gateway/gateway-execution/src/invoke/ward_layout_adapter.rs`. Template
  selection can therefore enter through one narrow creation seam rather than
  a repository-wide constructor rewrite.
- `GatewayWardLayoutAccess`, `WardLayoutAccess`, `load_ward_layout`,
  `WardLayoutDocument`, `ward_layout_template`, `WardLayoutState`,
  `ward_layout_snapshot`, `LoadedWardLayout`, and `CompiledWardLayout` form the
  highest-relevance cluster for the proposed change. This matches the existing
  service/adapter/tool boundary and argues against adding a parallel routing
  subsystem.
- `WardRecommendation` resolves directly to the intent-analysis middleware,
  confirming that automatic archetype selection belongs in the existing intent
  contract.
- `VaultPaths` is the repository's highest PageRank-ranked symbol. Adding an
  archetype registry beneath it is feasible but is the most structurally
  sensitive part of Slice 1; path semantics and seed behavior need focused
  regression tests.
- None of the top five repository-wide bridge symbols belongs to ward creation.
  Together with the one-caller blast radius, this suggests low direct
  call-graph risk in the creation service and moderate integration risk in
  state propagation and path configuration.

The daemon entry point remains `main`, which constructs and starts
`GatewayServer`; no new application entry point or top-level service is needed.

## Current seams and gaps

| Area | Existing capability | Required change |
| --- | --- | --- |
| Intent | Classifies the ask, recommends skills, and chooses a reusable ward name | Emit one validated archetype id for `create_new`; remove the free-form structure proposal |
| Ward creation | Validates one vault template and atomically snapshots it | Resolve an allowlisted archetype template before the existing validation/copy path |
| Runtime | Injects the active ward snapshot and digest into root context | Carry the chosen archetype through the cold-path creation handoff |
| Existing wards | Reuse their own `ward-conf.yaml` snapshot | Ignore new ask classifications once a ward exists |
| Conformance | Generic rule compiler/linter supports different declared trees | Add construction tests for every bundled archetype |
| Product contract | ADR-0001 permits explicit versioned profiles, but RFC-0016 Phase 1 assumes one generic template and names domain profiles as a non-goal | Amend the decision boundary for task archetypes before implementation |

## Scope / Non-goals

**In scope:**

- A small, fixed initial archetype set:
  - `generic`: neutral notes, sources, attachments, and outputs; the safe
    fallback when no specialized archetype is selected.
  - `coding`: code-native layout; source, tests, project configuration, build
    artifacts, and technical docs are first-class. OKF applies to supporting
    Markdown knowledge, not to source-code organization.
  - `documentation`: OKF-first concepts, specifications, analyses, and
    navigable knowledge documents.
  - `journal`: chronological entries, recurring reflections, topics,
    attachments, and user-editable entry templates.
  - `ebook`: a library index plus source-preserving book folders, chapter
    navigation, reading notes, concepts, and derived outputs.
  - `research`: sources/evidence, notes, analysis runs, data, and deliverables
    with provenance-friendly organization.
  - `news`: source intake, fact checking, topic/event tracking, drafts, and
    published briefings with date-oriented organization.
- User-editable archetype templates seeded under the vault configuration
  directory.
- Automatic archetype selection only for a safe `create_new` recommendation.
- An explicit archetype override for deterministic/manual ward creation.
- Persisting the selected archetype id as bounded metadata while treating the
  copied `ward-conf.yaml` bytes as the active structural authority.
- Tests proving the selected template, copied snapshot, materialized files,
  digest, lint behavior, and reuse semantics agree.

**Non-goals:**

- Reconfiguring an existing ward on each prompt.
- Generating YAML or filesystem rules from model-authored free-form
  `WardRecommendation.structure`.
- Automatically migrating or rewriting existing wards.
- Domain-specific profiles such as finance, medicine, or books in the first
  release.
- Executable templates, hooks, shell interpolation, or remotely fetched
  profiles.
- A new UI for editing layouts; vault files and existing inspection surfaces
  are sufficient initially.

## Assumptions requiring confirmation

- “Dynamic based on the type of ask” means **choose once when creating a new
  ward**, not mutate an existing ward for every task.
- “Coding with docs/OKF taking a back seat” means coding wards use conventional
  code/project folders and keep OKF only for supporting Markdown documentation.
- News and research should be separate archetypes even though both use external
  sources, because their lifecycle and deliverables differ.

## Recommended selection contract

1. Intent analysis returns one value from the closed initial archetype set only
   when recommending `create_new`.
2. Runtime validates that id against the locally seeded registry and places the
   resolved id in trusted root tool state.
3. `ward(action="create")` consumes that trusted value. A manual caller may
   supply an explicit allowlisted override; untrusted paths and arbitrary
   template names are rejected.
4. Creation passes the selected template through the existing bounded YAML
   loader and atomic snapshot publisher.
5. Reusing an existing ward loads only `<ward>/ward-conf.yaml`; the current ask
   and current archetype templates cannot alter it.

This keeps classification probabilistic but makes filesystem policy
deterministic.

## Success measures

- Every bundled archetype passes the same loader, compiler, creation, and lint
  construction-test suite.
- A request that creates a ward receives its selected archetype snapshot and
  never receives another archetype's required paths.
- A second request of a different type routed to an existing ward does not
  change that ward's snapshot or digest.
- Invalid, missing, or model-invented archetype ids cannot select a file path or
  partially create a ward.
- Low-confidence or unavailable intent analysis has one documented,
  deterministic fallback behavior.

## Risks and constraints

- **Misclassification:** a wrong archetype creates a durable but unsuitable
  ward. Mitigate with a conservative generic fallback and an explicit override
  before creation.
- **Authority duplication:** skill `ward_setup`, intent `structure`, prompts,
  and archetype YAML could all compete. The archetype snapshot must be the only
  structural authority; legacy structure/scaffolding paths need removal or
  strict reconciliation.
- **Template drift:** later edits must affect future wards only. Existing wards
  remain snapshot-isolated.
- **Profile explosion:** keep the initial set task-oriented and closed; defer
  domain-specific variants until evidence shows they are needed.
- **Current worktree:** the generic ward-layout implementation is still present
  as uncommitted work, so implementation should build on and review that change
  rather than start a parallel routing system.

## Proposed shippable cut

### Slice 1 — Ward archetype registry and explicit creation

Ship a local archetype registry, safe identifier and path resolution, explicit
creation, snapshot provenance, starter-document rendering, and `generic` plus
`coding` reference archetypes. This is independently useful and testable
without automatic classification.

### Slice 2 — Intent-to-archetype selection and binding

Ship bounded intent classification and a trusted cold-path handoff so automatic
new-ward creation uses Slice 1. Remove the free-form structure authority,
define low-confidence/failure fallback, and prove existing wards never change
profile due to a later ask.

### Slice 3 — Bundled ward archetype pack

Ship the remaining `documentation`, `journal`, `ebook`, `research`, and `news`
templates, doctrine, starter documents, and shared construction-test harness.
This slice depends on Slice 1 but not on automatic selection in Slice 2.

No outcome is left uncovered by these three slices. Slices 2 and 3 depend on
Slice 1 and may then proceed independently.

## Confirmed product decisions

- Archetypes are selected when a new ward is created; an existing ward is not
  mutated or reclassified by later asks.
- `generic` is the automatic fallback when classification is unavailable or
  uncertain and may also be selected explicitly.
- The initial set is `generic`, `coding`, `documentation`, `journal`, `ebook`,
  `research`, and `news`.
- Coding is code-native with OKF limited to supporting Markdown knowledge.
- Layout-directed typed operations such as `add_book` and
  `create_daily_entry` are deferred from this brief; initial archetypes provide
  structure, doctrine, and starter documents.

## Spec map

<!-- Specs are added only after the proposed cut is confirmed. Status is
auto-derived from each linked spec and must not be maintained here. -->

| Spec | Status |
| --- | --- |
| `ward-archetype-registry-and-creation` | <auto> |
| `intent-ward-archetype-selection` | <auto> |
| `bundled-ward-archetypes` | <auto> |
