# Plan: Durable Queue Worker Runtime

- **Spec:** [`spec.md`](spec.md)
- **Status:** Done

> **Plan contract:** this is the implementation strategy. Unlike the spec, this
> document is allowed to change as implementation reveals new facts.

## Approach

Extend `gateway-bus` with a handler registry and a supervised Tokio worker over
the existing `WorkStore` contract. The worker performs synchronous SQLite calls
through `spawn_blocking`; an execution-state cancellation token sets claim
cancellation intent immediately, interrupts the active SQLite query, and uses
deadline-bounded settlement to fence the final lease commit before successful
shutdown completion. The worker claims only its configured target and runs
each authorized handler in an isolated abort-on-drop task under a deadline
while a sibling branch renews the lease. Wire one worker into `GatewayServer`
using the same `DatabaseManager` as execution state with an empty production
registry. Stop claims on shutdown, drain within a bound, then abort without
writing through an expired lease. Existing producers and execution paths remain
untouched.

## Constraints

- Follow the shipped durable-queue authority, bounds, fencing, retry,
  redaction, and broker-neutrality contract.
- Keep Pattern 4 peer messaging and cross-daemon federation as a northstar,
  not an implementation dependency.
- Reuse Tokio, async-trait, Chrono, tracing, `WorkStore`, and
  `DatabaseManager`; add no third-party crate or public protocol.
- Typed payload validation and handler authorization are required and
  default-deny; the worker never derives authority from target, kind, source,
  or payload alone.

## Construction tests

**Integration tests:** run the real SQLite store with local wake notifications
through worker success, retry, timeout, panic, lease renewal, store-error
backoff, restart polling, and graceful shutdown. A gateway lifecycle test
observes that start creates one worker with an empty registry and shutdown joins
it within the configured bound.

**Manual verification:** none; this slice exposes no user-invokable surface.

**Execution assumptions:**

- Expected touches: `gateway/gateway-bus/src/{lib.rs,work.rs,worker.rs}`,
  `gateway/gateway-bus/tests/durable_work_worker.rs`, `gateway/src/{server.rs,state/mod.rs}`,
  `services/execution-state/src/work.rs`, focused gateway lifecycle tests, and
  this spec's documentation.
- Done is demonstrated by deterministic worker integration tests, real SQLite
  fencing/recovery tests, gateway start/shutdown coverage, formatting, Clippy,
  workspace check, and workspace tests.
- Not changing: queue schema, current producers, execution/delegation/
  continuation paths, public APIs, UI, settings, broker adapters, or deployment.

**Declined temptations:**

- Generic agent-task handler — declined because model/tool authorization and
  side-effect idempotency need their own behavioral spec.
- Config-file knobs — declined because unused public configuration would freeze
  limits before an operator-facing workflow exists.
- Multi-target scheduler — declined because one local target and exact registry
  dispatch prove the lifecycle without speculative federation structure.
- New cancellation/metrics dependency — declined because Tokio task control and
  tracing already cover this internal slice.

**Resolve-vs-surface record:** no unresolved domain claim; handler side effects,
remote identity, public configuration, flow migration, and replay operations are
explicitly outside the authorized scope and must surface rather than expand.

## Design (LLD)

### Design decisions

The worker lives in `gateway-bus` beside the queue coordinator because it owns
transport wake consumption and store claims but no agent execution policy.
`GatewayServer` owns its task handle so lifecycle is explicit. Store calls use
`spawn_blocking`; handler futures remain asynchronous and isolated. Traces to
**AC-worker-loop**, **AC-gateway-wiring**, and **AC-scope**.

### Data & schema

No schema changes occur. The worker consumes the existing version-1 envelope,
lease token, attempt count, and status fields. Its validated configuration is
process memory only. Traces to **AC-bounded-execution**, **AC-lease-safety**,
and **AC-scope**.

### Interfaces & contracts

`WorkHandler` declares exact target/kind, required typed payload validation,
required synchronous authorization, and an async handle operation returning a
closed success/retry/permanent outcome. The worker invokes those gates in that
order. `WorkHandlerRegistry` rejects duplicate keys. `DurableWorkWorker`
accepts `Arc<dyn WorkStore>`, `Arc<dyn WorkWake>`, the registry, and validated
configuration; `WorkWorkerHandle` exposes bounded shutdown with a closed
settlement timeout/failure result instead of presenting an incomplete claim
fence as success. These are internal Rust interfaces and introduce no
repo-level protocol contract. Traces to
**AC-handler-boundary**, **AC-outcomes**, and **AC-scope**.

### Failure, edge cases & resilience

Wake signals coalesce safely because polling remains authoritative. Unknown,
malformed, or unauthorized work is permanently failed through the current
lease. Timeout and panic are normalized retryable failures. Recovery/claim
errors pause the iteration for the bounded poll interval; renewal or blocking
task failure cancels local progress and performs no terminal write; completion
or failure write errors leave the lease for fenced recovery. Shutdown closes
claim intake, drains, and then aborts so lease expiry—not an unfenced cleanup
write—recovers unfinished work; claim-settlement timeout is returned explicitly
and logged by the gateway lifecycle. Traces to **AC-worker-loop**,
**AC-lease-safety**, **AC-outcomes**, **AC-store-fail-closed**, and
**AC-shutdown**.

### Quality attributes (NFRs)

The registry is immutable after worker start, concurrency is semaphore-bounded,
every duration is validated against the spec table, blocking DB operations do
not occupy Tokio executor threads, and tracing omits payload/raw errors. Traces
to **AC-bounded-execution** and **AC-observability**.

## Tasks

### T1: Handler contracts and worker limits fail closed

**Depends on:** none

**Touches:** `gateway/gateway-bus/src/{lib.rs,worker.rs}`,
`gateway/gateway-bus/tests/durable_work_worker.rs`

**Tests:**

- TDD: `ac_handler_registry_is_exact_typed_and_default_deny` proves duplicate
  keys, unknown kinds, malformed typed payloads, and authorization rejection
  cannot reach handler logic; validation precedes authorization. Covers
  **AC-handler-boundary**. `stub: draft (uncompiled)` because the Rust contract
  types intentionally do not exist until EXECUTE and Rust is absent from the
  PLAN compile table; this exact test is the first red artifact.
- TDD: `ac_worker_limits_are_bounded` rejects every table boundary violation
  and accepts exact limits. Covers **AC-bounded-execution**.
  `stub: draft (uncompiled)` for the same greenfield Rust contract reason.

**Approach:**

- Add closed handler outcome/error types, required typed payload validation and
  authorization, exact-key registry, validated worker configuration, and
  redacted Debug implementations.

**Done when:** registry and boundary tests pass without a running worker.

### T2: Supervised processing preserves leases and isolates failures

**Depends on:** T1

**Touches:** `gateway/gateway-bus/src/{work.rs,worker.rs}`,
`gateway/gateway-bus/tests/durable_work_worker.rs`,
`services/execution-state/src/work.rs`

**Tests:**

- TDD: `ac_worker_wakes_polls_recovers_and_bounds_concurrency` uses the real
  SQLite store and a deliberately long poll interval to prove notification
  wake occurs before that deadline, plus missed-wake polling, restart lease
  recovery, authoritative claims, and the concurrency ceiling. Covers
  **AC-worker-loop** and **AC-bounded-execution**.
  `stub: draft (uncompiled)` until T1 creates the worker contract.
- TDD: `ac_worker_renews_and_rejects_stale_authority` holds a handler across a
  heartbeat, proves renewal, then forces stale authority and proves no terminal
  transition. Covers **AC-lease-safety**.
  `stub: draft (uncompiled)` until T1 creates the worker contract.
- TDD: `ac_worker_maps_success_retry_permanent_timeout_and_panic` proves every
  closed outcome and that later work survives one panic. Covers
  **AC-outcomes**. `stub: draft (uncompiled)` until T1 creates the contract.
- TDD: `ac_worker_store_errors_back_off_and_fail_closed` injects recovery,
  claim, renewal, completion, failure, and blocking-task errors; it proves no
  tight retry, no post-renewal-loss transition, normalized diagnostics, and
  continued later polling. Covers **AC-store-fail-closed**.
  `stub: draft (uncompiled)` until T1 creates the worker contract.
- TDD: `ac_worker_shutdown_drains_then_leaves_aborted_lease_recoverable` proves
  bounded drain, immediate cancellation under a saturated blocking pool, no
  late lease commit after bounded settlement/shutdown return, abort-on-drop
  handler ownership, and no fabricated terminal write. Covers **AC-shutdown**
  and **AC-lease-safety**.
  `stub: draft (uncompiled)` until T1 creates the contract.
- TDD: `ac_worker_diagnostics_are_bounded_and_payload_free` captures every
  worker transition and rejects payload/raw-error leakage. Covers
  **AC-observability**. `stub: draft (uncompiled)` until T1 creates the contract.

**Approach:**

- Add the wake port to `LocalWorkTransport`, the poll/claim loop, blocking-store
  adapter calls, per-item heartbeat/deadline/panic isolation, semaphore bound,
  bounded error backoff, normalized transitions, and bounded shutdown handle.
- Extend `WorkStore` with an internal cancellable-claim boundary whose SQLite
  implementation exposes immediate nonblocking interrupt signaling and gates
  the final commit behind deadline-bounded settlement, so shutdown remains
  bounded and a successfully settled claim cannot commit after shutdown
  returns.

**Done when:** focused worker integration tests pass against real SQLite.

### T3: Gateway owns exactly one local worker lifecycle

**Depends on:** T1, T2

**Touches:** `gateway/src/{state/mod.rs,server.rs}`,
`gateway/src/server.rs` tests

**Tests:**

- TDD: `gateway_start_and_shutdown_own_one_durable_worker` constructs a real
  temporary gateway, observes one empty-registry worker start, and verifies
  shutdown joins it. Covers **AC-gateway-wiring**. `stub: draft (uncompiled)`
  until the worker handle exists.

**Approach:**

- Reuse the AppState `DatabaseManager` to expose an `Arc<dyn WorkStore>` and
  shared local wake transport.
- Start an empty-registry worker after the gateway shutdown channel exists and
  await its bounded shutdown before returning. Test-only handlers exercise
  dispatch in `gateway-bus` without adding a production capability.

**Done when:** the gateway lifecycle integration test observes a clean worker
start and stop without an external producer.

### T4: Shipped documentation and workspace gates match the hardened runtime

**Depends on:** T1, T2, T3

**Touches:** `docs/specs/durable-queue-worker-runtime/{spec.md,plan.md}`,
`docs/specs/README.md`

**Tests:**

- Goal-based: `cargo fmt --all -- --check`, focused worker/gateway tests,
  `cargo check --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`,
  and `cargo test --workspace` pass. Covers all criteria. `no stub (goal-based)`.
- Goal-based: spec-status lint reports every criterion checked, references
  resolve, and no public contract is claimed. Covers **AC-scope**.
  `no stub (goal-based)`.
- Goal-based: compare the final diff with `origin/develop` and fail if it
  touches `contracts/`, dependency manifests/lockfiles, existing producer or
  execution-flow modules (`gateway-execution`, `gateway-cron`,
  `gateway-connectors`), public HTTP/WebSocket/CLI/UI surfaces, or queue schema.
  Covers **AC-scope**. `no stub (goal-based)`.

**Approach:**

- Record exact verification evidence, set shipped statuses only after clean
  reviews, and add the feature to the spec index as the single canonical home.

**Done when:** all mechanical gates and warranted reviews are clean.

## Rollout

The worker starts inside every daemon after the gateway shutdown channel is
created. With no migrated producer or production handler, deployment activates
only bounded polling and lifecycle supervision. Rollback is a code rollback:
pending rows remain in SQLite and no schema or data reversal is required. A producer or
remote transport ships only after its own consumer-first compatibility spec.

## Risks

- A handler may perform a side effect immediately before cancellation or lease
  loss; handler authorization and idempotency remain mandatory, and this slice
  registers no production handler.
- Synchronous SQLite work may starve the async runtime; every store operation is
  isolated through Tokio blocking tasks and covered by concurrency tests.
- Shutdown may falsely mark unfinished work; the worker writes no terminal
  state after abort and relies on lease recovery.
- Polling may create idle load; the bounded interval and notification fast path
  keep it predictable without adding configuration or a broker.

## Changelog

- 2026-08-04: initial hardened local worker plan; real agent-task execution and
  existing-flow migration remain separate slices.
