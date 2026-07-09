# RFC-0008: Simplified Provider Model Configuration

- **Status:** Accepted
- **Author:** phanijapps
- **Approver:** phanijapps
- **Date opened:** 2026-06-14
- **Date closed:** 2026-06-15
- **Related:** `apps/ui/src/features/settings/providerPresets.ts`; `apps/ui/src/features/agent/AgentEditPanel.tsx`; `apps/ui/src/features/settings/WebSettingsPanel.tsx`; `gateway/gateway-services/src/providers.rs`; `gateway/gateway-services/src/models.rs`; `gateway/gateway-execution/src/invoke/executor.rs`

## The ask

Approve a simpler provider/model configuration contract:

- z-Bot keeps a small provider preset list, matching the providers already
  exposed in the UI.
- z-Bot stops maintaining a large authoritative model capability catalog as the
  happy-path source of truth.
- Unknown or unconfigured models default to `200000` maximum input tokens and
  `32000` maximum output tokens.
- Users can override model, maximum input tokens, and maximum output tokens at
  the agent level and in Advanced settings.

The current implementation has provider presets, per-provider models, a bundled
model registry, per-agent max output tokens, root/orchestrator settings,
task-specific Advanced routing, and multiple hardcoded fallbacks. The
complication is that this makes configuration feel heavier than the user
decision actually is: "which provider/model should this agent use, and what
token limits should z-Bot assume?" The question is whether to make that decision
explicit at the agent and Advanced levels while using broad defaults everywhere
else.

Decisions requested:

1. Use the current zbot provider presets as the curated provider list.
   Recommended: accept. Default if no objection by 2026-06-21: keep provider
   presets as the product-maintained provider list.
2. Replace the broad maintained model/capability catalog as the primary source
   of model limits with defaults plus user/provider overrides. Recommended:
   accept. Default if no objection by 2026-06-21: use `200000` input and
   `32000` output for unknown models.
3. Add explicit max input and max output controls where users choose models:
   per-agent configuration and Advanced settings. Recommended: accept. Default
   if no objection by 2026-06-21: add both fields to those surfaces.
4. Keep compatibility for existing config. Recommended: accept. Default if no
   objection by 2026-06-21: read existing `maxTokens` as max output tokens and
   existing provider `contextWindow` as max input tokens.

## Problem & goals

The current provider/model setup asks the product to maintain more model detail
than is needed for normal configuration. It also spreads token defaults across
several layers:

- Provider records support `models`, `defaultModel`, `contextWindow`, and
  per-model `modelConfigs` with `maxInput` and `maxOutput`.
- The old catalog approach maintained capability flags and context limits for
  many model IDs.
- Runtime execution clamps output tokens and chooses context-window budgets from
  provider overrides, the model registry, or fallback values.
- The agent editor exposes provider, model, and max output tokens, but not max
  input tokens.
- Advanced settings expose root/orchestrator and task-specific model routing,
  but only some slots have output-token controls.
- Setup and default-agent paths still carry smaller historic defaults such as
  `4096`, `8192`, and `16384`.

This creates two concrete problems:

- The model catalog becomes stale as providers release and rename models.
- Users cannot correct the most important runtime assumption, max input tokens,
  in the places where they choose a model.

Goals:

- Make the default model limit assumption simple and visible:
  `maxInputTokens = 200000`, `maxOutputTokens = 32000`.
- Let each agent override provider, model, max input tokens, and max output
  tokens.
- Let Advanced settings override provider, model, max input tokens, and max
  output tokens for root/orchestrator and system-purpose model slots.
- Keep the provider list aligned with the current zbot presets.
- Preserve OpenAI-compatible provider support and custom model entry.
- Preserve runtime safety by continuing to clamp output tokens when a more
  specific provider/model override is configured.
- Keep existing user config readable without a breaking migration.

Non-goals:

- No attempt to maintain an exhaustive, always-current model database.
- No automatic provider-specific capability detection in V1.
- No removal of provider credentials, default provider selection, rate limits,
  or provider connection testing.
- No change to the LLM wire protocol in this RFC.
- No hard migration that rewrites all user agent configs immediately.

## Proposal

Use a layered model configuration contract with a simple fallback:

```text
Effective model settings for a call:

providerId:
  agent override
  -> Advanced purpose override
  -> default provider

model:
  agent override
  -> Advanced purpose override
  -> provider.defaultModel
  -> provider.models[0]
  -> free-form user-entered model

maxInputTokens:
  agent override
  -> Advanced purpose override
  -> provider.modelConfigs[model].maxInput
  -> provider.contextWindow
  -> global default 200000

maxOutputTokens:
  agent override
  -> Advanced purpose override
  -> provider.modelConfigs[model].maxOutput
  -> legacy maxTokens where present
  -> global default 32000
```

### Provider Presets

Keep the current preset list as the curated product list:

- OpenAI
- Anthropic
- Ollama Cloud
- Ollama Local
- Google Gemini
- DeepSeek
- OpenRouter
- Z.AI
- Mistral
- Azure OpenAI

The preset list may include a few starter models per provider for convenience,
but those model lists are examples, not an authoritative capability registry.
Users can still type a model name not present in the suggestions.

### Model Limits

Introduce explicit names for new configuration:

```json
{
  "providerId": "provider-openai",
  "model": "gpt-4.1",
  "maxInputTokens": 200000,
  "maxOutputTokens": 32000
}
```

Compatibility rules:

- Existing `maxTokens` is read as `maxOutputTokens`.
- Existing provider `contextWindow` is read as `maxInputTokens`.
- Existing provider `modelConfigs.<model>.maxInput` and `maxOutput` remain valid
  and continue to take precedence over defaults.
- Serializers may continue writing `maxTokens` during a transition if needed,
  but new UI labels and docs should call the value "Max Output Tokens".

The old catalog can be reduced to a fallback-only compatibility layer with
unknown-model defaults.

It should not be treated as a product promise that z-Bot knows every current
model and capability.

### Agent Level

In agent creation/editing, expose:

- Provider
- Model
- Max Input Tokens
- Max Output Tokens
- Temperature
- Thinking Enabled

Provider/model suggestions come from the selected provider's model list.
Unknown models are allowed. If a selected model has no configured limits, the UI
shows `200K input / 32K output` as the assumed default.

### Advanced Settings

In Advanced settings, expose max input and max output tokens for every model
slot that can choose provider/model:

- Root/orchestrator
- Distillation
- Curator
- Intent analysis
- Multimodal
- Any future system-purpose LLM slot

Slots may still inherit provider/model from the orchestrator. Token limits
should inherit the same way unless explicitly overridden:

```text
Distillation maxInput/maxOutput unset
  -> inherit orchestrator limits
  -> use provider/model override
  -> use 200000/32000 fallback
```

### Runtime Behavior

Runtime should resolve effective limits before constructing the executor:

- `context_window_tokens` should use the effective max input tokens.
- LLM `max_tokens` should use the effective max output tokens.
- If provider/model-specific max output is lower than the requested output,
  clamp to that lower value and log the clamp.
- Unknown models should execute with the default limits rather than falling back
  to `8192`.
- Thinking mode remains user-driven. If a provider rejects a thinking payload,
  the normal provider error path reports it.

### Migration

No hard migration is required for V1.

Read path:

- Read `maxInputTokens` first.
- Fall back to provider `contextWindow`.
- Fall back to model registry input if the compatibility registry remains.
- Fall back to `200000`.
- Read `maxOutputTokens` first.
- Fall back to legacy `maxTokens`.
- Fall back to provider/model `maxOutput`.
- Fall back to compatibility registry output if present.
- Fall back to `32000`.

Write path:

- New or edited configs should prefer `maxInputTokens` and `maxOutputTokens`.
- During transition, APIs may include both `maxOutputTokens` and `maxTokens`
  where frontend/backend compatibility requires it.
- Existing config files should not be rewritten unless the user saves that
  agent or settings page.

## Options considered

The option space is MECE along the axis of who owns model metadata.

| Option | Description | Trade-off |
| --- | --- | --- |
| Do nothing | Keep presets, bundled model registry, current UI fields, and current defaults. | Lowest implementation cost, but preserves stale catalog pressure and hides max input configuration from users. |
| Full bundled registry | Invest in a larger authoritative model catalog with capabilities and limits for every known model. | Better display metadata when current, but ongoing maintenance cost is high and the list will lag providers. |
| Provider-discovered metadata only | Depend on provider `/models` or provider-specific metadata endpoints. | Good when metadata exists, but OpenAI-compatible providers vary widely and some only expose model IDs. |
| Defaults plus user/provider overrides | Use broad defaults, preserve optional overrides, and put controls where users choose models. | Recommended. Simple, works for custom providers, and keeps safety where users need it. |

Prior art supports the recommended option:

- OpenAI's model list API returns model identifiers and ownership metadata, not
  a universal capability/limit schema.
- OpenRouter exposes richer model metadata such as context length and max
  completion tokens, showing provider metadata can help when available but
  should not be assumed everywhere.
- LiteLLM models this as configured model-specific settings plus optional model
  info, rather than requiring every provider/model to fit one static product
  catalog.

## Risks & what would make this wrong

Pre-mortem:

- **Some providers reject `32000` output tokens.** Mitigation: keep clamp logic
  for provider/model-specific `maxOutput`, surface errors clearly, and let users
  lower output limits at agent or Advanced level.
- **A 200k input assumption delays compaction for smaller local models.**
  Mitigation: make max input editable anywhere a model is chosen, and keep
  provider-level overrides for Ollama/local models.
- **Removing capability badges makes setup feel less guided.** Mitigation: keep
  capability display only when metadata is available from provider/user
  overrides; do not block unknown models because a badge is missing.
- **Two field names create confusion during transition.** Mitigation: UI labels
  say "Max Output Tokens"; code reads legacy `maxTokens`; new docs use
  `maxOutputTokens`.

Key assumptions:

- Most users benefit more from a working default plus easy override than from a
  comprehensive model capability table.
- `200000` input and `32000` output are reasonable defaults for a modern
  agent-oriented product, provided users can lower them.
- Runtime safety comes from clamping and editable overrides, not from a static
  catalog that must be kept current.
- Provider/model capability metadata is helpful display information, but it
  should not be a hard prerequisite for model use.

Drawbacks:

- The UI may show less rich capability metadata for unknown or newly released
  models.
- Some bad model/limit combinations will be discovered at provider-call time
  instead of prevented by registry validation.
- Implementers must touch several surfaces because the old defaults are spread
  across backend services, runtime setup, setup wizard, agent editor, settings
  UI, tests, and templates.

## Evidence & prior art

Spike / de-risk result:

- Provider records already have `contextWindow`, `defaultModel`, and
  `modelConfigs` with `maxInput` and `maxOutput`.
- The current registry fallback already uses `200000` input tokens, proving a
  broad unknown-model fallback is compatible with current code shape.
- Runtime output clamping and context-window resolution already happen in
  executor construction, so the implementation can simplify data sources
  without removing the safety mechanism.
- Unknown-model thinking is already trusted by executor tests, which supports
  the principle that unknown models should not be blocked by missing registry
  metadata.

Repo precedent:

- Provider presets are defined in
  `apps/ui/src/features/settings/providerPresets.ts`.
- Provider model overrides are defined in
  `gateway/gateway-services/src/providers.rs`.
- Fallback token defaults are defined in
  `gateway/gateway-services/src/models.rs`; there is no maintained
  `models.json` catalog.
- Runtime clamping and context-window resolution live in
  `gateway/gateway-execution/src/invoke/executor.rs`.
- Agent model editing lives in
  `apps/ui/src/features/agent/AgentEditPanel.tsx`.
- Advanced model routing lives in
  `apps/ui/src/features/settings/WebSettingsPanel.tsx`.
- Existing docs/specs show runtime context management is an important safety
  boundary: `docs/specs/runtime-context-control/spec.md`.

External prior art:

- [OpenAI list models API](https://developers.openai.com/api/reference/resources/models/methods/list/)
  returns model identifiers and basic ownership metadata.
- [LiteLLM proxy config](https://docs.litellm.ai/docs/proxy/configs) supports
  model-specific settings such as API base, API key, temperature, and max
  tokens.
- [LiteLLM model management](https://docs.litellm.ai/docs/proxy/model_management)
  separates model info from sensitive credentials and allows additional model
  metadata.
- [OpenRouter models API](https://openrouter.ai/docs/api/api-reference/models/get-models)
  can return context length and max completion tokens for models.
- [OpenRouter parameters](https://openrouter.ai/docs/api/reference/parameters)
  documents output token limits as bounded by remaining context.
- [Anthropic context window docs](https://platform.claude.com/docs/en/build-with-claude/context-windows)
  describe input and output tokens as part of the model context-window behavior,
  with provider/model-specific details.

## Open questions

None for V1. The recommended default is to preserve legacy config reads, write
clearer field names on newly saved settings, and avoid a hard migration.

## Follow-on artifacts

- Spec: `docs/specs/simplified-provider-model-configuration/`
- ADR: record the replacement of the broad maintained model registry with
  defaults plus user/provider overrides if this RFC is accepted.
- Docs: update setup/settings/provider documentation once the implementation
  lands.
