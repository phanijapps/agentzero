# Spec: Conversation Compatibility Retirement

- **Status:** Shipped
- **Owner:** phanijapps
- **Plan:** [`plan.md`](plan.md)
- **Constrained by:** [`conversation-store-revamp`](../conversation-store-revamp/spec.md)
- **Brief:** none
- **Contract:** none
- **Shape:** mixed

> **Spec contract:** this document defines what "done" means. The implementing
> PR must match this spec, or update it. Verification must be derivable from it.

## Objective

Retire the legacy conversation compatibility layer now that Engram owns semantic
memory/knowledge and `zbot-conversation` owns the append-only conversation log.
After this change, live runtime, delegation, distillation, memory sleep, graph,
and gateway state code must no longer depend on `ConversationRepository`,
`ConversationStore`, or legacy message PODs. Transcript reads and writes go
through `MessageStore`; session metadata reads go through a narrow
conversation-store metadata port; semantic memory and knowledge remain on
Engram. The result is a leaner zbot persistence stack with one active
conversation DB path and no old facade keeping dead code alive.

## Boundaries

The three-tier guard that keeps an implementing agent inside the lines.

### Always do

- Treat Engram as already integrated and out of scope except where a caller
  already consumes Engram-backed trait stores.
- Route conversational transcript behavior through `MessageStore`: replay,
  append, child transcript lookup, tool sequence, and history bootstrap.
- Route session metadata reads through one narrow metadata port in
  `zbot-conversation`, not through the old repository.
- Preserve existing gateway/UI HTTP DTO field sets while changing internals.
- Keep the implementation mechanical and deletion-oriented: migrate consumers,
  prove grep-clean, then delete the old symbols.

### Ask first

- Changing `sessions` or `agent_executions` DDL.
- Changing the active on-disk DB location away from
  `~/Documents/zbot/data/conversations.db`.
- Moving semantic memory/knowledge responsibilities out of Engram.
- Deleting root-level `~/Documents/zbot/conversations.db` as part of this spec.

### Never do

- Do not create another broad facade such as a new `ConversationStore` mega-trait.
- Do not reintroduce `ConversationRepository` as a compatibility wrapper.
- Do not route tool args/results back into `execution_logs.metadata`.
- Do not change UI/gateway contracts to make the cleanup easier.
- Do not delete old repository definitions until consumers are grep-clean.

## Testing Strategy

- **TDD:** `zbot-conversation` metadata and chat-history conversion helpers.
  These are small deterministic transformations and query ports.
- **Goal-based checks:** each migration task must compile after moving a
  consumer group, and grep must show the expected old-symbol count shrinking.
- **Integration:** gateway-execution tests cover runner history, delegation,
  distillation, and `wait_agent`/Mission Control behavior through the new
  stores.
- **Manual QA:** after deletion, run one real daemon session with delegation and
  Mission Control inspection to confirm transcript, tool IO, child results, and
  semantic memory remain visible.

## Acceptance Criteria

- [x] No production Rust code references `ConversationRepository`.
- [x] No production Rust code references `ConversationStore`.
- [x] No production Rust code calls `append_session_message`,
  `get_session_conversation`, `get_session_ward_id`, `get_session_agent_id`, or
  `session_messages_to_chat_format`.
- [x] `AppState` no longer has a `conversations` field.
- [x] Runner first-turn and continuation history loads from
  `MessageStore::replay` and converts through a pure helper.
- [x] Delegation spawn/callback writes and reads use `MessageStore` plus the
  narrow session metadata port.
- [x] Distillation transcript and ward lookup use `MessageStore` plus the
  narrow session metadata port.
- [x] Memory sleep, pattern extraction, and handoff writing no longer consume
  `ConversationStore`.
- [x] `stores/zbot-stores-sqlite/src/repository.rs`,
  `stores/zbot-stores-traits/src/conversation.rs`, and
  `stores/zbot-stores-domain/src/message.rs` are deleted or reduced so no
  superseded public symbols remain.
- [x] `cargo check --workspace --locked` and relevant Rust/UI tests pass.

## Assumptions

- Technical: Engram is already integrated as the runtime semantic
  memory/knowledge provider, through `zbot-engram-adapter` and
  `build_engram_store_bundle` (source: `gateway/Cargo.toml`,
  `gateway/src/state/mod.rs`, `gateway/src/state/persistence_factory.rs`).
- Technical: the new conversation store exists and is wired into `AppState` as
  `MessageStore`/`CheckpointStore` via `zbot-conversation` and
  `open_conversation_pool` (source: `stores/zbot-conversation/src/messages.rs`,
  `stores/zbot-conversation/src/checkpoints.rs`, `gateway/src/state/mod.rs`).
- Technical: old conversation compatibility surfaces still have live references;
  targeted grep found 133 hits for the old symbols before this spec was drafted
  (source: `rg ... | wc -l` returned `133`).
- Technical: there is no `docs/architecture/reference.md`, so the plan derives
  stack/components from existing repo structure and the current conversation
  store revamp spec (source: probe `test -f docs/architecture/reference.md`
  returned `1`).
- Process: the prior conversation-store revamp explicitly forbids a god facade
  and in-place migration of `zbot-stores-sqlite` (source:
  `docs/specs/conversation-store-revamp/spec.md`).
- Product: this spec targets complete removal of the old conversation
  compatibility layer, not another intermediate shim (source: user confirmation
  2026-07-08).
- Product: it is acceptable to introduce one narrow `SessionMetaStore` in
  `zbot-conversation` for ward/agent/session metadata reads (source: user
  confirmation 2026-07-08).
- Product: root-level `~/Documents/zbot/conversations.db` cleanup is outside
  this spec; `~/Documents/zbot/data/conversations.db` remains the active
  conversation DB (source: user confirmation 2026-07-08).
