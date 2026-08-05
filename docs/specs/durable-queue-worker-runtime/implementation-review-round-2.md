# Implementation Review — Round 2

## Blockers

**1. Panic isolation still leaked raw panic payloads outside tracing.** `gateway/gateway-bus/src/worker.rs:564`

Rust's panic hook ran before Tokio converted handler and blocking-store panics
into a `JoinError`, while the diagnostics test captured tracing only.

Fix: add a redacting panic boundary for worker-owned handler and blocking-store
tasks, then capture real panic-hook/stderr output in the diagnostics coverage.

## Disposition

Findings remain. Repaired in the next implementation pass by installing a
worker-aware panic hook, scoping redaction to handler future polls and blocking
store calls, and exercising both panic sources in a subprocess that captures
stderr.
