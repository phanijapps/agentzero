# Spec: Compact Bundled Ward Archetypes

- **Status:** Shipped
- **Owner:** phanijapps
- **Plan:** [`plan.md`](plan.md)
- **Constrained by:** [RFC-0017](../../rfc/0017-ward-layout-archetypes.md), [RFC-0018](../../rfc/0018-filesystem-authoritative-llm-wiki-wards.md), [ADR-0002](../../adr/0002-select-complete-ward-archetypes-at-creation.md), [ADR-0003](../../adr/0003-use-filesystem-authoritative-llm-wiki-wards.md)
- **Brief:** `dynamic-ward-layout-archetypes`
- **Contract:** none
- **Shape:** data

Product provenance: [Dynamic Ward Layout Archetypes brief](../../product/briefs/dynamic-ward-layout-archetypes.md).

> **Spec contract:** this document defines what "done" means. The implementing
> PR must match this spec, or update it. Verification must be derivable from it.

## Objective

Provide complete local bundles for `generic`, `coding`, `documentation`,
`journal`, `ebook`, `research`, and `news` without creating empty work trees.
Every new Ward starts with one canonical `<ward-id>.md`, `AGENTS.md`,
append-only `log.md`, and its immutable `ward-conf.yaml` snapshot. Its selected
bundle declares a compact, specialized
set of optional work areas plus one shared planning convention at
`.zbot/specs/<concept>/`. Journal Wards additionally preserve daily chronology
as one file per day under `entries/YYYY/YYYY-MM-DD.md`.

## Boundaries

### Always do

- Keep every bundle complete, local, data-only, user-editable, and independently
  valid without a base/overlay.
- Keep all non-core work directories optional so a new Ward contains no empty
  scaffolding.
- Give every Ward one plain-Markdown canonical page whose concepts and tags use
  links instead of a Ward-local index.
- Declare `.zbot/specs/<concept>/spec.md`, `plan.md`, and an optional
  `tasks/` directory consistently in all seven complete bundles.
- Use the existing bounded `ward-agent.md` renderer for doctrine and declare
  every starter destination in the selected layout.
- Preserve source material separately from notes/derivatives in ebook,
  research, and news structures.
- Run the same construction harness and resource ceilings against all seven
  initial archetypes.

### Ask first

- Adding a new archetype or changing the seven public identifiers.
- Adding a required path beyond the canonical page, `AGENTS.md`, and `log.md`,
  removing/renaming a declared optional path, or adding
  archetype-specific runtime behavior.
- Introducing binary starter assets, generated content, or a typed mutation
  operation.

### Never do

- Never add executable hooks, remote content, shell/environment interpolation,
  inheritance, or a bundle-specific parser/validator.
- Never encode daily-entry, book, research-run, or publishing operations into
  this data slice; those need separate specs.
- Never migrate, rewrite, or delete an existing Ward as part of bundle
  installation.
- Never flatten multiple daily journal entries into one compiled document
  unless the user explicitly requests a compilation.
- Never create a new top-level crate/module boundary or make a skill/prompt the
  structural authority.
- Never require OKF metadata or force Wiki organization onto code,
  non-Markdown source assets, or immutable raw sources.

## Testing Strategy

- **Bundle validity and compact tree — parameterized construction tests:** each
  bundle compiles, materializes, lints, and produces the same four-entry
  initial tree: `<ward-id>.md`, `AGENTS.md`, `log.md`, and `ward-conf.yaml`.
- **Doctrine and starter safety — goal-based boundary tests:** named loader
  artifacts exercise placeholder, link/injection, byte/count/depth, and
  declared-destination invariants through the real bundle path.
- **User usefulness — content assertions:** verify canonical pages use ordinary
  Markdown and link to concepts and tags without freezing all prose.
- **All-archetype clean start — goal-based E2E:** a fresh database and vault
  must seed and explicitly create every archetype through the normal path.
- **Journal daily-route regression — TDD construction test:**
  `ward_layout::create::tests::journal_bundle_routes_daily_entries_without_eager_scaffolding`
  proves compact creation, accepts a nested dated entry, rejects Markdown
  directly under `entries/`, and pins the doctrine/canonical-page route
  language.
- **Existing-Ward preservation — goal-based live-template audit:** compare a
  recursive type/path/content manifest of `~/Documents/zbot/wards` before and
  after the create-new backup and journal-template replacement; the digests
  must be identical.

## Acceptance Criteria

- [x] A fresh vault non-destructively seeds complete bundles for all seven
  public archetypes; repeat bootstrap preserves every existing bundle file.
- [x] Creating any archetype produces exactly `<ward-id>.md`, `AGENTS.md`,
  `log.md`, and `ward-conf.yaml` at the Ward root, with no optional directory
  materialized.
- [x] Every bundle declares the shared optional planning tree
  `.zbot/specs/<concept>/spec.md`, `.zbot/specs/<concept>/plan.md`, and
  repeatable Markdown files beneath `.zbot/specs/<concept>/tasks/`; no task
  index is required.
- [x] `generic` permits optional `pages/`, `sources/`, and `outputs/`;
  `coding` permits `src/`, `tests/`, `docs/`, `scripts/`, and `artifacts/`.
- [x] `documentation` permits `topics/`, `sources/`, and `outputs/`; `journal`
  permits `entries/`, `topics/`, and `attachments/`.
- [x] `ebook` permits `books/`, `sources/`, and `outputs/`; `research` permits
  `subjects/`, `sources/`, `data/`, and `outputs/`; `news` permits
  `briefings/`, `sources/`, and `archive/`.
- [x] Every canonical starter is valid plain Markdown with concept and tag
  links that can be expanded as optional structures appear; no Ward-local or
  nested `index.md` is declared.
- [x] Each bundle's `ward-agent.md` renders through the existing byte limit,
  placeholder, no-link/injection, and diagnostic controls; no doctrine falls
  back to or overlays another archetype.
- [x] Every starter is validated before mutation as bounded Markdown: reject
  remote or absolute links, role impersonation, instruction resets, privileged
  prompt references, unsupported placeholders, symlinks, hardlinks, excessive
  byte/count/depth totals, and diagnostics containing source content or
  absolute paths. Relative and same-document anchor links remain allowed.
- [x] All seven archetypes pass one parameterized suite for safe loading,
  compilation, exact snapshot bytes/digest, compact required tree, starter
  ceilings, doctrine rendering, lint, and post-creation registry-edit
  isolation.
- [x] No bundle contains hooks, remote references, undeclared destinations,
  executable starter files, hidden inheritance, environment/shell tokens, or
  archetype-specific schema/parser fields.
- [x] With a fresh database and vault, normal bootstrap and explicit creation
  produce a compact selected archetype; no test pre-copies templates or
  database records.
- [x] Applying this change never modifies existing Ward directories. Live
  development templates may be synchronized only after a recoverable backup;
  the user will delete old Wards before acceptance testing.
- [x] Live synchronization is rooted at the canonical
  `<vault>/config/templates/wards` registry, refuses to access `<vault>/wards`,
  and rejects absolute/dot/separator components, symlinks, hardlink aliases,
  case-fold collisions, and unexpected file types. The backup uses a
  create-new destination and template publication must not overwrite any file
  outside that explicitly resolved registry.
- [x] A journal bundle declares `entries/<year>/*.md` as an optional two-level
  structural namespace; Ward lint rejects Markdown placed directly under
  `entries/`. Its doctrine and canonical starter name the
  `entries/YYYY/YYYY-MM-DD.md` route and require one output file per source day
  unless the user explicitly requests a compilation. Calendar syntax and
  parent-year/filename agreement are doctrine, not validated by the current
  wildcard matcher.
- [x] Updating the journal bundle preserves compact creation and does not
  rewrite the snapshot or contents of any existing Ward.

## Assumptions

- Technical: optional directories and repeatable nodes fit the existing
  data-only Ward Layout compiler; no schema change is needed (source:
  `gateway/gateway-services/src/ward_layout/` and codegraph impact scan,
  2026-07-27).
- Technical: bundles can share a construction harness without a standalone
  contract or new parser (source: user confirmation 2026-07-26 and
  `gateway/gateway-services/src/ward_layout/`).
- Process: identifiers and snapshot behavior are governed by RFC-0017 and
  ADR-0002; material changes update this spec (source:
  `docs/CONVENTIONS.md`).
- Product: all seven archetypes, including `generic`, must use the compact,
  lazy structure (source: user confirmation 2026-07-27).
- Product: planning belongs under `.zbot/specs/<company-or-concept>/`, and
  indexes link concepts and tags (source: user confirmation 2026-07-27).
- Product: no Ward migration is required because acceptance uses deleted Wards
  and a fresh database (source: user confirmation 2026-07-27).
- Product: a daily journal preserves each source day as its own dated file,
  rather than flattening days into one compiled journal (source: failed session
  `sess-d2213c89-2da3-4eb9-91dd-74325b848ecd` and user confirmation
  2026-07-27).
- Technical: the current layout matcher can enforce directory depth and
  Markdown placement but not calendar-valid wildcard components; adding a
  date-pattern validator is outside this template-only fix (source:
  `gateway/gateway-services/src/ward_layout/rules.rs`, verified 2026-07-27).
