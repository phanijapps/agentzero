# Plan: Prompt Cache and Tool Footprint

- **Spec:** [`spec.md`](spec.md)
- **Status:** Done <!-- Drafting | Executing | Done -->

> **Plan contract:** this is the implementation strategy. Unlike the spec, this
> document is allowed to change as you learn. When it changes substantially,
> record why in the changelog.

## Approach

Harden the existing `OpenAiClient::build_request_body` boundary rather than
introducing a parallel cache subsystem. A private preparation function validates
the supplied JSON array, orders tools by function name, rejects duplicates and
budget violations, computes aggregate metadata and a SHA-256 fingerprint, and
returns the canonical array used by every request path. Existing provider usage
parsing remains unchanged.

## Constraints

- Preserve unrelated working-tree edits in `runtime/agent-runtime/src/llm/openai.rs`.
- No new dependency, module, configuration surface, API, or persisted schema.
- Validation errors must not include tool descriptions, parameter schemas, or credentials.

## Construction tests

**Integration tests:** existing `agent-runtime` request, wrapper, executor, and Rig adapter tests.

**Manual verification:** run the focused tests with debug logging and confirm the footprint record contains aggregate fields and no schema content.

## Design (LLD)

### Interfaces & contracts

`OpenAiClient::build_request_body` remains private but changes to return
`Result<Value, LlmError>`. Its callers propagate `InvalidRequest`, making this
the common fail-before-network boundary for chat, structured chat, streaming,
and streaming fallback paths. Traces to: AC1-AC6.

### Failure, edge cases & resilience

Tool JSON must be an array. Each element must carry a non-empty
`function.name` within the function-tool envelope and grammar defined by AC2;
names must be unique. A bounded `Write` implementation counts and hashes the
canonical serialization, aborting at the AC3 limit + 1 instead of materializing
an unbounded byte buffer. The accepted canonical `Value` is cloned only after
validation and measurement pass. Boundary values pass; only values strictly
above a limit fail. Rejections use stable rule codes and numeric metadata only.
Traces to: AC2-AC4.

### Quality attributes (NFRs)

Canonical sorting and SHA-256 fingerprinting make cross-execution drift
observable. The token estimate is `ceil(canonical_bytes / 4)` and the
fingerprint is lowercase SHA-256 hex over those canonical bytes. Structured
diagnostics contain only counts, byte sizes, token estimates, and the
fingerprint. Traces to: AC1, AC4-AC6.

## Tasks

### T1: Hardened request construction rejects unstable or excessive tool inventories

**Depends on:** none

**Touches:** `runtime/agent-runtime/src/llm/openai.rs`

**Tests:**
- TDD (`stub: true`): equivalent inventories serialize identically through `build_request_body` and sort by function name (AC1).
- TDD (`stub: true`): non-array, invalid/unknown envelope fields, invalid field types, invalid name grammar/length, and duplicate names return redacted `InvalidRequest` rule codes before network I/O across legacy and Rig paths (AC2, AC5).
- TDD (`stub: true`): exact and above-boundary cases for tool count, individual UTF-8 bytes, and total bytes, including early writer termination at limit + 1 (AC3).
- TDD (`stub: true`): byte count, token estimate, and fingerprint are deterministic and exclude schema text (AC4).
- Manual QA (`no stub (manual QA)`): the isolated tracing capture records exactly one aggregate footprint event and no schema content (AC4-AC5).
- Regression: existing request stability and cached-token shape tests remain green (AC6).

**Approach:**
- Add private constants and a private footprint/preparation helper in `llm/openai.rs`.
- Make `build_request_body` return `Result` and propagate it through every request path.
- Validate borrowed tool values, canonicalize references, and use a bounded
  counting/hash writer before cloning the accepted canonical array into the request body.
- Emit one structured debug event for each accepted non-empty inventory.

**Done when:** focused request-construction tests, `cargo test -p agent-runtime`, and strict clippy pass.

## Rollout

The change ships directly in the existing client. Rollback is a code revert;
there is no migration, persistent state, deployment sequencing, or external
system prerequisite.

## Risks

- A currently tolerated duplicate or oversized MCP inventory becomes a local error; messages must identify the violated rule without echoing untrusted schema content.
- An MCP tool name longer than the provider-compatible 64-character maximum now fails locally; automatic truncation would create authority collisions and remains out of scope.
- Canonical ordering may change first-turn tool order once, but remains stable thereafter and does not change tool names or schemas.
- Tests that call the private request builder must be updated for its `Result` return without weakening their assertions.

## Changelog

- 2026-08-03: Initial plan, expanded to fail-closed tool-schema hardening after user confirmation.
- 2026-08-03: Implementation completed; review added outbound-boundary, Rig-path, unknown-field, and UTF-8 byte-accounting coverage.
