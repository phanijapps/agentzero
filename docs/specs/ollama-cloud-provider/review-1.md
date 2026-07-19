# Review 1

## Blockers

**1. Provider identity mutation bypasses fixed-origin enforcement.** `gateway/gateway-services/src/providers.rs:300`. Force updates to retain the path-selected stored provider ID and test retained-key rename attacks. Fix: preserve identity unconditionally.

**2. Ollama multimodal model is accepted as the orchestrator model.** `gateway/src/http/commissioning.rs:944`. Fix: require `glm-5.2:cloud` as the only Ollama Cloud commissioning selection while retaining both catalog models.

**3. Pending retries and provider mutations are not transaction-bound.** `gateway/src/http/commissioning.rs:609`. Fix: store a secret-free request identity, reject mismatched retries, create the marker safely, and share mutation exclusion.

**4. Chunked provider responses bypass the byte limit.** `gateway/gateway-services/src/providers.rs:400`. Fix: stream at most 1 MiB plus a sentinel byte before parsing.

## Concerns

**5. Secret scrubbing is incomplete.** `gateway/gateway-services/src/providers.rs:433`. Fix: centrally scrub every returned message and model field.

**6. Security boundaries lack direct regression tests.** `gateway/gateway-services/src/providers.rs:481`. Fix: add attack-shaped tests for identity, model selection, marker conflicts, and response limits.

## Nits

**7. A stale UI provider type still exposes apiKey.** `apps/ui/src/shared/types/index.ts:246`. Fix: align it with hasApiKey.
