# Plan: LLM Wiki Ward Foundation

- **Spec:** [`spec.md`](spec.md)
- **Status:** Done

## Approach

Add one root-only `{ward}` placeholder to the existing data-only Ward Layout
grammar and resolve it from the validated Ward ID during lint and atomic
materialization. Replace the bundle loader's hard-coded index starter with a
bounded canonical-page starter plus a log starter. Update the global Ward
catalog to link canonical pages, then convert all seven bundles and doctrines
from required OKF indexes to compact plain-Markdown LLM Wiki foundations.
Prove the shared behavior through parameterized Rust construction tests before
running the fresh-vault E2E and the recoverable live-template synchronization.
Do not touch the database Wiki compiler, recall, HTTP content, or UI.

## Constraints

- RFC-0018 defines filesystem authority, one global index, canonical Ward
  identity, optional bounded metadata, and the staged rollout; implementation
  cannot start until the RFC is accepted.
- ADR-0003 retains validated, versioned, exact-byte Ward snapshots,
  confinement, no fallback activation, and non-destructive existing-Ward
  behavior while replacing ADR-0001's OKF/index clauses.
- ADR-0002 still requires seven complete, closed, local bundles selected only
  at creation and isolated after publication.
- No new dependency, executable template, inheritance/merge mechanism, remote
  reference, schema generator, database migration, API, or UI is introduced.
- Security invariants retain no-follow/no-clobber publication, bounded regular
  files, safe Ward IDs, case-fold collision rejection, and failure before
  partial Ward publication.

## Construction tests

**Integration tests:**

- Parameterized fresh-vault creation of all seven bundles through
  `create_ward_from_archetype`, asserting the exact four-file root, canonical
  filename/content, log content, snapshot bytes/digest, lint, and registry-edit
  isolation.
- Normal Ward tool creation/registration proving the one global catalog link
  and idempotent preservation behavior.
- Fresh-database Playwright E2E creates every explicit archetype and observes
  the resulting files/catalog without pre-seeded records.

**Goal-based audits:**

- Fail if any bundled Ward template contains `okf-v0.1` or
  `starters/index.md`.
- Compare recursive repository/live template manifests after backup.
- Compare existing-Ward manifests before and after live synchronization.

**Manual verification:** none beyond inspecting the backed-up live template
location reported by the synchronization audit.

T1–T4 include candidate red stubs below. Per `new-spec` step 4, they are not
committed to production test modules during spec authoring. The full
`work-loop` PLAN phase must materialize each snippet at its named path, run one
compile pass plus one bounded correction pass, and replace its pending marker
with `stub: true` before plan approval and EXECUTE.

## Design (LLD)

### Design decisions

- Add `{ward}` rather than repurpose nested `{name}`: its scope and authority
  are explicit, it resolves only at the root, and it cannot be model-authored.
  Traces to AC 2 and AC 5.
- Keep `ward-conf.yaml` as the structural contract; `markdown` relaxes document
  metadata without weakening path policy. Traces to AC 5 and AC 6.
- Seed a canonical starter and log starter in every complete bundle rather than
  introduce inheritance. Traces to AC 1, AC 2, AC 3, and AC 6.
- Preserve the database Wiki untouched until its owning projection spec.
  Traces to AC 11.

### Data & schema

Extend `RuleNode.match_pattern` validation with the literal `{ward}` token:
exactly once, only on a required non-repeat root file rule, with the suffix
`.md`. Compilation rejects `{ward}` in definitions, nested nodes, directories,
optional/repeat nodes, excludes, or combined wildcard patterns. At activation,
the token resolves to the already validated lowercase Ward slug plus `.md`;
new creation rejects uppercase rather than normalizing it, and lint derives the
same ID from the real Ward root selected by `VaultPaths`.

Bundle starter storage uses fixed safe source names, not brace-bearing paths:
`starters/canonical.md` and `starters/log.md`. The loader represents the
canonical starter as a role and binds its destination only after receiving the
validated Ward ID. Both retain current byte/count/depth, UTF-8, regular-file,
single-link, placeholder, link, and injection validation.

### Interfaces & contracts

The external Ward tool request schema is unchanged. The created-Ward
filesystem contract changes from `index.md` plus two control files to
`<ward-id>.md`, `log.md`, plus the same control files. The active
`ward-conf.yaml` projection exposes `{ward}.md` as untrusted layout data under
the existing context boundary. No REST/event contract changes.

### Component / module decomposition

- `ward_layout/rules.rs`: compile and validate the root-only placeholder.
- `ward_layout/loader.rs`: seed/load canonical and log starter roles safely.
- `ward_layout/create.rs`: resolve canonical destination and materialize the
  exact required tree on Linux and portable paths.
- `ward_layout/lint.rs`: resolve the root placeholder using the validated Ward
  identity and report structural mismatch.
- `runtime/agent-tools/src/tools/ward.rs`: seed/register the sole global
  catalog using canonical relative links.
- `gateway/templates/wards/*`: seven layouts, canonical/log starters, and
  archetype doctrines.
- `e2e/playwright/full-mode/ward-archetypes.full.spec.ts`: fresh acceptance
  path.

### State & control flow

Creation remains validate → load complete selected bundle → resolve the
validated Ward ID → stage required files and exact snapshot → lint staged
tree → atomically publish → register the canonical link. A failure before
publication leaves no Ward; a catalog registration failure immediately uses
the existing rollback primitive to remove the newly published, still-empty
Ward and reports creation failure. Rollback is limited to the Ward created by
that invocation and never applies when reusing an existing Ward.

### Behavior & rules

- Only `wards/index.md` is special; no bundle may declare or seed another.
- Canonical filenames use Ward IDs, while H1 titles use the existing display
  name renderer.
- `log.md` is created empty of fake operations but contains its append-only
  format contract.
- Frontmatter is not required or interpreted in this slice; the four starter
  files need no OKF `type`.
- Planning paths remain `.zbot/specs/<concept>/spec.md`, `plan.md`, and
  optional named task Markdown files; there is no tasks index.
- Every bundle declares lazy `pages/` and `sources/` namespaces in addition to
  its archetype-specific optional work areas.

### Failure, edge cases & resilience

Reject misplaced/multiple `{ward}` tokens, unsafe IDs, reserved or case-fold
collisions, starter link/hardlink/special-file substitution, oversized content,
snapshot mismatch, existing destinations, and staged-tree lint failures.
Catalog reads/writes retain same-handle validation and cannot follow a replaced
link. Existing Wards and their snapshots are never opened for mutation during
template synchronization.

### Quality attributes (NFRs)

Fresh Ward creation remains bounded by existing template byte/count/depth and
publication limits. No network, embedding, LLM, or database call is added to
layout resolution. Cross-platform tests cover Linux descriptor-relative
publication and the portable fallback.

### Dependencies & integration

Reuse `serde_yaml`, existing Ward Layout types, `VaultPaths`, confined
publication helpers, the Ward tool, and current E2E harness. No new crate or
external service is added.

## Tasks

### T0: Accepted governance points every active contract at LLM Wiki Wards

**Depends on:** none

**Touches:** `docs/rfc/0015-okf-aligned-ward-layout-and-capabilities.md`,
`docs/rfc/0018-filesystem-authoritative-llm-wiki-wards.md`,
`docs/rfc/README.md`, `docs/specs/okf-ward-foundation/**`,
`docs/specs/okf-mini-obsidian-ui/**`,
`docs/specs/bundled-ward-archetypes/**`, `docs/specs/README.md`

**Verification mode:** goal-based; no stub (mode).

**Tests:**

- RFC-0018 is Accepted and RFC-0015's status points to its superseding RFC.
- Draft OKF foundation/UI specs use the valid `Archived` status, while the
  active bundled-archetypes spec and plan no longer require OKF metadata,
  Ward-local/nested indexes, or `tasks/index.md`.
- `lint-spec-status.py` reports no new lifecycle/metadata error from these
  transitions.

**Approach:**

- Perform status-only updates to frozen RFC history after explicit acceptance.
- Archive obsolete Draft specs and amend the still-active archetype contract in
  place before implementation changes its promised behavior.

**Done when:** governance indexes and active specs consistently name the LLM
Wiki foundation and no active contract still mandates the replaced OKF/index
shape.

### T1: Root canonical placeholders compile, resolve, and fail safely

**Depends on:** T0

**Touches:** `gateway/gateway-services/src/ward_layout/rules.rs`,
`gateway/gateway-services/src/ward_layout/lint.rs`,
`gateway/gateway-services/src/ward_layout/create.rs`

**Verification mode:** TDD.

**Tests:**

- `root_ward_placeholder_compiles_as_one_required_file` and
  `new_ward_ids_require_lowercase_canonical_slugs` (AC 5)
  `stub: true`

  ```rust
  // STUB: AC5 — rules.rs tests
  #[test]
  fn root_ward_placeholder_compiles_as_one_required_file() {
      let yaml = "apiVersion: zbot.dev/v1alpha1\nkind: WardLayout\nroot:\n  kind: directory\n  children:\n    - { id: canonical, match: '{ward}.md', kind: file, format: markdown }\n";
      let layout = compile(yaml).expect("root canonical placeholder must compile");
      assert_eq!(
          layout.root.children[0].match_pattern.as_deref(),
          Some("{ward}.md")
      );
  }

  // STUB: AC5 — create.rs tests
  #[test]
  fn new_ward_ids_require_lowercase_canonical_slugs() {
      assert!(validate_ward_id("financial-analysis").is_ok());
      assert!(validate_ward_id("Financial-Analysis").is_err());
  }
  ```

- Red tests cover the accepted root `{ward}.md` form and reject nested,
  optional, repeated, directory, wildcard-combined, duplicate-token, reserved,
  separator, dot, and case-fold collision cases (AC 5).
- New creation rejects uppercase Ward IDs and does not silently normalize them
  into a possibly colliding canonical filename (AC 5).
- Lint accepts the resolved canonical page and reports missing/wrong canonical
  identities without following links (AC 2, AC 5).

**Approach:**

- Extend placeholder validation without changing `{name}` semantics.
- Tighten only new-Ward ID validation to the RFC lowercase slug grammar;
  existing-Ward reuse remains snapshot-governed.
- Carry the validated Ward identity into root lint matching and keep dynamic
  resolution confined to one component.

**Done when:** focused rule/lint tests prove one safe root placeholder and every
invalid placement fails before mutation.

### T2: Complete bundles load canonical and log starter roles

**Depends on:** T1

**Touches:** `gateway/gateway-services/src/ward_layout/loader.rs`,
`gateway/gateway-services/src/ward_layout/mod.rs`

**Verification mode:** TDD.

**Tests:**

- `default_bundle_seeds_canonical_and_log_starters` (AC 1–3)
  `stub: true`

  ```rust
  // STUB: AC1, AC2, AC3 — loader.rs tests
  #[test]
  fn default_bundle_seeds_canonical_and_log_starters() {
      let vault = tempfile::tempdir().unwrap();
      let paths = VaultPaths::new(vault.path().to_path_buf());
      paths.ensure_dirs_exist().unwrap();
      seed_default_ward_archetypes(&paths).unwrap();

      let starters = paths
          .ward_archetype_bundle(WardArchetypeId::Generic)
          .join("starters");
      assert!(starters.join("canonical.md").is_file());
      assert!(starters.join("log.md").is_file());
      assert!(!starters.join("index.md").exists());
  }
  ```

- Loader tests require exactly one bounded canonical starter and one bounded
  log starter for every bundle (AC 1–3).
- Missing, duplicate, undeclared, symlinked, hardlinked, non-Markdown,
  oversized, placeholder-injected, and remote-link starters fail closed
  without changing the registry (AC 5).
- Repeat bootstrap preserves user-customized bundle files.

**Approach:**

- Replace the hard-coded `index` bundle field with canonical/log starter roles.
- Seed fixed `starters/canonical.md` and `starters/log.md`; bind the canonical
  destination only during creation.

**Done when:** all bundle loader safety tests pass and no loader assumes a root
`index.md`.

### T3: Atomic creation emits the exact four-file LLM Wiki foundation

**Depends on:** T1, T2

**Touches:** `gateway/gateway-services/src/ward_layout/create.rs`

**Verification mode:** TDD plus integration.

**Tests:**

- `fresh_generic_ward_has_exact_llm_wiki_root` (AC 1–3)
  `stub: true`

  ```rust
  // STUB: AC1, AC2, AC3 — create.rs tests
  #[test]
  fn fresh_generic_ward_has_exact_llm_wiki_root() {
      let vault = tempfile::tempdir().unwrap();
      let paths = VaultPaths::new(vault.path().to_path_buf());
      paths.ensure_dirs_exist().unwrap();
      let bundle = paths.ward_archetype_bundle(WardArchetypeId::Generic);
      std::fs::create_dir_all(bundle.join("starters")).unwrap();
      std::fs::write(
          bundle.join("ward-conf.yaml"),
          "apiVersion: zbot.dev/v1alpha1\nkind: WardLayout\nroot:\n  kind: directory\n  children:\n    - { id: canonical, match: '{ward}.md', kind: file, format: markdown }\n    - { id: agent-instructions, match: AGENTS.md, kind: file, format: markdown }\n    - { id: log, match: log.md, kind: file, format: markdown }\n",
      )
      .unwrap();
      std::fs::write(bundle.join("ward-agent.md"), "# {{display_name}} Agent\n")
          .unwrap();
      std::fs::write(
          bundle.join("starters/canonical.md"),
          "# {{display_name}}\n",
      )
      .unwrap();
      std::fs::write(bundle.join("starters/log.md"), "# Log\n").unwrap();

      let created = create_ward_from_archetype(
          &paths,
          "financial-analysis",
          Some(WardArchetypeId::Generic),
      )
      .unwrap();
      let mut names = std::fs::read_dir(&created.path)
          .unwrap()
          .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
          .collect::<Vec<_>>();
      names.sort();
      assert_eq!(
          names,
          ["AGENTS.md", "financial-analysis.md", "log.md", "ward-conf.yaml"]
      );
  }
  ```

- Both Linux and portable materializers render `<ward-id>.md` from the
  validated ID, materialize `log.md`, and keep optional nodes absent (AC 1–3).
- A synthetic complete bundle exercises canonical/log starter binding,
  snapshot bytes/digest, valid lint, and registry-edit isolation without
  depending on the later bundled-template conversion (AC 1–3, AC 5).
- Publication collision and staged validation failures leave no partial Ward
  or staging entry (AC 5).

**Approach:**

- Resolve canonical starter destination inside the existing materialization
  plan.
- Build materializer tests from an isolated synthetic bundle so T3 does not
  assume T5's template contents.
- Reuse current confined staging and atomic no-replace publication.

**Done when:** synthetic-bundle tests prove the exact four-file root and
creation safety on both supported materialization paths without reading the
seven production bundles.

### T4: The sole global catalog registers canonical Ward links

**Depends on:** T3

**Touches:** `runtime/agent-tools/src/tools/ward.rs`,
`gateway/src/state/mod.rs`

**Verification mode:** TDD.

**Tests:**

- `ward_catalog_registers_one_canonical_wikilink` (AC 4)
  `stub: true`

  ```rust
  // STUB: AC4 — ward.rs tests
  #[test]
  fn ward_catalog_registers_one_canonical_wikilink() {
      let root = tempfile::tempdir().unwrap();
      WardTool::ensure_ward_catalog(root.path()).unwrap();
      WardTool::register_ward(root.path(), "financial-analysis").unwrap();
      WardTool::register_ward(root.path(), "financial-analysis").unwrap();

      let content = std::fs::read_to_string(root.path().join("index.md")).unwrap();
      let expected =
          "- [[financial-analysis/financial-analysis|Financial Analysis]]";
      assert_eq!(content.lines().filter(|line| *line == expected).count(), 1);
  }
  ```

- First use creates plain `wards/index.md`; registration adds exactly
  `- [[<ward-id>/<ward-id>|<display-name>]]` and repeated registration is
  idempotent (AC 4).
- Existing regular-file prose is preserved; symlink, special-file, replacement,
  hardlink-alias, and same-handle identity failures remain closed through the
  existing catalog metadata helper.
- A forced catalog registration failure rolls back only the just-created Ward,
  leaves the catalog unchanged, and cannot remove a pre-existing Ward.
- Startup and Ward tool share one catalog shape without duplicate writers.

**Approach:**

- Remove OKF frontmatter from the global catalog starter.
- Centralize canonical-link rendering while retaining current safe file update
  mechanics.

**Done when:** catalog tests prove one global index, canonical links, and
non-destructive idempotence.

### T5: All archetype templates express the compact LLM Wiki foundation

**Depends on:** T3

**Touches:** `gateway/templates/wards/**`,
`gateway/gateway-services/src/ward_layout/create.rs`

**Verification mode:** goal-based construction and source audit; no stub
(mode).

**Tests:**

- A source audit fails on `okf-v0.1`, `starters/index.md`, any nested
  `match: index.md`, or missing shared doctrine across the seven bundles
  (AC 6, AC 7).
- Parameterized construction creates all seven converted archetypes and
  asserts their exact four-file roots, snapshot bytes/digests, valid lint,
  canonical title/Concepts/Tags, parseable log contract without a fake entry,
  and registry-edit isolation (AC 1–3, AC 6–7).
- Every layout declares lazy `pages/` and `sources/` plus the shared hidden
  planning namespace and its archetype-specific optional work areas (AC 6).
- Journal construction uses valid plain Markdown to prove nested date success
  and flat entry structural failure for the correct finding code/path (AC 8).

**Approach:**

- Convert planning and knowledge formats to `markdown`, remove every nested
  tasks index, add lazy `pages/` and `sources/`, and add required root
  `{ward}.md` plus `log.md`.
- Rewrite canonical starters and doctrines per archetype while retaining only
  useful optional work namespaces and the journal date route.
- Add the all-bundle and journal integration tests only after the production
  bundles have the new contract.

**Done when:** all seven bundles pass the common construction suite and the
source audit reports no OKF or nested-index assumptions.

### T6: Fresh acceptance and live registry synchronization preserve Wards

**Depends on:** T3, T4, T5

**Touches:** `e2e/playwright/full-mode/ward-archetypes.full.spec.ts`,
`docs/product/changelog.md`, `docs/specs/README.md`, live template registry

**Verification mode:** goal-based E2E and manifest audit.

**Tests:**

- A fresh database/vault E2E explicitly creates all seven archetypes through
  the normal Ward tool and asserts exact files plus global links (AC 9).
- Focused crate tests, workspace check, clippy, and formatting gates pass.
- Before/after existing-Ward manifests match; repository/live bundle manifests
  match only after a create-new timestamped backup (AC 10).
- Rollout tests reject symlinked ancestors, link/hardlink/special-file
  substitution, case-fold collisions, same-handle identity changes, path
  escape, and existing backup/staging destinations before replacement (AC 10).
- A scoped diff audit confirms no changes to the compiler, Wiki stores, recall,
  Ward HTTP content, embeddings, or UI (AC 11).

**Approach:**

- Update the existing full-mode archetype journey without test-only template
  injection.
- Synchronize the complete validated registry through a staging directory and
  no-follow/no-replace publication after resolving the canonical registry
  through `VaultPaths` and validating ancestors, literal components, regular
  single-link files, handle identity, and case-fold uniqueness.
- Record the user-visible clean-break behavior and backup location.

**Done when:** the fresh acceptance path is green, live templates match the
repository, existing Wards are unchanged, and the deferred subsystem diff is
empty. The completed rollout evidence is recorded in
[`notes/live-template-sync-audit.md`](notes/live-template-sync-audit.md).

## Rollout

This is a clean break for future Wards. Implementation remains blocked until
RFC-0018 and this spec are approved. After gates pass, create a new timestamped
backup of the live `config/templates/wards` registry, stage and validate the
entire repository registry, atomically replace the live registry, and verify
existing-Ward manifests are unchanged. The user deletes test Wards and starts
with a fresh database. Rollback restores the backed-up template registry;
existing Ward snapshots never participate.

Backup, staging, replacement, and rollback use `VaultPaths`-resolved canonical
parents with no-follow opens, regular single-link validation, same-handle
revalidation, and no-replace destinations. Any ancestor link, path escape,
case-fold collision, special file, hardlink alias, raced handle, or pre-existing
backup/staging target aborts before the live registry changes.

No infrastructure, network, secret, database schema, external service, API
version, or UI deployment changes in this slice.

## Risks

- `{ward}` could accidentally interact with nested `{name}` matching; keep the
  grammar disjoint and cover every forbidden placement.
- Loader special-casing could become a hidden second schema; expose canonical
  identity in `ward-conf.yaml` and limit the starter role to content binding.
- Removing mandatory frontmatter could weaken tests that currently fail for
  OKF reasons; structural tests must assert finding codes and paths so they
  cannot pass accidentally.
- Startup and Ward tool both touch the global catalog; centralize the rendered
  shape and retain same-file validation.
- The repo is already dirty with related and unrelated changes; scope every
  diff and preserve user-owned work.

## Changelog

- 2026-07-28: initial plan following RFC-0018 research and user-confirmed
  assumptions.
