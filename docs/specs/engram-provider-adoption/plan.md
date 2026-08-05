# Plan: Engram Provider Adoption

- **Spec:** [`spec.md`](spec.md)
- **Status:** Done

> **Plan contract:** this is the implementation strategy. The implementation must update it if a material approach changes.

## Approach

Replace the adapter's conformance bootstrap seam with the SQLite-enabled Engram provider facade while retaining the existing trusted-root configuration path. Add an adapter-internal semantic-services boundary that gates every operation on both capability and handle, maps errors to stable redacted adapter results, and distinguishes raw Engram recall readiness from policy-safe z-Bot recall. Exercise it directly in adapter tests; do not add a gateway, agent, UI, CLI, or model-tool consumer. Export/import is deferred.

## Constraints

- Follow [RFC-0011](../../rfc/0011-engram-memory-engine-cutover.md): Engram owns semantic persistence; z-Bot owns product orchestration.
- Follow [RFC-0012](../../rfc/0012-engram-upstream-risk-reduction.md): keep a narrow fail-closed adapter boundary. The sibling path dependency cannot yet enforce an immutable upstream revision; record that prerequisite rather than claim it complete.
- Preserve [unified-recall-default](../unified-recall-default/spec.md): raw Engram recall is not `AdapterFeature::Recall` and cannot change safe recall behavior.
- `engram-integration` needs its existing `sqlite` feature after `engram-conformance` is removed. This adds no crate.
- No raw `ExportImport` handle or operation lands here; portability needs a separate constrained design.

## Construction tests

**Integration tests:** temporary-root tests in `stores/zbot-engram-adapter/tests/` verify public bootstrap confinement and reopen a versioned pre-facade fixture. Source `#[cfg(test)]` modules verify crate-private capability gates, redacted operation failure, batch outcome forwarding, scoped provenance, inspection/dry-run migration, diagnostics, and raw recall readiness.

**Manual verification:** none; there is no user-facing flow.

## Design (LLD)

### Design decisions

- `engram_integration::EngramProvider::open` is the only provider bootstrap entry, called only with `AdapterConfig::to_engram_config()` output. Traces to: AC 1, AC 2, AC 6.
- `zbot_engram_adapter::EngramProvider` remains the single wrapper. New operations are `pub(crate)` and return adapter-owned redacted results; no raw provider trait handle becomes a crate API. Traces to: AC 3, AC 5.
- A private `EngramSemanticService` gate evaluates capability plus handle. `EngramUnifiedRecall` is separate from product `AdapterFeature::Recall`. Traces to: AC 3, AC 4.
- Export/import, backend selection, and typed-port dependency retirement remain upstream/portability work. Traces to: AC 7.

### Interfaces & contracts

- Internal Rust-only boundary; no `contracts/` artifact. A private `semantic_services.rs` module owns adapter-private unified recall, batch, provenance, `schema_version`/`adapter_version`/`dry_run_import`, and diagnostics methods with stable `AdapterError` mapping and source-unit tests.
- Existing memory, knowledge, belief, hierarchy, taxonomy accessors, public capability report, `AdapterFeature::Recall`, and `RecallSupportReport` remain compatible.

### Failure, edge cases & resilience

- Extend the existing configuration seam only as needed to canonicalize and prefix-verify data-root/storage paths, reject symlink escapes, and never build `EngramConfig` by raw path joining.
- Capability/handle disagreement fails closed. Operation failures return a stable adapter code without a raw provider message; diagnostics use structured tracing with no path, SQL, or payload exposure. No retry is added. `apply_import`, automatic migration execution, and migration-mode mutation have no wrapper.
- Batch maps its guarantee, complete/partial state, and safe step codes without inventing rollback semantics.

### Dependencies & integration

- Remove `engram-conformance`; configure existing `engram-integration` with `features = ["sqlite"]`.
- Keep direct `engram-*` port dependencies used by existing store mappings. The gaps record owns their retirement criteria.

## Tasks

### T0: Capture a versioned pre-facade semantic-data baseline

**Depends on:** none

**Touches:** `stores/zbot-engram-adapter/tests/fixtures/pre-facade-engram-data.db`, `stores/zbot-engram-adapter/tests/fixtures/pre-facade-engram-data.manifest.json`, `stores/zbot-engram-adapter/tests/bootstrap.rs`

**Tests:**
- Goal-based: the fixture manifest records that the synthetic database was generated through the pre-T1 `bootstrap_provider` path, its SHA-256, schema/data expectations, and has no user data. Verifies AC 6.
- Integration: the existing public adapter tests can read the fixture's representative fact, graph, and sidecar records before T1 changes bootstrap. Verifies AC 6.

**Approach:**
- Generate a synthetic, committed single-file fixture with the current implementation before changing bootstrap; do not generate it during or after the facade change.
- Record its checksum and expected records in a manifest so a fixture refresh requires an explicit compatibility decision.

**Done when:** a reproducible pre-facade baseline exists and current adapter tests prove it contains only the intended compatibility records.

### T1: Bootstrap through the SQLite-enabled facade with confined paths

**Depends on:** T0

**Touches:** `stores/zbot-engram-adapter/Cargo.toml`, `stores/zbot-engram-adapter/src/bootstrap.rs`, `stores/zbot-engram-adapter/src/config.rs`, `stores/zbot-engram-adapter/src/dependency_checklist.rs`, `stores/zbot-engram-adapter/tests/bootstrap.rs`, `stores/zbot-engram-adapter/tests/config.rs`

**Tests:**
- TDD: a valid config opens with the facade and preserves existing core handles. Verifies AC 1.
- TDD: traversal, absolute outside-root, symlinked parent, and symlinked storage-file paths are rejected before bootstrap. Verifies AC 2.
- Goal-based: `cargo tree -p zbot-engram-adapter -e features` proves `engram-integration/sqlite`; no `engram-conformance` or runtime `bootstrap_provider` use remains. Verifies AC 1.

**Approach:**
- Enable the existing `sqlite` feature and replace `bootstrap_provider` with `UpstreamEngramProvider::open`.
- Route all configuration through `to_engram_config` / `resolve_engram_path`; strengthen the existing helper only where adversarial tests expose a missing canonical/symlink check.
- Materialize red stubs before implementation and retain the current adapter error shape.

**Done when:** facade bootstrap is feature-enabled, confined, and free of the conformance runtime dependency.

### T2: Add adapter-private service gates without changing product recall capability

**Depends on:** T1

**Touches:** `stores/zbot-engram-adapter/src/bootstrap.rs`, `stores/zbot-engram-adapter/src/capabilities.rs`, `stores/zbot-engram-adapter/src/lib.rs`, `stores/zbot-engram-adapter/src/semantic_services.rs`

**Tests:**
- TDD: every semantic service requires both report support and a provider handle; supported/absent and unsupported/present disagreement fixtures fail closed. Verifies AC 3.
- TDD: `EngramUnifiedRecall` readiness can be true while `AdapterFeature::Recall` and `RecallSupportReport` retain their prior values. Verifies AC 4.

**Approach:**
- Add a private semantic-service enum/gate and narrow `pub(crate)` operation entry points in `semantic_services.rs`; materialize its red tests in that source module.
- Represent raw Engram recall separately; do not change model-safe recall capability logic or external product reporting.

**Done when:** no service can be used from capability alone, and raw Engram recall cannot silently alter z-Bot recall semantics.

### T3: Add redacted adapter-private recall, diagnostics, and migration operations

**Depends on:** T2

**Touches:** `stores/zbot-engram-adapter/src/semantic_services.rs`

**Tests:**
- TDD: supported calls work; unavailable and provider-operation failure cases return stable adapter errors without paths, SQL, or original error text. Verifies AC 3.
- TDD/source-unit: a temporary SQLite provider performs bounded raw Engram recall, reads diagnostics, and calls only `schema_version`, `adapter_version`, and `dry_run_import` through the adapter. Verifies AC 3 and AC 4.

**Approach:**
- Add `pub(crate)` wrappers for unified recall, observability, `schema_version`, `adapter_version`, and `dry_run_import` only; map `CoreError` to redacted `AdapterError` and add no retry/fallback.
- Exclude `apply_import`, automatic migration execution, and migration-mode mutation. Keep outputs inside source-unit tests; do not introduce runtime callers.

**Done when:** adapter-private operations prove capability gating and error redaction while product recall remains unchanged.

### T4: Add redacted semantic batch and provenance operations

**Depends on:** T2

**Touches:** `stores/zbot-engram-adapter/src/semantic_services.rs`

**Tests:**
- TDD: batch/provenance unavailable and operation-failure paths are redacted and fail closed. Verifies AC 3.
- TDD/source-unit: semantic batch forwarding preserves actual SQLite guarantee and safe per-step outcome codes; entity/relationship provenance remains scope-bound. Private-operation coverage stays out of `stores/zbot-engram-adapter/tests/`. Verifies AC 3 and AC 5.

**Approach:**
- Add adapter-private wrappers that transform raw upstream batch errors into safe codes while preserving guarantee/status/step identity.
- Do not force a synthetic partial-failure fixture; add one upstream first if Engram later supplies it.
- Do not modify distillation, memory-write, ontology/taxonomy governance, or runtime callers.

**Done when:** batch/provenance are safe, truthful adapter operations with no new production consumer.

### T5: Prove existing-data compatibility and record deferred upstream work

**Depends on:** T0-T4

**Touches:** `stores/zbot-engram-adapter/tests/bootstrap.rs`, `docs/history/engram-provider-adoption-gaps.md`, `docs/specs/engram-provider-adoption/spec.md`, `docs/specs/engram-provider-adoption/plan.md`, `docs/specs/README.md`

**Tests:**
- Integration: copy the versioned pre-facade fixture to a temporary root, reopen it only through facade bootstrap, verify manifest-described reads and `PRAGMA integrity_check`, and assert no extra SQLite files. Verifies AC 6.
- Goal-based: `cargo fmt --check`, `cargo check -p zbot-engram-adapter`, focused adapter tests, and a diff check showing no production changes under `gateway/`, `runtime/`, or `apps/`. Verifies AC 6.
- Goal-based: the gaps record names deferred export/import constraints, host DTO/trait reexports, taxonomy expansion, generic evidence, maintenance jobs, production backend/conformance, context-subgraph production, and upstream revision pinning. Verifies AC 7.

**Approach:**
- Keep the pre-facade compatibility fixture immutable and isolated to a temporary root; never recreate it through the facade implementation under test.
- Record the sibling-checkout revision issue as an explicit RFC-0012 prerequisite rather than asserting it is pinned.
- Update status/criteria as tasks ship.

**Done when:** the adapter reopens existing semantic data intact, scope containment is mechanically clean, and deferred upstream work is discoverable.

## Rollout

- **Delivery:** one adapter-only PR. New operations are adapter-private with no runtime caller, so behavior is inert until a future approved semantic-memory spec adopts one.
- **Infrastructure:** none; `engram_data.db` and its existing root remain unchanged.
- **Deployment sequencing:** T1 precedes service work. Rollback is a code revert; no data migration or export/import runs.

## Risks

- Facade compatibility can drift across the sibling Engram checkout; tests catch API drift, while immutable revision pinning remains a documented prerequisite.
- Path or symlink behavior may differ by OS. Tests cover supported Unix behavior and code fails closed where canonicalization is unavailable.
- Provider errors can reveal storage internals. Adapter-private wrappers redact them and do not retry.
- Removing direct typed-port dependencies now would require host-safe upstream exports and is deferred.

## Changelog

- 2026-07-16: Initial full-mode plan; scope confirmed as adapter-only semantic-memory integration.
- 2026-07-16: Incorporated plan-round-1 security and adversarial review: feature-enable SQLite, enforce confined paths, separate raw recall readiness, defer raw export/import, redact operation errors, and require existing-data compatibility proof.
- 2026-07-16: Incorporated plan-round-2 review: test crate-private operations in source modules, capture a pre-facade compatibility baseline before bootstrap changes, and limit migration access to inspection/dry-run.
- 2026-07-16: Incorporated plan-round-3 review: keep T4 coverage in source-unit tests and name the only permitted migration methods explicitly.
