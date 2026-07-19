# Spec: Rig Engine Migration

- **Status:** Shipped
- **Owner:** phanijapps
- **Plan:** [`plan.md`](plan.md)
- **E2E Test Cases:** [`e2e_test_cases.md`](e2e_test_cases.md)
- **Constrained by:** [`runtime-context-control`](../runtime-context-control/spec.md);
  [`mcp-oauth`](../mcp-oauth/spec.md);
  [`subagent-role-gating`](../subagent-role-gating/spec.md);
  [`agent-handoff-notes`](../agent-handoff-notes/spec.md);
  [`builder-delegation-hygiene`](../builder-delegation-hygiene/spec.md);
  [`simplified-provider-model-configuration`](../simplified-provider-model-configuration/spec.md)
- **Brief:** none
- **Contract:** none; preserves existing REST invoke/session behavior, `ClientMessage` controls, direct `ServerMessage` responses, and `StreamEvent` -> `GatewayEvent` -> `ServerMessage` behavior with parity tests instead of introducing a new `contracts/` tree.
- **Shape:** mixed

> **Spec contract:** this document defines what "done" means. The implementing
> PR must match this spec, or update it. Verification must be derivable from it.

## Objective

Replace AgentZero's current `zero-*` framework layer, `zero-stores*` crate names, and active runtime engine internals with a Rig-backed engine while preserving the zbot product contract: existing gateway HTTP/WebSocket behavior, UI event semantics, agent/provider/tool config formats, memory and knowledge stores, delegation/continuation behavior, and conversation history compatibility. The migration succeeds when zbot executes through a Rig-backed `AgentExecutor` facade, no active `zero-*` crates/imports remain in production code or manifests, and replay/parity checks against the current conversation database plus synthetic E2E fixtures show that gateway-visible behavior remains stable.

## Boundaries

The three-tier guard that keeps an implementing agent inside the lines.
*Always do* applies without asking; *Ask first* requires human sign-off
before proceeding; *Never do* is a hard rule, even under time pressure.

### Always do

- Preserve gateway and UI contracts: current `GatewayEvent`, `ServerMessage`, websocket routing, REST invocation shape, and session-scoped event delivery remain stable unless this spec is updated first.
- Preserve existing user-facing configs for agents, providers, MCPs, MCP OAuth, skills, connectors, tool settings, and data directories; map them into Rig internally.
- Keep memory and knowledge ownership in the existing stores and services; Rig memory interfaces may adapt to them but must not replace them.
- Use the current conversation database as an external parity baseline before migration, and generate sanitized synthetic E2E fixtures when private data cannot be committed.
- Maintain actor-kind tool policy as a hard executable-tool filter outside prompt text, including root, delegated executor, delegated reviewer, and ward-agent distinctions.
- Preserve raw tool result, model-visible context result, persisted session message, and UI event payload distinctions.
- Remove active `zero-*` ownership after the migration: no workspace crates/packages named `zero-*`, no `zero_core`/`zero_agent`/`zero_stores*` production imports, and no active architecture docs describing `zero-*` as the live framework or persistence layer.

### Ask first

- Changing the `GatewayEvent`, `ServerMessage`, or client websocket protocol shape instead of adapting Rig output into the existing shape.
- Changing durable schema for `conversations.db`, `knowledge.db`, or store traits beyond compatibility migrations required by the engine adapter.
- Dropping support for existing OpenAI-compatible provider configuration in favor of Rig-native providers only.
- Weakening MCP OAuth token separation, runtime bearer injection, or no-secret response behavior from `mcp-oauth`.
- Weakening subagent actor capability policy from `subagent-role-gating`.
- Weakening handoff/steering ownership or current-session validation from `agent-handoff-notes`.
- Weakening delegation mode propagation, inference, or mode-specific executor rules from `builder-delegation-hygiene`.
- Committing real conversation database content, raw private transcripts, API keys, connector credentials, or user-specific vault data into the repository.
- Removing or renaming public CLI/UI commands, REST routes, or agent config fields that users already depend on.
- Keeping any active `zero-*` crate or import as a long-term shim after the migration is declared complete.

### Never do

- Never rewrite gateway/UI behavior around Rig-native event types directly; Rig stream output must be adapted into AgentZero's existing runtime/gateway event chain first.
- Never make prompt instructions the enforcement boundary for tool authorization, filesystem access, connector auth, or actor role policy.
- Never treat `conversations.db` as durable semantic memory or replace `knowledge.db` with it.
- Never commit the current production conversation database as a fixture; use an external snapshot path or sanitized synthetic data.
- Never leave a second live execution engine path that bypasses the Rig-backed `AgentExecutor` facade after cutover.
- Never leave active `zero-*` production crates, Cargo dependencies, imports, re-exports, or docs as the runtime or persistence architecture after cleanup.

## Testing Strategy

- Contract and parity behavior: **goal-based check plus E2E/integration surface**. The migration preserves an existing contract rather than adding a new one, so tests should compare runtime/gateway outputs before and after against a captured old-engine signature from the current conversation DB baseline and sanitized synthetic fixtures.
- Engine adapter logic: **TDD**. Mapping from AgentZero configs, messages, tools, Rig stream items, hooks, and errors into existing runtime types has compact invariants that should be unit-tested.
- Tool policy and actor roles: **TDD plus integration**. Capability filtering, hidden runtime context injection, and model-visible schemas must be proven with focused tests and gateway-execution actor tests.
- Runtime/gateway integration: **goal-based check**. Targeted `cargo test` gates must prove delegation, continuation, stop/cancel, context state, and websocket mapping stay stable.
- Cleanup of `zero-*`: **goal-based check**. Repository searches and workspace build checks must prove no active `zero-*` crates, imports, dependencies, persistence crate names, or architecture docs remain.
- Manual smoke after implementation: **manual QA**. After migration, the user will create a new database for manual testing; the daemon/UI should run through normal chat, tool use, delegation, and continuation flows against that fresh database.

## Acceptance Criteria

- [x] A reproducible parity baseline exists before engine migration starts: the current conversation DB is snapshotted or referenced outside the repo, an old-engine baseline artifact records sanitized session/event signatures and deterministic comparison dimensions, and committed synthetic E2E fixtures are derived from observed session/event patterns without private content.
- [x] `AgentExecutor` or its direct successor remains the gateway-facing execution facade, and gateway-execution continues to consume AgentZero `StreamEvent` values rather than Rig-native stream items.
- [x] Rig dependency source/version is pinned deliberately in manifests and lockfiles, and `cargo metadata` identifies the selected Rig package/version or Git revision.
- [x] Existing agent, provider, MCP, MCP OAuth, skill, connector, and tool settings load without user-facing format changes and are mapped into Rig builder/runner/provider/tool configuration internally.
- [x] Rig model execution, tool execution, hooks, memory loading/appending, streaming, and errors are adapted into current runtime events with stable ordering for token, thinking, tool call, tool result, respond, done, context state, and error cases.
- [x] Delegation and continuation behavior remains gateway-owned: delegated agent completion still persists the parent callback before completion and still triggers `SessionContinuationReady` when the last delegation finishes.
- [x] Actor tool policy remains hard-filtered by runtime/gateway code, and tests prove root, delegated executor, delegated reviewer, and ward agent receive only the expected executable tools.
- [x] MCP OAuth token storage, dynamic client secret handling, no-secret API responses, and runtime bearer injection remain separated from model-visible tool arguments and Rig-visible configuration.
- [x] Skill loading/listing, connector configs, connector auth scopes, and tool settings flow through the Rig adapter without changing existing config files or UI/API payloads.
- [x] Tool execution preserves hidden runtime context without exposing session IDs, auth scopes, ward IDs, filesystem roots, or store handles in model-visible arguments.
- [x] Tool result handling preserves the distinction between raw result, model-visible `context_result`, persisted session content, and UI-visible event payload.
- [x] Current memory and knowledge layers remain authoritative; no durable facts or knowledge graph state are moved into Rig-owned storage or `conversations.db`.
- [x] Existing live context control and summarization behavior preserves the invariants from `runtime-context-control`; changing those invariants requires updating both specs before implementation proceeds.
- [x] Inbound gateway protocol remains stable: REST invoke/session payloads, `ClientMessage` controls, direct `ServerMessage` responses, subscription behavior, pause/resume/cancel/end, and stop/cancel abort semantics continue to work through the Rig-backed engine.
- [x] Event-derived `GatewayEvent` -> `ServerMessage` mapping remains stable for every current non-deferred event-derived wire message: `AgentStarted`, `AgentCompleted`, `AgentStopped`, `Token`, `Thinking`, `ToolCall`, `ToolResult`, `TurnComplete`, `Error`, `Iteration`, `ContinuationPrompt`, `DelegationStarted`, `DelegationCompleted`, `Heartbeat`, `MessageAdded`, `TokenUsage`, `WardChanged`, `IterationsExtended`, `PlanUpdate`, `IntentAnalysisStarted`, `IntentAnalysisComplete`, `IntentAnalysisSkipped`, `RecallTrace`, and `SessionTitleChanged`; `SessionContinuationReady` remains internal-only.
- [x] Existing store crates are rehomed or renamed away from `zero-stores*`/`zero_stores*` while preserving schema compatibility, store traits, conformance tests, and memory/knowledge behavior.
- [x] No active production Rust crate, workspace package, Cargo dependency, import, or re-export remains for these exact legacy names: `zero-core`, `zero-agent`, `zero-llm`, `zero-tool`, `zero-mcp`, `zero-session`, `zero-prompt`, `zero-middleware`, `zero-app`, `zero-stores`, `zero-stores-domain`, `zero-stores-traits`, `zero-stores-sqlite`, `zero-stores-conformance`, `zero_core`, `zero_agent`, `zero_llm`, `zero_tool`, `zero_mcp`, `zero_session`, `zero_prompt`, `zero_middleware`, `zero_app`, `zero_stores`, `zero_stores_domain`, `zero_stores_traits`, `zero_stores_sqlite`, or `zero_stores_conformance` after the cleanup phase.
- [x] Active architecture docs, AGENTS files, README files, and memory-bank component docs describe the Rig-backed engine and renamed persistence crates, and do not present `zero-*`/`zero-stores*` as the current live framework or persistence architecture.
- [x] Targeted Rust gates pass for runtime, gateway execution, tools, stores touched by type changes, and websocket/event mapping.
- [x] A fresh database manual smoke run can invoke an agent through the UI or CLI, stream tokens, execute at least one first-party tool, complete a delegated-agent continuation, and persist/reload the resulting session.
- [x] Intent analysis no longer depends on prompt-authored JSON or markdown-fence stripping: it uses Rig typed structured output through the `agent-runtime` adapter boundary, keeps `GatewayEvent::IntentAnalysis*` and persisted JSON field names stable, and rejects invalid `ward_recommendation.action` / `execution_strategy.approach` values instead of accepting arbitrary strings.
- [x] Delegation mode behavior from `builder-delegation-hygiene` remains intact through the Rig-backed engine: `delegate_to_agent` mode arguments and inference propagate through `DelegationRequest`, `DelegationContext`, child executor initial state, and all four mode-specific rule paths (`DirectArtifact`, `WardHygiene`, `WardBackedBuild`, and `StepExecutor`).
- [x] The parity sanitizer is schema-aware, positive-allowlist based, and fail-closed: any unhandled text, JSON, blob, tool payload, connector field, vault/user-data field, or sanitizer error aborts before writing committed artifacts; sentinel-secret tests prove raw private strings cannot appear in signatures or fixtures.
- [x] The parity harness canonicalizes `ZBOT_PARITY_DB` and fallback DB paths before opening, rejects ambiguous discoveries and disallowed roots, treats symlink escapes as errors, and writes exact local source metadata only to gitignored local manifests; committed artifacts contain only a non-identifying source label and coarse provenance.

## Assumptions

- Technical: active branch is `rig-engine-migration-spec` (source: `git branch --show-current`, 2026-06-27).
- Technical: repository docs name `conversations.db` as the SQLite conversation database (source: `AGENTS.md`).
- Technical: the local current conversation DB used for parity is referenced through `ZBOT_PARITY_DB` or the parity harness fallback discovery; exact local path, size, and mtime belong only in `.rig-parity/local_manifest.json`, which is gitignored.
- Technical: gateway/UI event parity is centered on `StreamEvent` -> `GatewayEvent` -> `ServerMessage`, with `SessionContinuationReady` as an internal continuation trigger (source: `memory-bank/websocket-events.md`; `docs/research/rig-engine-migration.md`).
- Technical: existing `zero-*` references are broad, not isolated; a repository search found 380 source/manifest references during spec drafting (source: `rg "zero_core|zero-agent|zero_agent|zero_llm|zero_tool|zero_mcp|zero_session|zero_prompt" ... | wc -l`, 2026-06-27).
- Technical: Rig was previously indexed and analyzed at `rig checkout`, commit `6b1991bf`; the migration report recommends a Rig-backed adapter behind `AgentExecutor` (source: `docs/research/rig-engine-migration.md`).
- Process: this repo currently has no `docs/CONVENTIONS.md`, `docs/CHARTER.md`, `docs/architecture/reference.md`, or `contracts/` tree, so this spec preserves existing documented event contracts instead of creating new formal contract infrastructure (source: repository probe 2026-06-27; user delegated contract decision 2026-06-27).
- Product: the migration must remove active `zero-*` framework ownership entirely, not merely hide it behind compatibility shims (source: user confirmation 2026-06-27).
- Product: the current conversation DB should be used as a parity reference before migration; if it cannot be used directly, its observed patterns should drive synthetic E2E fixture generation (source: user confirmation 2026-06-27).
- Product: after migration, the user will create a new database for manual testing (source: user confirmation 2026-06-27).
