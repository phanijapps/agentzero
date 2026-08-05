# Spec: Durable Peer Messaging

- **Status:** Shipped
- **Owner:** @phanijapps
- **Plan:** [`plan.md`](plan.md)
- **Constrained by:** [Durable Work Queue](../durable-work-queue/spec.md); [Durable Queue Worker Runtime](../durable-queue-worker-runtime/spec.md); [Durable Generic Agent Tasks](../durable-generic-agent-tasks/spec.md); [Pattern 4 peer-messaging northstar](../../architecture/future-state/2026-05-11-pattern4-peer-messaging-design.md)
- **Brief:** none
- **Discovery:** none
- **Contract:** [`contracts/jsonschema/agent-peer-message.schema.json`](../../../contracts/jsonschema/agent-peer-message.schema.json)
- **Shape:** mixed

> **Spec contract:** this document defines what "done" means. The implementing
> PR must match this spec, or update it. Verification must be derivable from it.

## Objective

Let agents running in parallel on one zBot daemon exchange short asynchronous
messages without blocking either execution or losing an accepted message when
the daemon restarts. Root and ward agents can initiate a message to a running
execution in their current session. Any running recipient can answer through a
reply token bound to the received message, and either side can continue the
reply chain. Accepted messages use the existing durable work store and local
worker; a future transport adapter can carry the same versioned payload without
changing agent-facing tools or authorization rules.

## Boundaries

### Always do

- Persist and authorize `agent.peer-message.v1` before returning `queued`; use
  the existing work store as authority and `WorkTransport` only as a wake hint.
- Derive sender agent, session, execution, node, and actor kind from trusted
  runtime context. Re-check sender, target, session membership, actor policy,
  and reply-chain binding before enqueue and again before steering delivery.
- Allow `message_agent` initiation only to root and ward actors. Allow
  `reply_to_agent` to any runtime actor only when its current execution is the
  exact recipient of the referenced durable message; a reply reverses the
  sender/recipient direction and creates a new independently durable message.
- Keep every message within one local session and address agents by canonical
  `exec-*` execution ID. A target must be running when accepted; a terminal or
  cross-session target fails closed without revealing whether an out-of-scope
  execution exists.
- Treat peer content as untrusted data. Inject a bounded, attributed envelope
  as a user/peer steering message with an explicit instruction-vs-data marker,
  stable work ID, sender identity, and reply token; never present peer content
  as system policy or trusted host instructions.
- Preserve at-least-once semantics across queue retries. Re-delivery may occur
  after a crash boundary, so each injection carries the stable work ID and tells
  the recipient to treat duplicate IDs idempotently.
- Use strict deserialization, canonical IDs, bounded UTF-8 payloads, capped
  attempts/backoff inherited from the worker, and identifier-only structured
  diagnostics. Authorization and storage uncertainty always deny or retry;
  neither may fail open.

### Ask first

- Add cross-session or cross-daemon routing, remote identities, role/name based
  addressing, discovery beyond `list_session_agents`, or a broker adapter such
  as MQTT, Kafka, NATS, or a child broker process.
- Add blocking request/reply waits, delivery/read acknowledgements, inbox UI,
  message history APIs, broadcast/fan-out, priority controls, attachments, or
  payloads larger than this contract permits.
- Let ordinary delegated actors initiate arbitrary messages, change their
  existing tool privileges beyond the scoped reply capability, or expose peer
  messages to memory/Engram automatically.
- Change the public WebSocket/HTTP protocol, conversation transcript schema,
  queue schema, work failure vocabulary, or current handoff/steer/wait tools.

### Never do

- Trust model-supplied sender/session/actor fields, use routing labels as
  authorization, accept a reply token without looking up and validating its
  durable original message, or authorize after persistence/delivery.
- Route a message to an unknown, self, terminal, or out-of-session execution;
  widen authority at a delegation boundary; or let a reply token target anyone
  other than the original sender.
- Execute, parse as commands, persist to semantic memory, or interpolate peer
  content into SQL, shell, templates, logs, error strings, or system prompts.
- Log message content, reply content, raw deserialization/storage errors,
  prompts, model/tool output, credentials, internal paths, or provider details.
- Claim exactly-once delivery, exactly-once interpretation, or exactly-once
  downstream tool side effects.

## Testing Strategy

- **Payload, provenance, and policy:** TDD with strict-schema, canonical-ID,
  byte-bound, actor, same-session, self-target, and reply-binding cases.
- **Durability and delivery:** TDD with temporary SQLite state/work stores and
  a real local transport plus steering queue, proving persist-before-queued,
  restart recovery, stable IDs, bounded retry, and terminal failure behavior.
- **Tool exposure:** TDD over actor inventories, proving root/ward initiation,
  scoped reply availability for recipients, trusted execution context, and no
  arbitrary initiation for ordinary delegated actors.
- **Prompt boundary and diagnostics:** TDD on the exact attributed peer
  envelope plus goal-based diff/log checks proving content is data-delimited
  and absent from diagnostics, memory, public APIs, and transcript persistence.
- **Production lifecycle:** TDD at gateway composition, proving one supervised
  worker registers both `agent.task.v1` and `agent.peer-message.v1` exact
  handlers and shuts down cleanly.
- **Manual QA:** run two parallel agents in one Research session, have A send B
  a message while both continue work, have B reply, restart the daemon with a
  pending message, and observe eventual attributed delivery without a blocking
  tool call.
- **Stub tally:** all behavioral criteria are TDD-mode and have Rust stub intent
  recorded in `plan.md`; the stubs are draft/uncompiled because the planned Rust
  contract types do not exist yet and Rust is not listed in the work-loop stub
  compile table. Scope exclusions use goal-based checks and manual QA has no
  construction stub.

## Acceptance Criteria

The JSON payload contract has these exact decoded-value rules in addition to
the existing durable-work envelope limits:

| Field | Accepted value |
| --- | --- |
| `target_execution_id` | `exec-` plus a canonical lowercase UUID |
| `content` | non-empty UTF-8, at most 1,000 Unicode code points and 4,000 bytes, no NUL |
| `reply_to_work_id` | absent for initiation; otherwise `work-` plus a canonical lowercase UUID |
| complete serialized payload | at most 65,536 bytes |

- [x] **AC-contract:** `agent.peer-message.v1` accepts only the exact local
  target, version-1 envelope, strict JSON payload, table bounds, and canonical
  IDs; the schema's `maxLength` enforces the code-point bound while its
  `x-maxUtf8Bytes` and `x-maxSerializedUtf8Bytes` annotations are enforced by
  Rust byte validation and contract tests. Unknown fields, invalid UTF-8/JSON,
  NUL, empty/oversize content, and malformed IDs fail before enqueue or steering.
- [x] **AC-durable-acceptance:** a successful tool result is `queued` and
  includes the durable `message_id` (`work-*`) only after insert-or-equivalent
  succeeds; storage/policy failures return normalized results and produce no
  steering side effect.
- [x] **AC-initiation-authz:** `message_agent` is present only for root and ward
  actors and accepts only a different running execution in the caller's exact
  current session. Missing, cross-session, self, or terminal targets return the
  same normalized non-disclosing rejection and enqueue nothing.
- [x] **AC-reply-authz:** `reply_to_agent` is available to runtime actors only
  when peer messaging is wired, but succeeds only if the caller's trusted
  execution is the exact target of the referenced durable peer message. It
  creates a new durable message aimed only at that message's authenticated
  sender; forged, cross-session, non-peer, malformed, or reversed tokens fail
  closed and enqueue nothing.
- [x] **AC-handler-reauthorization:** before delivery, the exact handler strictly
  decodes the payload and re-checks source, provenance, sender execution,
  session membership, target execution, actor initiation policy or reply-chain
  binding, and target status. Any mismatch is a normalized permanent integrity
  failure with no steering side effect.
- [x] **AC-asynchronous-delivery:** a queued message never blocks its sender.
  The worker injects it before the target's next LLM call as an attributed peer
  data envelope containing stable message ID, sender execution/agent, content,
  and the same message ID as a reply token.
- [x] **AC-prompt-boundary:** the peer envelope explicitly labels content as
  untrusted peer-provided data, delimits it without interpreting it, uses a peer
  steering source rather than system authority, and tells the recipient to
  ignore duplicate message IDs already handled.
- [x] **AC-restart-and-retry:** an accepted pending message survives daemon
  restart and is recovered by the existing worker. A running state with a
  temporarily absent steering handle retries with existing capped backoff; a
  terminal/missing target or authorization mismatch dead-letters permanently.
- [x] **AC-at-least-once:** the same durable message retains one stable work ID
  through recovery/retry; tests and docs explicitly permit duplicate injection
  across the steer/complete crash gap and make no exactly-once side-effect
  claim.
- [x] **AC-production-wiring:** production starts one durable worker with exact
  handlers for both `agent.task.v1` and `agent.peer-message.v1`, rejects duplicate
  registration/startup, and drains/shuts down through the current lifecycle.
- [x] **AC-observability:** enqueue, delivery, retry, rejection, and completion
  diagnostics contain canonical IDs and normalized reason codes only; peer
  content and raw errors do not appear in tracing, work failure text, tool
  errors, public APIs, conversation rows, or memory ingestion.
- [x] **AC-compatibility:** existing `handoff_to_agent`, `steer_agent`,
  `list_session_agents`, `wait_agent`, Research durable tasks, public HTTP/WS
  contracts, transcript behavior, and actor tool policies remain unchanged
  except for the two explicitly described peer tools.
- [x] **AC-open-transport-seam:** agent-facing tools and the JSON payload depend
  on the existing `WorkTransport`/durable-work boundary rather than a broker
  implementation; no broker, child process, network listener, dependency, or
  remote routing is added.

## Assumptions

- Technical: the durable work store is authoritative, supports insert-or-existing
  and restart lease recovery, and the local transport is only a wake hint.
  (source: `services/execution-state/src/work.rs`; `gateway/gateway-bus/src/work.rs`)
- Technical: the worker registry supports multiple exact `(target, kind)`
  handlers under one worker and denies unknown handlers. (source:
  `gateway/gateway-bus/src/worker.rs`)
- Technical: running executors already expose a steering queue immediately
  before each LLM call, but its enqueue/worker-complete boundary cannot be made
  atomic; stable-ID at-least-once delivery is therefore the honest guarantee.
  (source: `runtime/agent-runtime/src/steering.rs`;
  `runtime/agent-runtime/src/executor.rs`)
- Technical: execution state records canonical session, execution, agent,
  parent, delegation type, and status fields needed for same-session policy.
  (source: `services/execution-state/src/types.rs`)
- Product: Phase 1 is same-daemon asynchronous communication: root/ward may
  initiate, any valid recipient may reply, and federation/broker work remains
  open but excluded. (source: user confirmation 2026-08-05)
- Process: no dedicated JSON Schema authoring skill is installed, so the
  contract is authored directly without type-specific rule enforcement.
