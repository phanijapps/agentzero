# Spec: Simplified Provider Model Configuration

- **Status:** Shipped
- **Owner:** phanijapps
- **Plan:** [`plan.md`](plan.md)
- **Constrained by:** [RFC-0008: Simplified Provider Model Configuration](../../rfc/0008-simplified-provider-model-configuration.md)

> **Spec contract:** this document defines what "done" means. The implementing
> PR must match this spec, or update it. Verification must be derivable from it.

## Objective

Make provider/model setup in z-Bot default to a simple `200000` maximum input
tokens and `32000` maximum output tokens contract while letting users override
those values wherever they choose a model. Users should be able to set provider,
model, max input tokens, and max output tokens per agent and in Advanced
settings for system-purpose model slots without depending on a large maintained
model capability catalog.

## Boundaries

The three-tier guard that keeps an implementing agent inside the lines.
*Always do* applies without asking; *Ask first* requires human sign-off before
proceeding; *Never do* is a hard rule, even under time pressure.

### Always do

- Preserve existing `maxTokens` config as a readable alias for max output
  tokens.
- Preserve existing provider `contextWindow` config as a readable alias for max
  input tokens.
- Use `200000` max input tokens and `32000` max output tokens whenever no more
  specific agent, Advanced, provider, or model override exists.
- Allow unknown/free-form model names to execute with the default limits.

### Ask first

- Adding provider-specific live discovery beyond current provider model testing.
- Removing existing provider fields from persisted config instead of reading
  them compatibly.
- Changing provider credential, default provider, rate-limit, or connection-test
  behavior.

### Never do

- Never introduce a new maintained exhaustive model catalog to replace the old
  registry.
- Never block a custom model solely because z-Bot lacks capability metadata for
  it.
- Never hard-migrate or rewrite user agent/settings files unless the user saves
  that agent or settings surface.
- Never change the OpenAI-compatible wire protocol as part of this feature.

## Testing Strategy

- Limit-resolution behavior: **TDD**. Defaults, legacy aliases, provider/model
  overrides, and runtime clamping are compact invariants and should have focused
  Rust tests.
- Backend schema compatibility: **TDD**. Agent and settings deserialization
  should prove old `maxTokens` / `contextWindow` data still reads while new
  `maxInputTokens` / `maxOutputTokens` reads correctly.
- UI type and render wiring: **goal-based check plus focused component tests**.
  Existing React tests should be updated where they assert defaults or form
  payloads; `npm test`/targeted Vitest should catch type and payload regressions.
- End-to-end build gates: **goal-based check**. `cargo check --workspace` and
  targeted UI tests must pass before handoff.

## Acceptance Criteria

- [x] Unknown models resolve to `200000` max input tokens and `32000` max output
  tokens by default.
- [x] Existing agent `maxTokens` values are still accepted as max output tokens.
- [x] Existing provider `contextWindow` values are still accepted as max input
  tokens.
- [x] Agent create/edit APIs expose and persist max input and max output token
  overrides.
- [x] Agent UI creation/editing exposes max input and max output token fields,
  defaulted to `200000` and `32000`.
- [x] Advanced settings exposes max input and max output token fields for
  orchestrator, distillation, curator, intent analysis, and multimodal model
  slots.
- [x] Runtime executor context-window budgeting uses the effective max input
  tokens instead of falling back to `8192` for unknown models.
- [x] Runtime LLM max output uses the effective max output tokens and still
  clamps when a provider/model-specific lower max output is configured.
- [x] Provider cards/details do not require model capability metadata to show a
  usable provider/model configuration.

## Assumptions

- Technical: provider records already support `contextWindow`, `defaultModel`,
  and per-model `maxInput` / `maxOutput` overrides (source:
  `gateway/gateway-services/src/providers.rs`).
- Technical: runtime executor setup already resolves output clamping and
  context-window tokens before building the executor (source:
  `gateway/gateway-execution/src/invoke/executor.rs`).
- Technical: the agent editor and Advanced settings are the current UI surfaces
  for model selection (source: `apps/ui/src/features/agent/AgentEditPanel.tsx`;
  `apps/ui/src/features/settings/WebSettingsPanel.tsx`).
- Process: RFC-0008 is accepted and constrains this implementation (source:
  user approval 2026-06-15).
- Product: users want simplified defaults with override controls at agent and
  Advanced level, not a large maintained model/capability list (source: user
  confirmation 2026-06-15).
