# Spec: Unique Model Tool Inventory

- **Status:** Shipped
- **Owner:** phanijapps
- **Plan:** [`plan.md`](plan.md)
- **Constrained by:** none
- **Brief:** none
- **Discovery:** none
- **Contract:** none
- **Shape:** service

> **Spec contract:** this document defines what "done" means. The implementing
> PR must match this spec, or update it. Verification must be derivable from it.

## Objective

Every agent execution reaches provider request construction with one model-visible definition per tool name, so ward and reviewer delegations can start normally instead of failing locally with `tool_schema_rule=duplicate_name`.

## Boundaries

### Always do

- Preserve strict duplicate-name validation at the provider-request boundary.
- Preserve each actor kind's existing capability policy and model-visible tool membership.
- Exercise the raw registry inventory in tests so duplicate entries cannot be hidden by set collection.

### Ask first

- Change the global `ToolRegistry` registration semantics.
- Change MCP name normalization or collision policy.
- Add or remove an actor capability.

### Never do

- Silently deduplicate schemas at the provider boundary.
- Weaken or bypass model-tool schema validation.
- Modify user provider, MCP, skill, or agent configuration as part of this fix.

## Testing Strategy

The unique-name invariant uses TDD at the gateway registry boundary: a regression test first fails on the raw inventory and then passes after the minimum registration fix. Actor capability preservation uses goal-based assertions over the same real registry builder. Workspace formatting, compilation, lint, and test commands provide the broader goal-based checks.

## Acceptance Criteria

- [x] The raw built-in tool registry has no duplicate names for Root, DelegatedExecutor, DelegatedReviewer, and WardAgent with file tools both disabled and enabled.
- [x] Each actor and file-tools combination retains its characterized distinct tool-name inventory; the only raw-inventory change is removal of the redundant second `read` entry.
- [x] WardAgent and DelegatedReviewer retain `read` and `glob`, and the fix does not change any actor's allowed capability set.
- [x] Strict model-tool validation remains enabled and no longer rejects these built-in actor inventories with `tool_schema_rule=duplicate_name`.
- [x] No provider, MCP, skill, agent, or user configuration changes are required.

## Assumptions

- Technical: `ReadTool` is registered once through `ToolCapability::FileRead` and a second time in the legacy file-tools block for ward and reviewer actors (source: `gateway/gateway-execution/src/invoke/executor.rs`).
- Technical: request preparation intentionally rejects duplicate model-visible tool names before a provider call (source: `runtime/agent-runtime/src/llm/openai.rs`).
- Technical: the duplicate branch predates strict request validation, which exposed the latent registry defect (source: git history probes for commits `c929d7da` and `d3bd0036`).
- Product: retain strict validation and repair the duplicate registration without changing messaging or capability policy (source: user confirmation 2026-08-04).
