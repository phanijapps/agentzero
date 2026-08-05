# Spec: Durable Generic Agent Tasks

- **Status:** Shipped
- **Owner:** @phanijapps
- **Plan:** [`plan.md`](plan.md)
- **Constrained by:** [Durable Work Queue](../durable-work-queue/spec.md); [Durable Queue Worker Runtime](../durable-queue-worker-runtime/spec.md); [Pattern 4 peer-messaging northstar](../../architecture/future-state/2026-05-11-pattern4-peer-messaging-design.md)
- **Brief:** none
- **Discovery:** none
- **Contract:** none
- **Shape:** service

> **Spec contract:** this document defines what "done" means. The implementing
> PR must match this spec, or update it. Verification must be derivable from it.

## Objective

Research requests survive daemon crashes between client acceptance and terminal
agent completion. A Research invocation is persisted as a versioned
`agent.task.v1` work item before zBot acknowledges it, then a local durable-work
handler owns the task until the existing agent runtime reaches a terminal state
with the same session, execution, message, mode, and tool policy that the direct
path uses. Retries resume the initial Research bootstrap from the one durable
root message instead of inserting the prompt twice or entering the
post-delegation continuation path, while the current WebSocket protocol and live
Research experience remain unchanged.

## Boundaries

### Always do

- Persist and authorize the task before sending `invoke_accepted`; reserve
  stable server-generated `sess-*` and `exec-*` identities, preserve a valid
  client `msg-*` identity (otherwise mint one server-side), and use the queue's
  dedupe, attempt, lease, and fencing rules.
- Treat every task payload as untrusted: decode one strict bounded schema,
  authorize trusted envelope provenance before execution, and invoke only the
  existing `RuntimeService` setup path so current prompt boundaries,
  tool allowlists, confirmations, sandboxes, token limits, and memory gates
  remain authoritative.
- Preserve zBot's current single-vault access scope: only a connection admitted
  by the existing WebSocket gateway can enqueue; its server-issued connection
  ID is the task actor; a continued session must exist in the same local vault
  and name the same root agent; and the handler must re-check the persisted
  actor/session/execution/message binding before any effect. This slice does
  not invent multi-user ownership or widen local/LAN admission policy.
- Make dispatch restart-idempotent: zero or one root user message exists for a
  task; a retry with an exact matching persisted message resumes that session;
  any identity or content mismatch fails closed as an integrity violation.
- Keep the work lease active until the session completes or is user-cancelled;
  monitor an exact live execution rather than relaunching it, and retry a
  crashed/paused incomplete execution through initial-bootstrap resume.
- Emit identifier-only structured diagnostics for enqueue, dedupe, dispatch,
  replay, retry, completion, rejection, and integrity failure.

### Ask first

- Route a mode other than Research, a producer other than the local Research
  WebSocket flow, or a delegated/continuation/cron/connector workflow through
  `agent.task.v1`.
- Change the public WebSocket/HTTP protocol, expose queue status or replay to a
  client, or change what tools, wards, credentials, memory, or network access
  the selected agent receives.
- Add a remote target, broker adapter, MQTT/Kafka/NATS process, cross-daemon
  identity, result/reply envelope, or agent-to-agent messaging behavior.

### Never do

- Acknowledge an invocation that was not durably inserted or verified as the
  exact deduplicated task, trust source/provenance fields supplied in client
  metadata, or use payload routing fields as authorization.
- Reinsert a prompt after its deterministic root message is durable, execute a
  mismatched duplicate, default-allow an unknown kind/version/mode, or claim
  exactly-once model/tool side effects.
- Log the prompt, arbitrary metadata, model/tool output, raw deserialization
  errors, credentials, internal paths, or provider diagnostics through stdout,
  stderr, tracing, queue failure text, or WebSocket errors.

## Testing Strategy

- **Task schema, provenance, dedupe, and stable identity:** TDD with table and
  property-style cases because strict decoding, bounds, exact-match policy,
  and duplicate equivalence are compressible invariants.
- **Initial dispatch and crash replay:** TDD through integration tests using a
  real temporary SQLite work/message/state store and a stub runtime boundary;
  this is where zero-or-one prompt persistence and resume-vs-reinsert behavior
  are observable together.
- **Research migration and compatibility:** TDD at the WebSocket/gateway
  boundary, proving subscription ordering, durable-before-accepted ordering,
  unchanged `invoke_accepted`, and normalized enqueue failures.
- **Production lifecycle and restart recovery:** TDD with a temporary gateway
  plus deterministic handlers, proving the registered handler is supervised by
  the existing worker and expired work resumes after restart.
- **Scope and diagnostics:** goal-based diff/log checks for absence of public
  contract, broker, dependency, tool-policy, and payload-leak changes.
- **Stub tally:** AC-task-contract, AC-agent-identity, AC-authorization,
  AC-durable-acceptance, AC-live-compatibility, AC-initial-dispatch,
  AC-restart-idempotency, AC-terminal-ownership, AC-failure-policy,
  AC-agent-safety, AC-observability, and
  AC-production-wiring have Rust TDD stubs recorded as draft/uncompiled in the
  plan because the planned Rust contract types do not exist yet and Rust is
  absent from the work-loop stub compile table; AC-scope uses goal-based checks
  and the user journey uses manual QA, so neither has a stub.

## Acceptance Criteria

- `agent.task.v1` uses these canonical decoded-value limits in addition to the
  queue's existing envelope/provenance limits:

  | Field | Accepted value |
  | --- | --- |
  | `agent_id` | exactly `root`, or an existing configured ID accepted by the shared `AgentService` validator: lowercase kebab-case, at most 64 bytes, non-reserved, no leading/trailing hyphen |
  | `conversation_id` | `1..=128` UTF-8 bytes, no ASCII control characters |
  | `message` | `1..=60,000` UTF-8 bytes |
  | `mode` | exactly `research` |
  | `session_id` | exactly `sess-` plus a canonical UUID |
  | `execution_id` | exactly `exec-` plus a canonical UUID |
  | `message_id` | exactly `msg-` plus a canonical UUID |
  | complete serialized payload | at most `65,536` bytes |

- [x] **AC-task-contract:** `agent.task.v1` accepts only its current version,
  exact local target, `#[serde(deny_unknown_fields)]`, the table's exact bounds,
  and the queue's version-1 envelope; unknown fields, versions, modes,
  malformed/non-canonical IDs, control-bearing identifiers, empty/oversize
  values, invalid UTF-8/JSON, and an oversize serialized payload fail before
  runtime invocation.
- [x] **AC-agent-identity:** before enqueue and again before dispatch,
  `agent_id` is either the special runtime root identity or passes the shared
  `AgentService` validator and resolves to an existing configured agent;
  invalid, reserved, or missing agents fail permanently with normalized output
  and no runtime invocation.
- [x] **AC-authorization:** enqueue provenance is derived from server-owned
  admitted-connection/node context, never client metadata. Before enqueue, the
  service binds the server-issued connection actor to the requested
  conversation and reserved identities; for a continued session it loads the
  same-vault session and verifies its root agent and root execution. Before any
  state mutation, model call, or tool exposure, the handler re-loads and
  verifies exact kind/target/source, node, actor, conversation, session,
  execution, message, and root-agent relationships. Missing objects, mismatches,
  and every lookup/admission/authorization dependency error deny execution.
- [x] **AC-durable-acceptance:** a new Research invocation is durably inserted
  or verified as an exact deduplicated item before the server emits the existing
  `invoke_accepted { session_id, conversation_id }`; a connection-owned watcher
  emits that event only after the exact session, root execution, and root user
  message are queryable, and reports normalized terminal queue failure before
  readiness. Enqueue failure emits the existing normalized `invocation_failed`
  response and invokes no runtime.
- [x] **AC-live-compatibility:** the client is subscribed to the reserved
  session before the persisted task becomes claimable, new and continued
  Research invocations retain their effective Research mode and current agent,
  conversation, session, and client-message semantics, and no public protocol
  or UI type changes. A valid client `msg-<uuid>` remains the durable root
  message ID; an absent or invalid value is replaced by one server-minted
  canonical ID exactly as the current runtime does.
- [x] **AC-initial-dispatch:** an authorized first attempt creates or verifies
  the reserved session and root execution, persists exactly one deterministic
  root user message, starts the existing runtime through its sanctioned
  configuration path, and retains the queue lease after durable bootstrap until
  terminal session state.
- [x] **AC-restart-idempotency:** if a lease is retried after the deterministic
  root message is durable, the handler verifies the stored role, session,
  execution, agent, and content against the task and uses a narrow
  initial-bootstrap resume entry point that skips only append while preserving
  Research hook context, mode, prompt/history shape, intent analysis, and
  runtime/tool gates. It never uses the post-delegation continuation path or
  appends another user message; a mismatch permanently fails with
  `integrity_violation` and invokes neither model nor tools.
- [x] **AC-terminal-ownership:** the handler retains and renews its fenced work
  lease while the session is queued/running, completes work only after the
  session completes or the user cancels it, monitors an already-running exact
  execution without relaunching it, and retries crashed/paused incomplete state
  through initial-bootstrap resume. A daemon restart after `invoke_accepted`
  but before terminal output therefore leaves eligible work to recover.
- [x] **AC-failure-policy:** transient bootstrap/resume unavailability uses the
  queue's capped retry/backoff, invalid or unauthorized tasks fail permanently,
  pre-subscription failure persists no task and sends no wake or acceptance,
  lease loss cancels local authority, exhausted attempts dead-letter, and no
  retry path bypasses handler timeout, fencing, shutdown drain, or runtime
  resource/token limits.
- [x] **AC-agent-safety:** the task cannot select prompts, providers, models,
  tools, MCP servers, connectors, credentials, filesystem paths, network
  destinations, memory destinations, or confirmation policy; those remain the
  existing agent/runtime decisions, and task text remains untrusted user data.
- [x] **AC-observability:** stdout, stderr, tracing logs, persisted queue failure
  codes, and WebSocket errors contain bounded identifiers and normalized reason
  codes but zero prompt, arbitrary metadata, model/tool output, raw parser or
  provider errors, credentials, or internal paths.
- [x] **AC-production-wiring:** `GatewayServer` registers exactly one production
  handler for `(zbot.local, agent.task.v1)` in the existing worker, starts no
  second scheduler, and recovers persisted eligible Research work after daemon
  restart without a broker or third-party dependency.
- [x] **AC-scope:** only the local Research WebSocket producer is migrated; no
  public contract/UI, broker, remote worker, reply/result envelope,
  agent-to-agent message, or other workflow migration is introduced.

## Verification

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`
- Focused gateway tests cover durable-before-accepted ordering, exact dedupe and
  conflict handling, persisted bootstrap resume, terminal ownership, and
  restart polling without a transport wake.
- Live provider-backed crash testing was not run; deterministic SQLite-backed
  lifecycle tests exercise the same acceptance-to-restart boundary without
  spending provider quota.

## Assumptions

- Technical: the SQLite `WorkStore` remains authoritative and transport wakes
  remain best-effort hints (source: `gateway/gateway-bus/src/work.rs`).
- Technical: exact handler lookup, typed payload validation, provenance
  authorization, deadlines, capped retries, lease renewal, and fenced terminal
  writes already exist (source: `gateway/gateway-bus/src/worker.rs`).
- Technical: `InvokeBootstrap` persists the root user message before publishing
  the session or spawning model work, and its finish-setup/stream phases are the
  reusable runtime seam for a narrow initial-bootstrap resume entry point
  (source: `gateway/gateway-execution/src/runner/{invoke_bootstrap.rs,core.rs}`).
- Technical: Research currently returns `invoke_accepted` over WebSocket and
  session-scoped subscriptions carry later events (source:
  `gateway/src/websocket/handler.rs` and
  `gateway/gateway-ws-protocol/src/messages.rs`).
- Technical: zBot currently has one local vault authority rather than
  per-user session ownership; the server-issued WebSocket connection ID is an
  audit/provenance actor, while same-vault and root-agent checks prevent this
  new asynchronous path from widening existing access (source:
  `gateway/src/websocket/{handler.rs,session.rs,subscriptions.rs}` and user
  confirmation of local-first scope 2026-08-05).
- Technical: work payloads are capped at 65,536 serialized bytes (source:
  `services/execution-state/src/work.rs`).
- Process: production LLM/tool handlers and workflow migration require a
  separate approved spec from the shipped worker-runtime slice (source:
  `docs/specs/durable-queue-worker-runtime/spec.md`).
- Product: this slice migrates Research to local `agent.task.v1`, preserves
  public behavior, leaves the messaging infrastructure broker-neutral, and
  excludes remote messaging/replies (source: user confirmation 2026-08-05).
