# Spec: Connector Resource Invoke Split

- **Status:** Shipped
- **Owner:** phanijapps
- **Plan:** [`plan.md`](plan.md)
- **Constrained by:** [`context-capability-registry`](../context-capability-registry/spec.md)
- **Brief:** none
- **Contract:** none; preserves existing connector provider trait and gateway/UI route contracts while changing the model-visible first-party tool surface.
- **Shape:** integration

## Objective

Finish the context capability tool cleanup by replacing the broad
model-visible `query_resource` connector facade with two narrow tools:
`connector_resource` for connector discovery/read-only resource queries and
`connector_invoke` for side-effecting connector capability calls. The existing
`query_resource` implementation may remain registered for internal
compatibility, but it must be hidden from model-visible schemas and no active
prompt/template should teach the model to call it.

## Boundaries

### Always do

- Preserve the existing `ConnectorResourceProvider` trait and bridge/gateway
  connector behavior.
- Keep read-only connector resource queries distinct from side-effecting
  capability invokes in tool names, schemas, catalog metadata, and prompts.
- Preserve explicit resource-read evidence recording behavior for connector
  reads.
- Keep actor capability enforcement in gateway execution code.

### Ask first

- Changing public HTTP connector APIs, WebSocket events, or provider trait
  method signatures.
- Deleting the internal `query_resource` implementation before compatibility
  callers and replay paths are audited.

### Never do

- Never leave `query_resource` visible in the default model tool schema after
  the narrow tools are registered.
- Never route connector invoke through the read-only connector resource tool.
- Never record connector reads as durable evidence unless the caller explicitly
  asks for `record_evidence=true`.

## Testing Strategy

- **TDD:** `agent-tools` connector tests prove the narrow tools expose distinct
  schemas and delegate to the correct provider methods.
- **TDD:** gateway-execution catalog/registry tests prove the broad wrapper is
  still internally registered but hidden from the model while
  `connector_resource` and `connector_invoke` are model-visible.
- **Goal-based checks:** prompt/template grep and cleanup deny-list prove
  active guidance no longer references `query_resource`.

## Acceptance Criteria

- [x] `connector_resource` supports connector listing and read-only resource
  queries, including explicit resource-read evidence recording.
- [x] `connector_invoke` supports connector capability invocation without
  exposing read-only query parameters or evidence recording controls.
- [x] `query_resource` remains internally executable for compatibility but is
  hidden from default model-visible schemas and catalog metadata points to the
  split targets.
- [x] Gateway actor capability registration exposes the narrow connector tools
  wherever connector querying was previously allowed.
- [x] Active gateway templates and live prompts do not instruct models to call
  `query_resource`.
- [x] Targeted Rust tests, cleanup checks, formatting, typecheck, and clippy
  pass.

## Assumptions

- Technical: `ConnectorResourceProvider` already has separate
  `list_connectors`, `query_resource`, and `invoke_capability` methods, so the
  split can happen at the tool facade without changing provider internals
  (verified in `runtime/agent-primitives/src/connectors.rs`).
- Technical: first-party tool actor policy is enforced in
  `gateway/gateway-execution/src/invoke/executor.rs`, so model visibility can
  be changed independently from internal registration (verified in existing
  `model_hidden_tools` support).
- Product: connector actions must keep working; the cleanup target is the broad
  model-facing wrapper, not connector functionality (settled by backlog entry
  `connector-resource-invoke-split`).
- Process: context capability work tracks deferred cleanup in `docs/backlog.md`
  until a focused spec closes it (verified in `docs/backlog.md`).
