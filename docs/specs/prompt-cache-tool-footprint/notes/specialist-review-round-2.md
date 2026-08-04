## Blockers

**1. UTF-8 byte counting is not verified.** `runtime/agent-runtime/src/llm/openai.rs:1310`. The AC3 boundary tests size schemas with ASCII-only strings, so a regression that counts characters instead of UTF-8 serialized bytes would still pass. **Fix:** Add a non-ASCII description fixture that asserts `serialized_bytes`, exact-boundary acceptance, and above-boundary rejection use serialized UTF-8 byte length.

## Concerns

**2. Function envelopes are only minimally validated before being forwarded unchanged.** `runtime/agent-runtime/src/llm/openai.rs:151`. A malicious tool can carry a valid function name while adding unknown top-level or function fields that a compatible provider may interpret outside the declared contract. **Fix:** Reject unknown envelope keys and validate the supported OpenAI function fields before measurement and insertion.
