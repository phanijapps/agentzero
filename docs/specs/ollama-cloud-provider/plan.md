# Plan: Ollama Cloud Provider

- **Spec:** [`spec.md`](spec.md)
- **Status:** Done

> **Plan contract:** this is the implementation strategy. Unlike the spec, this
> document is allowed to change as implementation evidence arrives.

## Approach

Correct the existing misleading Settings preset and add the same distinct Ollama Cloud identity to commissioning. Reuse the OpenAI-compatible request/response shape at `https://ollama.com/v1`, with redirects disabled for that fixed origin. Make commissioning provider-aware: `ollama_cloud` assigns the orchestrator and multimodal fallback, then retargets only provider/model fields on every persisted non-root agent present at submission. A durable marker prevents false completion and makes retry convergent. Redact provider responses, define retain/rotate semantics, bound/sanitize verification results, and keep optional execution/ward configurations inheriting.

## Constraints

- `contracts/openapi/commissioning.yaml` is the source-of-truth API contract and must land with both server and client changes.
- Provider inference and multimodal calls use the existing OpenAI-compatible `/chat/completions` transport.
- API keys remain write-only commissioning input and use existing provider secret persistence/redaction behavior.
- Existing stored providers and non-Ollama commissioning flows require no migration.
- Upstream provider error text may remain observable only after every occurrence of the configured API-key sentinel has been scrubbed for all providers.
- Public provider reads return `hasApiKey`; create requires a key, update omission or a blank value retains the stored key, and a non-empty value replaces it.
- Provider redaction, rotation semantics, verification bounds, and no-forward verification redirects are intentional cross-provider security corrections; non-Ollama commissioning and model routing remain behaviorally unchanged.
- `provider-ollama-local` is the only server-defined keyless-capable provider. Editable names, URLs, and client flags never confer that status.

Declined additions: a native Ollama `/api/chat` transport, because the existing OpenAI-compatible shape is sufficient; a general provider plugin/factory abstraction, because one fixed preset does not justify a new layer; and configurable Ollama Cloud endpoints, because configurability would weaken the server-owned outbound allowlist.

## Construction tests

**Integration tests:** Extend commissioning completion coverage to deserialize `ollama_cloud`, persist the expected provider, and compare before/after execution settings so only orchestrator plus Ollama-specific multimodal fields change. Add a non-Ollama control case proving no multimodal pinning occurs.

**Manual verification:** In a fresh data directory, select Ollama Cloud, inspect the recommendation panel, submit a valid test key, confirm setup completion, then run one text prompt and one multimodal analysis. Repeat the intelligence step with Ollama Local and confirm it remains keyless and cloud models are absent.

## Design (LLD)

### Design decisions

- Use `https://ollama.com/v1`, not native `/api/chat`, because it matches the existing provider verification, text inference, and multimodal clients without protocol branching. Traces to AC 3, 5, 7.
- Give cloud and local stable identities (`provider-ollama-cloud` and `provider-ollama-local`) so credentials and models cannot be conflated. Traces to AC 3, 7, 8.
- Enforce the exact Ollama Cloud origin at storage and every runtime use, so a retained key cannot be redirected by editing or hand-modifying `baseUrl`.
- Preserve inheritance by omission for optional execution and ward configurations; retarget the required provider/model strings on all persisted non-root agents in the first-time setup snapshot. Traces to AC 4, 6, 11.
- Treat the submission-time `AgentService::list` result, excluding reserved root/orchestrator identities, as the exact retarget set. Names, bundled-template membership, and display names do not determine ownership.

### Data & schema

- Add `ollama_cloud` to `ProviderSelection.presetId` in the OpenAPI and TypeScript union.
- No persistent schema migration: providers and execution settings already carry the required base URL, key, provider ID, and model fields.
- The persisted cloud provider catalog contains `glm-5.2:cloud` and `gemma4:31b-cloud`, with the former as default.
- Ollama-only completion sets `execution.multimodal.provider_id = provider-ollama-cloud` and `execution.multimodal.model = gemma4:31b-cloud`, retaining its current temperature/token settings.

### Interfaces & contracts

- `POST /api/commissioning/complete` accepts `presetId: ollama_cloud`, `model: glm-5.2:cloud`, and a write-only API key under `contracts/openapi/commissioning.yaml`.
- Provider calls resolve to `GET https://ollama.com/v1/models` and `POST https://ollama.com/v1/chat/completions` with the existing bearer header.

### Component / module decomposition

- `gateway/src/http/commissioning.rs`: authoritative cloud preset and conditional multimodal settings policy.
- `apps/ui/src/features/commissioning/CommissioningScreen.tsx`: selectable preset, fixed initial recommendation, API-key UX, and compact recommendation panel.
- `apps/ui/src/features/settings/providerPresets.ts`: correct the already-present Ollama Cloud preset while retaining Ollama Local separately.
- Contract/client types and their tests keep the boundary synchronized.

### State & control flow

1. Every provider-kind or preset transition clears the controlled API-key field. Selecting Ollama Cloud then chooses `glm-5.2:cloud` and shows the two-model recommendation.
2. Submission sends the selected ordinary model and API key; the server resolves all endpoint/provider identity data from its allowlist.
3. Completion holds an exclusive process lock and writes a secret-free pending marker after validation/preflight. It advances provider/default, agents, then settings, with setup-complete saved last. An interruption leaves a needs-attention recovery code and blocks provider mutations; resubmitting the same Ollama Cloud setup idempotently converges the files and clears the marker.

### Failure, edge cases & resilience

- Server-side preset/model allowlisting and branching on the resolved preset ID—not names, URLs, model suffixes, or request text—prevents a client from substituting local models or arbitrary endpoints.
- Missing, blank, or oversized keys fail before persistence; existing error redaction continues to prevent secret exposure.
- Authentication success is sufficient even when `/models` omits a recommended model. If a later chat request rejects that model, surface the ordinary provider error; do not fall back to local Ollama.
- Settings preset correction affects only newly instantiated presets; explicitly avoid silently rewriting an existing user provider.
- Provider verification scrubs the configured key from all upstream body/error text before constructing `ProviderTestResult`, including repeated or embedded occurrences.
- Provider verification caps the response body at 1 MiB, discovered models at 1,000, and model IDs at 160 characters before returning any result.
- Public provider handlers map stored domain objects to credential-redacted response DTOs. Update requests use an optional credential field so omission cannot overwrite the stored key with a UI placeholder.
- `ProviderResponse.hasApiKey` replaces response `apiKey`. Settings edit initializes a blank secret field: blank omits/retains and non-empty replaces.

### Quality attributes (NFRs)

- Security: API key remains write-only and absent from logs/responses, verified by response serialization and log/redaction tests.
- Accessibility: the recommendation panel is associated with the selected provider and remains keyboard/screen-reader understandable using existing commissioning patterns.

### Dependencies & integration

- External dependency: Ollama Cloud's OpenAI-compatible API and model availability.
- Internal dependencies: provider persistence, execution settings, commissioning contract, and the existing multimodal tool.

## Tasks

### T1: Contract and server preset tests accept only valid Ollama Cloud selections

**Depends on:** none

**Touches:** `contracts/openapi/commissioning.yaml`, `gateway/src/http/commissioning.rs`, `apps/ui/src/services/transport/types.ts`

**Tests:**
- Rust tests cover accepted `ollama_cloud` + `glm-5.2:cloud`, missing/oversized key rejection, wrong-model rejection, and distinct cloud identity (AC 3, 8, 9).
- Contract/type checks accept `ollama_cloud` without weakening other preset validation (AC 9).

**Approach:**
- Add the server allowlisted preset with `provider-ollama-cloud`, `https://ollama.com/v1`, and the commissioned model.
- Update the OpenAPI preset enum and TypeScript union in the same change.

**Done when:** focused Rust tests and contract/client typechecks pass.

### T2: Ollama Cloud completion aligns every initial agent while preserving all unrelated configuration

**Depends on:** T1

**Touches:** `gateway/src/http/commissioning.rs`, `gateway/gateway-services/src/agents.rs`, `gateway/src/http/providers.rs`, `gateway/src/state/mod.rs`

**Tests:**
- Completion tests prove the orchestrator, multimodal, and all submission-time persisted non-root agent assignments exactly (AC 3-5).
- Execution-level tests resolve root, one persisted specialist, one untouched ward, distillation, curator, and intent analysis to `glm-5.2:cloud` (AC 4).
- Comparison tests cover both absent and pre-existing optional overrides: first-time absent overrides remain absent; pre-existing explicit user overrides are preserved rather than silently cleared (AC 4-6).
- Regression cases cover OpenAI, another cloud preset, Ollama Local, both memory profiles, and a customized multimodal configuration (AC 6).
- Injected failures and simulated termination at marker creation, staging, provider persistence, agent commit, and settings save prove recovery, incomplete status until the last step, confinement, cleanup/cache invalidation, and idempotent retry; concurrent or different retries are rejected deterministically (AC 13, 17).

**Approach:**
- Branch only on the trusted server-resolved preset identity.
- Snapshot the exact `AgentService::list` non-root set, change only provider/model, and keep a secret-free pending marker until settings save succeeds.
- Apply multimodal/provider changes only for Ollama Cloud; save completion state last and clear the marker only after durable success.
- Never stage or snapshot provider bytes: the only secret-bearing durable location is `providers.json`; retry rolls forward from that record.
- Reject concurrent commissioning and conflicting provider mutations while the pending marker is owned; recovery invalidates agent, provider, and settings caches before clearing it.

**Done when:** commissioning settings tests demonstrate the Ollama-only delta and non-Ollama regression case.

### T3: First-time setup presents a cloud-only Ollama recommendation and requires its key

**Depends on:** T1

**Touches:** `apps/ui/src/features/commissioning/**`

**Tests:**
- Component tests cover preset selection, default model, key requirement, recommendation content, and key clearing for OpenAI → Ollama Cloud, Ollama Cloud → OpenAI, and Ollama Cloud → Ollama Local transitions (AC 1, 2, 8-9).
- Accessibility query verifies the recommendation is named and associated with Ollama Cloud (AC 2).

**Approach:**
- Add Ollama Cloud to the commissioning provider map.
- Render a compact recommendation panel only for that preset, naming the ordinary and multimodal recommendations.
- Clear the key on every provider identity transition, in both directions.
- Reuse existing secret input/submission handling.

**Done when:** UI tests and typecheck pass and no local models appear in the cloud recommendation.

### T4: Settings creates correct, authenticated Ollama Cloud providers without changing Ollama Local

**Depends on:** T1, T2

**Touches:** `apps/ui/src/features/settings/providerPresets.ts`, `apps/ui/src/features/settings/providerPresets.test.ts`, `apps/ui/src/features/settings/ProviderSlideover.tsx`, `gateway/gateway-services/src/providers.rs`, `gateway/src/http/providers.rs`, `gateway/src/http/openapi.yaml`, `apps/ui/src/services/transport/types.ts`, `runtime/agent-runtime/src/llm/openai.rs`, `runtime/agent-tools/src/tools/multimodal.rs`

**Tests:**
- Preset test asserts the cloud URL, API-key requirement, exact ordered recommendation list, and separate local URL/keyless behavior (AC 7).
- Slideover test confirms Ollama Cloud cannot be saved without a key (AC 7, 8).
- Provider-service tests use an echoing mock upstream and prove the configured secret never appears in success/error messages or serialized results, for Ollama Cloud and a generic provider (AC 9).
- HTTP tests prove list/get/create/update/default/test redact credentials and implement omit/replace/clear rotation semantics (AC 12).
- Mock redirect tests prove Ollama Cloud rejects same-host, cross-host, downgrade, and loop redirects without a second authorized request (AC 11).
- Separate tests prove origin enforcement and no-redirect behavior for provider verification, ordinary text inference, and multimodal inference, including a hand-edited invalid stored record (AC 11, 15).
- Oversized/echoing success and error tests prove response/model bounds and complete sentinel scrubbing (AC 14).

**Approach:**
- Correct the existing Ollama Cloud preset from localhost/keyless to `https://ollama.com/v1`/authenticated.
- Put `glm-5.2:cloud` then `gemma4:31b-cloud` first, followed by the current cloud-only recommendations (`nemotron-3-super:cloud`, `gemini-3-flash-preview:cloud`, `deepseek-v3.2:cloud`, `kimi-k2.5:cloud`, `qwen3.5:cloud`, `devstral-2:cloud`, `minimax-m2.7:cloud`); keep local models only in Ollama Local.
- Scrub the provider's configured API-key value from upstream error bodies before returning provider-test results; retain non-secret diagnostic text.
- Introduce redacted provider response/request DTO mapping at the HTTP boundary; never serialize the storage-domain `Provider` directly.
- Build all provider-verification clients with redirect policy `none`; Ollama Cloud inference clients also use `none`, while other providers retain their inference behavior.
- Reject any `provider-ollama-cloud` create/update or runtime load whose parsed origin/base path differs from `https://ollama.com/v1`; cover direct and hand-edited invalid records.
- Adapt Settings to `hasApiKey` and blank-retain/non-empty-rotate behavior.

**Done when:** provider preset and slideover tests pass with cloud/local separation explicit.

### T5: Cross-surface verification proves Ollama Cloud setup is shippable

**Depends on:** T2-T4

**Touches:** `gateway/src/http/commissioning.rs`, `apps/ui/src/features/commissioning/**`, `docs/specs/ollama-cloud-provider/**`, `docs/specs/README.md`

**Tests:**
- Run focused backend and UI suites, mocked `/models` and `/chat/completions` integration tests, `cargo fmt --all -- --check`, relevant Clippy/typecheck gates, and commissioning contract parity validation.
- Assert a sentinel key is absent from commissioning responses, public provider serialization, provider-test errors, and captured structured tracing; execute the live-key smoke test only as optional release evidence.

**Approach:**
- Resolve review findings, record any unavoidable live-key test limitation, and update spec criteria/status only from evidence.

**Done when:** mechanical gates pass, adversarial review has no unresolved blocker, and each acceptance criterion has cited verification evidence.

## Rollout

- **Delivery:** ship as an additive commissioning preset plus correction to the not-yet-correct Settings preset; no feature flag or data migration.
- **Infrastructure:** none. Users supply their own Ollama Cloud key.
- **External-system integration:** Ollama Cloud must continue exposing the documented OpenAI-compatible endpoints and requested models.
- **Rollback:** remove the commissioning preset and restore the Settings metadata; existing provider records remain ordinary editable provider data.

## Risks

- Exact cloud model availability can change independently of z-Bot; keep server recommendations explicit and make provider errors actionable.
- Correcting the Settings preset may surprise users who associated its existing name with localhost; the separate Ollama Local card and regression tests make the distinction visible.
- Broadly replacing execution settings during completion would erase user defaults; tests must compare the full settings object outside the intended fields.
- Live API verification requires a secret and should remain an opt-in smoke test rather than a mandatory CI dependency.

## Changelog

- 2026-07-19: Initial plan; selected Ollama's OpenAI-compatible `https://ollama.com/v1` after official-document review and live endpoint probes.
- 2026-07-19: Replaced staged rollback with a smaller secret-free pending marker and idempotent retry after implementation showed that provider secrets need no secondary staging location.
