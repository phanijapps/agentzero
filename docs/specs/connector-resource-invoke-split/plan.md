# Plan: Connector Resource Invoke Split

- **Spec:** [`spec.md`](spec.md)
- **Status:** Done

## Approach

Add two narrow tool wrappers in `runtime/agent-tools/src/tools/connectors.rs`
over the existing provider trait. Keep `QueryResourceTool` as the internal
compatibility implementation, but hide it from model schemas in
`gateway-execution` once the new tools are registered. Update templates and the
spec/backlog state in the same change so the documented model surface matches
runtime behavior.

Tempted to add a new connector service layer; declining because the provider
trait already separates read and invoke. Tempted to delete `query_resource`;
declining until replay/compatibility paths are audited. Tempted to add a
config flag; declining because the target state is unconditional cleanup.

## Tasks

### T1: Add narrow connector tools

**Depends on:** none

**Status:** Done on 2026-07-09.

**Mode:** TDD

**Touches:** `runtime/agent-tools/src/tools/connectors.rs`,
`runtime/agent-tools/src/tools/mod.rs`, `runtime/agent-tools/src/lib.rs`

**Tests:**

- `cargo test -p agent-tools connector_resource --locked`
- `cargo test -p agent-tools connector_invoke --locked`

**Approach:**

- Add `ConnectorResourceTool` with `list` and `query` actions.
- Add `ConnectorInvokeTool` with `connector_id`, `capability`, and `payload`.
- Reuse the existing provider trait and evidence intake helper.

### T2: Register narrow tools and hide compatibility wrapper

**Depends on:** T1

**Status:** Done on 2026-07-09.

**Mode:** TDD

**Touches:** `gateway/gateway-execution/src/invoke/executor.rs`

**Tests:**

- `cargo test -p gateway-execution connector_split --locked`
- `cargo test -p gateway-execution broad_tools --locked`

**Approach:**

- Map `connector_resource` and `connector_invoke` to connector capability.
- Register all three tools internally when a connector provider exists.
- Add `query_resource` to the model-hidden set and catalog it as hidden with
  split targets.

### T3: Prompt/spec cleanup and gates

**Depends on:** T2

**Status:** Done on 2026-07-09.

**Mode:** Goal-based check

**Touches:** `gateway/templates/**`, `docs/backlog.md`,
`docs/specs/connector-resource-invoke-split/*`,
`docs/specs/README.md`, `tools/context_capability_cleanup.py`

**Tests:**

- `python3 tools/context_capability_cleanup.py`
- `rg -n 'query_resource' gateway/templates /home/videogamer/Documents/zbot/config --glob '*.md' --glob '*.json'`
- `cargo fmt -p agent-tools -p agent-runtime -p gateway-execution -p gateway -- --check`
- `cargo check -p gateway --locked`
- `cargo clippy -p gateway --all-targets --locked -- -D warnings`

**Approach:**

- Replace model-facing prompt references with `connector_resource` and
  `connector_invoke`.
- Remove the backlog item once the spec ACs are checked.
- Keep historical docs and tests allowed to mention `query_resource`.

## Changelog

- 2026-07-09: initial focused implementation plan.
- 2026-07-09: implemented split. Added `connector_resource` and
  `connector_invoke`, hid `query_resource` from model-visible schemas while
  preserving internal compatibility registration, and removed the backlog
  deferral.
