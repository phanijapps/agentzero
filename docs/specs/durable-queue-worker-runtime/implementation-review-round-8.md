# Implementation Review Round 8

## Blockers

**1. Cancellation intent can be delayed behind the blocking pool.** `gateway/gateway-bus/src/worker.rs:709`

The atomic flag and interrupts were set inside a newly spawned blocking task,
so blocking-pool saturation could let the shutdown timeout win first.

Fix: signal cancellation synchronously, then queue only commit-gate settlement
under the absolute deadline. A one-worker/one-blocking-thread runtime now keeps
the settlement task queued behind a two-second real SQLite claim and proves a
one-second shutdown remains bounded and the item remains pending.

## Disposition

Findings remain. Immediate signaling and the saturated-pool regression were
implemented in the next pass.
