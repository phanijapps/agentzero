# Spec: OKF Ward Foundation

- **Status:** Archived
- **Owner:** phanijapps
- **Plan:** [`plan.md`](plan.md)
- **Constrained by:** [RFC-0015](../../rfc/0015-okf-aligned-ward-layout-and-capabilities.md)
- **Brief:** none
- **Contract:** [`contracts/openapi/okf-wards.yaml`](../../../contracts/openapi/okf-wards.yaml)
- **Shape:** mixed

> **Spec contract:** this document defines what "done" means. The implementing
> PR must match this spec, or update it. Verification must be derivable from it.

## Objective

Replace the current ward, memory-bank, and dedicated wiki layouts with one
strict Open Knowledge Format (OKF) hierarchy rooted at the configured
`VaultPaths::wards_dir()`. The wards root is the single OKF bundle whose
`index.md` catalogs every ward. Each ward is a nested progressive-disclosure
directory in that bundle, and each product
"subdomain" is represented as an OKF concept with an optional same-stem
companion directory for current refinement work, history, and child concepts.

Deliver the canonical filesystem model, validation/indexing/search services,
formal HTTP APIs, agent capabilities, and Phase 1 read-only UI. This is a clean
cutover: existing wards and the dedicated wiki implementation are deleted, not
migrated or supported through compatibility aliases.

## Canonical model

```text
<wards-root>/
├── index.md
├── log.md
└── <ward>/
    ├── index.md
    ├── architecture.md
    ├── AGENTS.md
    ├── config.yaml
    ├── <subdomain>.md
    ├── <subdomain>/
    │   ├── index.md
    │   ├── spec.md
    │   ├── plan.md
    │   ├── tasks/
    │   │   ├── index.md
    │   │   └── <task>.md
    │   ├── history/runs/<run-id>/
    │   │   ├── spec.md
    │   │   ├── plan.md
    │   │   └── tasks/
    │   └── <child-concept>.md
    ├── src/<subdomain>/
    ├── data/<subdomain>/
    ├── reports/<subdomain>/
    ├── output/<subdomain>/
    └── references/
```

`<subdomain>.md` is the durable concept identity and current knowledge.
`<subdomain>/` is created only when the concept needs children or refinement
state. Subdomain and concept are interchangeable product terms; paths and API
fields use `subdomain` only where compatibility with existing product language
improves clarity.

Every non-reserved Markdown document beneath the wards root has valid YAML
frontmatter with a non-empty OKF `type`. Reserved `index.md` and `log.md` files
follow their OKF-defined forms. A concept and companion directory may share a
stem; two files or two directories may not claim the same canonical concept
path. At ward root, `src`, `data`, `reports`, `output`, `references`,
`index.md`, `log.md`, `AGENTS.md`, `architecture.md`, and `config.yaml` are
reserved. Inside a concept companion, `index.md`, `log.md`, `spec.md`,
`plan.md`, `tasks`, and `history` are reserved; `runs` is reserved directly
beneath `history`. The validator applies the appropriate level-specific set at
every recursive companion and rejects case-folded collisions.

The active `spec.md`, `plan.md`, and `tasks/` describe the next or current
refinement of the durable concept—not a product feature. Before replacing a
terminal refinement, the complete prior set is atomically snapshotted to
`history/runs/<run-id>/`, internal links are rewritten to immutable snapshot
targets where appropriate, and the snapshot validates before current files are
replaced. Reports and outputs are assets linked from concepts;
Markdown reports intended as knowledge must themselves conform to OKF.
Dependencies, caches, virtual environments, and build output must remain
outside the wards root.

## Boundaries

### Always do

- Resolve the wards root through `VaultPaths`; never hard-code
  `~/Documents/zbot/wards`.
- Create and maintain root and ward `index.md` catalogs with relative links,
  descriptions, OKF types/tags, and validation state.
- Validate recursively and fail the entire root when any ward contains a
  malformed, legacy, or non-OKF Markdown document.
- Confine all paths canonically beneath the configured wards root, reject
  symlink escapes, bound reads/search results, and make destructive operations
  explicit.
- Expose the HTTP surface in `okf-wards.yaml` and model-visible context
  capabilities for catalog, concept retrieval, search, backlinks, and
  validation. Search is a read-only Context Capability, not
  `ward(action="search")`.
- Keep the existing UI application and reusable components. Retarget Phase 1
  Vault/Memory navigation to an OKF catalog, ward tree, filtered search, concept
  preview, backlinks, and validation status.
- Update planner, spec-writing, ward-management, builder, and research prompts
  or skills so concept refinement is written beneath the concept companion and
  complete terminal runs are archived before replacement.
- Remove the dedicated wiki ward seeding, wiki promotion/template skill,
  wiki-only settings and routes, wiki compilation/storage wiring, and obsolete
  memory-bank scaffolding after all consumers use OKF concepts.
- Delete all existing user wards during the cutover after an explicit
  destructive confirmation at the CLI/UI boundary; immediately seed the empty
  conforming wards-root bundle. No automatic backup is implied.

### Ask first

- Changing the accepted canonical path rules, frontmatter requirements,
  refinement archive semantics, or the API contract.
- Preserving any existing ward data instead of applying the accepted destructive
  cutover.
- Storing executable dependency trees or generated caches inside an OKF bundle.

### Never do

- Add migration, fallback reads, legacy aliases, dual writes, or compatibility
  flags for old ward/wiki layouts.
- Treat `spec.md` as a feature spec detached from its concept, or create a
  companion directory merely to satisfy a template.
- Derive filesystem paths directly from unchecked API or model input.
- Return absolute host paths through the API.
- Retire the desktop UI or remove reusable UI primitives.

## API and capability behavior

The OpenAPI contract is normative. API identities are root-relative POSIX paths
and opaque ward IDs; responses never expose resolved host paths. Search accepts
text plus bounded `ward`, `path_prefix`, `types`, `tags`, and refinement-status
filters. Results include match provenance and concept metadata. Backlinks are
derived from relative Markdown links and OKF references. Validation is
deterministic and reports path-addressable errors. Reindex rebuilds only derived
catalog/search/link state and never rewrites authored documents. One discovery
request visits at most 10,000 files, reads at most 64 MiB total and 64 KiB of
frontmatter per document, accepts at most 16 YAML nesting levels and 32
directory levels, and spends at most two seconds before returning explicit
truncation diagnostics. The derived cache holds at most 50,000 entries or 256
MiB with least-recently-used eviction. Capability metadata advertises the same
bounds; API response bounds may be smaller.

Concept writes use optimistic concurrency (`If-Match`/ETag), validate the
resulting document before atomic replacement, and reject reserved/colliding
paths. Ward deletion requires the exact ward ID in a confirmation field and is
idempotent only after the first successful deletion. These mutation endpoints
are implemented in the foundation but need not be exposed by Phase 1 UI.

Normal catalog, tree, concept, search, link, graph, mutation, and reindex APIs
fail closed while the root is nonconformant. Only validation, confirmed
per-ward deletion, and cutover preview/execute remain available for repair. A
bulk cutover preview returns the exact ward-ID snapshot and a short-lived token;
execution requires that token plus the identical ID set, aborts without deletion
if the root changed, and stages the old root so deletion plus fresh scaffolding
is atomic from the product's perspective. Every API requires the local session
boundary, every browser mutation requires CSRF protection, and tests cover
unauthorized and cross-origin requests.

## Testing Strategy

- Unit-test path normalization, recursive OKF parsing, reserved names,
  same-stem pairing, frontmatter, indexes, archive transitions, link extraction,
  ETags, and result bounds.
- Contract-test every success and error response in `okf-wards.yaml`, including
  traversal/symlink attempts, stale ETags, malformed Markdown, collisions, and
  destructive confirmation mismatch.
- Integration-test create/read/update/search/backlink/validate/reindex/delete
  against temporary configurable vault roots on Linux-style and Windows-style
  paths.
- Test startup/cutover from a fixture with legacy wards: confirmation deletes
  every old ward and produces a conforming root; cancellation changes nothing.
- UI-test Phase 1 catalog/tree/search/preview/error/empty states and confirm no
  authoring control is visible.
- Run workspace format, lint, typecheck, Rust tests, UI tests, and an OKF
  conformance fixture suite.

## Acceptance Criteria

- [ ] A fresh vault contains a conforming wards-root OKF bundle and no seeded
      scratch/wiki legacy structure.
- [ ] A confirmed cutover deletes all old wards and creates the fresh root;
      cancellation is non-destructive, and no compatibility path remains.
- [ ] Each ward and nested concept follows the canonical model, including lazy
      companions and immutable terminal-run archives.
- [ ] Recursive validation rejects one malformed document anywhere beneath the
      wards root and returns actionable relative-path diagnostics.
- [ ] Root and ward catalogs stay synchronized after supported mutations and
      can be rebuilt deterministically without changing authored content.
- [ ] The contracted APIs pass conformance, confinement, concurrency, and bound
      tests without leaking absolute paths.
- [ ] Agents can catalog, retrieve, search, inspect backlinks, validate, and
      refine OKF concepts through named capabilities; `ward(Search, ...)` does
      not exist.
- [ ] Planner/spec/task workflows operate at concept-companion paths and archive
      completed runs before a new refinement replaces active files.
- [ ] The old-path removal inventory covers ward tooling; execution delegation,
      invoke, middleware, runner, and session context; archiver/distillation;
      ward curator; planner/builder/writer/solution prompts and shards;
      spec-builder/plan-composer/ward-designer examples; and pipeline tests.
- [ ] Dedicated wiki seeding, templates, promotion, settings, persistence,
      routes/DTO names, and memory-bank scaffolding are absent from runtime code.
- [ ] The retained Phase 1 UI browses and searches OKF knowledge and shows
      backlinks and validation status without editing.
- [ ] `cargo test --workspace`, `cargo clippy --all-targets -- -D warnings`, UI
      typecheck/tests, and OKF fixtures pass.

## Assumptions

| Assumption | Type | Resolution |
| --- | --- | --- |
| The wards root is configured, not fixed to the default Documents path. | Technical | Verified in `gateway/gateway-services/src/paths.rs` (`VaultPaths::wards_dir`). |
| Current ward creation scaffolds `memory-bank` and `specs`. | Technical | Verified in `runtime/agent-tools/src/tools/ward.rs`. |
| The runtime currently seeds a dedicated wiki ward and wires wiki persistence/recall. | Technical | Verified in `gateway/src/state/mod.rs`, `gateway/src/state/persistence_factory.rs`, and `gateway/gateway-execution/src/ward_wiki.rs`. |
| Old wards may be deleted with no migration or backward compatibility. | Product | Confirmed by user, 2026-07-19. |
| Subdomain and concept are interchangeable; the filesystem may retain “subdomain” language. | Product | Confirmed by user, 2026-07-19. |
| Specs belong to a concept companion and represent repeatable future refinement. | Product | Confirmed by user, 2026-07-19. |
| The UI remains and Phase 2 becomes a mini-Obsidian experience. | Product | Confirmed by user, 2026-07-19. |
| Formal OKF APIs are part of the cutover. | Product | Confirmed by user, 2026-07-19. |
| RFC-0015 is the accepted governing decision. | Process | Verified by RFC status and user acceptance, 2026-07-19. |
| OpenAPI is hand-authored because the `api-contract` skill is unavailable. | Process | Verified against the active skill roster; authored without that skill’s rule enforcement. |
