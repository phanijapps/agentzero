## Blockers

**1. AC1 is not verified at the outbound request-body boundary.** `runtime/agent-runtime/src/llm/openai.rs:1221`. The canonical-order test calls
`prepare_tools` directly, so a regression that measures canonical tools but
inserts the original ordered array into `build_request_body` would still pass
while violating byte-identical request JSON. **Fix:** Add a
`build_request_body` test that serializes reversed equivalent inventories,
asserts full request bytes match, and asserts `/tools` is sorted by function
name.

**2. Invalid-tool propagation is only tested for non-streaming chat.** `runtime/agent-runtime/src/llm/openai.rs:1406`. The only pre-network rejection
test exercises `chat`, leaving structured chat, streaming, streaming fallback,
and the Rig adapter paths unguarded despite the plan making
`build_request_body` the common fail-before-network boundary. **Fix:** Add
invalid-inventory tests for `chat_with_schema`, `chat_stream`, and the Rig
completion/stream adapter path that assert `InvalidRequest` is returned before
any HTTP request.
