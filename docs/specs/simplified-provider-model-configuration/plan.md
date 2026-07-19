# Plan: Simplified Provider Model Configuration

- **Spec:** [`spec.md`](spec.md)
- **Status:** Implemented

> **Plan contract:** this is the implementation strategy. Unlike the spec, this
> document is allowed to change as you learn. When it changes substantially (a
> different approach, not just a re-ordering), note why in the changelog at the
> bottom.

## Approach

Add explicit max input/output token fields to the existing provider/model
configuration path without adding a new registry or discovery layer. First,
define shared backend defaults and compatibility aliases. Then thread effective
limits through agent storage, settings storage, runtime executor setup, and UI
forms. Finally, update tests and docs surfaces that assert the old smaller
defaults.

## Constraints

- Follow [RFC-0008](../../rfc/0008-simplified-provider-model-configuration.md):
  defaults plus user/provider overrides, not a broad maintained model catalog.
- Preserve legacy `maxTokens` as max output and provider `contextWindow` as max
  input.
- Keep OpenAI-compatible request payload behavior unchanged.
- Keep provider credentials, rate limits, default provider, and provider testing
  behavior unchanged.

## Construction tests

**Integration tests:**
- `cargo test -p gateway-services simplified_provider_model_configuration`
- `cargo test -p gateway-execution simplified_provider_model_configuration`
- `npm test -- --run AgentEditPanel WebSettingsPanel`

**Manual verification:**
- Open Agents and confirm create/edit forms show max input/output fields.
- Open Settings > Advanced and confirm each provider/model slot exposes max
  input/output fields with 200k/32k defaults or inherited values.

## Tasks

### T1: Backend config reads legacy and explicit token limit fields

**Depends on:** none

**Touches:** `gateway/gateway-services/src/agents.rs`,
`gateway/gateway-services/src/settings.rs`, `gateway/gateway-services/src/models.rs`,
`gateway/gateway-services/src/providers.rs`

**Tests:**
- TDD: deserializing an agent with only `maxTokens` produces
  `max_output_tokens == 32000` fallback only when `maxTokens` is absent.
- TDD: deserializing settings with explicit `maxInputTokens` and
  `maxOutputTokens` uses those values.
- TDD: unknown-model fallback returns `200000` input and `32000` output.

**Approach:**
- Introduce shared default functions/constants for max input/output tokens.
- Add `maxInputTokens` and `maxOutputTokens` fields to agent and settings
  structs.
- Keep `maxTokens` in serialized responses as a compatibility alias for max
  output where needed.

**Done when:** backend config can read old and new token-limit shapes.

### T2: Runtime executor uses effective input/output limits

**Depends on:** T1

**Touches:** `gateway/gateway-execution/src/invoke/executor.rs`,
`gateway/gateway-execution/src/invoke/setup.rs`,
`runtime/agent-tools/src/tools/agent.rs`

**Tests:**
- TDD: executor setup uses an agent's explicit max input as
  `context_window_tokens`.
- TDD: executor setup uses an agent's explicit max output as LLM max tokens.
- TDD: provider/model lower `maxOutput` still clamps requested output.

**Approach:**
- Resolve effective max input/output once during executor setup.
- Map create-agent tool `maxTokens` to max output while adding explicit
  max input/output parameters.
- Replace unknown-model `8192` runtime fallback with the `200000` default.

**Done when:** runtime calls can execute unknown models with 200k/32k defaults
and still clamp provider-specific lower output limits.

### T3: HTTP and transport types expose explicit fields

**Depends on:** T1

**Touches:** `gateway/src/http/agents.rs`, `gateway/src/http/openapi.yaml`,
`apps/ui/src/services/transport/types.ts`

**Tests:**
- TDD/goal-based: focused API tests prove create/update responses include
  `maxInputTokens`, `maxOutputTokens`, and legacy `maxTokens`.
- Goal-based: TypeScript typecheck accepts the new request/response fields.

**Approach:**
- Add request/response fields for max input/output.
- Continue accepting and returning `maxTokens` during transition.
- Update OpenAPI schema enough to keep generated/consumer docs honest.

**Done when:** backend and frontend transport agree on explicit token fields.

### T4: Agent UI exposes max input/output controls

**Depends on:** T3

**Touches:** `apps/ui/src/features/agent/AgentEditPanel.tsx`,
`apps/ui/src/features/agent/WebAgentsPanel.tsx`,
`apps/ui/src/features/setup/steps/AgentsStep.tsx`,
`apps/ui/src/features/setup/steps/ReviewStep.tsx`,
`apps/ui/src/features/setup/SetupWizard.tsx`

**Tests:**
- Component tests update old small max output expectations where the new
  default applies.
- Component tests prove edited payloads include max input and max output token
  fields.

**Approach:**
- Add max input and max output form controls beside model controls.
- Keep labels clear: "Max Input Tokens" and "Max Output Tokens".
- Preserve legacy display of existing max output values.

**Done when:** users can set both token limits during agent creation/editing and
setup.

### T5: Advanced settings exposes max input/output controls

**Depends on:** T3

**Touches:** `apps/ui/src/features/settings/WebSettingsPanel.tsx`,
`apps/ui/src/features/settings/WebSettingsPanel.test.tsx`

**Tests:**
- Component tests prove orchestrator and multimodal defaults are 200k/32k unless
  existing values override them.
- Component tests prove distillation/curator/intent analysis can persist
  explicit token overrides.

**Approach:**
- Add max input/output controls to orchestrator and multimodal cards.
- Add compact max input/output controls to inherited model slots:
  distillation, curator, and intent analysis.
- Make unset task-slot limits inherit from orchestrator.

**Done when:** every Advanced provider/model slot exposes the override fields.

### T6: Gates and docs are updated

**Depends on:** T1-T5

**Touches:** `docs/specs/README.md`, tests touched by T1-T5

**Tests:**
- Goal-based: `cargo check --workspace`.
- Goal-based: targeted Rust and UI tests from this plan pass.
- Goal-based: stale-default search returns no old small user-facing
  provider/model default docs for changed surfaces.

**Approach:**
- Update specs README.
- Run format/type/test gates.
- Fix stale tests and comments in touched areas only.

**Done when:** code and docs agree on the simplified provider/model contract.

## Rollout

Ship as a backward-compatible config change. Existing settings and agent files
continue to read. New values are written when the user saves an edited agent or
settings page.

## Risks

- The UI touches several setup/settings/agent tests that encode the old 4096 or
  16384 defaults.
- Some providers may reject 32k output for a model without provider metadata;
  users can lower output limits and provider/model overrides still clamp.
- Missing an inherited Advanced slot could make runtime behavior differ from
  visible settings.

## Changelog

- 2026-06-15: initial plan from accepted RFC-0008.
- 2026-06-15: implemented backend defaults/aliases, runtime effective token
  limits, HTTP/OpenAPI fields, and UI controls for agent/setup/Advanced model
  slots.
