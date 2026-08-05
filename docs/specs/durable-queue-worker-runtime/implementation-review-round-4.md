# Implementation Review Round 4

## Blockers

**1. Handler authorization can still consume routing and raw payload data.** `gateway/gateway-bus/src/worker.rs:165`

Fix: make validation return a handler-owned typed command, give authorization
only constrained trusted provenance plus that command, and prevent handling
from receiving the raw envelope.

**2. Heartbeat renewal can suspend the handler deadline.** `gateway/gateway-bus/src/worker.rs:686`

Fix: race renewal against the handler deadline and a renewal budget, aborting
local handling without a stale terminal write when either bound wins.

**3. Worker shutdown is not bounded during store calls.** `gateway/gateway-bus/src/worker.rs:553`

Fix: race recovery and claim calls against shutdown, and bound the worker join
by the configured shutdown drain.

**4. Observability coverage does not verify the promised transition set.** `gateway/gateway-bus/tests/durable_work_worker.rs:1032`

Fix: exercise and assert start/stop, claim, authorization rejection, renewal
failure, timeout, panic, completion, retry, and dead-letter diagnostics.

## Concerns

**5. Store diagnostics collapse distinct normalized errors.** `gateway/gateway-bus/src/worker.rs:573`

Fix: preserve the closed `WorkError` class through the blocking boundary and
map each class to a bounded reason code.

**6. Backoff coverage relies on a scheduler-sensitive call-count window.** `gateway/gateway-bus/tests/durable_work_worker.rs:917`

Fix: assert the minimum interval between timestamped calls instead of an exact
number observed during a wall-clock sleep.

## Disposition

Findings remain. These items are assigned to the next implementation pass.
