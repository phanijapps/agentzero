# Implementation Review Round 1

## Blockers

**1. Spec metadata does not match the implemented diff.** `docs/specs/durable-queue-worker-runtime/spec.md:3`

Fix: move spec/plan status and acceptance checks only after verification and
review complete.

**2. Worker-loop recovery is not verified.** `gateway/gateway-bus/tests/durable_work_worker.rs:431`

Fix: cover missed notification polling and expired-lease recovery at worker
start.

**3. Lease, handler-outcome, and store-fail-closed cases are incomplete.** `gateway/gateway-bus/tests/durable_work_worker.rs:459`

Fix: add stale renewal, retry/permanent/panic, store-error, blocking-task, and
bounded backoff coverage.

**4. Gateway lifecycle coverage bypasses GatewayServer start.** `gateway/src/server.rs:554`

Fix: drive real start and shutdown on an isolated port.

**5. Panic and renewal-failure diagnostics omit per-item context.** `gateway/gateway-bus/src/worker.rs:575`

Fix: include bounded work ID, target, kind, and attempt and test payload/raw-error
redaction.
