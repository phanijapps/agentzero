# Spec: Agent Commissioning

- **Status:** Shipped
- **Owner:** phanijapps
- **Plan:** [`plan.md`](plan.md)
- **Constrained by:** [`RFC-0011`](../../rfc/0011-engram-memory-engine-cutover.md); [`RFC-0012`](../../rfc/0012-engram-upstream-risk-reduction.md)
- **Brief:** none
- **Contract:** [`contracts/openapi/commissioning.yaml`](../../../contracts/openapi/commissioning.yaml)
- **Shape:** mixed

> **Spec contract:** this document defines what "done" means. The implementing
> PR must match this spec, or update it. Verification must be derivable from it.

## Objective

Replace the current six-step setup wizard with a single first-run Agent
Commissioning flow. A new user chooses a purpose and domain, verifies one
cloud or local model, provides a minimal working profile plus a local-only
personal profile, and receives a configured root agent. Commissioning does
not present a boundaries questionnaire: it uses safe defaults and plainly
warns that the agent is autonomous, guarded, and not sandboxed for execution.
The gateway owns durable readiness state and configuration; the browser does
not decide completion. Commissioning persists a portable semantic profile
containing the base taxonomy/ontology pack IDs and selected domain packs. A
later version may materialize local governance definition files while the
reported `deferred` state continues to mean that Engram import/bootstrap has
not run; commissioning does not import, start, or call Engram. Existing configured installations keep
working without being forced through the new flow; incomplete legacy installs
receive a focused recovery flow. The old setup UI and setup-only endpoints are
deleted at cutover.

## Boundaries

### Always do

- Require a primary purpose and at least one confirmed domain before a user can
  commission an agent; defaults may be accepted but are never silently skipped.
- Verify the selected provider and model before persisting a completed
  commission. Local failures must name a recoverable state such as unavailable
  runtime, stopped runtime, or no installed model.
- Offer only known OpenAI-compatible cloud presets in commissioning; providers
  needing custom authentication or endpoints remain an explicit Settings task.
- Persist the commissioning profile, selected semantic pack IDs, privacy
  setting, and autonomy setting in zbot-owned settings without any Engram type
  or direct database dependency. The user cannot override these safe defaults
  during commissioning.
- Require a user name and at least one interest; accept hobbies and date of
  birth only as optional local profile details. Keep those details out of
  status responses, `SOUL.md`, and commissioning-created model context.
- Give a clear, friendly warning that z-Bot is autonomous with safety
  guardrails, and that execution is not sandboxed yet.
- Keep current provider storage in `providers.json` for this cutover, while
  ensuring the new commissioning endpoints never echo API keys or include them
  in error messages or logs.
- Migrate a fully usable legacy configuration to complete without interrupting
  the user; route only incomplete configurations to recovery commissioning.
- Use a safe default: guided autonomy, no automatic MCP/skill activation, no
  automatic background task, and no automatic third-party software install.

### Ask first

- Moving provider credentials from `providers.json` to an OS credential broker
  or changing the existing provider API's credential representation.
- Installing, starting, or downloading local model software without an
  explicit action in the UI.
- Making Engram import/bootstrap a commissioning completion gate once Engram
  integration is available. Local definition-file materialization alone does
  not change the semantic profile's `deferred` state.
- Expanding commissioning to import MCP servers, connectors, or skills by
  default.

### Never do

- Never import `engram-*` crates, invoke `zbot-engram-adapter`, or open a
  semantic database from commissioning code.
- Never accept arbitrary provider URLs in commissioning; it supports known
  presets and the fixed local runtime endpoint only. Custom endpoints remain a
  Settings action.
- Never return, log, persist outside `providers.json`, or put an API key into a
  UI error, status payload, OpenAPI example, or test fixture.
- Never copy personal-profile fields into `SOUL.md`, a commissioning status
  response, or a model request as part of commissioning.
- Never let an unavailable readiness check fail open into the application for a
  new or incomplete installation.
- Never accept a browser commissioning completion command from a non-local
  origin, even when broad CORS is enabled for other gateway development paths.
- Never retain the old setup wizard, its browser completion flag, or
  setup-only endpoints after the cutover compatibility redirect is in place.

## Verification approach

- Gateway configuration state, legacy classification, provider selection, and
  error-code mapping: **TDD**, because the states are finite and must never
  report completion prematurely.
- Commissioning REST contract and route wiring: **TDD plus goal-based gateway
  tests**, proving request/response redaction, route registration, and
  recovery behavior.
- React selection, validation, and route behavior: **component tests plus
  visual/manual QA**, because the user-visible journey matters more than
  internal reducer state.
- Legacy cleanup: **goal-based repository checks**, proving no production UI
  imports the old setup feature and no route calls the retired setup endpoints.

## Acceptance criteria

- [x] Given an empty data directory, a user is routed to `/commission` and sees
  the commissioning flow rather than the legacy wizard.
- [x] A user cannot continue past the personal-focus step until they choose a
  primary purpose and confirm at least one suggested domain; accepting a
  suggested default is valid.
- [x] A cloud provider is tested before it is persisted, and commissioning
  completion configures it as the default provider with the selected model.
- [x] Local setup gives a distinct, actionable result for unavailable runtime,
  stopped/unreachable runtime, and no available model; it does not create then
  delete a phantom provider.
- [x] A successful commission persists the root agent display name, working
  profile, safe server-owned autonomy/privacy defaults, selected provider/model, and a
  versioned semantic profile with `zbot.base:v1`, `zbot.general:v1`, and the
  selected domain pack IDs.
- [x] The personal profile requires a name and at least one interest, accepts
  hobbies and date of birth as optional values, and retains them only in local
  z-Bot settings rather than status payloads or agent working instructions.
- [x] The final commissioning screen explains autonomous behavior and safety
  guardrails, prominently warning that execution is not sandboxed yet.
- [x] The commissioning API returns redacted status and typed error codes; API
  keys are absent from responses, logs, errors, OpenAPI examples, and tests.
- [x] Commissioning has no compile-time or runtime dependency on Engram or an
  Engram storage backend; it records portable semantic intent only.
- [x] An existing install with a verified default provider, selected model, and
  root configuration is classified as complete without forcing a new flow; an
  incomplete legacy install is directed to recovery commissioning.
- [x] `/setup` remains a bookmark-safe redirect to `/commission`, while the
  previous setup components, sessionStorage completion flag, and setup-only
  API routes are removed.

## Testing Strategy

- Technical: at planning time, the legacy setup UI was a six-step React feature
  gated by `SetupGuard` and recorded browser completion in `sessionStorage`.
  Those files were removed at cutover.
- Technical: current `ProviderService` serializes a raw `apiKey` in
  `providers.json`; this feature retains that user-approved storage only for
  the current cutover (source: `gateway/gateway-services/src/providers.rs`;
  user confirmation 2026-07-11).
- Technical: the existing Engram adapter idempotently bootstraps
  `zbot.base:v1` and `zbot.general:v1`, but commissioning must not depend on
  it while the upstream integration API is being completed (source:
  `stores/zbot-engram-adapter/src/governance/bootstrap.rs`; user confirmation
  2026-07-11).
- Product: taxonomy and ontology choices are personalized during first-run
  setup, while their actual semantic provisioning stays independently
  consumable by Engram (source: user confirmation 2026-07-11).
- Product: existing complete configurations remain usable and incomplete ones
  recover through commissioning (source: conservative migration required to
  retire the legacy wizard without disrupting users, 2026-07-11).
- Process: the new REST surface is documented in
  `contracts/openapi/commissioning.yaml`; no `api-contract` skill is installed,
  so the contract is hand-authored and verified by focused tests (source:
  available-skills roster; `docs/CONVENTIONS.md`).
