# Spec: Durable Queue Worker Runtime

- **Status:** Shipped
- **Owner:** @phanijapps
- **Plan:** [`plan.md`](plan.md)
- **Constrained by:** [Durable Work Queue](../durable-work-queue/spec.md);
  [Pattern 4 peer-messaging northstar](../../architecture/future-state/2026-05-11-pattern4-peer-messaging-design.md)
- **Brief:** none
- **Discovery:** none
- **Contract:** none
- **Shape:** service

> **Spec contract:** this document defines what "done" means. The implementing
> PR must match this spec, or update it. Verification must be derivable from it.

## Objective

zBot runs a supervised local worker that turns persisted durable-work rows into
bounded handler executions. The worker reacts to a local notification without
waiting for the next poll interval, polls so missed notifications and restarts cannot strand work, renews active
leases, and converts handler success, rejection, timeout, panic, and shutdown
into fenced queue transitions. One failed or malicious work item cannot stop
the worker, disclose its payload, exceed configured concurrency, or authorize a
side effect from routing fields alone.

## Boundaries

### Always do

- Treat the SQLite `WorkStore` as authoritative and claim every item with a
  fenced lease before inspecting or dispatching its handler.
- Match handlers by exact target and kind, require handler re-authorization of
  trusted provenance before any side effect, and treat payload content as
  untrusted data.
- Bound concurrency, handler runtime, polling, lease renewal, and shutdown
  drain; emit identifier-only structured lifecycle diagnostics.

### Ask first

- Register a handler that invokes an LLM, tool, connector, filesystem, network,
  or other externally visible side effect.
- Route delegation, continuation, cron, connector, session execution, or any
  other existing workflow through the worker.
- Add a public API, user-facing configuration, dead-letter replay operation,
  network transport, or remote worker identity.

### Never do

- Claim exactly-once side effects, complete work without the current unexpired
  owner/token, or let a transport notification replace a store claim.
- Dispatch an unknown target/kind, default-allow handler authorization, log a
  payload/raw handler error, or retry without the queue's capped policy.
- Add a broker, third-party dependency, peer-message/reply type, federation
  path, or generic agent-task execution in this slice.

## Testing Strategy

- **Worker state machine and bounds:** TDD with deterministic fake handlers and
  the real temporary SQLite store, because claim, heartbeat, timeout, retry,
  panic isolation, stale lease, and bounded concurrency are compressible
  invariants.
- **Wake, polling, and shutdown:** TDD with Tokio integration tests, because a
  missed wake, restart recovery, or shutdown race is observable only across the
  worker/store/transport boundary.
- **Gateway lifecycle wiring:** TDD through an integration test of
  `GatewayServer::start`/`shutdown`, because daemon ownership is proven by the
  constructed gateway rather than a mock of its fields.
- **Scope exclusions:** goal-based diff checks, because absence of dependency,
  public-contract, producer, and execution-flow changes is a repository-shape
  property rather than runtime behavior.
- **Coverage tally:** AC-worker-loop, AC-handler-boundary, AC-bounded-execution,
  AC-lease-safety, AC-outcomes, AC-store-fail-closed, AC-shutdown,
  AC-observability, and AC-gateway-wiring use TDD; AC-scope uses goal-based
  checks.

## Acceptance Criteria

The worker's internal limits are canonical here. Implementations may choose
stricter handler-specific limits but cannot exceed these bounds.

| Behavior | Limit |
| --- | --- |
| Concurrent claimed items per worker | `1..=32` |
| Poll interval | `100ms..=60s` |
| Lease duration | inherited queue bound `1s..=300s` |
| Heartbeat interval | at least `100ms` and strictly less than half the lease |
| Handler timeout | `1s..=3600s` |
| Graceful shutdown drain | `1s..=300s` |

- [x] **AC-worker-loop:** the supervised worker recovers expired work before
  claiming, wakes and claims after a local transport notification without
  waiting for the next configured poll deadline, polls after missed
  notifications or restart, and never dispatches an item that was not returned
  by an authoritative store claim for its configured target.
- [x] **AC-handler-boundary:** dispatch requires an exact registered
  `(target, kind)`, successful kind-specific typed payload validation, and a
  successful handler authorization check over trusted envelope provenance—in
  that order; malformed payloads, unknown handlers, and rejected provenance
  enter normalized permanent failure without invoking handler logic or
  exposing raw payload bytes.
- [x] **AC-bounded-execution:** invalid worker limits fail construction, and a
  valid worker never exceeds its configured concurrency or handler deadline.
- [x] **AC-lease-safety:** an active handler renews its lease before the
  deadline; a stale token or failed renewal cancels local handling and cannot
  complete, retry, or dead-letter the item with obsolete authority.
- [x] **AC-outcomes:** handler success completes the current lease; retryable
  rejection, timeout, and panic use normalized capped retry; permanent
  rejection enters dead-letter; one handler failure or panic does not stop
  subsequent eligible work.
- [x] **AC-store-fail-closed:** recovery, claim, renewal, completion, failure,
  and blocking-task errors stop dispatch for the affected item or iteration,
  preserve lease fencing, emit only a normalized reason, and resume only after
  the bounded poll interval; renewal errors cancel local handling and no store
  error is swallowed or retried in a tight loop.
- [x] **AC-shutdown:** shutdown stops new claims, drains in-flight handlers for
  at most the configured bound, and aborts any remainder without fabricating a
  terminal queue transition, leaving its lease recoverable after expiry.
- [x] **AC-observability:** worker start/stop, claim, authorization rejection,
  heartbeat failure, timeout, panic, completion, retry, and dead-letter logs
  contain bounded identifiers, target/kind, attempt, and normalized reason but
  contain zero payload or raw handler-error bytes.
- [x] **AC-gateway-wiring:** `GatewayServer` constructs the worker over the same
  runtime SQLite database, starts it once with no production handlers, and
  awaits bounded worker shutdown before returning; test-only handlers prove
  dispatch without adding a dormant production capability.
- [x] **AC-scope:** no existing producer or execution flow is migrated; no LLM,
  tool, connector, filesystem, or network side effect is executed; no public
  contract, broker, remote transport, dead-letter replay, or third-party
  dependency is introduced.

## Follow-on compatibility

The later A2A federation feature registers exact inbound, outbound-dispatch,
and outbound-poll handlers in this same supervised worker only while A2A is
enabled. Handler timeouts, lease renewal, normalized outcomes, duplicate
registration rejection, and bounded shutdown are unchanged. See
[`A2A Federation and Discovery`](../a2a-federation-discovery/spec.md).

## Assumptions

- Technical: `WorkStore` is authoritative and `LocalWorkTransport` is a
  wake-only hint (source: `gateway/gateway-bus/src/work.rs`).
- Technical: `GatewayServer` owns background startup and graceful shutdown
  (source: `gateway/src/server.rs`).
- Technical: Tokio, async-trait, Chrono, tracing, and the runtime SQLite manager
  are existing workspace dependencies (source: `gateway/Cargo.toml` and
  `gateway/gateway-bus/Cargo.toml`).
- Process: migrating an existing flow, adding a broker, or exposing a public
  surface requires a separate approved spec (source:
  `docs/specs/durable-work-queue/spec.md`).
- Product: this slice hardens local worker infrastructure and defers real
  generic agent-task execution (source: user confirmation 2026-08-04).
- Product: the feature is an internal service with no external contract or UI
  (source: user confirmation 2026-08-04).
