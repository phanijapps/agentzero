# Plan: Durable Generic Agent Tasks

- **Spec:** [`spec.md`](spec.md)
- **Status:** Drafting

> **Plan contract:** this is the implementation strategy. Unlike the spec, this
> document is allowed to change as implementation reveals new facts.

## Approach

Add a gateway-owned `DurableAgentTaskService` that is the only Research
producer for the versioned `agent.task.v1` envelope. It generates stable task
identities, pre-subscribes the requesting WebSocket, authorizes and persists a
strict payload through `DurableWorkQueue`, verifies any deduplicated envelope,
then returns the reserved session ID through the unchanged acceptance message.
Register a single handler in the existing gateway worker. The handler creates
or verifies exact session/execution rows and calls `RuntimeService` for the
first bootstrap, then monitors durable session state to terminal completion. A
deterministic message lookup acts as the replay marker: after a crash, an exact
stored root prompt enters a narrow initial-bootstrap resume path that skips
only append and otherwise reuses normal Research setup; a mismatch fails
closed. Queue completion means terminal task completion, while an exact live
execution is monitored instead of launched twice.

## Constraints

- Preserve the shipped queue's SQLite authority, persist-first ordering,
  dedupe semantics, broker-neutral transport, lease fencing, capped attempts,
  redacted failures, and worker lifecycle.
- Preserve the current WebSocket contract and Research mode normalization; do
  not add a BFF/event/API contract or UI change.
- Use the existing `RuntimeService` and `ExecutionRunner` bootstrap machinery,
  agent loader, prompt construction, tool registration, confirmation gates,
  sandboxing, memory gates, and token/resource limits.
- Use strict serde decoding with `deny_unknown_fields`, explicit string/ID
  bounds, exact enum values, and constant-shape normalized errors.
- Keep dependency order bottom-to-top: conversation/state primitives, gateway
  execution/runtime adapters, gateway task service/handler, WebSocket producer,
  server wiring.

## Construction tests

**Integration tests:** a real temporary conversation database runs the queue,
message store, state service, handler, and a deterministic runtime adapter
through new dispatch, duplicate enqueue, crash-after-message replay, mismatch,
retry, dead-letter, and lease-loss cases. Gateway/WebSocket tests prove
subscribe-before-enqueue, durable-before-accepted, unchanged response shape,
continued-session behavior, and restart polling.

**Manual verification:** from the Research page, submit a generic research
task, restart `zbotd` immediately after acceptance, reconnect, and observe one
session with one user prompt and a terminal answer recovered through the
normal Research snapshot/event flow.

**Execution assumptions:**

- Expected touches: `stores/zbot-conversation`, `services/execution-state`,
  `gateway/gateway-execution`, `gateway/src/{state,services,websocket,server}`,
  focused tests, and this spec directory.
- No schema migration is expected: deterministic IDs use existing primary keys
  and `MessageStore::get` reads the existing `messages.id` key.
- Execution-state and the existing runner remain authoritative for terminal
  agent results; the worker owns retry eligibility until terminal state.
- Declined: a parallel scheduler, result table, public work-status endpoint,
  response envelope, broker adapter, or generic tool-policy DSL. None is needed
  to make Research handoff durable.

## Design (LLD)

### Design decisions

- `agent.task.v1` is a production kind but not a public protocol. One exact
  local handler keeps the future transport seam open without freezing an
  MQTT/Kafka representation. Traces to **AC-task-contract**,
  **AC-production-wiring**, **AC-scope**.
- Queue completion records a terminal completed or user-cancelled session. The
  handler monitors durable state while the worker renews its lease; live exact
  execution is never relaunched, and crashed/paused incomplete state remains
  retryable. The handler uses an inner execution deadline that settles state
  before the worker's bounded outer deadline. Traces to **AC-initial-dispatch**,
  **AC-terminal-ownership**, and **AC-failure-policy**.
- The deterministic root message is the replay marker. Reusing the append-only
  conversation ledger avoids a second transactional result table; exact row
  comparison prevents a primary-key collision from becoming false success.
  Traces to **AC-restart-idempotency**.

### Data & schema

`AgentTaskV1` contains only `agent_id`, `conversation_id`, `message`, `mode`,
`session_id`, `execution_id`, and `message_id`, decoded with
`deny_unknown_fields`. Agent ID is `root` or reuses the shared configured-agent
validator and resolves through `AgentService`; conversation ID is `1..=128`
UTF-8 bytes without ASCII controls; message is `1..=60,000` UTF-8 bytes; mode
is exactly `research`; prefixed IDs contain canonical UUIDs; and the serialized
payload is at most 65,536 bytes. A valid client message ID is preserved and
otherwise the producer mints one. The
queue envelope supplies version, kind, target, source, provenance,
correlation, dedupe, attempts, and timestamps. No task field selects runtime
capabilities. Existing `sessions`, `agent_executions`, `messages`, and
`durable_work_items` tables remain unchanged. Traces to **AC-task-contract**,
**AC-agent-safety**, **AC-scope**.

### Interfaces & contracts

- `DurableAgentTaskService::enqueue_research` accepts a server-owned connection
  identity plus the already-normalized Research request, reserves IDs, verifies
  the same-vault/root-agent binding for continuations, verifies duplicate
  equivalence, and returns `{work_id, session_id, inserted}`. The existing
  gateway admission policy remains the caller authority; this slice does not
  add a parallel auth mechanism.
- `AgentTaskHandler` implements the existing `WorkHandler` exact route,
  deserializes to `AgentTaskV1`, re-loads and checks actor/conversation/session/
  execution/message/root-agent provenance relationships, and calls a narrow
  injected `AgentTaskRuntime` adapter.
- `AgentTaskRuntime` exposes first bootstrap, initial-bootstrap resume from an
  exact persisted root message, terminal-state observation, and bounded abort.
  Its production implementation delegates to `RuntimeService` and never calls
  the post-delegation continuation API; tests use a recording stub. These are
  internal Rust interfaces; the public WebSocket schema is unchanged. Traces to **AC-authorization**,
  **AC-live-compatibility**, **AC-initial-dispatch**.

### Failure, edge cases & resilience

- Pre-subscribe must succeed before enqueue so a fast worker cannot publish
  before the current client is listening. Subscription failure persists no work
  and sends no wake. After enqueue, a connection-owned acceptance watcher
  observes the exact durable root message/session and emits the unchanged event;
  terminal queue failure before readiness emits the existing normalized failure.
- First dispatch creates exact missing session/execution rows, verifies any
  existing rows belong to the requested agent/session, then invokes with the
  deterministic message ID. If that message already exists, exact role,
  session, execution, and content equality routes to initial-bootstrap resume
  when crashed/paused/queued, monitoring when already running, completion when
  terminal-successful, and no resurrection when user-cancelled.
- Duplicate work whose stored payload/provenance differs, identity collisions,
  malformed rows, and authorization failures are permanent integrity/handler
  failures. Runtime unavailable before a durable prompt and initial-bootstrap
  resume unavailable after one are retryable under queue policy. Lease loss and
  shutdown retain the worker's existing cancellation/fencing behavior.
  Traces to **AC-durable-acceptance**, **AC-restart-idempotency**,
  **AC-failure-policy**.

### Quality attributes (NFRs)

No task-controlled capability expansion is introduced. Bounded strict input,
default-deny authorization, exact replay comparison, capped queue attempts,
runtime limits, and payload-free normalized logging cover security and
operability. Focused integration tests assert that sentinel prompt/provider
strings never appear in captured stdout, stderr-equivalent tracing, queue
failure text, or WebSocket errors. Traces to **AC-agent-safety** and
**AC-observability**.

## Tasks

### T1: Strict task identities, payload, and policy fail closed

**Depends on:** none

**Touches:** `services/execution-state/src/types.rs`,
`stores/zbot-conversation/src/messages.rs`, `gateway/src/durable_agent_tasks.rs`,
focused unit tests

**Tests:**

- TDD: table tests reject unknown fields/version/mode, malformed IDs, each
  canonical field boundary (including controls and escaped JSON expansion),
  and the 65,536-byte serialized limit; valid exact boundaries round-trip.
  Covers **AC-task-contract**. `stub: draft (uncompiled)` because
  the Rust task contract does not exist until EXECUTE and Rust is absent from
  the work-loop stub compile table.
- TDD: `root` is accepted, configured IDs reuse the shared `AgentService`
  validator and must resolve, and invalid/reserved/missing agents fail at both
  producer and handler gates. Covers **AC-agent-identity**. `stub: draft
  (uncompiled)` until the gateway task contract exists.
- TDD: policy tests prove client metadata cannot set source/provenance, every
  actor/conversation/session/execution/message/root-agent relationship is exact,
  a continued session is same-vault and same-agent, authorization precedes
  mutation, and missing/look-up/admission/policy errors deny. Covers
  **AC-authorization**. `stub: draft (uncompiled)` for the
  same greenfield Rust contract reason.
- TDD: deterministic-ID and duplicate-equivalence tests prove the same client
  message maps to the same new session/execution/message and a conflicting
  duplicate is rejected. Covers **AC-durable-acceptance**. `stub: draft
  (uncompiled)` for the same greenfield Rust contract reason.
- TDD: `MessageStore::get` returns one exact row or none without changing
  append/replay ordering. Covers **AC-restart-idempotency**. `stub: draft
  (uncompiled)` because the trait method is introduced during EXECUTE.

**Approach:**

- Add caller-ID constructors/validators for exact queued sessions and root
  executions without relaxing ordinary random-ID constructors.
- Add the keyed message read and the gateway task value types, bounds, policy,
  redacted Debug, and enqueue receipt verification.

**Done when:** focused contract/policy/store tests pass with no runtime call.

### T2: Handler bootstraps once and resumes an exact durable prompt

**Depends on:** T1

**Touches:** `gateway/src/durable_agent_tasks.rs`,
`gateway/src/services/runtime.rs`, `gateway/gateway-execution/src/runner/*`,
focused handler/integration tests

**Tests:**

- TDD: a first claim creates/verifies exact state, appends one deterministic
  root prompt through normal bootstrap, monitors the session, and completes
  only after terminal completion/cancellation. Covers **AC-initial-dispatch**
  and **AC-terminal-ownership**. `stub: draft (uncompiled)` until T1 creates
  the task contract and runtime test port.
- TDD: crash-window cases before append retry bootstrap; cases after append
  compare the row and resume initial bootstrap without a second append or the
  continuation path; running state is monitored without relaunch. Covers
  **AC-restart-idempotency**. `stub: draft (uncompiled)` until T1 creates the
  task contract and runtime test port.
- TDD: mismatched role/session/execution/agent/content permanently returns
  `integrity_violation` without model/tool calls; transient runtime errors
  retry, invalid/auth failures remain permanent, and no raw error escapes.
  Covers **AC-authorization**, **AC-failure-policy**, **AC-observability**.
  `stub: draft (uncompiled)` until T1 creates the handler surface.
- TDD: command fields cannot alter runtime tool/provider/prompt/credential/
  path/network/memory settings. Covers **AC-agent-safety**. `stub: draft
  (uncompiled)` until T1 creates the handler surface.

**Approach:**

- Implement `AgentTaskHandler` over a narrow `AgentTaskRuntime` port.
- Factor the current invoke bootstrap so the production adapter either appends
  a new root message or validates/reuses the exact existing message, then runs
  one shared finish-setup/stream path preserving hooks, mode, intent, history,
  and tool gates. Monitor session state to terminal; do not reuse the
  post-delegation continuation implementation.

**Done when:** real SQLite handler integration tests prove one prompt and the
recording runtime proves bootstrap-vs-resume selection.

### T3: Research acknowledges only persisted tasks without protocol drift

**Depends on:** T1, T2

**Touches:** `gateway/src/websocket/handler.rs`, gateway state/services,
WebSocket and Research integration tests

**Tests:**

- TDD: the connection subscribes to the reserved session before enqueue/wake,
  and `invoke_accepted` is sent only after insert or exact duplicate
  verification. Covers **AC-durable-acceptance**, **AC-live-compatibility**.
  `stub: draft (uncompiled)` until the task-service injection surface exists.
- TDD: pre-subscription failure sends normalized `invocation_failed`, persists
  no work, sends no wake, and invokes no runtime. Covers
  **AC-live-compatibility**, **AC-failure-policy**. `stub: draft (uncompiled)`
  until the task-service injection surface exists.
- TDD: new and continued Research requests preserve normalized mode, agent,
  conversation, session, and valid client-message identity behavior; invalid
  or missing message IDs are minted server-side; non-Research invocation
  remains on the direct path. Covers **AC-live-compatibility**, **AC-scope**.
  `stub: draft (uncompiled)` until the task-service injection surface exists.
- TDD: enqueue, duplicate-conflict, and store failures return only normalized
  `invocation_failed` and do not invoke runtime. Covers **AC-failure-policy**,
  **AC-observability**. `stub: draft (uncompiled)` until the task-service
  injection surface exists.

**Approach:**

- Inject the task service into WebSocket handling.
- Split only the normalized Research branch to pre-subscribe, enqueue, and send
  the unchanged acceptance event; retain the current direct branch for all
  other modes.

**Done when:** protocol snapshots are byte-shape compatible and gateway tests
prove durable-before-accepted ordering.

### T4: The existing gateway worker owns one production handler and recovers it

**Depends on:** T2, T3

**Touches:** `gateway/src/{server.rs,state/mod.rs,durable_agent_tasks.rs}`,
gateway lifecycle/restart tests

**Tests:**

- TDD: server start registers exactly `(zbot.local, agent.task.v1)` in the one
  existing worker and shutdown drains it. Covers **AC-production-wiring**.
  `stub: draft (uncompiled)` until the production handler exists.
- TDD: persisted eligible work is claimed after restart without a wake; an
  expired post-message lease resumes initial bootstrap, yields one user prompt,
  and remains owned until terminal state. Covers **AC-restart-idempotency**,
  **AC-terminal-ownership**, **AC-production-wiring**. `stub: draft
  (uncompiled)` until the production handler exists.
- Goal-based: dependency manifests, public protocol/UI files, and transport
  implementations show no broker, new crate, public schema, or unrelated
  producer. Covers **AC-scope**. `no stub (goal-based)`.

**Approach:**

- Construct the handler from existing AppState runtime/message/state handles
  and replace the empty registry with the exact one-handler registry.
- Keep worker ownership, target, transport, polling, and shutdown unchanged.

**Done when:** gateway lifecycle and restart integration tests pass without an
external service.

### T5: Shipped docs and repository gates match durable Research handoff

**Depends on:** T1-T4

**Touches:** `docs/specs/durable-generic-agent-tasks/{spec.md,plan.md}`,
`docs/specs/README.md`, living architecture/product docs only if behavior is
already documented there

**Tests:**

- Goal-based: focused tests, `cargo fmt --all -- --check`,
  `cargo check --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`,
  and `cargo test --workspace` pass. Covers all criteria. `no stub
  (goal-based)`.
- Manual: submit a Research task, restart after acceptance, reconnect, and
  observe one prompt and one terminal answer in the same session. Covers
  **AC-live-compatibility**, **AC-restart-idempotency**. `no stub (manual QA)`.
- Review: adversarial, security, and quality passes report clean, including
  spec/code drift, prompt/tool authority, retry amplification, redaction, and
  restart observability. Covers all criteria. `no stub (review gate)`.

**Approach:**

- Mark criteria only from evidence, update the spec/plan if implementation
  differs, and record the manual restart result.
- Update the specs index and only the living docs whose current behavior
  changes.

**Done when:** every criterion is checked, every required gate is green, and
all review passes are clean.

## Rollout

- **Delivery:** ships as the Research path on daemon start; rollback is the
  code revert to direct invocation. Existing queue/session/message rows remain
  valid and require no down migration.
- **Infrastructure:** reuses the conversation SQLite database, current worker,
  and local wake transport; no process, port, secret, broker, or dependency.
- **External-system integration:** none beyond the configured LLM/tool runtime
  Research already uses.
- **Deployment sequencing:** handler registration and producer migration ship
  atomically in one daemon build. The handler is available when the worker
  starts before the HTTP/WebSocket server accepts traffic.

## Risks

- A crash between message persistence and queue completion can duplicate agent
  side effects unless replay selects initial-bootstrap resume before any new
  append and monitors an exact live execution; the deterministic message lookup
  and restart integration test guard this.
- A long-running Research task holds one queue lease. The handler's inner
  execution deadline must settle/mark the session before the worker's outer
  timeout so a timed-out runtime is not relaunched concurrently.
- Pre-subscribing a reserved session that later fails enqueue can leave a
  harmless connection-local subscription until disconnect; it must not create
  session/runtime state or emit acceptance.
- Continued sessions reuse one root execution. Exact agent/session checks must
  preserve that existing contract without letting a task cross session
  identity boundaries.
- The 65,536-byte envelope cap is smaller than some model context windows;
  oversize Research requests fail predictably rather than bypassing durability.

## Changelog

- 2026-08-05: initial plan; selected deterministic message-ledger replay to
  preserve the current WebSocket contract without a broker or result table.
- 2026-08-05: retained queue ownership to terminal session state and replaced
  continuation replay with a narrow initial-bootstrap resume after design
  review found post-acceptance and prompt-shape crash gaps.
