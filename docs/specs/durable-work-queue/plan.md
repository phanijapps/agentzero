# Plan: Durable Work Queue

- **Spec:** [`spec.md`](spec.md)
- **Status:** Done

> **Plan contract:** this is the implementation strategy. Unlike the spec, this
> document is allowed to change as implementation reveals new facts.

## Approach

Add an additive v26 table to `conversations.db`, then introduce queue-owned
domain types and a `WorkStore` persistence port in `execution-state`. Implement
the SQLite repository with transactional claims and lease-token-fenced updates.
Place the independent async `WorkTransport` port and an in-process wake-only
adapter in `gateway-bus`, where a small `DurableWorkQueue` coordinator persists
before publishing. The coordinator exposes enqueue and store access but starts
no worker and migrates no current execution path in this slice.

## Constraints

- Reuse `zbot-runtime-sqlite::DatabaseManager`, its startup migration pipeline,
  `StateDbProvider`, rusqlite transactions, Tokio, serde, chrono, and uuid.
- Keep the durable source of truth in `conversations.db`; transport hints never
  authorize a transition and never substitute for a store claim.
- Every mutation uses bound SQL parameters and a conditional state/lease-token
  predicate. A host `WorkPolicy` attaches provenance, allowlists kind/target,
  validates kind-scoped payloads, and enforces envelope/diagnostic bounds before
  writes; routing labels never confer execution authority.
- Preserve current gateway events, bridge outbox, execution-state APIs, and all
  current session/continuation/delegation behavior.
- Preserve Agent Handoff Notes as one-way current-session steering and keep the
  Pattern 4 peer-message/reply/federation northstar unimplemented.

## Construction tests

**Integration tests:** initialize a version-25 fixture through the real runtime
SQLite schema path, then exercise the queue through the real SQLite repository
and local/failing transport implementations.

**Manual verification:** none; this slice intentionally has no user-invokable
surface. `cargo test -p execution-state -p zbot-runtime-sqlite -p gateway-bus`
and workspace checks exercise the built Rust artifacts.

## Execution assumptions

- **Expected touches:** `services/execution-state/src/{lib.rs,work.rs}`,
  `stores/zbot-runtime-sqlite/src/schema.rs`,
  `gateway/gateway-bus/{Cargo.toml,src/lib.rs,src/work.rs}`, focused tests, and
  this spec's documentation/README entry.
- **Done is demonstrated by:** real-SQLite migration, dedupe, concurrent claim,
  lease fencing/recovery, capped retry/dead-letter, bounds, and transport-failure
  tests plus formatting, Clippy, focused crate tests, and workspace check.
- **Not changing:** current execution dispatch, continuation watcher, bridge
  worker protocol/outbox, public APIs/contracts, UI, broker processes, or
  deployment configuration.

**Declined temptations:**

- Generalize the existing bridge outbox — declined because it is an outbound
  connector delivery contract with ACK semantics, not executable agent work.
- Route continuations through the new queue — declined because consumer
  idempotency and execution acknowledgement require a separate behavioral spec.
- Add configurable broker selection — declined because no broker adapter exists
  in V1 and speculative configuration would create an unused public surface.
- Promise exactly-once execution — declined because process failure between an
  external side effect and acknowledgement makes at-least-once the honest base.

**Resolve-vs-surface record:** no unresolved product or technical decisions;
MQTT/Kafka selection, remote trust/authentication, existing-flow migration, and
dead-letter operations are explicitly outside this spec.

## Design (LLD)

### Design decisions

`WorkStore` owns durable correctness; `WorkTransport` only hints that a bounded,
versioned envelope exists. `DurableWorkQueue` orders the dual write as
persist-first then best-effort publish, so a failed notification leaves work
discoverable through polling. At-least-once delivery plus stable IDs and
dedupe keys replaces an impossible exactly-once promise. Traces to
**AC-enqueue-dedupe**, **AC-transport-port**, and **AC-scope**.

### Data & schema

`durable_work_items` stores envelope identity/version, host provenance, kind,
source, target, JSON payload, correlation/dedupe metadata, priority,
status, attempt budget, availability, fenced lease fields, normalized failure
metadata, and timestamps. A partial unique index enforces
`(source, dedupe_key)` when present; a claim index orders pending work by target
and the spec's total claim order. Status values are `pending`, `leased`, `completed`, and
`dead_letter`. Traces to **AC-schema**, **AC-envelope-bounds**,
**AC-enqueue-dedupe**, and **AC-atomic-claim**.

### Interfaces & contracts

`WorkPolicy` turns a producer draft into a validated envelope only after
kind/target allowlisting and kind-scoped payload validation, attaching source
and caller provenance from trusted host context rather than producer fields.
`WorkStore` provides insert-or-existing, get, claim, renew, complete, fail, and
expired-lease recovery operations. `WorkTransport::publish(&WorkEnvelope)` is
an internal async Rust port; `LocalWorkTransport` converts publish to a Tokio
notification and exposes a waiting primitive for local consumers. There is no
published protocol contract in V1. Traces to **AC-envelope-authority**,
**AC-fenced-transitions**, **AC-recovery**, and **AC-transport-port**.

### Component / module decomposition

`execution-state::work` owns drafts, provenance, validated domain values, store
result types, retry policy, the `WorkPolicy`/`WorkStore` traits, and the SQLite
repository over `StateDbProvider`.
`zbot-runtime-sqlite::schema` owns table creation and migration. `gateway-bus::work`
owns the transport port, local wake adapter, and persistence-before-notification
coordinator. Traces to all criteria without creating a new crate or package.

### State & control flow

Enqueue validates and inserts, then publishes only for a newly inserted item.
A consumer polls/awakens, recovers expired leases, and atomically claims one due
item for a target. The claim returns an opaque token. Renew, complete, or fail
passes `now` and requires matching owner/token plus an unexpired deadline; zero
updated rows means the lease is stale, including after expiry but before any
reassignment. Retry returns the item to pending with capped backoff, while
permanent/exhausted failure enters dead-letter. Claim follows the spec's total
priority/availability/creation/ID order. Traces to **AC-enqueue-dedupe**, **AC-atomic-claim**,
**AC-fenced-transitions**, **AC-recovery**, and **AC-bounded-retry**.

### Behavior & rules

All validation uses the canonical limit table in `spec.md`; this plan defines
no duplicate numeric limits. Failures retain only the normalized code allowed
by that table, and raw internal error strings never cross the queue boundary.
Traces to **AC-envelope-bounds**, **AC-envelope-authority**,
**AC-lease-bounds**, **AC-bounded-retry**, and **AC-error-redaction**.

### Failure, edge cases & resilience

Publish errors are normalized to a bounded code and returned as enqueue
metadata without deleting the row or logging raw details. Empty polling
recovers missed notifications. Lease expiry permits replay;
token fencing prevents a stale owner from completing or rescheduling reassigned
work. Exhausted leases are recovered directly to dead-letter. Invalid stored
status or JSON fails closed as a repository error and is never dispatched.
Traces to **AC-recovery**, **AC-fenced-transitions**, **AC-transport-port**, and
**AC-observability**, and **AC-error-redaction**.

### Quality attributes (NFRs)

All claim/state transitions are one SQLite transaction or one conditional
update. Focused tracing tests cover claim, retry, completion, expired-lease
recovery, and dead-letter transitions. Logs carry work ID, kind, target,
attempt, transition, and reason code but not payload/correlation content. Bounds cap storage and retry amplification.
No new process or dependency changes local availability. Traces to
**AC-atomic-claim**, **AC-bounded-retry**, and **AC-observability**.

### Dependencies & integration

The dependency direction stays bottom-up: runtime SQLite supplies schema and
`StateDbProvider`; execution-state supplies queue domain/store behavior;
gateway-bus supplies transport coordination. Existing gateway execution and
bridge crates do not depend on the new queue in V1. Traces to **AC-scope**.

## Tasks

### T1: Durable work schema and bounded envelopes survive migration

**Depends on:** none

**Touches:** `stores/zbot-runtime-sqlite/src/schema.rs`,
`services/execution-state/src/{lib.rs,work.rs}`

**Tests:**

- TDD: fresh and v25 databases initialize v26 tables/indexes, rerun cleanly,
  and preserve an existing session row. Covers **AC-schema**.
- TDD: envelope construction rejects every empty, oversize, unsupported-version,
  and invalid-attempt boundary and accepts exact limits. Covers
  **AC-envelope-bounds**.
- TDD: a deny-by-default test policy rejects unknown kinds/targets and malformed
  kind payloads, while an allowed draft receives host provenance; routing data
  alone cannot construct a persistable envelope. Covers
  **AC-envelope-authority**.

**Stub:** `stores/zbot-runtime-sqlite/src/schema.rs::tests::v26_durable_work_migration_preserves_v25_data`
and `services/execution-state/tests/durable_work_queue.rs::{ac_envelope_bounds,ac_envelope_authority}` —
`stub: draft (uncompiled)`. Rust is absent from the skill's PLAN compile table,
and the reviewed public types intentionally do not exist before human approval;
the first EXECUTE red step materializes these exact functions before production
behavior and records their failing output.

**Approach:**

- Add the v26 additive migration and identical fresh-schema definition.
- Add validated envelope/status/lease/retry types without broker-specific data.

**Done when:** migration and envelope-boundary tests pass against real SQLite.

### T2: SQLite work store enforces dedupe, atomic claims, and fenced recovery

**Depends on:** T1

**Touches:** `services/execution-state/src/work.rs`,
`stores/zbot-runtime-sqlite/tests/durable_work_store.rs`

**Tests:**

- TDD: duplicate `(source, dedupe_key)` returns the original row while distinct
  sources/keys insert independently. Covers **AC-enqueue-dedupe**.
- TDD: two concurrent claimers obtain one item total, and eligible items follow
  the exact contract order `priority DESC, available_at ASC, created_at ASC,
  id ASC`, including creation and ID tie-breaks. Covers **AC-atomic-claim**.
- TDD: stale tokens cannot renew, complete, retry, or dead-letter; the current
  token can. Covers **AC-fenced-transitions**.
- TDD: claim and renew reject values below/above the canonical lease-duration
  limits and invalid-time inputs without writing a lease; exact limits work.
  Covers **AC-lease-bounds**.
- TDD: expiry requeues within budget, exhaustion dead-letters, and retry delay
  follows the canonical retry policy. Covers **AC-recovery** and
  **AC-bounded-retry**.
- TDD: every claim, retry, complete, recovery, and dead-letter transition emits
  identifier/state/reason fields and no payload/correlation data. Covers
  **AC-observability**.

**Stub:** `stores/zbot-runtime-sqlite/tests/durable_work_store.rs::{ac_enqueue_dedupe,ac_atomic_claim_total_order,ac_fenced_transitions_reject_expired_owner,ac_lease_bounds,ac_recovery_and_bounded_retry,ac_store_transition_observability}` —
`stub: draft (uncompiled)` for the same greenfield Rust contract reason named in
T1; these exact tests are the first T2 red artifacts.

**Approach:**

- Implement `SqliteWorkStore<D>` with immediate transactions for dedupe/claim
  and conditional updates for every lease-owned mutation.
- Keep caller-provided time/lease duration in repository inputs so tests remain
  deterministic; production helpers use UTC.

**Done when:** focused repository tests prove all state transitions and fences.

### T3: Broker-neutral coordinator persists before local notification

**Depends on:** T1, T2

**Touches:** `gateway/gateway-bus/{Cargo.toml,src/lib.rs,src/work.rs}`

**Tests:**

- TDD: a newly inserted envelope wakes a local waiter; a deduplicated enqueue
  emits no second notification. Covers **AC-enqueue-dedupe** and
  **AC-transport-port**.
- TDD: a transport that returns failure still leaves one claimable persisted
  item and exposes the notification failure without treating enqueue as failed.
  Covers **AC-transport-port** and **AC-recovery**.
- TDD: diagnostics capture identifiers/state/reason but debug values never
  contain payload text. Malicious transport/repository errors containing SQL,
  paths, and secret-shaped values yield only normalized codes in logs and the
  enqueue receipt. Covers **AC-observability** and **AC-error-redaction**.

**Stub:** `gateway/gateway-bus/tests/durable_work_queue.rs::{ac_local_transport_wakes_once,ac_transport_failure_preserves_work,ac_transport_errors_are_normalized}` —
`stub: draft (uncompiled)` until the approved internal Rust port exists; these
exact tests are the first T3 red artifacts.

**Approach:**

- Define the async `WorkTransport` port and Tokio `LocalWorkTransport` wake
  implementation using existing workspace dependencies.
- Add `DurableWorkQueue` to validate/store first, publish only on insertion,
  and return an enqueue receipt that distinguishes dedupe and notify failure.

**Done when:** gateway-bus tests prove persistence-first behavior and local wake.

### T4: Documentation and mechanical gates describe the shipped internal core

**Depends on:** T1, T2, T3

**Touches:** `docs/specs/durable-work-queue/{spec.md,plan.md}`,
`docs/specs/README.md`

**Tests:**

- Goal-based: focused tests, `cargo fmt --all -- --check`,
  `cargo check --workspace`, and `cargo clippy --all-targets -- -D warnings`
  pass. Covers all acceptance criteria.
- Goal-based: spec-status/traceability lint passes with every criterion linked
  to evidence and no public contract claimed. Covers **AC-scope**.

**Stub:** no stub (goal-based checks).

**Approach:**

- Record exact test/symbol evidence, update the active-spec index, and keep the
  spec as the single source of truth for this slice.

**Done when:** all focused/workspace gates and required reviews are clean.

## Rollout

The additive v26 schema appears at daemon database initialization. No producer
or worker uses it yet, so deployment changes only storage capability. Rollback
is code rollback: the unused additive table remains harmless; no down migration
or destructive cleanup runs. A future consumer migration requires its own spec.

## Risks

- SQLite timestamp formatting or transaction behavior could make expired work
  invisible; deterministic real-database tests cover ordering and expiry.
- A transport abstraction could imply guarantees it does not provide; naming
  publish as advisory and polling the store preserves the authority boundary.
- Generic payloads can leak through logs or errors; bounds and identifier-only
  diagnostics are acceptance criteria and security-review targets.
- The unused core could invite premature integration; explicit scope boundaries
  keep existing flows unchanged until consumer idempotency is designed.

## Changelog

- 2026-08-04: initial plan for a local durable queue core with a broker-neutral
  transport port and no current execution-flow migration.
- 2026-08-04: implementation completed with full-precision timestamps,
  fail-closed corrupt-row quarantine, per-item redacted diagnostics, and clean
  adversarial, quality, and security reviews.
