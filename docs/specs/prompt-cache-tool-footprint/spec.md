# Spec: Prompt Cache and Tool Footprint

- **Status:** Shipped <!-- Draft | Approved | Implementing | Shipped | Archived -->
- **Owner:** agentzero
- **Plan:** [`plan.md`](plan.md)
- **Constrained by:** none
- **Brief:** none
- **Discovery:** none
- **Contract:** none
- **Shape:** service

> **Spec contract:** this document defines what "done" means. The implementing
> PR must match this spec, or update it. Verification must be derivable from it.

## Objective

Keep OpenAI-compatible prompt prefixes cache-friendly while making model-visible
tool inventories deterministic, measurable, and bounded before they cross the
provider network boundary. Equivalent tool inventories produce byte-identical
request JSON, malformed or excessive inventories fail locally with actionable
errors, and operators can inspect footprint metadata without logging tool
schemas or their contents.

## Boundaries

### Always do

- Canonicalize and validate tool inventories before constructing an outbound request.
- Preserve existing cached-token extraction and byte-stability behavior.
- Emit only aggregate footprint metadata; tool descriptions and schemas remain out of the new diagnostic event.

### Ask first

- Change any hard limit after the initial measured baseline is recorded.
- Expose footprint data through a public API, event, or persisted schema.
- Remove, defer, or hide a tool automatically to fit a budget.

### Never do

- Add provider-specific cache directives or change message content in this scope.
- Add a dependency or a new module boundary for the footprint calculation.
- Send an invalid or over-budget tool inventory and rely on the provider to reject it.

## Testing Strategy

- **TDD:** Canonical ordering, duplicate detection, aggregate measurement, and
  each boundary condition are pure request-construction invariants exercised in
  `runtime/agent-runtime/src/llm/openai.rs` tests.
- **Goal-based check:** `cargo test -p agent-runtime` and `cargo clippy -p
  agent-runtime --all-targets -- -D warnings` prove the common non-streaming,
  structured, streaming, retry, throttle, and Rig adapter paths still compile
  and consume the hardened OpenAI-compatible client.

## Acceptance Criteria

- [x] Equivalent tool inventories produce byte-identical request JSON regardless of their input ordering, with tools canonically ordered by function name.
- [x] Only OpenAI function-tool envelopes are accepted: the inventory is an array, every item has `type: "function"`, `function` is an object, and `function.name` is a string of 1-64 ASCII letters, digits, underscores, or hyphens; missing, malformed, provider-native, or duplicate names return `LlmError::InvalidRequest` before an HTTP request is attempted.
- [x] Tool inventories accept at most 128 tools, at most 64 KiB per serialized tool, and at most 256 KiB for the canonical serialized tool array; UTF-8 byte counting and hashing abort at limit + 1 without first allocating a complete serialized buffer, inputs above each limit fail locally, and the exact boundary remains accepted.
- [x] Every accepted non-empty tool inventory emits one structured debug record containing tool count, canonical UTF-8 serialized bytes, an estimated token count of `ceil(bytes / 4)`, and a lowercase 64-character SHA-256 hex fingerprint of the canonical serialized tool array, without logging schema content.
- [x] Every rejected inventory uses a stable rule code plus numeric metadata only; the returned error and diagnostic fields never contain a tool name, description, parameter schema, credential, prompt fragment, or provider payload.
- [x] Existing deterministic request-body tests and OpenAI plus GLM/DeepSeek/z.ai cached-token extraction tests remain green.

## Assumptions

- Technical: OpenAI-compatible request construction is the shared provider boundary used by both the legacy executor and Rig adapter (source: `runtime/agent-runtime/src/llm/openai.rs` and `runtime/agent-runtime/src/rig_adapter/model.rs`).
- Technical: the running root inventory contains 23 tools, 31,557 serialized bytes, and a 4,934-byte largest tool, leaving substantial headroom below the hard limits (source: read-only `GET /api/tools` probe on 2026-08-03).
- Technical: `sha2` is already a direct `agent-runtime` dependency (source: `runtime/agent-runtime/Cargo.toml`).
- Process: validation of external MCP/tool schemas crosses an untrusted-input and network boundary, so the work-loop runs in full mode (source: `docs/CONVENTIONS.md` risk triggers).
- Product: phase one includes deterministic ordering, footprint diagnostics, and fail-closed hard limits, but no automatic pruning or public metrics surface (source: user confirmation 2026-08-03).
