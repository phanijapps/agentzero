# Plan: Conversation Compatibility Retirement

- **Spec:** [`spec.md`](spec.md)
- **Status:** Drafting

> **Plan contract:** this is the implementation strategy. Unlike the spec, this
> document is allowed to change as we learn. When it changes substantially,
> note why in the changelog at the bottom.

## Approach

Peel the old conversation facade off in dependency order. First add the one
missing narrow capability (`SessionMetaStore`) and a pure message-to-chat
conversion helper. Then migrate consumers by live path: runner history,
delegation, distillation, memory sleep/handoff, graph/gateway state. After each
group compiles and tests pass, remove the old wiring. Only once production
grep is clean, delete `ConversationRepository`, `ConversationStore`, and the
legacy message POD. Engram remains untouched except where existing callers
already use Engram-backed trait stores.

## Constraints

- Follows `conversation-store-revamp`: no god facade, no in-place migration of
  `zbot-stores-sqlite`, and no returning payloads to `execution_logs.metadata`.
- Active conversation DB stays `VaultPaths::conversations_db()`
  (`~/Documents/zbot/data/conversations.db`).
- Semantic memory/knowledge stays Engram-backed.
- HTTP/UI DTO field sets stay stable.

## Construction tests

**Integration tests:**
- Existing `gateway-execution` session-state and delegation tests must pass.
- Add focused tests where a migrated consumer previously depended on
  `ConversationRepository` behavior.

**Manual verification:**
- Start daemon, run one session with at least one tool call and one delegated
  child, inspect Mission Control, and confirm tool IO, child result, session
  title, token totals, and semantic memory visibility still work.

## Design (LLD)

### Design decisions

- Replace old broad conversation facade with two narrow concerns:
  `MessageStore` for transcript behavior and `SessionMetaStore` for session
  metadata.
- Keep conversion to `agent_runtime::ChatMessage` as a pure helper, not a store
  method, so persistence stays storage-shaped and runtime formatting stays
  runtime-shaped.
- Delete only after grep-clean to avoid compile-time breakage or hidden runtime
  regressions.

### Data & schema

- No schema migration is planned.
- `SessionMetaStore` reads existing `sessions`/`agent_executions` data from the
  active conversation DB.
- `MessageStore` continues to own `messages` replay and append behavior.

### Interfaces & contracts

- No public HTTP/API contract changes.
- Internal contract changes:
  - add `SessionMetaStore` to `zbot-conversation`;
  - add message-to-chat conversion helper in `gateway-execution` or
    `agent-runtime` adjacent code;
  - remove old public exports once consumers are gone.

### Component / module decomposition

- `stores/zbot-conversation`: own message log and narrow session metadata port.
- `gateway-execution`: own runtime conversion from stored messages to LLM chat
  messages, plus runner/delegation/distillation migrations.
- `gateway-memory`: consume `MessageStore`/`SessionMetaStore` for sleep and
  pattern extraction.
- `gateway`: stop exposing `AppState.conversations` and pass narrow stores.
- `stores/zbot-stores-sqlite` and `zbot-stores-traits`: lose legacy
  conversation compatibility symbols after cutover.

### State & control flow

Runtime reads should flow:

`session_id` -> `MessageStore::replay` -> pure conversion/helper -> runtime
consumer.

Metadata reads should flow:

`session_id` -> `SessionMetaStore` -> ward/agent/session metadata consumer.

Writes should flow:

runtime/delegation callback -> `MessageStore::append`.

### Failure, edge cases & resilience

- If a session has no messages, history bootstrap returns an empty history
  without failing the session.
- If metadata is absent, consumers preserve existing fallback behavior.
- Conversion helper must tolerate malformed `tool_calls` JSON the same way the
  old path did: skip malformed tool-call details rather than panic.

### Quality attributes (NFRs)

- The final grep-clean delete gate is the main maintainability bar.
- No additional runtime DB files are introduced.
- No new broad abstraction or dependency is introduced.

### Dependencies & integration

- Depends on the already-wired `zbot-conversation` and Engram adapter.
- Does not depend on changing Engram.
- Does not require root-level stale DB cleanup.

## Tasks

### T1: Add `SessionMetaStore` to `zbot-conversation` — DONE

**Depends on:** none

**Tests:**
- TDD: seed `sessions` and `agent_executions`; verify ward and agent lookups.
- TDD: missing rows return `None`.

**Approach:**
- Add `SessionMetaStore` trait and `SqliteSessionMetaStore`.
- Re-export from `zbot-conversation`.
- Wire it into `AppState` using the same shared pool as `MessageStore`.

**Done when:** `cargo test -p zbot-conversation` and `cargo check -p gateway`
are green. Completed 2026-07-08.

### T2: Add pure stored-message to chat-message conversion — DONE

**Depends on:** none

**Tests:**
- TDD: user/assistant/tool/system rows convert to the same chat shape as the old
  helper.
- TDD: malformed `tool_calls` JSON is tolerated.

**Approach:**
- Move the behavior of `session_messages_to_chat_format` into a pure helper that
  accepts `zbot_conversation::Message`.
- Keep it near runtime execution code, not in persistence.

**Done when:** helper tests pass and no production caller needs the old
repository conversion method. Helper added 2026-07-08; production caller
migration follows in T3.

### T3: Move runner history bootstrap off `ConversationRepository` — DONE

**Depends on:** T2

**Tests:**
- Existing runner/invoke bootstrap tests pass.
- Add/adjust a test proving existing-session history is loaded from
  `MessageStore::replay`.

**Approach:**
- Update `runner/core.rs` and `runner/invoke_bootstrap.rs`.
- Replace `get_session_conversation(..., 200)` with `MessageStore::replay`.
- Replace old conversion method with the pure helper.

**Done when:** `cargo test -p gateway-execution --lib` is green and old history
methods are gone from runner code. Completed 2026-07-08.

### T4: Move delegation off `ConversationRepository`

**Depends on:** T1, T2

**Tests:**
- Existing delegation tests pass.
- Add/adjust a test for callback system-message persistence via `MessageStore`.

**Approach:**
- Update `delegation/spawn.rs`, `delegation/callback.rs`,
  `runner/continuation_watcher.rs`, and `runner/delegation_dispatcher.rs`.
- Use `SessionMetaStore` for ward lookup.
- Use `MessageStore::replay` for child transcript lookup.
- Use `MessageStore::append` for callback system messages.

**Done when:** delegation code has no `ConversationRepository` references.

### T5: Move distillation off `ConversationRepository`

**Depends on:** T1, T2

**Tests:**
- Existing distillation tests pass.
- Add/adjust transcript-building test from `MessageStore` rows.

**Approach:**
- Change `SessionDistiller` dependencies to `MessageStore` plus
  `SessionMetaStore`.
- Replace transcript and ward lookups.

**Done when:** `distillation.rs` has no `ConversationRepository` references.

### T6: Move memory sleep/handoff off `ConversationStore`

**Depends on:** T1, T2

**Tests:**
- Existing `gateway-memory` sleep/pattern tests pass.
- Existing `gateway-execution` handoff writer tests pass.

**Approach:**
- Update `gateway-memory/src/services.rs`,
  `gateway-memory/src/sleep/worker.rs`,
  `gateway-memory/src/sleep/pattern_extractor.rs`, and
  `gateway-execution/src/sleep/handoff_writer.rs`.
- Replace transcript and tool-sequence reads with `MessageStore`.
- Replace ward/agent metadata reads with `SessionMetaStore`.

**Done when:** no production code references `ConversationStore`.

### T7: Remove `AppState.conversations` and gateway metadata callers

**Depends on:** T1-T6

**Tests:**
- `cargo check -p gateway`.
- Relevant graph/session handler tests pass.

**Approach:**
- Delete `AppState.conversations`.
- Update gateway runtime/state construction.
- Replace `state.conversations.get_session_agent_id` style calls with
  `SessionMetaStore`.

**Done when:** gateway has no `ConversationRepository` construction or field.

### T8: Delete superseded legacy code

**Depends on:** T1-T7

**Tests:**
- Goal-based grep gate:
  `rg "ConversationRepository|ConversationStore|append_session_message|get_session_conversation|get_session_ward_id|get_session_agent_id|session_messages_to_chat_format"`.
- Full verification command set.

**Approach:**
- Delete legacy repository definitions, trait, POD, exports, and obsolete tests.
- Remove stale comments/docs in touched files.
- Keep historical mentions in specs only if explicitly useful.

**Done when:** production grep is clean and workspace checks pass.

## Rollout

- **Delivery:** one branch, sequenced commits. No runtime flag; each commit keeps
  the workspace compiling.
- **Infrastructure:** no new infrastructure.
- **External-system integration:** Engram remains the semantic memory provider.
- **Deployment sequencing:** T1/T2 first, consumer groups next, delete last.

## Risks

- Runner/delegation history conversion may subtly differ from the old helper if
  tests do not cover tool-call rows.
- Memory sleep code may have assumptions hidden behind `ConversationStore`
  mocks.
- Deleting old exports may surface test-only imports late in the sequence.

## Changelog

- 2026-07-08: initial spec and plan.
- 2026-07-08: T1 completed; added `SessionMetaStore` and wired it into
  `AppState` on the shared conversation pool.
- 2026-07-08: T2 completed; added a pure stored-message to runtime chat-history
  conversion helper with malformed tool-call tolerance.
- 2026-07-08: T3 completed; runner continuation and invoke bootstrap history
  now read from `MessageStore::replay`.
