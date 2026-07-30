# Plan: Fluid Ward Template and Conformance

- **Spec:** [`spec.md`](spec.md)
- **Status:** Implemented

## Approach

Build the smallest generic path: a safe YAML envelope plus arbitrary data body,
a generic structure-rule interpreter/linter, snapshot-on-create, then the ward
lint tool and middleware. Tests use two incompatible templates so any compiled
artifact role fails immediately.

## Constraints

- The YAML snapshot is the sole ward-shape authority.
- `config.yaml` remains the unrelated provider/model override.
- Preserve gateway/runtime dependency direction.
- No repair/approval subsystem, mutation API expansion, migration, UI, or new
  dependency.

## Declined additions

- Tempted to model each default role with Rust structs; declining because roles
  are user data and may disappear.
- Tempted to build a transaction/approval framework; declining because this
  slice performs create-once setup and read-only lint/context injection.
- Tempted to add a general template engine; declining because bounded path
  placeholders and generic rule nodes are sufficient.

## Construction tests

- Unit tests cover envelope validation, arbitrary-key round trips, budgets,
  generic rule parsing, placeholders, containment, and lint findings.
- Temporary-vault tests cover seed-once, snapshot creation, recursive lint, and
  preservation of operational non-Markdown files.
- E2E compares the default template with a renamed minimal template containing
  no spec, plan, or task roles.

## Tasks

### T1: Generic envelope and editable default template load without role structs

**Depends on:** none

**Touches:** `gateway/templates/ward-conf.yaml`, `gateway/gateway-services/src/paths.rs`, `gateway/gateway-services/src/ward_layout/{mod,schema,loader}.rs`, `gateway/gateway-services/src/lib.rs`

**Tests:**

- TDD tests cover supported envelope, arbitrary nested keys, unsafe YAML,
  duplicate keys, limits, exact-byte digest, seed-once, and preserved edits
  (AC 1–2).
- File-open tests reject symlinked/swapped/special/multiply-linked template and
  snapshot files and prove expected-parent confinement (AC 1–3).

**Approach:**

- Type only `apiVersion`/`kind`; retain the remaining mappings as
  `serde_yaml::Value`.
- Seed the bundled default with create-new semantics.

**Done when:** the default and a no-spec/no-plan template parse and round-trip,
while unsafe/oversized inputs fail.

### T2: Generic rule interpreter resolves declared paths and lints the ward

**Depends on:** T1

**Touches:** `gateway/gateway-services/src/ward_layout/{rules,resolver,lint,okf}.rs`

**Tests:**

- TDD/property tests cover arbitrary role names, optional/required nodes,
  repeatable nested concept rules, placeholder values, collisions, traversal,
  symlinks, all traversal/file/byte/time/finding limits, OKF vs structure
  findings, and resource files (AC 4–9).

**Approach:**

- Strictly parse only `id`, `kind`, `match`, `required`, `repeat`, `exclude`,
  `format`, `children`, and `$ref`; arbitrary role IDs never affect mechanics.
- Walk beneath the opened ward root without following symlinks.

**Done when:** two incompatible templates lint correctly with no role-name
match in Rust source.

### T3: Ward creation copies the snapshot and scaffolds only declared nodes

**Depends on:** T2

**Touches:** `gateway/gateway-services/src/ward_layout/create.rs`

**Tests:**

- Integration tests create wards from default and minimal templates, compare
  exact snapshots, and assert absent roles are not created (AC 3–4, 9–10).
- Confinement tests cover existing destinations, traversal, symlinked wards
  roots/intermediates, target swaps, and overwrite attempts (AC 3, 7).

**Approach:**

- Read/validate bytes through one template handle, open the canonical wards root
  once, create the new ward handle-relatively, and materialize only required
  literal nodes with create-new/no-follow semantics.

**Done when:** a template without spec/plan/tasks creates none of those paths.

### T4: Ward lint and lifecycle middleware share one nudge-producing service

**Depends on:** T2, T3

**Touches:** `runtime/agent-tools/src/tools/ward.rs`, `gateway/gateway-execution/src/invoke/{ward_layout_adapter,executor,setup}.rs`, `gateway/gateway-hooks/src/**`

**Tests:**

- Tool/integration tests cover `ward(action="lint")`, pre-use/setup lint,
  post-write lint, digest binding, bounded findings, and nudge redaction
  (AC 5–8, 10).

**Approach:**

- Define a lower-level `WardLayoutAccess` trait in `agent-tools`, implement it
  in gateway execution over `gateway-services`, and delegate create/lint/use
  through that adapter. Middleware returns the same service nudge.

**Done when:** tool and middleware return identical reports for the same ward.

## Rollout

- Clean break after the user deletes old wards/database state.
- Seed the editable template; no feature flag, migrator, or compatibility path.

## Risks

- A generic rule language can grow accidentally; keep primitives small and
  versioned.
- Raw YAML is untrusted prompt data; normalization and delimiting are required.
- Lint must never infer missing roles from the default template.

## Changelog

- 2026-07-19: Replaced the rigid role-specific implementation plan with the
  user-confirmed fluid template, context injection, and generic linter scope.
