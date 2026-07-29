# Spec: LLM Wiki Ward Foundation

- **Status:** Shipped
- **Owner:** phanijapps
- **Plan:** [`plan.md`](plan.md)
- **Constrained by:** [RFC-0018](../../rfc/0018-filesystem-authoritative-llm-wiki-wards.md), [ADR-0002](../../adr/0002-select-complete-ward-archetypes-at-creation.md), [ADR-0003](../../adr/0003-use-filesystem-authoritative-llm-wiki-wards.md)
- **Brief:** none
- **Contract:** `gateway/gateway-services/src/ward_layout/rules.rs::WardLayoutDocument` and the created-Ward filesystem contract
- **Shape:** mixed

> **Spec contract:** this document defines what "done" means. The implementing
> PR must match this spec, or update it. Verification must be derivable from it.

## Objective

Give a user who creates any bundled Ward archetype a compact,
filesystem-authoritative LLM Wiki foundation: exactly one canonical
`<ward-id>.md` page, `AGENTS.md`, append-only `log.md`, and the copied
`ward-conf.yaml` snapshot at the Ward root, with every optional work directory
remaining lazy. The configured `wards/index.md` is the only `index.md` and
links each Ward to its canonical page. All Ward Markdown in this slice uses
ordinary UTF-8 Markdown rather than mandatory OKF `type` metadata, while
existing snapshot isolation, archetype selection, confinement, atomic
creation, and journal day routing remain unchanged.

## Boundaries

### Always do

- Preserve the exact validated `ward-conf.yaml` bytes as the created Ward's
  structural authority and preserve the seven closed archetype identifiers.
- Derive the canonical filename only from the already validated Ward ID and
  publish the complete required tree through the existing confined,
  no-clobber creation boundary.
- Keep `wards/index.md` idempotent and link every registered Ward to
  `<ward-id>/<ward-id>.md`.
- Keep optional archetype directories and `.zbot/specs/<concept>/` lazy.
- Treat plain Markdown without frontmatter as valid and leave optional
  frontmatter parsing/validation to the Wiki operations slice.
- Preserve the journal rule that one source day maps to one
  `entries/YYYY/YYYY-MM-DD.md` file unless compilation is explicitly requested.

### Ask first

- Adding a new public archetype, a new required root file, or any parsing,
  interpretation, or validation of optional frontmatter.
- Changing an existing Ward snapshot, migrating old Ward contents, or importing
  database Wiki articles into files.
- Adding a dependency or expanding this slice into wikilink resolution,
  backlinks, ingest mutations, database projection, HTTP/UI work, or semantic
  lint.

### Never do

- Never retain a Ward-local or nested `index.md`; the global Ward catalog is
  the only reserved index.
- Never make `WikiStore`, `__index__`, embeddings, FTS, or model output
  authoritative for the created filesystem.
- Never generate a canonical path from free-form display text, an alias, model
  output, or an unvalidated path component.
- Never claim this slice enforces post-ingest source immutability, calculates
  backlinks, resolves cross-Ward links, or safely publishes multi-file Wiki
  edits; those controls ship with their owning follow-on specs.
- Never rewrite existing Wards or synchronize live templates without a
  create-new backup and a before/after existing-Ward manifest.

## Testing Strategy

- **Canonical placeholder and path safety — TDD unit/property tests:** prove
  the root-only Ward placeholder compiles, renders exactly one safe lowercase
  `<ward-id>.md`, rejects uppercase or unsupported placement and unsafe
  components, and cannot escape or collide with reserved files.
- **Bundle construction — TDD parameterized integration tests:** create all
  seven archetypes through the real loader/materializer and assert their exact
  four-file roots, snapshot bytes, valid lint, plain-Markdown canonical/log
  files, and absence of optional directories or nested indexes.
- **Global catalog behavior — TDD tool tests:** prove first-use creation,
  canonical links, idempotent registration, symlink/special-file rejection,
  and preservation of user-authored catalog content.
- **Archetype doctrine and layout — goal-based source audit plus construction
  tests:** verify every bundle contains no `okf-v0.1`, no index starter, the
  shared LLM Wiki workflow, and only its declared lazy structures; separately
  prove the journal dated route remains accepted and a flat entry remains
  structurally rejected.
- **Fresh acceptance environment — goal-based E2E:** with a fresh vault and
  database, normal bootstrap and explicit creation produce each selectable
  archetype and its global canonical link without pre-copying templates or
  records.
- **Live-template safety — goal-based manifest audit:** after a create-new
  backup, repository and live template manifests match while the recursive
  type/path/content manifest of existing Wards is byte-identical before and
  after synchronization.
- **Stub coverage before execution:** T1–T4 carry candidate Rust red stubs for
  AC 1–5 in the plan; the `work-loop` PLAN phase must materialize and compile
  them in their named test modules before EXECUTE. AC 6–11 are goal-based
  construction, E2E, manifest, or scoped-diff checks and intentionally carry
  no TDD stub.

## Acceptance Criteria

- [x] A fresh creation of each bundled archetype produces exactly
  `AGENTS.md`, `<ward-id>.md`, `log.md`, and `ward-conf.yaml` at the Ward root;
  no optional directory or other Markdown file is materialized.
- [x] The required canonical page is named from the validated Ward ID, contains
  a human-readable title plus `Concepts` and `Tags` navigation, and lints as
  valid plain Markdown without requiring or interpreting frontmatter.
- [x] The required `log.md` starts with an append-only operational-log contract
  and documents the parseable `## [YYYY-MM-DD] <operation> | <subject>`
  heading form without fabricating an initial ingest.
- [x] `wards/index.md` is the only `index.md` seeded or registered by this
  feature; registering `<ward-id>` adds exactly one relative link to
  `[[<ward-id>/<ward-id>|<display-name>]]`, is idempotent, and preserves
  existing regular-file content. Registration requires a no-follow regular
  file with exactly one hard link and rejects symlink, hardlink-alias, and
  special-file catalogs before writing. If registration fails after a new
  Ward is published, creation rolls that Ward back and reports failure rather
  than leaving an unlisted Ward.
- [x] The Ward Layout grammar supports one root-only canonical Ward placeholder
  whose materialized component is derived solely from a validated lowercase
  ASCII Ward slug; new Ward creation rejects uppercase IDs rather than silently
  normalizing them. Unsupported placement, reserved-name collision, case-fold
  collision, separators, dot components, links, hardlink aliases, and special
  files fail before publication.
- [x] Every bundled `ward-conf.yaml` uses `markdown` rather than `okf-v0.1` for
  Markdown files; declares shared lazy `pages/`, `sources/`, and
  `.zbot/specs/<concept>/` namespaces without a tasks index; and preserves its
  archetype-specific optional work areas.
- [x] Every bundled `AGENTS.md` doctrine defines immutable-source intent and
  the ingest/query/lint maintenance lifecycle while explicitly avoiding claims
  that this slice has implemented source mutation enforcement, backlinks,
  database projection, or cross-Ward traversal.
- [x] The journal bundle accepts `entries/2026/2026-07-28.md`, rejects Markdown
  directly under `entries/`, and instructs the agent to preserve one file per
  source day unless the user explicitly requests compilation.
- [x] Fresh-vault and fresh-database E2E coverage explicitly creates all seven
  archetypes through the normal Ward tool path and verifies canonical global
  catalog links without test-only template or record injection.
- [x] Applying and synchronizing this feature never modifies an existing Ward;
  the live template registry matches the repository only after a recoverable
  create-new backup and an unchanged before/after Ward manifest. Backup,
  staging, replacement, and rollback resolve only through the
  `VaultPaths`-confined template registry; reject symlinked ancestors,
  symlinks, hardlink aliases, case-fold collisions, special files, handle
  identity changes, and any existing backup/staging destination, and publish
  with no-follow/no-replace operations.
- [x] This slice makes no behavioral change to `compile_ward_wiki`,
  `WikiStore`, Ward recall, Ward content HTTP responses, embeddings, the UI, or
  existing Ward snapshots, and does not parse or validate optional page
  frontmatter beyond the existing ordinary-Markdown read limits.

## Assumptions

- Technical: Ward layouts already support plain Markdown alongside OKF
  (source: `gateway/gateway-services/src/ward_layout/rules.rs`).
- Technical: current materialization skips dynamic `{name}` paths, so the
  canonical root page needs a bounded runtime feature
  (source: `gateway/gateway-services/src/ward_layout/create.rs` and focused
  construction probe, 2026-07-28).
- Technical: the current compiler and Ward content API use a separate database
  Wiki and `__index__`, which are outside Slice 1
  (source: `gateway/gateway-execution/src/ward_wiki.rs` and
  `gateway/src/http/ward_content.rs`).
- Technical: the implementation spans the Rust `gateway-services`,
  `gateway-execution`, and `agent-tools` crates plus YAML/Markdown templates
  and E2E fixtures, so the feature shape is mixed (source: workspace
  `Cargo.toml` files and user confirmation 2026-07-28).
- Product: a fresh Ward has exactly four required files and optional
  directories remain lazy (source: user confirmation 2026-07-28).
- Product: `wards/index.md` is the sole index, with Ward-local identity carried
  by `<ward-id>.md` (source: user confirmation 2026-07-28).
- Product: Slice 1 excludes link/backlink indexing, ingest mutation,
  filesystem compiler/projection, API cutover, UI, and legacy migration
  (source: user confirmation 2026-07-28).
- Product: acceptance uses deleted Wards and a fresh database
  (source: user confirmation 2026-07-28).
- Process: implementation waits for explicit RFC and spec approval
  (source: `docs/CONVENTIONS.md` and user confirmation 2026-07-28).
- Interface: Slice 1 changes the existing Ward layout/filesystem contract but
  introduces no REST, event, GraphQL, or RPC contract
  (source: user confirmation 2026-07-28).
