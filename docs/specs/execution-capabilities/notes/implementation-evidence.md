# Implementation evidence — Execution Capabilities

Recorded: 2026-08-04

All commands ran from the isolated
`/home/videogamer/projects/agentzero-execution-capabilities` worktree on
`feat/execution-capabilities`, based on `origin/develop`. The original
`/home/videogamer/projects/agentzero` worktree was not modified.

## Automated gates

- `cargo fmt --all -- --check` — pass.
- `cargo check -p agent-primitives -p agent-runtime -p gateway-services -p gateway-templates -p gateway-execution` — pass.
- `cargo clippy -p agent-primitives -p agent-runtime -p gateway-services -p gateway-templates -p gateway-execution --all-targets -- -D warnings` — pass.
- `cargo test -p agent-primitives -p agent-runtime -p gateway-services -p gateway-templates -p gateway-execution` — pass. Key totals: `agent-primitives` 40, `agent-runtime` 414, `gateway-execution` 555, `gateway-services` 264, and `gateway-templates` 9; gateway integration and doc tests also passed. After the final lifecycle and deterministic review-test hardening, the full `gateway-execution` suite passed again; its `cold_boot_under_10s_with_10k_entities` case completed successfully in 99.05 seconds of wall-clock setup/test time and reported its internal threshold as satisfied.
- `git diff --check` — pass.
- `cargo build -p daemon` — pass.
- `cd e2e/playwright && npx playwright test full-mode/simple-qa.full.spec.ts` — pass after the final lifecycle hardening: 1 Chromium test, 12.6 seconds total, 5.8-second test body.

Finish-time documentation lints were also invoked. The execution-capabilities
status transition, acceptance criteria, links, and scope are valid, but the
repository-wide commands retain unrelated baseline failures: four unresolved
deferral anchors in existing specs from `lint-spec-status.py`, and ten existing
malformed producer pointers from `lint-traceability.py`. `git diff --quiet
origin/develop` confirms the reported specs and `workspace.toml` are unchanged
on this branch.

Focused review-remediation tests also passed:

- `delegation::spawn::tests::spawn_failures_complete_parent_and_child_lifecycle`
- `invoke::executor::tests::step_executor_cannot_lookup_capabilities_or_delegate`
- `delegation::spawn::tests::persisted_capability_log_uses_canonical_ids_and_closed_rejection_codes`
- `gateway_services::mcp::tests::get_multiple_exact_id_wins_over_colliding_display_name`
- `gateway_services::mcp::tests::dynamic_runtime_resolution_distinguishes_deleted_from_unknown_ids`
- `gateway_templates::tests::plan_composer_requires_exact_capability_briefing_fields`
- `session_state::tests::extract_plan_preserves_capability_briefing_fields_verbatim`
- `agent_runtime::tools::capabilities::tests::lookup_sanitizes_and_bounds_display_names`

## Real Blender MCP smoke

The branch-built daemon was started against an isolated temporary vault copied
from the local non-database configuration. No user database was modified.

1. `POST /api/mcps/blender-mcp/test` returned success, discovered 22 tools,
   and included `get_scene_info`, `get_object_info`,
   `get_viewport_screenshot`, and `execute_blender_code`.
2. A real Research request asked for graph execution, a planner, one delegated
   step, and exactly one read-only `get_scene_info` call. The terminal session
   was `completed` and returned a Blender scene summary.
3. The sanitized execution trace showed:
   - root: `ward`, then `delegate_to_agent`;
   - planner: `lookup_capabilities`, with no Blender MCP tool mounted;
   - delegated `general-purpose` step: capability audit origin `dynamic`,
     `requested_mcps = ["blender-mcp"]`,
     `effective_mcps = ["blender-mcp"]`, and `rejection_codes = []`;
   - delegated tool call: `blender-mcp__get_scene_info`.

The prompt prohibited scene mutation, and the trace contained no mutating
Blender MCP tool call. Temporary vaults were deleted after inspection.

## Acceptance-criterion anchors

| Criterion | Evidence |
|---|---|
| AC-root-simple | `root_assignment_skills_are_merged_into_lazy_recommendations`; root runtime assignment resolution in `invoke_bootstrap`; dynamic runtime resolver tests. |
| AC-graph-catalog | `planner_delegate_carries_host_capability_catalog`; `planner_capability_lookup_requires_host_catalog_state`; real graph trace with planner `lookup_capabilities`. |
| AC-delegated-discovery | Exact briefing tests plus real delegated `blender-mcp__get_scene_info` call with effective assignment before child execution. |
| AC-planner-nonceiling | Complete host catalog handoff/lookup tests and real planner canonical-ID lookup. |
| AC-catalog-complete | `lookup_is_bounded_and_sanitized`; host catalog handoff tests. |
| AC-target-validation | `spawn_failures_complete_parent_and_child_lifecycle`; `invalid_dynamic_target_is_rejected_without_creating_a_specialist`. |
| AC-step-least-privilege | `step_executor_cannot_lookup_capabilities_or_delegate`; `ward_backed_planner_retains_lookup_and_delegation`. |
| AC-fallback-semantics | Dynamic resolver tests, exact-ID collision test, delegate assignment transport tests, and unchanged legacy fallback path in the affected suite. |
| AC-runtime-revalidation | Ready/disabled/OAuth/deleted/unknown resolution tests and persisted safe-log test. |
| AC-startup-failure | `mcp_discovery_failure_is_nonfatal_and_not_retried`; `spawn_error_is_redacted_to_a_safe_host_diagnostic`. |
| AC-lookup-bounds | `lookup_is_bounded_and_sanitized`; `lookup_caps_description_even_for_malformed_host_catalog`; `lookup_sanitizes_and_bounds_display_names`. |
| AC-audit-logging | `persisted_capability_log_uses_canonical_ids_and_closed_rejection_codes` plus real dynamic requested/effective trace. |
| AC-verification | Automated gates and full-mode E2E listed above. |
