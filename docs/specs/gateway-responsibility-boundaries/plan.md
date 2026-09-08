# Plan: Gateway responsibility boundaries

- **Spec:** [spec.md](spec.md)
- **Status:** Done

## Approach

Extract focused services while preserving public facades. Execute sequentially. Baseline is 789fa55a, which includes the previously dirty session-stop changes. Avoid touching invocation, continuation execution, or delegation spawn bodies.

## Constraints

Use named-field dependencies and the existing runner conventions. No crate moves, schema changes, new dependencies, new authorization rules, or lifecycle fixes bundled into structural work.

## Construction tests

Existing exact/recursive cancellation tests and capability catalog tests remain executable. Add direct control tests for missing handles, stop propagation, and iteration extension as needed; exercise stateful operations with real temporary SQLite when necessary. Integration tests must still traverse AppState and ExecutionRunner public methods. Gate commands: cargo check -p gateway -p gateway-execution; cargo test -p gateway-execution --features test-stubs; cargo test -p gateway --lib; cargo clippy -p gateway -p gateway-execution --all-targets --features gateway-execution/test-stubs -- -D warnings. Formatting only task-owned files.

## Design (LLD)

### Component / module decomposition

- runner/session_control.rs: SessionControl owns live-control operations and shares the existing handles with invocation/streaming. Its explicit dependencies are handles, DelegationRegistry and StateService. ExecutionRunner retains recovery orchestration and delegates live operations.
- state/capability_catalog.rs: focused catalog service(s) own tool snapshot construction and resource enrichment. Dependencies are explicit references or Arc clones, assembled at each facade call so mutable AppState fixtures and runtime configuration remain visible. No service caches the catalog or stores a parent facade.

### Failure, edge cases & resilience

Preserve error-before-signal ordering and existing legacy broad-control semantics; preserve exact-cancel traversal and isolation. Preserve minimal-state fallback and metadata discovery failure handling. Do not turn unavailable capabilities into enabled ones.

### Interfaces & contracts

Existing public methods remain unchanged. Internal control access can use named methods or crate-private fields; no Deref escape hatch. Catalog dependency bundles may group coherent inputs without creating a second AppState.

## Tasks

### T1: Execution control delegates through a focused owner

**Depends on:** none
**Touches:** gateway/gateway-execution/src/runner/*
**Verification mode:** goal-based check; no stub (preserved behavior).
**Tests:** SessionControl tests cover missing-handle errors, lookup sharing (a returned handle signals the registered execution), iteration extension clearing stop, direct-child stop propagation, pause/live-resume persisted status plus handle flags, legacy broad cancel/end semantics, exact recursive cancellation excluding unrelated handles, and failed database transitions leaving flags unchanged. Existing runner exact/recursive cancellation and setup/recovery tests remain. Run `cargo test -p gateway-execution --features test-stubs`, typecheck and clippy. Satisfies AC1, AC2, AC4, AC5.
**Approach:** Move control methods into SessionControl; wire its shared dependencies once; keep persisted recovery orchestration in runner. Move associated private helper/tests with ownership, updating only references needed for extraction.
**Done when:** behavior tests pass and public methods delegate without duplicating control logic.

### T2: Capability inspection delegates through focused services

**Depends on:** T1
**Touches:** gateway/src/state/*, gateway/gateway-execution/src/runner/AGENTS.md, gateway/AGENTS.md
**Verification mode:** goal-based check; no stub (preserved behavior).
**Tests:** Preserve `local_context_provider_catalog_entries_report_resource_health`, `connector_catalog_entries_use_logical_resource_uris`, and `resource_catalog_filters_entries_by_actor_and_dedupes`. Add facade tests proving an installed runner wins over divergent fallback stores and changing fallback/provider availability between calls is reflected without stale caching. `cargo test -p gateway --test api_tests tools_` exercises minimal-state fallback, delegated actor filtering, unknown actor rejection, session/agent metadata and path-free JSON through real HTTP routes. Metadata discovery errors retain the base catalog, tested with a failing connector provider; disabled local resources retain Disabled health. Run gateway library tests and affected-crate gates. Satisfies AC3, AC4, AC5.
**Approach:** Move capability builders, enrichment, mapping helpers and focused tests to the owning module. Keep AppState methods as explicit wiring delegates. Update module maps to explain remaining facade responsibilities.
**Metadata inspection test (AC6):** successful and failing connector-list test providers panic if query_resource or invoke_capability is called; catalog enrichment must finish without either call. Only MCP list_summaries and connector list_connectors are used, verified in review for source-mutation absence.
**Done when:** catalog output semantics remain covered, no component retains AppState/ExecutionRunner, and gates/review pass.

## Rollout

Ordinary rebuild; no deployment sequencing or data migration. Revert only this refactor to roll back.

## Risks

Shared registry aliasing and stale optional service captures are the main risks. Fresh catalog dependency assembly and existing Arc identity preserve both. The existing broad pause/cancel/end behavior is intentionally not changed by this extraction.

## Resolve versus surface

- Resolved: direct user authorization covers internal responsibility extraction; unrelated backlog does not replace this task.
- Resolved: runtime stack and preservation baseline verified from source and Git.
- Declined: startup redesign, crate repartition, and new traits without a consumer; each expands this slice beyond a verifiable refactor.
- Review findings and verification outcomes are recorded in review.md.

## Changelog

- 2026-09-06: Initial two-step responsibility extraction plan.
