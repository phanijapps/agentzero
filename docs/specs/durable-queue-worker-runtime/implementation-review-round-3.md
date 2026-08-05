# Implementation Review Round 3

## Blockers

**1. Synchronous handler gate panics bypass the redacted panic boundary.** `gateway/gateway-bus/src/worker.rs:607`

The handler future and blocking-store calls were redacted, but synchronous
`validate_payload` and `authorize` calls could still invoke the default panic
hook with untrusted payload or provenance data.

Fix: execute both gates inside the same redacted panic boundary, catch gate
panics as a normalized retryable internal failure, and cover validator and
authorizer panic secrets in the subprocess stderr regression.

## Disposition

Findings remain. The gate boundary and subprocess coverage were repaired in the
next implementation pass.
