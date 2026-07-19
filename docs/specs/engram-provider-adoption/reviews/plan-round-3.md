# Plan review — round 3

## Blockers

**1. Keep T4 private-operation coverage in source.** `plan.md:T4` assigns an integration test to crate-private operations. Fix: make the batch/provenance operation test a source-unit TDD test in `semantic_services.rs` and explicitly exclude such coverage from external integration crates.

## Nits

**2. Name allowed migration methods exactly.** The plan excludes apply/import but the read-only dry run is `MigrationService::dry_run_import`. Fix: permit only `schema_version`, `adapter_version`, and `dry_run_import`; forbid `apply_import`, automatic execution, and migration-mode mutation.
