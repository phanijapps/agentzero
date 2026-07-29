# Plan: Ward Archetype Registry and Creation

- **Spec:** [`spec.md`](spec.md)
- **Status:** Done

## Approach

Extend the existing Ward Layout path rather than introduce a router. Add the
closed identifier and registry paths first, then bounded bundle loading and
seeding, then thread the typed value through the existing creation trait and
adapter. Keep exact-byte snapshotting and atomic publication intact. Finish
with shared construction tests and a clean-start application test.

## Constraints

- Follow [RFC-0017](../../rfc/0017-ward-layout-archetypes.md),
  [ADR-0002](../../adr/0002-select-complete-ward-archetypes-at-creation.md),
  and the safe loader/snapshot rules in
  [RFC-0016](../../rfc/0016-generic-ward-configuration-and-layout-resolution.md).
- No new standalone contract: Rust types and the existing `ward` tool schema
  are the interface.
- Preserve unrelated user changes in the current worktree.

## Construction tests

**Integration tests:** one parameterized construction harness compares bundle
bytes, materialized nodes, digest, lint report, and provenance; one clean-start
test uses a fresh database and vault through normal bootstrap.

**Manual verification:** create a coding ward from a fresh local profile,
inspect its `ward-conf.yaml`, edit the source bundle, restart, and confirm the
created ward is unchanged.

## Design (LLD)

### Data & schema

Add `WardArchetypeId` as the only serialized selection value. Add provenance
to existing creation/layout state without making it structural authority.
Bundle resource limits are named constants with boundary fixtures. Traces to:
AC1, AC7, AC9.

### Interfaces & contracts

Change `WardLayoutAccess::create` and `create_ward_from_template` to accept an
optional typed archetype, defaulting at the service boundary to `generic`.
Expose the optional closed string in `ward(action="create")`; do not accept a
path. Traces to: AC1, AC3, AC8-AC10.

### Failure, edge cases & resilience

Resolve and validate the whole bundle before publication. Reuse current
no-follow/no-replace operations, clean staging on every error, distinguish
classifier absence from corrupt registry content, and map detailed internal
errors to bounded codes. Traces to: AC4, AC5, AC7, AC10.

### Quality attributes (NFRs)

Enforce file-count, depth, per-file, and total-byte ceilings before
materialization; preserve exact bytes and deterministic digest. Traces to:
AC5-AC8, AC13.

## Tasks

### T1: Closed identifiers resolve only to confined registry bundles

**Depends on:** none

**Touches:** `runtime/agent-primitives/src/ward.rs`, `runtime/agent-primitives/src/lib.rs`, `gateway/gateway-services/src/paths.rs`

**Verification mode:** TDD.

**Tests:**

- TDD table tests for every valid and invalid `WardArchetypeId` (AC1).
- T1 path tests prove every closed identifier maps to exactly one literal child
  beneath the canonical registry root and no caller-supplied component enters
  the mapping (AC4). Symlink, hard-link, case-fold, and non-directory
  filesystem substitution tests run with the T2 bounded bundle loader, where
  filesystem metadata is actually opened.
- Stub handoff: `gateway/gateway-services/src/ward_layout/archetype.rs`
  `tests::ward_archetype_id_is_closed` and
  `tests::ward_archetype_bundle_is_confined`; `stub: deferred to work-loop
  PLAN` because `new-spec` step 4 forbids committing stubs during spec
  authoring.

**Approach:**

- Define the shared `WardArchetypeId` value in `agent-primitives` so
  `agent-tools`, `gateway-services`, and `gateway-execution` use one closed
  type without reversing crate dependencies.
- Add `VaultPaths::ward_archetype_registry_dir` and a closed
  `ward_archetype_bundle` mapping; use directory-relative/no-follow helpers
  already present in `ward_layout::create`.

**Done when:** identifier and confinement tests pass without accepting an
arbitrary string path.

### T2: Fresh vaults seed bounded generic and coding bundles non-destructively

**Depends on:** T1

**Touches:** `gateway/gateway-services/src/paths.rs`, `gateway/gateway-services/src/ward_layout/loader.rs`, `gateway/gateway-services/src/ward_layout/create.rs`, `gateway/src/state/mod.rs`, `gateway/templates/wards/generic/ward-conf.yaml`, `gateway/templates/wards/generic/ward-agent.md`, `gateway/templates/wards/coding/ward-conf.yaml`, `gateway/templates/wards/coding/ward-agent.md`, `docs/guides/how-to/customize-ward-archetypes.md`

**Verification mode:** TDD for resource/doctrine boundaries; goal-based
integration for seeding and legacy transition.

**Tests:**

- Fresh-vault and repeat-bootstrap tests prove all bundle files seed once and
  user edits survive (AC2, AC11).
- Legacy-transition fixtures start with user-edited singular
  `config/templates/ward-conf.yaml` and `ward-agent.md`, run bootstrap, and
  prove those exact edits become the generic bundle, are never overwritten,
  and the singular paths are inactive for subsequent creation (AC11).
- `docs/guides/how-to/customize-ward-archetypes.md` documents the canonical
  registry paths, one-time singular transition, future-ward-only edit
  semantics, and corrupt-bundle failure behavior (AC11).
- Boundary tests cover named constants
  `MAX_WARD_ARCHETYPE_STARTER_FILES = 128`,
  `MAX_WARD_ARCHETYPE_STARTER_DEPTH = 16`,
  `MAX_WARD_ARCHETYPE_STARTER_FILE_BYTES = 256 * 1024`, and
  `MAX_WARD_ARCHETYPE_STARTER_TOTAL_BYTES = 4 * 1024 * 1024` at the limit and
  one unit over, plus unsupported YAML and undeclared/colliding destinations
  (AC5, AC7).
- Doctrine tests route the selected bundle through the existing
  `load_ward_agent_template`/renderer controls and reject oversized or invalid
  UTF-8 content, unsafe links, unsupported placeholders, and injection
  patterns with bounded diagnostics; missing/invalid doctrine neither overlays
  nor falls back to another bundle (AC6).

**Approach:**

- Move or companion the singular seed into the generic bundle without
  overwriting user content.
- Add the coding reference bundle and a bounded bundle loader that reuses the
  current YAML parser and compiler.
- Generalize the existing doctrine loader to the confined selected bundle
  path while preserving its limits, rendering, diagnostics, and no-fallback
  behavior.

**Done when:** seed idempotence, every bundle resource boundary, and every
selected-doctrine control test passes.

### T3: Explicit creation snapshots the selected bundle and provenance

**Depends on:** T1, T2

**Touches:** `runtime/agent-tools/src/tools/ward.rs`, `runtime/agent-tools/src/lib.rs`, `gateway/gateway-execution/src/invoke/ward_layout_adapter.rs`, `gateway/gateway-execution/src/invoke/ward_usage_adapter.rs`, `gateway/gateway-services/src/ward_layout/create.rs`, `gateway/gateway-services/src/ward_layout/lint.rs`, `gateway/gateway-services/src/ward_usage.rs`

**Verification mode:** goal-based integration for creation/reuse/provenance;
TDD for corrupt-bundle failure.

**Tests:**

- `WardLayoutAccess::create` adapter tests cover explicit generic/coding and
  omitted/default selection (AC3).
- Creation tests compare selected bytes, digest, exact nodes, starter content,
  returned/state provenance, and post-edit reuse (AC8, AC9).
- Metadata-authority tests delete `WardRecord.archetype`, then set it to a
  deliberately different archetype, and in both cases prove load, lint, and
  reuse derive structure/digest from the copied snapshot while returning only
  bounded/optional provenance (AC9).
- Creation tests render both generic and coding `ward-agent.md` into the
  created `AGENTS.md` and prove selected-only precedence/no fallback (AC6).
- Failure tests prove corrupt/missing bundles produce bounded errors and no
  destination or staging residue (AC10).
- `#[cfg(not(target_os = "linux"))]` tests
  `portable_creation_snapshots_selected_bundle` and
  `portable_failure_cleans_destination` exercise exclusive generic/coding
  creation, exact snapshot/provenance, existing-destination rejection, and
  cleanup on the portable implementation.

**Approach:**

- Thread `Option<WardArchetypeId>` through the tool trait, adapter, and service.
- Preserve the current staged publication algorithm and add provenance to
  `CreatedWard`/`WardLayoutState`.
- Add serde-defaulted `WardRecord.archetype: Option<WardArchetypeId>` to the
  atomic `wards/.usage.json` sidecar and update it only after successful ward
  publication; loading and lint continue to trust snapshot bytes if metadata
  is absent or disagrees.

**Done when:** explicit creation and immutable-reuse integration tests pass.

### T4: Shared construction and clean-start gates protect the registry

**Depends on:** T2, T3

**Touches:** `gateway/gateway-services/src/ward_layout/create.rs`, `gateway/gateway-services/src/ward_layout/lint.rs`, `gateway/gateway-execution/src/invoke/ward_layout_adapter.rs`, `e2e/playwright/full-mode/ward-archetypes.full.spec.ts`, `e2e/playwright/lib/harness-full.ts`, `.github/workflows/test.yml`

**Verification mode:** goal-based integration and E2E.

**Tests:**

- Run one parameterized bundle construction/lint suite for `generic` and
  `coding` (AC12).
- The same suite renders both doctrines and re-runs the doctrine safety
  controls against selected-only precedence (AC6).
- Start normal bootstrap with fresh database/vault and create `coding` without
  fixture seeding (AC13).
- Add a focused macOS/Windows CI matrix in `.github/workflows/test.yml` that
  runs the two portable Ward creation tests, making the current non-Linux
  guarantees an executed artifact rather than Linux-only compiled code.
- Run `cargo test -p gateway-services`, `cargo test -p gateway-execution`, and
  `cargo test -p agent-tools`.

**Approach:**

- Extract construction assertions shared by all future bundles.
- Add the clean-start fixture at the lowest existing E2E surface that exercises
  application seeding plus ward creation.

**Done when:** the parameterized and clean-start gates pass from an empty temp
directory.

## Rollout

Ship registry support and both reference bundles together. The transition is
non-destructive: existing user-edited generic configuration becomes the
generic seed and existing ward snapshots are untouched. Rollback restores
singular-template creation for future wards only; no created ward is rewritten.

## Risks

- Existing seed/migration code may have multiple call sites; constrain the
  change to current bootstrap helpers and test idempotence.
- Portable creation has weaker platform primitives than Linux; retain its
  current exclusive destination and cleanup guarantees.
- Provenance must not create a second authority; all load/lint decisions
  continue to derive from snapshot bytes.

## Changelog

- 2026-07-26: initial plan.
