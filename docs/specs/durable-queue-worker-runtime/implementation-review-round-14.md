# Implementation Review Round 14

## Blockers

**1. Shutdown can block before its timeout starts.** `gateway/gateway-bus/src/worker.rs:727`; `services/execution-state/src/work.rs:696`; `services/execution-state/src/work.rs:736`

Publishing cancellation under the commit gate made synchronous `signal()` wait
for an in-progress SQLite commit, preventing the async shutdown deadline from
preempting that wait.

Fix: keep cancellation flag publication and SQLite interruption nonblocking;
retain deadline-bounded exclusive settlement as the point that waits for any
already-authorized commit. Update the regression and plan to define the
no-late-commit guarantee at successful settlement/shutdown return.

## Disposition

Findings remain. Nonblocking signal plus deadline-bounded commit settlement is
implemented in the next pass.
