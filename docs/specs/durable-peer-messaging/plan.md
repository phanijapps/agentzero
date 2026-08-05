# Plan: Durable Peer Messaging

- **Spec:** [`spec.md`](spec.md)
- **Status:** Drafting

> **Plan contract:** this is the implementation strategy. Unlike the spec, this
> document is allowed to change as you learn. When it changes substantially,
> note why in the changelog at the bottom.

## Approach

Add one versioned peer-message producer/handler module to `gateway-execution`,
backed by the existing `DurableWorkQueue`. Thread a shared producer through the
execution runner so first-party tools can persist messages using trusted tool
context. Register a second exact handler in the gateway's existing worker; it
re-authorizes stored provenance and injects an attributed peer steering message
into the current target. Replies look up the referenced durable message and
reverse only its authenticated endpoints. The riskiest seams are trusted
execution identity propagation and the non-atomic steer/complete boundary, so
those are specified and tested before production wiring.

## Constraints

- Preserve dependency direction and use the existing queue/worker/steering
  abstractions; `gateway-execution` may depend on lower-level `gateway-bus`, but
  no lower crate may depend on `gateway-execution`.
- No persistence migration, public HTTP/WS change, broker dependency, child
  process, remote target, or semantic-memory write.
- Existing Research durable work remains behaviorally unchanged.
- The approved plan becomes immutable during execution; discoveries that
  require a structural change stop for re-approval.

## Construction tests

**Integration tests:** temporary SQLite work/state stores, real local transport,
real steering queue, and deterministic worker runtime cover producer/handler,
retry/recovery, and combined registry wiring.

**Manual verification:** run two parallel agents in a local Research session;
send A→B, continue A, reply B→A, then repeat with a daemon restart while the
message is pending. This session verifies same-daemon asynchronous delivery;
remote routing, read acknowledgements, and exactly-once side effects are
explicitly deferred.

## Design (LLD)

### Design decisions

- `gateway-execution::peer_messaging` owns the strict payload, producer,
  first-party tools, and exact work handler so authorization logic is shared by
  root and delegated execution paths.
- The producer receives `Arc<dyn WorkStore>`, `Arc<dyn WorkTransport>`, and
  `StateService`; it constructs a request-scoped `WorkPolicy` from trusted tool
  context and uses `DurableWorkQueue` to authorize/persist/publish.
- Every executor context gets host-derived `execution_id` alongside existing
  `session_id` and `app:actor_kind`. Tool arguments contain only target/content
  or reply-token/content.
- `message_agent` is capability-gated to root/ward. `reply_to_agent` has a
  separate `AgentReply` capability granted to all actor profiles, but its
  object-level authorization is the referenced durable message.
- Delivery adds `SteeringSource::Peer`; the handler supplies a fixed data
  envelope and never promotes peer content to a system message.

### Data & schema

- `contracts/jsonschema/agent-peer-message.schema.json` defines
  `agent.peer-message.v1` payloads with `additionalProperties: false` and an
  `x-spec` backlink.
- `PeerMessageV1 { target_execution_id, content, reply_to_work_id }` uses
  `#[serde(deny_unknown_fields)]`; Rust validation mirrors the schema's standard
  code-point bound and its explicit UTF-8/serialized-byte annotations.
- The work envelope ID is the public `message_id` and reply token. Replies form
  a chain by referencing the immediate message, not by minting bearer secrets.
  Tokens are identifiers plus server-side object authorization, not secrets.
- Queue defaults provide five attempts and bounded exponential backoff. No new
  table or migration is introduced.

### Interfaces & contracts

- `message_agent(execution_id, message) -> {status, message_id, execution_id}`
  returns `queued` only after durable insert/equivalent verification.
- `reply_to_agent(reply_to, message) -> {status, message_id, execution_id}`
  resolves and authorizes `reply_to` before creating the reverse message.
- Rejections use closed statuses such as `target_not_found`, `not_authorized`,
  `invalid_request`, and `temporarily_unavailable`; no raw storage or parser
  text crosses the tool boundary.
- Handler registration is exact `(zbot.local, agent.peer-message.v1)` and shares
  the one production worker with `(zbot.local, agent.task.v1)`.

### Failure, edge cases & resilience

- Storage/policy/lookup errors fail closed; transient local availability may
  retry but never changes target identity.
- A target that becomes terminal before delivery dead-letters. A running target
  whose handle is briefly absent retries. Missing/cross-session/integrity
  failures are permanent and content-free.
- A crash after steering but before work completion can cause duplicate
  injection. The stable message ID and prompt guidance are the dedupe aid; no
  exactly-once claim is made.
- Content never enters trace fields, error strings, transcripts, or memory.

### Quality attributes (NFRs)

- Security: check-before-effect object authorization at enqueue and delivery;
  least-privilege initiation/reply surfaces; peer content is untrusted data.
- Reliability: durable-before-ack, at-least-once recovery, bounded attempts,
  fail-closed integrity checks, graceful worker shutdown.
- Performance: constant-number indexed state/work lookups per send/delivery;
  4,000-byte messages and existing worker concurrency cap bound memory/cost.
- Maintainability: one payload contract and producer/handler module, no broker
  coupling, and exact tests at each policy boundary.

## Tasks

### T1: Define and persist an authorized peer message

**Depends on:** none

**Touches:** `contracts/jsonschema/agent-peer-message.schema.json`,
`gateway/gateway-execution/Cargo.toml`, `gateway/gateway-execution/src/peer_messaging.rs`,
`gateway/gateway-execution/src/lib.rs`

**Tests:**
- `peer_message_v1_enforces_strict_schema_and_bounds` (AC-contract)
- `enqueue_persists_before_queued_and_redacts_failures` (AC-durable-acceptance, AC-observability)
- `initiation_requires_trusted_root_or_ward_same_session_target` (AC-initiation-authz)
- `reply_reverses_only_an_authorized_durable_message` (AC-reply-authz)
- stub: draft (uncompiled) — planned Rust types do not exist yet and Rust is
  absent from the work-loop stub compile table.

**Approach:**
- Add strict payload validation and a request-scoped policy that attaches only
  host-derived provenance.
- Implement producer methods for initiation and reply lookup/reversal using the
  existing durable queue.
- Return closed result/error enums with redacted `Debug`/display behavior.

**Done when:** producer tests prove strict contract, durable-before-success,
same-session initiation, reply binding, and content-free failures.

### T2: Expose least-privilege peer tools with trusted execution identity

**Depends on:** T1

**Touches:** `gateway/gateway-execution/src/tools/*`,
`gateway/gateway-execution/src/invoke/executor.rs`,
`gateway/gateway-execution/src/runner/*`, `gateway/gateway-execution/src/delegation/spawn.rs`

**Tests:**
- `actor_tool_inventories_gate_initiation_and_allow_scoped_reply` (AC-initiation-authz, AC-reply-authz, AC-compatibility)
- `root_delegated_and_continuation_contexts_carry_execution_id` (AC-handler-reauthorization)
- `peer_tools_never_accept_sender_or_session_arguments` (AC-initiation-authz)
- stub: draft (uncompiled) — planned tool types do not exist yet and Rust is
  absent from the work-loop stub compile table.

**Approach:**
- Add `message_agent` and `reply_to_agent` tools over the shared producer.
- Introduce the narrow reply capability and preserve existing actor policies.
- Inject canonical host execution identity on every root, continuation, and
  delegated executor build path; register tools only when producer wiring exists.

**Done when:** all actor inventories and execution paths expose exactly the
intended tools and trusted identity state.

### T3: Re-authorize and deliver through the existing worker

**Depends on:** T1

**Touches:** `runtime/agent-runtime/src/steering.rs`,
`runtime/agent-runtime/src/steering_registry.rs`,
`gateway/gateway-execution/src/peer_messaging.rs`

**Tests:**
- `handler_reauthorizes_provenance_and_reply_chain_before_steering` (AC-handler-reauthorization)
- `handler_injects_attributed_untrusted_peer_envelope` (AC-asynchronous-delivery, AC-prompt-boundary)
- `running_without_handle_retries_but_terminal_target_is_permanent` (AC-restart-and-retry)
- `delivery_keeps_stable_id_and_documents_duplicate_window` (AC-at-least-once)
- stub: draft (uncompiled) — planned handler/source types do not exist yet and
  Rust is absent from the work-loop stub compile table.

**Approach:**
- Add `SteeringSource::Peer` and a fixed formatter that data-delimits content.
- Implement strict handler validation, synchronous authorization lookup, and
  retry/permanent outcomes without payload-bearing diagnostics.

**Done when:** real steering-queue tests prove exact envelope shape, fail-closed
authorization, and retry/terminal behavior.

### T4: Wire the shared producer and second handler into production

**Depends on:** T2, T3

**Touches:** `gateway/src/state/mod.rs`, `gateway/src/services/runtime.rs`,
`gateway/src/server.rs`, `gateway/src/lib.rs`, `docs/specs/README.md`,
`docs/architecture/components/*`

**Tests:**
- `production_worker_registers_agent_task_and_peer_message_handlers` (AC-production-wiring, AC-compatibility)
- `expired_peer_work_recovers_after_worker_restart` (AC-restart-and-retry)
- `peer_messaging_adds_no_public_or_broker_surface` (AC-open-transport-seam)
- stub: draft (uncompiled) — planned production wiring does not exist yet and
  Rust is absent from the work-loop stub compile table.

**Approach:**
- Build one shared producer from AppState work/state dependencies and thread it
  into RuntimeService/ExecutionRunner.
- Register the peer handler beside the Research task handler in the existing
  worker registry and preserve one lifecycle handle.
- Add component documentation and active-spec index entry; run manual QA at the
  documented same-daemon boundary.

**Done when:** one production worker starts both exact handlers, restart tests
recover pending peer work, and workspace gates plus manual QA pass.

## Rollout

Ship as an additive local capability with no migration or feature flag. Rollback
removes the two tool registrations and peer handler; already-pending peer work
then safely dead-letters as handler-unavailable rather than being misrouted.
Future MQTT/Kafka adapters must implement the existing transport boundary and
preserve this payload/provenance policy.

## Risks

- Duplicate injection is possible across the steering/complete crash gap; the
  contract explicitly exposes a stable ID and limits the guarantee to
  at-least-once.
- Missing execution identity on any build path would deny legitimate replies or
  tempt unsafe inference; targeted root/delegated/continuation tests guard it.
- Tool inventory changes can accidentally widen ordinary subagent authority;
  actor matrix tests pin initiation separately from scoped reply.
- Large AppState/runtime constructor surfaces make wiring errors possible;
  named config fields and production registry tests constrain the change.

## Changelog

- 2026-08-05: replaced scaffold with full Phase-1 design using the existing
  durable queue, scoped reply chains, and an open transport seam.
