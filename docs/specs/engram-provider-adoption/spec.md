# Spec: Engram Provider Adoption

- **Status:** Shipped
- **Owner:** @phanijapps
- **Plan:** [`plan.md`](plan.md)
- **Constrained by:** [`RFC-0011`](../../rfc/0011-engram-memory-engine-cutover.md); [`RFC-0012`](../../rfc/0012-engram-upstream-risk-reduction.md); [`unified-recall-default`](../unified-recall-default/spec.md)
- **Brief:** none
- **Contract:** none
- **Shape:** integration

> **Spec contract:** this document defines what "done" means. The implementing PR must match this spec, or update it. Verification must be derivable from it.

## Objective

Make `zbot-engram-adapter` use Engram's stable provider facade for semantic-memory lifecycle and prepare capability-gated, adapter-internal access to Engram unified recall, batch ingest, provenance, migration, and diagnostics. The change preserves canonical trusted-root confinement and current SQLite data, and does not change the agent, gateway, UI, model-tool contracts, user configuration, or semantic-memory behavior.

## Boundaries

### Always do

- Keep production changes inside `stores/zbot-engram-adapter`; update only related documentation and workspace dependency metadata.
- Construct provider configuration exclusively through `AdapterConfig::to_engram_config` and its canonical trusted-root resolution; reject traversal, outside-root paths, and storage/root symlinks that escape the configured data root.
- Preserve `AdapterConfig`, `engram_data.db` single-file layout, fail-closed behavior, existing store contracts, and semantic data.
- Keep new semantic-service operations `pub(crate)` and convert provider-operation failures to stable redacted adapter errors. Preserve only batch guarantee, status, and safe per-step outcome codes; do not expose raw provider paths, SQL, stack traces, or error strings. Migration operations are limited to `schema_version`, `adapter_version`, and `dry_run_import`.
- Check each new service against both its upstream capability and typed handle, and verify crate-private behavior through source-unit tests plus temporary-directory integration tests for public bootstrap/store compatibility.

### Ask first

- Delete or replace live typed Engram port dependencies and store adapters.
- Change semantic storage files, migration policy, retention policy, or existing user data.
- Wire a service into `gateway/`, `runtime/`, `apps/`, model tools, REST/WebSocket/AG-UI contracts, or user configuration.
- Add a backend, select Postgres/Surreal, add a dependency, or expose export/import operations.

### Never do

- Never change UI, agent execution, delegation, prompt/context assembly, or model-tool behavior.
- Never bypass capability gating, trusted-root validation, canonical confinement, scope handling, or fail-closed errors.
- Never introduce a top-level directory, provider registry, service locator, backend-selection layer, or package dependency. The existing `engram-integration` dependency may enable its existing `sqlite` feature.
- Never expose raw export/import paths or operations; that requires a dedicated portability spec with server-owned confined roots and budgets.
- Never expose `apply_import`, automatic migration execution, migration-mode mutation, or any write-capable migration wrapper.
- Never claim atomic cross-store writes or working non-SQLite backend support.

## Testing Strategy

- **TDD:** provider forwarding, capability/handle agreement, error redaction, and unsupported paths; these are compact adapter invariants.
- **Goal-based check:** dependency/feature selection and source containment, verified with `cargo tree`, `cargo check -p zbot-engram-adapter`, and scoped diff review.
- **Integration:** temporary-root SQLite tests for canonical confinement (including symlinks) and a versioned pre-facade `engram_data.db` compatibility fixture. Source-unit tests cover crate-private unified recall, batch guarantee/outcomes, scoped provenance, `schema_version`/`adapter_version`/`dry_run_import`, and diagnostics.

## Acceptance Criteria

- [x] Bootstrap uses `engram_integration::EngramProvider::open` with its `sqlite` feature explicitly enabled; z-Bot no longer depends on or imports `engram-conformance` for runtime provider construction.
- [x] Provider opening uses the adapter's trusted-root path resolution only, rejecting traversal, outside-root paths, and root/storage symlink escapes before Engram receives configuration.
- [x] Adapter-internal operations for unified recall, batch ingest, provenance, `schema_version`/`adapter_version`/`dry_run_import`, and observability require both a supported upstream capability and present typed handle; absent/mismatched services return stable redacted adapter errors without retries. `apply_import`, automatic execution, and migration-mode mutation remain unavailable.
- [x] Raw Engram unified-recall availability is separately named as adapter-internal readiness and does not alter existing `AdapterFeature::Recall`, `RecallSupportReport`, ranking, scope, authorization, taxonomy, or model-visible output.
- [x] Batch forwarding preserves Engram's actual best-effort guarantee and safe per-step statuses without asserting an artificial or unsupported partial-failure scenario.
- [x] Facade bootstrap reopens a representative existing `engram_data.db` without data loss, integrity failure, or additional SQLite files; existing adapter contracts, focused tests, and `cargo check` pass.
- [x] A durable gaps record defers raw export/import, complete host DTO/trait reexports, taxonomy expansion, generic evidence, maintenance jobs, production backends/conformance, context-subgraph production, and enforceable upstream revision pinning.

## Assumptions

- Technical: `engram_integration::EngramProvider::open` is the current provider entry point and publishes capability-gated handles (source: `/home/videogamer/projects/mem-alpha/core/integration/src/provider.rs`).
- Technical: the adapter currently uses `engram_conformance::bootstrap_provider` and has a direct conformance dependency (source: `stores/zbot-engram-adapter/src/bootstrap.rs`, `stores/zbot-engram-adapter/Cargo.toml`).
- Technical: z-Bot's unified-recall contract retains scope, authorization, taxonomy, output safety, and model-visible behavior outside Engram (source: `docs/specs/unified-recall-default/spec.md`).
- Technical: trusted-root confinement is currently centralized by `AdapterConfig::to_engram_config` and `resolve_engram_path` (source: `stores/zbot-engram-adapter/src/config.rs`).
- Process: active specs require verifiable criteria, task dependencies, testing strategy, and status metadata (source: `docs/CONVENTIONS.md` §4).
- Product: this is adapter-only semantic-memory integration; UI, agent, gateway, user configuration, and model-tool changes are out of scope (source: user confirmation 2026-07-16).
- Product: the integration exposes no new external REST, WebSocket, AG-UI, or model-tool contract (source: user confirmation 2026-07-16).
