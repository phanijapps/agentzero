# Spec: Durable Work Queue

- **Status:** Shipped
- **Owner:** @phanijapps
- **Plan:** [`plan.md`](plan.md)
- **Constrained by:** [Agent Handoff Notes](../agent-handoff-notes/spec.md);
  [Pattern 4 peer-messaging northstar](../../architecture/future-state/2026-05-11-pattern4-peer-messaging-design.md)
- **Brief:** none
- **Discovery:** none
- **Contract:** none
- **Shape:** mixed

> **Spec contract:** this document defines what "done" means. The implementing
> PR must match this spec, or update it. Verification must be derivable from it.

## Objective

zBot provides an internal durable work queue for executable agent work. A work
envelope is persisted before any delivery hint is emitted, concurrent workers
claim it through an expiring fenced lease, and process restarts or missed local
notifications cannot lose eligible work. Retryable failures use bounded backoff
and eventually reach a visible dead-letter state. The persistence contract and
the broker-neutral transport port stay independent so a later MQTT, Kafka, or
other network adapter can be added without changing queue correctness.

## Boundaries

### Always do

- Treat SQLite-backed `WorkStore` state as authoritative; transport delivery is
  an advisory wake-up and workers claim from the store.
- Use versioned, host-constructed envelopes with durable provenance,
  policy-allowlisted kinds/targets, kind-scoped payload validation,
  parameterized SQL, bounded fields, atomic claims, and lease-token fencing on
  every terminal state transition.
- Emit structured lifecycle diagnostics using identifiers and reason codes,
  never the work payload or unbounded raw errors.

### Ask first

- Add a network broker adapter, accept remotely-authored envelopes, or expose a
  public REST, WebSocket, AsyncAPI, CLI, or UI surface.
- Route an existing execution, continuation, cron, connector, or delegation
  workflow through this queue.
- Change retention policy, automatically replay dead-letter work, or add a
  destructive migration or cleanup operation.

### Never do

- Claim exactly-once execution; the queue is at-least-once and consumers remain
  idempotent for a stable work identifier.
- Use the in-memory event bus, transport acknowledgement, or broker state as the
  source of truth for whether work exists or completed.
- Treat `source`, `target`, `kind`, or persisted payload as authorization or as
  trusted model instructions; a future consumer re-authorizes the caller's
  actor/session/tool scope before any side effect and treats payload as data.
- Add MQTT, Kafka, NATS, another broker, or a new third-party dependency in this
  slice; allow unbounded payloads, leases, retries, or retry delays.
- Add `PeerMessageBus`, `ReplyStore`, reply handles/channels, request/reply
  waits, handoff routing, role/remote addressing, daemon federation, or any
  other Pattern 4 behavior in this queue-core slice.

## Testing Strategy

- **Queue state and fencing:** TDD against a real temporary SQLite database,
  because atomic claims, lease expiry, deduplication, retry scheduling, and
  stale-owner rejection are compressible persistence invariants.
- **Transport separation:** TDD with local and deliberately failing transport
  implementations, because a persisted enqueue must survive a missed or failed
  delivery hint and a local hint must wake a waiting consumer.
- **Migration and integration wiring:** goal-based checks plus migration tests,
  using crate tests, `cargo check`, Clippy, and the workspace gate because no
  user-facing surface exists in this slice.

## Acceptance Criteria

The limits below are the canonical queue contract. The plan and implementation
reference this table rather than defining parallel values.

| Field / behavior | Limit |
| --- | --- |
| Envelope version | exactly `1` |
| `kind`, `source`, `target` | 1–128 UTF-8 bytes each |
| Provenance actor/session/execution IDs | 1–128 UTF-8 bytes each |
| Serialized JSON payload | at most 65,536 bytes |
| Correlation and deduplication values | at most 256 UTF-8 bytes each |
| Priority | signed 16-bit integer; higher values claim first |
| Maximum attempts | `1..=20` |
| Claim/renew lease duration | `1..=300` seconds |
| Retry delay | `min(1s * 2^(attempt-1), 300s)` |
| Normalized failure code | at most 64 ASCII bytes |
| Raw transport/repository error exposure | zero bytes |

- [x] **AC-schema:** schema version 26 additively creates the durable work table
  and its claim/deduplication indexes on both fresh and version-25 databases;
  rerunning initialization is idempotent and preserves existing session data.
- [x] **AC-envelope-bounds:** enqueue rejects unsupported envelope versions,
  any value outside the canonical limits table, and invalid JSON/timestamps
  before persistence.
- [x] **AC-envelope-authority:** enqueue accepts only envelopes produced through
  a host policy that attaches caller provenance, allowlists the work kind and
  target, and validates the kind-scoped payload; routing fields grant no
  authority, and the internal contract requires every future consumer to
  re-check caller-bounded actor/session/tool capability before side effects.
- [x] **AC-enqueue-dedupe:** enqueue atomically persists a versioned envelope;
  repeating a non-empty `(source, dedupe_key)` returns the existing work item
  without inserting or dispatching a duplicate.
- [x] **AC-atomic-claim:** concurrent claimers cannot both obtain the same work
  item; eligible work has `available_at <= now` and is selected by the total
  order `priority DESC, available_at ASC, created_at ASC, id ASC`; every claim
  records owner, opaque lease token, expiry, and incremented attempt count.
- [x] **AC-fenced-transitions:** acknowledge, lease renewal, retry, and permanent
  failure succeed only when owner and opaque token match and
  `lease_expires_at > now`; an expired worker is stale even before reassignment
  and cannot mutate the item.
- [x] **AC-lease-bounds:** claim and renewal accept only finite lease durations
  from 1 through 300 seconds, reject invalid timestamps or durations before a
  write, and never persist a non-expiring lease.
- [x] **AC-recovery:** expired leases become claimable again, while an item whose
  attempts reach its configured maximum transitions to `dead_letter` instead of
  retrying forever.
- [x] **AC-bounded-retry:** retryable failure schedules deterministic exponential
  backoff capped at five minutes; permanent or exhausted failure records only
  the canonical normalized reason code and enters `dead_letter`.
- [x] **AC-transport-port:** `WorkTransport` receives a versioned envelope but
  owns no queue state; the local implementation is only a wake signal, and a
  transport publication failure does not roll back or hide persisted work.
- [x] **AC-observability:** enqueue, claim, retry, completion, lease recovery,
  and dead-letter transitions emit structured identifiers and state/reason
  metadata without payload contents.
- [x] **AC-error-redaction:** transport, repository, deserialization, and
  integrity failures cross the queue API and logs only as normalized bounded
  codes; raw errors containing payload text, SQL, filesystem paths, or
  secret-shaped values are neither logged nor returned in enqueue receipts.
- [x] **AC-scope:** no existing execution flow is migrated, no public contract is
  introduced, no broker or new third-party dependency is added, and no peer
  messaging, reply, handoff, role/remote addressing, or federation type/path is
  introduced.

## Verification Evidence

- `cargo fmt --all -- --check`, `cargo check --workspace`, and
  `cargo clippy --workspace --all-targets -- -D warnings` pass.
- `cargo test --workspace` passes, including the real SQLite v25-to-v26
  migration, dedupe/claim/fencing/recovery tests, nanosecond ordering and lease
  boundary regressions, corrupt-row quarantine, redacted diagnostics, and local
  transport persistence-first behavior.
- Focused queue coverage passes with
  `cargo test -p execution-state -p zbot-runtime-sqlite -p gateway-bus`.
- Adversarial, quality, and security implementation reviews report
  `Clean — ready to commit.` after three remediation rounds.
- Repository-wide documentation lints retain only the pre-existing baseline:
  four stale-deferral findings and ten dangling traceability pointers; this
  spec adds no new documentation-lint finding.

## Follow-on compatibility

The later A2A federation feature reuses this queue without changing its
authority model: peer ownership is expressed through `WorkScope`, cancellation
uses the generic fenced `canceled` transition, and outbound dispatch/poll remain
ordinary immutable work items. No broker was added; `WorkTransport` remains a
wake-only replaceable seam. See
[`A2A Federation and Discovery`](../a2a-federation-discovery/spec.md).

## Assumptions

- Technical: execution state already uses bundled SQLite/rusqlite (source:
  `services/execution-state/Cargo.toml`).
- Technical: sessions and executions already model queued/running/terminal
  states but do not provide work leases, retry schedules, or idempotent work
  envelopes (source: `services/execution-state/src/types.rs`).
- Technical: continuation readiness currently crosses an in-memory gateway
  event boundary (source: `gateway/gateway-execution/src/continuation.rs`).
- Process: a behavior-changing persistence feature uses a spec and plan in one
  feature directory (source: `docs/CONVENTIONS.md §4`).
- Product: V1 is local-daemon durability and distributed broker adapters remain
  future work (source: user confirmation 2026-08-04).
- Product: queued messages represent executable work rather than a general
  human-style conversational inbox (source: user confirmation 2026-08-04).
- Product: V1 exposes an internal Rust port only, with mixed data/service shape
  and no public protocol or UI (source: user confirmation 2026-08-04).
