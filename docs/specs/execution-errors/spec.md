# Spec: Execution error taxonomy (Wave 5)

- **Status:** Implementing
- **Branch:** `op_clean_crap`
- **Shape:** refactor (typed errors replace stringly results)

## Objective

One `ExecutionError` enum (thiserror) in gateway-execution replaces the
169 `Result<_, String>` sites. Errors become matchable; redaction becomes
one Display impl; the 5 upstream string-match sites in gateway/src become
`matches!`.

## Design

```rust
#[derive(Debug, thiserror::Error)]
pub enum ExecutionError {
    #[error("configuration: {0}")] Config(String),
    #[error("provider unavailable: {0}")] Provider(String),
    #[error("store: {0}")] Store(String),
    #[error("session: {0}")] Session(String),
    #[error("delegation: {0}")] Delegation(String),
    #[error("continuation: {0}")] Continuation(String),
    #[error("resource: {0}")] Resource(String),
}
```

Store/service-layer String errors wrap at the port (one `.map_err` at each
call boundary), carrying context. Client-visible messages route through one
`client_message()` method that returns the safe constant per variant —
replacing per-site safe-string constants.

## Acceptance Criteria

- [ ] AC1: Zero `Result<_, String>` in gateway-execution/src (grep;
      `Result<ExecutionError>` and typed alternatives only).
- [ ] AC2: The 5 upstream `.contains(...)` string-matches in gateway/src +
      apps/ become variant matches.
- [ ] AC3: Crash-event redaction flows through `client_message()`; no raw
      error strings published to the event bus.
- [ ] AC4: Full suites green (600 + 453 + parity), clippy -D warnings, fmt.

## Non-goals

agent-runtime's internal error types (already `ExecutorError`); stores'
internal errors (wrapped at the port, not rewritten).
