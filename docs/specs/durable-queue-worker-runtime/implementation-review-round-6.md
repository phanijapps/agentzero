# Implementation Review Round 6

## Blockers

**1. A claim slower than the shutdown drain can still mutate after return.** `gateway/gateway-bus/src/worker.rs:695`

Aborting the worker join could detach an uninterruptible blocking claim after
the drain deadline.

Fix: add a store-boundary claim cancellation fence. Shutdown publishes intent
before waiting on the worker; authoritative claim transactions hold a commit
gate and cannot begin or commit after cancellation completes; the worker also
refuses dispatch after cancellation. The shutdown race now delays the claim
beyond the drain and proves the item remains pending after the detached call
would have completed.

## Disposition

Findings remain. The cancellation fence and slower-than-drain regression were
implemented in the next pass.
