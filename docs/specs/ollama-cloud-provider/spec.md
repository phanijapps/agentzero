# Spec: Ollama Cloud Provider

- **Status:** Shipped
- **Owner:** AgentZero maintainers
- **Plan:** [`plan.md`](plan.md)
- **Constrained by:** none
- **Contract:** `contracts/openapi/commissioning.yaml`, `gateway/src/http/openapi.yaml`
- **Shape:** mixed

> **Spec contract:** this document defines what "done" means. The implementing
> PR must match this spec, or update it. Verification must be derivable from it.

## Objective

Make Ollama Cloud a correct, first-class cloud provider in first-time commissioning and provider settings. A user supplies an Ollama API key, receives a small cloud-only model recommendation, and finishes setup with the orchestrator and every persisted specialist agent initially using `glm-5.2:cloud` while the universal multimodal fallback uses `gemma4:31b-cloud`. Configurations that support inheritance continue to inherit from the orchestrator.

## Boundaries

### Always do

- Treat Ollama Cloud and Ollama Local as separate presets with separate authentication and model lists.
- Use Ollama Cloud's OpenAI-compatible `https://ollama.com/v1` base URL and the existing bearer-token provider transport.
- Disable redirects for Ollama Cloud verification and inference so bearer credentials cannot leave the exact HTTPS origin.
- Enforce `provider-ollama-cloud`'s exact `https://ollama.com/v1` origin on create, update, verification, and text/multimodal inference, including hand-edited stored records.
- Preserve every non-model commissioning and model-routing behavior for all non-Ollama-Cloud providers; credential redaction, rotation, bounded verification, and no-forward verification redirects intentionally apply to every provider.
- For Ollama Cloud first-time setup, retarget only the required provider/model fields of every persisted non-root agent present at submission; retain every other agent field. Keep optional execution/ward overrides unset so they inherit the orchestrator.
- Use a secret-free durable pending marker and idempotent retry so interruption resumes forward to one complete provider/agent/settings state; never advertise setup complete while recovery is pending.
- Redact provider credentials from every public provider response and expose only whether a credential is configured.
- Serialize commissioning with an exclusive mutation owner; conflicting retries or provider mutations receive a finite recovery response until an idempotent retry completes and caches are refreshed.

### Ask first

- Change either recommended model identifier or add additional initially pinned agent-model overrides.
- Replace the OpenAI-compatible integration with Ollama's native `/api/chat` protocol.
- Change commissioning defaults or persistence outside the Ollama Cloud conditional path.

### Never do

- Allow server-resolved commissioning to send an Ollama Cloud API key anywhere except the fixed `https://ollama.com/v1` endpoint, log it, or return it in an API response.
- Mark Ollama Cloud as keyless or reuse the local preset's `noApiKey` behavior.
- Add `:cloud` models to the Ollama Local recommendation list.
- Silently fall back from Ollama Cloud to a local runtime or another provider.

## Testing Strategy

- Preset validation and conditional settings mutation: TDD through Rust commissioning unit/integration tests.
- UI preset, recommendation state, and API-key requirement: TDD through React component and preset tests.
- OpenAPI and TypeScript request alignment: goal-based contract checks and UI typecheck.
- Complete Ollama Cloud setup journey: mocked goal-based integration test; optional manual QA may use a non-production test key, and no live secret is committed.
- Regression protection for all other providers and local Ollama: existing commissioning/provider suites plus explicit non-Ollama assertions.

## Acceptance Criteria

- [x] Given first-time commissioning, when the user chooses Ollama Cloud, the UI requires an API key and offers `glm-5.2:cloud` as the initial ordinary-agent model.
- [x] Given Ollama Cloud is selected, the UI shows a compact cloud recommendation that identifies `glm-5.2:cloud` for agents and `gemma4:31b-cloud` for multimodal work without showing local-only models.
- [x] When Ollama Cloud commissioning completes, z-Bot persists a distinct `provider-ollama-cloud` provider with base URL `https://ollama.com/v1`, bearer API-key credentials, both recommended models in its catalog, and `glm-5.2:cloud` as its default model.
- [x] When Ollama Cloud commissioning completes, the orchestrator and every persisted non-root agent present at submission have provider/model fields set to Ollama Cloud and `glm-5.2:cloud`; all other agent fields are byte-equivalent in meaning; distillation, curator, intent analysis, and untouched ward configurations retain unset overrides and inherit from the orchestrator.
- [x] When Ollama Cloud commissioning completes, multimodal configuration explicitly selects the Ollama Cloud provider and `gemma4:31b-cloud`, while all other existing commissioning settings remain unchanged.
- [x] When any provider other than Ollama Cloud is commissioned, the current orchestrator, inheritance, multimodal, and setup behavior is unchanged.
- [x] In Settings, the Ollama Cloud preset uses `https://ollama.com/v1`, requires an API key, recommends cloud models only, and remains distinct from the keyless `http://localhost:11434/v1` Ollama Local preset.
- [x] Whenever the user changes provider kind or cloud preset during commissioning, the controlled API-key value is cleared before another provider can be submitted.
- [x] Commissioning rejects unknown presets, invalid models, missing or oversized Ollama Cloud keys, and never includes the submitted key in response bodies or logs.
- [x] The commissioning OpenAPI contract, Rust request model, and TypeScript request type all accept the `ollama_cloud` preset identifier.
- [x] Ollama Cloud verification and inference reject cross-host, HTTPS-to-HTTP, and redirect-loop responses without forwarding authorization beyond `https://ollama.com`.
- [x] Provider list/get/create/update/default/test responses never serialize stored API keys; they report only credential presence, and Settings distinguishes an omitted key (retain) from an explicit replacement (rotate).
- [x] If Ollama Cloud commissioning is interrupted, its durable marker prevents a false-complete status and an idempotent resubmission converges provider, agent, and settings state before clearing the marker.
- [x] Provider verification bounds upstream response bytes, discovered-model count, and model-ID length, and scrubs the configured key from every user-observable success or error field.
- [x] `provider-ollama-cloud` cannot be created, updated, verified, or used for text/multimodal inference with any origin other than exact HTTPS `ollama.com` on the default port and `/v1` base path.
- [x] Settings displays `hasApiKey` without the key; blank editing retains it and non-empty editing rotates it.
- [x] Concurrent commissioning is rejected, and a pending Ollama Cloud marker blocks provider mutations and exposes `ollama_cloud_commissioning_pending` until a successful retry clears it.

## Assumptions

- Technical: z-Bot's provider test and inference clients append `/models` and `/chat/completions`, respectively, and inject bearer authorization (source: `gateway/gateway-services/src/providers.rs`, `runtime/agent-runtime/src/llm/openai.rs`).
- Technical: Ollama documents an OpenAI-compatible API, and a 2026-07-19 live probe confirmed `https://ollama.com/v1/models` and authenticated `https://ollama.com/v1/chat/completions` are exposed (source: [Ollama OpenAI compatibility](https://docs.ollama.com/api/openai-compatibility), live HTTP probe 2026-07-19).
- Technical: Ollama's native cloud chat endpoint remains `https://ollama.com/api/chat` and accepts `Authorization: Bearer $OLLAMA_API_KEY` (source: [Ollama chat API](https://docs.ollama.com/api/chat), [Ollama authentication](https://docs.ollama.com/api/authentication)).
- Technical: specialized execution configurations already inherit from the orchestrator when their provider/model overrides are unset (source: `gateway/gateway-services/src/settings.rs`, `gateway/gateway-execution/src/invoke/setup.rs`).
- Technical: persisted non-root agent configurations require explicit provider/model strings and therefore must be retargeted during Ollama Cloud first-time setup rather than relying on inheritance (source: `gateway/gateway-services/src/agents.rs`).
- Product: "mini modal recommendation" means a compact Ollama Cloud recommendation panel in commissioning (source: user confirmation 2026-07-19).
- Product: use the exact `glm-5.2:cloud` and `gemma4:31b-cloud` identifiers even when model discovery does not return them (source: user confirmation 2026-07-19).
- Product: all non-Ollama-Cloud setup behavior remains unchanged (source: user confirmation 2026-07-19).
