# Implementation Review Round 11

## Blockers

**1. Active-handler shutdown regression is not deterministically exercised.** `gateway/gateway-bus/tests/durable_work_worker.rs:883`

Waiting only for the queue row to become leased can complete before the test
handler increments its active count, allowing the final zero-active assertion
to pass without exercising abort-on-drop cancellation.

Fix: wait under a bounded timeout for the handler's active count to reach one
before shutdown, then assert shutdown returns it to zero and leaves the lease
recoverable.

## Disposition

Findings remain. The deterministic running-handler precondition is added in
the next pass.
