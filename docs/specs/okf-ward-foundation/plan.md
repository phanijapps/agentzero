# Plan: OKF Ward Foundation

- **Spec:** [`spec.md`](spec.md)
- **Status:** Done

## Approach

Land one clean-break implementation PR in dependency order: contract and domain
model, filesystem service, gateway/capabilities, planning workflows, destructive
cutover and legacy deletion, then retained Phase 1 UI. Keep authored Markdown as
the source of truth; indexes, backlinks, search documents, and validation
summaries are disposable derived state.

Tempted to add a dedicated OKF crate; declining because `gateway-services`
already owns vault paths and filesystem services. Tempted to add a database
index immediately; declining because the accepted design starts with bounded
scan-on-demand and a disposable cache. Tempted to preserve the old Vault/wiki
routes behind a flag; declining because the accepted cutover explicitly forbids
compatibility surfaces.

## Constraints

- Follow RFC-0015 and the OpenAPI contract; update the spec first if either must
  change.
- Use `VaultPaths` for all filesystem resolution and shared confinement helpers
  for every route/tool.
- No legacy migration or compatibility behavior. The only transition is
  confirmed deletion followed by fresh scaffolding.
- Preserve unrelated Engram memory capabilities; remove only dedicated wiki
  concepts and integrations made obsolete by OKF.
- Follow `apps/ui/ARCHITECTURE.md`: semantic classes, design tokens, no inline
  styling.

## Construction tests

- Add golden valid/invalid OKF trees, nested concepts, collisions, symlink
  escapes, and refinement archives under test fixtures.
- Generate contract request/response cases from
  `contracts/openapi/okf-wards.yaml` before handler implementation.
- Add cutover fixtures before deleting legacy startup/scaffold behavior.
- Add Phase 1 UI interaction tests before changing Memory/Vault routing.

## Design (LLD)

- Introduce an OKF domain/service boundary (parser, validator, catalog, links,
  search projection, mutation transaction) below gateway handlers and tools.
- Address concepts by normalized root-relative paths. Use temporary-file plus
  rename for writes and content-derived ETags for optimistic concurrency.
- Serialize mutations per ward; update document and derived index transactionally
  from the caller’s perspective, with deterministic full reindex recovery.
- Adapt the context-capability registry with bounded, read-only OKF providers.
  Keep destructive/refinement actions on explicit action surfaces.
- Retarget existing tree/preview components and rename wiki-specific DTOs/routes;
  do not fork a second Vault UI.

## Tasks

### T1: Implement the confined OKF domain and validator

**Status:** Complete (2026-07-19)

**Depends on:** none

**Touches:** `gateway/gateway-services/src/okf.rs`, `gateway/gateway-services/src/lib.rs`, `gateway/gateway-services/Cargo.toml`

**Tests:**
- TDD: temporary roots accept the canonical root/ward/concept tree and reject
  missing root/ward indexes, missing or malformed concept frontmatter, empty
  `type`, unsafe/case-folded/reserved stems, illegal same-stem collisions,
  symlinks, FIFOs/sockets/devices, excessive depth/frontmatter/YAML nesting,
  oversized and growing files, metadata/open races, and legacy directories.
- TDD: valid reserved `index.md`/`log.md` forms do not require concept
  frontmatter, while reserved filenames used in the wrong role fail.

**Approach:** Add one existing-crate module containing normalized relative-path
types, document parsing, validation diagnostics, canonical templates, and
strict configurable-root traversal. Traverse with non-following metadata,
accept only regular files/directories, enforce per-file and aggregate limits
before allocation/read, and fail closed if metadata changes through open/read.
Materialize focused Rust tests before the production implementation.

**Done when:** `cargo test -p gateway-services okf` passes and no filesystem
result contains an absolute path.

### T2: Add catalog, tree, search, links, and deterministic reindex

**Status:** Complete (2026-07-19)

**Depends on:** T1

**Touches:** `gateway/gateway-services/src/okf.rs`

**Tests:**
- TDD: nested valuation/book fixtures produce paired concept trees, root/ward
  catalogs, filtered ranked search provenance, inbound/outbound/unresolved
  links, and bounded graph neighborhoods.
- TDD: every configured scan/cache bound reports explicit truncation; a clean
  reindex is deterministic and leaves authored bytes unchanged.

**Approach:** Build read projections directly over the validator and a bounded
in-memory cache keyed by relative path, metadata, and content hash.

**Done when:** focused service tests cover every read schema and bound in the
OpenAPI contract.

### T3: Add atomic OKF mutations and destructive cutover service

**Status:** Complete (2026-07-19)

**Depends on:** T1, T2

**Touches:** `gateway/gateway-services/src/okf.rs`, `gateway/gateway-services/src/paths.rs`

**Tests:**
- TDD: create/update uses atomic replacement and ETags; stale ETags, traversal,
  collisions, invalid documents, and symlink escapes fail without mutation.
- TDD: refinement archival rewrites snapshot-local links, validates before
  replacing active files, never clobbers a run ID, and preserves authored
  current files on failure.
- TDD: impact and cutover tokens bind exact path/hash sets; changed sets abort;
  confirmed concept deletion stays inside its companion subtree; staged bulk
  cutover either restores the old root or produces a valid empty root.

**Approach:** Serialize mutations per root, stage writes/renames beside their
targets, and keep all token material server-side with expirations.

**Done when:** mutation and failure-injection tests prove fail-closed atomicity.

### T4: Expose the contracted HTTP API and Context Capabilities

**Depends on:** T2, T3

**Touches:** `gateway/src/http/okf.rs`, `gateway/src/http/mod.rs`, `gateway/src/state/mod.rs`, `gateway/src/http/openapi.rs`, `contracts/openapi/okf-wards.yaml`

**Tests:**
- TDD: Axum route tests cover each contracted status/shape, root-invalid repair
  policy, bounds, ETags, impact/cutover tokens, and reindex job status.
- TDD/security: absent session, missing/invalid CSRF, cross-origin requests,
  traversal, encoded separators, symlinks, and host-path leakage fail closed.
- TDD/security: an authenticated same-origin client acquires and rotates a CSRF
  token, completes cutover preview/execute, and receives typed `403` responses
  for missing, stale, mismatched-session, or cross-origin tokens.
- TDD/security: concept PUT accepts boundary-valid ASCII, multi-byte Unicode,
  quotes/backslashes, and escaped controls up to 1,048,576 decoded UTF-8 bytes;
  it rejects larger decoded Markdown or declared/chunked raw bodies above
  6,300,000 bytes with typed `413` before mutation (and before full buffering
  for the raw cap).
- Goal-based check: OpenAPI YAML parses uniquely and every implemented operation
  ID has a route test.

**Approach:** Keep handlers thin over `OkfService`; add actor-filtered read-only
catalog/resource/context-graph capability descriptors and explicit mutation
actions without adding `ward(action="search")`.

**Done when:** focused gateway contract/security tests pass.

### T5: Cut ward scaffolding and planning workflows to concept companions

**Depends on:** T1, T3

**Touches:** `runtime/agent-tools/src/tools/ward.rs`, `gateway/gateway-execution/src/{invoke,middleware,runner,session_ctx}/*`, `gateway/gateway-services/src/ward_curator.rs`, `gateway/gateway-services/src/ward_curator_tests.rs`, `gateway/templates/agents/*`, `gateway/templates/shards/*`, `gateway/templates/skills/{spec-builder,plan-composer,ward-designer}/**/*`

**Tests:**
- TDD: ward create produces only canonical OKF files; legacy wards fail with the
  exact rebuild diagnostic; active plan/task paths survive delegation,
  snapshots, and continuation while deleted/legacy pointers terminate.
- Goal-based check: an `rg` inventory finds no live `memory-bank/`, global
  ward-local `specs/`, `step_N.md`, or hard-coded wards-root doctrine outside
  historical docs and explicit removal tests.

**Approach:** Replace producers and consumers as one path-contract cutover;
carry exact `active_plan_path` and `task_path` rather than rediscovering them.

**Done when:** focused runtime/gateway tests pass and the old-path inventory is
empty under live code/templates.

### T6: Retire dedicated wiki infrastructure

**Depends on:** T4, T5

**Touches:** `gateway/src/state/*`, `gateway/gateway-execution/src/ward_wiki.rs`, `gateway/templates/skills/wiki/**`, wiki-specific store traits/adapters/migrations/tests, wiki settings and routes/DTOs`

**Tests:**
- TDD: startup seeds only the OKF root and no wiki/scratch legacy ward; recall
  continues to expose non-wiki Engram facts/procedures/episodes.
- Goal-based check: repository inventory finds no live wiki seeding, promotion,
  configuration, persistence, or recall-source consumer. Historical migration
  files remain immutable; a new one-way schema-removal migration and its tests
  remove obsolete live tables without importing content.

**Approach:** Remove consumers from the leaves inward, then delete unreachable
wiki storage/adapters/templates and obsolete product tests. Never rewrite or
delete historical migrations; do not disturb unrelated semantic memory contracts.

**Done when:** focused startup/recall/migration tests pass and dead-code search
finds no dedicated wiki product path.

### T7: Retarget the retained UI to the Phase 1 OKF browser and cutover gate

**Depends on:** T4, T6

**Touches:** `apps/ui/src/features/vault/*`, `apps/ui/src/features/memory/*`, `apps/ui/src/services/transport/*`, `apps/ui/src/styles/{theme,components}.css`

**Tests:**
- TDD/UI: users see catalog, paired tree, filters/provenance, sanitized preview,
  backlinks, validation/truncation/errors, and no authoring controls.
- TDD/UI: legacy-root gate lists the exact wards, cancellation changes nothing,
  stale preview tokens abort, and confirmed cutover produces the empty catalog.
- Visual/manual QA: desktop/tablet/mobile panes remain keyboard accessible and
  use semantic classes/tokens with no absolute paths rendered.

**Approach:** Reuse existing Vault tree/preview components and transport
boundary; rename wiki DTOs/routes rather than retaining aliases.

**Done when:** focused Vitest/browser checks and manual responsive smoke pass.

### T8: Integrated gates, documentation, and implementation review

**Depends on:** T1-T7

**Touches:** `docs/architecture/*`, `docs/specs/okf-ward-foundation/*`, `docs/specs/README.md`

**Tests:**
- Goal-based: `cargo fmt --all -- --check`, `cargo check --workspace`,
  `cargo test --workspace`, `cargo clippy --all-targets -- -D warnings`, UI
  lint/typecheck/tests/build, OpenAPI validation, `git diff --check`, old-path
  inventory, and spec-status lint.
- Goal-based: codegraph impact/dead-code checks and adversarial, security, and
  quality reviews are clean or explicitly unavailable.

**Approach:** Update architecture and completion evidence, simplify new code,
run all gates, fix findings, and mark only mechanically demonstrated acceptance
criteria complete.

**Done when:** every acceptance criterion is checked, gates are green, and the
final work-loop review is clean.

## Rollout

Ship behind a release-level cutover gate, not a long-lived feature flag. On
first launch with any pre-OKF ward, block ward operations until the user either
cancels or enters the destructive confirmation. Successful confirmation removes
all wards, initializes the root bundle, rebuilds derived state, and permanently
records the cutover version outside the wards root. Fresh installs initialize
directly. There is no rollback that restores deleted ward content.

## Risks

- Accidental data loss: show exact affected ward IDs and require explicit typed
  confirmation immediately before deletion.
- Recursive strictness rejecting useful generated Markdown: keep non-knowledge
  artifacts outside the root or normalize them before entry.
- Removing wiki wiring harming unrelated recall: characterize existing recall
  sources first and retain Engram facts/procedures/episodes independently.
- Index drift: make reindex deterministic and compare incremental state with a
  clean rebuild in tests.
- Cross-platform path bugs: use path components internally and test alternate
  separator/case behavior without exposing host paths.

## Changelog

- 2026-07-19: Initial plan branched from accepted RFC-0015.
