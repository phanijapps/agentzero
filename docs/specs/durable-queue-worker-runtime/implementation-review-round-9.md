# Implementation Review Round 9

## Blockers

**1. Shutdown abort can detach the inner handler task.** `gateway/gateway-bus/src/worker.rs:868`

Aborting the outer item task dropped its nested handler `JoinHandle`, which
detached the handler instead of cancelling it. A handler could therefore keep
running after the worker reported shutdown complete.

Fix: wrap the nested task in an abort-on-drop guard, explicitly abort and await
it on timeout and renewal failures, and assert the long-running handler's
active count is zero when bounded shutdown returns.

## Disposition

Findings remain. Abort-on-drop ownership and the active-handler shutdown
regression were implemented in the next pass.
