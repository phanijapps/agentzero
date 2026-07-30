# Plan: Compact Bundled Ward Archetypes

- **Spec:** [`spec.md`](spec.md)
- **Status:** Done

## Approach

Generalize the existing two-bundle seed manifest to all seven archetypes, then
replace eager required trees with complete, independent YAML bundles whose work
areas are optional. Give each bundle one canonical page, append-only log, and concise
archetype doctrine. Prove the compact tree through parameterized construction
tests and the existing fresh-vault E2E path.

## Constraints

- Depends on T1, T2, and the construction harness from
  [`ward-archetype-registry-and-creation`](../ward-archetype-registry-and-creation/plan.md).
- Follow RFC-0017, ADR-0002, and the existing Ward Layout/doctrine limits.
- Use only the bounded loading, validation, and no-replace publication helpers
  named in the [architecture security
  boundary](../../architecture/architecture.md#ward-layout-and-template-security);
  template data must never broaden filesystem or tool authority.
- No automatic intent selection dependency; this data slice must be testable
  through explicit creation alone.

## Construction tests

**Integration tests:** parameterized all-bundle seed/load/create/snapshot/lint
suite, exact compact-tree assertion, shared planning-rule assertion, and
fresh-start creation through the normal path.

**Manual verification:** inspect one created ward per archetype and follow its
canonical-page links; no subjective prose review gates release.

## Design (LLD)

### Data & schema

Each archetype is a complete instance of the existing Ward Layout schema plus
bounded `ward-agent.md`, `starters/canonical.md`, and `starters/log.md`. Only
the canonical page, log, and agent doctrine are required. Optional typed directories and the
duplicated `.zbot/specs/*` rule retain complete-bundle semantics without
inheritance. Traces to: AC1-AC10.

### Interfaces & contracts

The bundle registry from Slice 1 is the only consumer-facing surface. Bundle
fixtures implement it as data and add no API or event contract. Traces to:
AC1, AC9-AC11.

## Tasks

### T1: Construction tests define the compact seven-bundle contract

**Depends on:** none

**Touches:** `gateway/gateway-services/src/ward_layout/create.rs`, `gateway/gateway-services/src/ward_layout/loader.rs`, `gateway/gateway-services/src/ward_layout/lint.rs`

**Verification mode:** goal-based construction test.

**Tests:**

- Parameterized tests iterate `WardArchetypeId::ALL`, load every bundle, create
  a Ward, assert the exact four-entry root tree, inspect canonical-page links,
  verify snapshot equality, and lint (AC1-AC10).
- Repeat bootstrap edits one existing bundle and proves preservation (AC1).
- Named artifact:
  `ward_layout::create::tests::all_archetypes_materialize_compact_complete_bundles`.

**Approach:**

- Update the existing reference-archetype tests before changing production
  manifests.

**Done when:** the tests compile and fail because five bundles are absent and
the two existing bundles are eager.

### T2: Seed and load all seven complete bundles

**Depends on:** T1

**Touches:** `gateway/gateway-services/src/ward_layout/loader.rs`,
`gateway/templates/wards/*`

**Verification mode:** goal-based integration test.

**Tests:**

- All seven defaults validate before any filesystem mutation and seed
  create-once (AC1).
- All seven load with exactly one declared root starter (AC1, AC7-AC9).
- Starter validation rejects remote/absolute links, role labels, instruction
  resets, privileged prompt references, and unsupported placeholders while
  accepting relative and anchor navigation.
- Named artifact:
  `ward_layout::loader::tests::all_default_archetypes_seed_once_and_load_bounded_bundles`.

**Approach:**

- Replace the pair of hardcoded seed functions with one static bundled manifest
  covering the closed enum.

**Done when:** every default bundle seeds and user-edited files remain intact.

### T3: Compact layouts and indexes specialize optional work areas

**Depends on:** T2

**Touches:** `gateway/templates/wards/*/ward-conf.yaml`,
`gateway/templates/wards/*/ward-agent.md`,
  `gateway/templates/wards/*/starters/canonical.md`,
  `gateway/templates/wards/*/starters/log.md`

**Verification mode:** goal-based construction checks.

**Tests:**

- Exact root entries are only `<ward-id>.md`, `AGENTS.md`, `log.md`, and
  `ward-conf.yaml` for every archetype (AC2).
- Compiled rules expose the shared `.zbot/specs/*` contract and each expected
  archetype-specific optional path (AC3-AC7).
- Canonical pages are plain Markdown and link concepts and tags (AC8).

**Approach:**

- Duplicate the small planning definition in each complete bundle as required
  by RFC-0017; keep specialized folders optional.

**Done when:** all parameterized construction tests pass.

### T4: Fresh-vault E2E and development-template rollout

**Depends on:** T3

**Touches:** `e2e/playwright/full-mode/ward-archetypes.full.spec.ts`,
`docs/product/changelog.md`, live development template directory

**Verification mode:** goal-based construction checks.

**Tests:**

- Fresh bootstrap creates a compact coding Ward and does not create optional
  paths (AC2, AC12).
- A rollout preflight resolves the canonical vault and registry, refuses any
  source or destination outside `<vault>/config/templates/wards`, rejects
  absolute/dot/separator components, links, hardlink aliases, case-fold
  collisions, and unexpected file types, and proves `<vault>/wards` is neither
  traversed nor changed (AC13-AC14).
- The timestamped backup destination is created new; synchronization may
  replace only the explicitly resolved registry files, and before/after Ward
  manifests remain byte-identical (AC13-AC14).

**Approach:**

- Update E2E expectations, run focused and workspace gates, then copy the
  validated bundle registry to the canonical development template root after a
  create-new timestamped backup. Resolve literal components, reject links,
  hardlink aliases and case-fold collisions, and never traverse the live
  `wards/` directory.

**Done when:** the clean-start path is compact and the live template backup
location is reported.

### T5: Journal daily entries remain discrete and date-routed

**Depends on:** T4

**Touches:** `gateway/templates/wards/journal/ward-conf.yaml`,
`gateway/templates/wards/journal/ward-agent.md`,
`gateway/templates/wards/journal/starters/canonical.md`,
`gateway/templates/wards/journal/starters/log.md`,
`gateway/gateway-services/src/ward_layout/create.rs`, live journal template

**Verification mode:** TDD plus goal-based live-template audit.

**Tests:**

- A regression test fails against the loose journal bundle, then proves a
  fresh journal Ward stays four-file compact, its layout accepts
  `entries/2026/2026-07-27.md`, and its doctrine/canonical page name the route
  and one-file-per-day rule.
- The same test proves an unstructured Markdown file directly under
  `entries/` fails Ward lint.
- The test does not claim calendar validation: malformed dates inside the
  required two-level structure remain a documented matcher limitation.
- Named regression artifact:
  `gateway/gateway-services/src/ward_layout/create.rs::tests::journal_bundle_routes_daily_entries_without_eager_scaffolding`.
- The live journal bundle matches the repository bundle after a create-new
  backup, while a before/after manifest proves existing Wards are unchanged.
- Named live-audit command (capture `ward_before` before synchronization, then
  execute the remaining assignments and assertions afterward):

  ```bash
  set -euo pipefail
  tree_manifest() {
    audit_root=$1
    (
      cd -- "$audit_root"
      {
        find -P . -mindepth 1 -printf '%y %P %l\n' | LC_ALL=C sort
        find -P . -type f -print0 |
          LC_ALL=C sort -z | xargs -0 -r sha256sum
      } | sha256sum | cut -d' ' -f1
    )
  }
  ward_before=$(tree_manifest "/home/videogamer/Documents/zbot/wards")
  ward_after=$(tree_manifest "/home/videogamer/Documents/zbot/wards")
  source_journal=$(tree_manifest "/home/videogamer/projects/agentzero/gateway/templates/wards/journal")
  live_journal=$(tree_manifest "/home/videogamer/Documents/zbot/config/templates/wards/journal")
  test "$ward_before" = "$ward_after"
  test "$source_journal" = "$live_journal"
  ```

**Approach:**

- Replace the optional leaf `entries/` rule with an optional year namespace
  containing repeatable plain Markdown entries.
- Tighten only journal doctrine and canonical-page guidance; do not add a runtime
  operation or change intent selection.

**Done when:** the regression is green, the focused Ward suite passes, and the
live journal template is synchronized without touching existing Wards.

## Rollout

Ship all seven compact bundles together. Seeding remains additive and
non-destructive. Existing vault bundles are intentionally preserved by normal
bootstrap, so local acceptance requires an explicit backed-up template sync.
No implementation step deletes or edits an existing Ward.

## Risks

- Optional directories are not auto-created by typed operations in this slice;
  agents create them with ordinary Ward file tools and lint against the
  snapshot.
- Complete bundles duplicate the planning rule; parameterized tests prevent
  drift across the seven copies.

## Changelog

- 2026-07-26: initial expansive plan.
- 2026-07-27: approved compact/lazy amendment for all seven archetypes.
- 2026-07-27: implemented, synchronized with a recoverable live-template
  backup, and verified through all-archetype construction and fresh-vault E2E.
- 2026-07-27: opened T5 after a journal request flattened 53 daily entries into
  one compiled document because the journal template left `entries/`
  structurally and doctrinally ambiguous.
