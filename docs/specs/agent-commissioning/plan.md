# Plan: Agent Commissioning

- **Spec:** [`spec.md`](spec.md)
- **Status:** Done

> **Plan contract:** this is the implementation strategy. Unlike the spec, this
> document is allowed to change as we learn.

## Approach

Build the gateway-owned commissioning state and REST contract first, using
existing provider, agent, and settings services rather than a second browser
orchestration layer. Then replace the setup route with a focused React flow
that renders the approved mock-up's information hierarchy. Commissioning saves
portable semantic intent but has no Engram dependency. After a fresh-data and
legacy-data journey passes, delete the current wizard and leave `/setup` as a
small route redirect only.

## Constraints

- [`RFC-0011`](../../rfc/0011-engram-memory-engine-cutover.md) and
  [`RFC-0012`](../../rfc/0012-engram-upstream-risk-reduction.md): Engram stays
  behind zbot boundaries; this feature must not introduce a dependency on it.
- Current provider credential storage remains `providers.json` by explicit
  user direction. The new commissioning surface still redacts credentials and
  must not broaden existing provider API behavior.
- The existing gateway is local and has no API authentication. The new routes
  keep same-origin/default CORS behavior and do not introduce network exposure.

## Construction tests

**Integration tests:**

- Commissioning completion against a temporary vault: verified cloud candidate
  produces default provider, root configuration, semantic profile, and complete
  status without an Engram instance.
- Legacy configuration fixtures classify complete and incomplete states without
  modifying provider credentials.
- Local diagnostic fixtures cover unavailable runtime, unreachable runtime, no
  model, and ready-to-commission states.

**Manual verification:**

- Start with a new data directory and complete a cloud-provider commission.
- Start with Ollama absent or stopped and confirm the UI provides a bounded
  recovery action rather than a generic provider error.
- Open `/setup` after cutover and confirm it redirects to `/commission`.

## Design (LLD)

### Design decisions

- Add a versioned `CommissioningSettings` section under the existing gateway
  settings model. It carries only user choices and readiness state; API keys
  remain in the existing provider file.
- Use one `POST /api/commissioning/complete` command for the final write. It
  tests the selected candidate, applies provider/root/settings changes in a
  retry-safe order, and marks complete last. Partial failure returns an
  actionable state and never claims success.
- Persist `SemanticProfile` as data (`base_pack_ids`, `domain_pack_ids`,
  policy version), not as an Engram call. An integration built later consumes
  the same configuration without changing the commissioning contract.
- Use known cloud presets and a fixed local runtime endpoint to avoid turning
  setup into an arbitrary outbound request surface.

### Data & schema

- `CommissioningSettings`: schema version, state, primary focus, confirmed
  domains, working profile, local-only user profile, server-owned safe
  autonomy/privacy defaults, selected provider/model IDs, and `SemanticProfile`.
- `SemanticProfile`: `zbot.base:v1`, `zbot.general:v1`, selected domain pack
  IDs, and a provisioning state of `deferred`; it contains no Engram IDs or
  database path.
- The REST shape is specified in
  `contracts/openapi/commissioning.yaml` and mirrored in the typed Rust and
  TypeScript transport DTOs.

### Interfaces & contracts

- `GET /api/commissioning/status` returns durable readiness and a redacted
  recovery hint.
- `POST /api/commissioning/local/diagnose` probes only the fixed local runtime
  and returns typed local states.
- `POST /api/commissioning/complete` accepts a known provider preset or local
  model selection, validates it, and applies the commission. `apiKey` is
  write-only and absent from every response.

### Component / module decomposition

- `gateway/gateway-services/src/settings.rs`: commissioning state and legacy
  classification helpers.
- `runtime/agent-tools/src/tools/mod.rs`: make the existing fresh-install tool
  settings default honour its documented safe offload values.
- `gateway/src/http/commissioning.rs`: request DTOs, redacted error mapping,
  completion orchestration, and local diagnostic handler.
- `apps/ui/src/features/commissioning/*`: focused selection screens and route
  guard; no direct configuration file access.
- `apps/ui/src/services/transport/*`: typed client methods only.
- `apps/ui/src/App.tsx`: `/commission` ownership and `/setup` redirect.

### State & control flow

1. The guard asks the gateway for durable commissioning status.
2. A new or incomplete installation opens `/commission`; a migrated/complete
   installation enters the application shell.
3. The user confirms focus/domain, then verifies cloud or local intelligence.
4. The user provides profile and accepts safe defaults.
5. Completion revalidates the provider, writes configuration, marks state
   complete last, and returns a redacted summary.
6. A later Engram integration reads the portable semantic profile and performs
   provisioning independently.

### Behavior & rules

- Guided autonomy and local-preferred privacy are server-owned initial defaults;
  the UI does not ask the user to configure boundaries. It instead warns that
  z-Bot is autonomous with safety guardrails and its execution is not sandboxed.
  Tool servers, plugins, skills, automation, and background work are not
  enabled during commissioning.
- A provider failure keeps the current selection editable and supplies a typed
  recovery action. Local failures never create persistent provider records.
- Existing user configuration is inspected, not overwritten, during legacy
  classification.

### Failure, edge cases & resilience

- Status-check failure renders a retryable blocking readiness screen rather
  than silently entering the app.
- Completion is idempotent by provider preset/local identity and only marks
  complete after all writes succeed. A retry must not duplicate providers.
- Provider/network errors use stable code plus safe user text; raw response
  bodies, URLs with credentials, and underlying error chains stay server-side.
- Semantic provisioning is explicitly `deferred` and never blocks the flow.

### Quality attributes (NFRs)

- Security: commissioning rejects arbitrary URLs, API keys are write-only, and
  no completion response or test fixture contains a credential. Browser writes
  additionally require a local z-Bot origin; native/CLI clients may omit it.
- Accessibility: all selection cards have keyboard semantics, visible focus,
  labels, and error announcements.
- Maintainability: no new dependency and no Engram import; the profile is a
  small versioned settings DTO.

### Dependencies & integration

- Reuse existing provider, agent, settings, and local HTTP client facilities.
- Do not add an Engram, secrets-vault, or operating-system installer dependency
  in this cutover.

## Tasks

### T1: Commissioning settings and REST contract describe a complete, portable profile

**Depends on:** none

**Touches:** `contracts/openapi/commissioning.yaml`,
`gateway/gateway-services/src/settings.rs`,
`gateway/gateway-services/src/settings_tests.rs` or the existing settings test
module, `docs/specs/agent-commissioning/*`

**Tests:**

- TDD: a default settings document reports `not_started` and emits the base
  semantic packs without an Engram dependency. Verifies AC5 and AC7.
- TDD: a complete legacy fixture and an incomplete legacy fixture classify
  deterministically without changing their provider data. Verifies AC8.
- Goal-based: the OpenAPI document parses and every API key field is
  `writeOnly`. Verifies AC6.

**Approach:**

- Define the versioned commissioning and semantic-profile DTOs in the existing
  settings layer.
- Add a side-effect-free legacy classifier over existing settings, providers,
  and root-agent configuration.
- Hand-author the REST OpenAPI contract because no contract-authoring skill is
  installed; link it back to this spec.

**Done when:** unit fixtures prove deterministic state/profile output and the
contract covers status, local diagnostics, and completion without credential
responses.

### T2: Gateway completes a commission with typed, redacted provider diagnostics

**Depends on:** T1

**Touches:** `gateway/src/http/commissioning.rs`, `gateway/src/http/mod.rs`,
`gateway/gateway-services/src/providers.rs`, gateway HTTP tests

**Tests:**

- TDD: cloud completion tests a known preset before writing it, then marks the
  commissioned provider default and complete. Verifies AC3 and AC5.
- TDD: each local diagnostic state maps to an actionable code and creates no
  provider record. Verifies AC4.
- TDD: completion failure leaves status incomplete/needs-attention and all
  returned/loggable DTOs omit API-key material. Verifies AC6.
- Goal-based: the new routes are registered and match the OpenAPI operation
  names. Verifies AC1 and AC6.

**Approach:**

- Add the redacted handlers and completion command; route known presets to the
  existing provider service only after validation succeeds.
- Configure the root agent and execution settings in retry-safe order, marking
  commissioning complete last.
- Use bounded error-code mapping for local and cloud failures; do not return
  raw provider response text.

**Done when:** gateway tests demonstrate successful, recoverable, and redacted
completion without any Engram store or runtime.

### T3: Transport and commissioning guard use server-owned readiness

**Depends on:** T2

**Touches:** `apps/ui/src/services/transport/interface.ts`,
`apps/ui/src/services/transport/http.ts`, `apps/ui/src/services/transport/types.ts`,
`apps/ui/src/features/commissioning/CommissioningGuard.tsx`, related tests

**Tests:**

- TDD: the guard redirects only when gateway status is non-complete and shows a
  retryable readiness error if status cannot be determined. Verifies AC1 and
  AC8.
- TDD: typed transport calls serialize a write-only API key only for completion
  and never retain it in returned state. Verifies AC6.

**Approach:**

- Add transport types and methods that match the committed OpenAPI contract.
- Replace the sessionStorage-based guard with server-owned commissioning
  readiness; retain only an in-memory loading state.

**Done when:** client tests prove no browser completion flag can bypass a fresh
or incomplete installation.

### T4: Commissioning UI provides the approved guided first-run journey

**Depends on:** T3

**Touches:** `apps/ui/src/features/commissioning/*`,
`apps/ui/src/styles/components.css`, `apps/ui/src/App.tsx`, UI tests

**Tests:**

- Component test: focus/domain controls block continuation until confirmed and
  expose accessible labels, focus, and validation messages. Verifies AC2.
- Component test: local diagnostic states show distinct recovery instructions;
  cloud completion surfaces a safe error message. Verifies AC3, AC4, and AC6.
- Visual/manual QA: the new screen matches the hierarchy recorded in
  `docs/architecture/future-state/assets/agent-commissioning-mockup.png` on a
  desktop viewport and remains usable at a narrow viewport. Verifies AC1-AC5.

**Approach:**

- Build the four commissioning commitments—focus, intelligence, world, and
  boundaries—using the existing design tokens and component conventions.
- Persist only the selection state necessary to submit; the gateway remains
  source of truth after refresh.
- Use the semantic profile selection as an honest “prepared for provisioning”
  summary rather than an Engram readiness claim.

**Done when:** a fresh browser can finish the commissioning journey with a
verified provider and sees a clear, redacted recovery path for failure.

### T5: Cut over routes and delete the legacy setup implementation

**Depends on:** T4

**Touches:** `apps/ui/src/App.tsx`, `apps/ui/src/features/setup/*` (delete),
`gateway/src/http/setup.rs` (delete), `gateway/src/http/mod.rs`, legacy tests,
setup-only transport methods

**Tests:**

- TDD: `/setup` redirects to `/commission`; `/commission` is accessible without
  the application shell. Verifies AC9.
- Goal-based: repository search finds no production import of `SetupWizard` or
  `SetupGuard`, no browser `sessionStorage` completion flag, and no setup-only
  endpoints. The legacy `ExecutionSettings.setup_complete` field remains only
  as a migration input. Verifies AC9.
- Goal-based: the full UI build and focused gateway tests pass after legacy
  files are removed.

**Approach:**

- Switch the app route and guard only after the new journey is verified.
- Delete the six-step UI, its tests/styles that are no longer shared, and the
  setup-only gateway endpoints/transport methods.
- Preserve the `/setup` URL as a simple redirect for bookmarks.

**Done when:** no executable path can use the previous setup implementation
and existing complete configurations still enter the app normally.

## Rollout

- **Delivery:** one compatibility cutover; no long-lived feature flag or second
  setup flow. `/setup` is a redirect after parity.
- **Infrastructure:** no new service, database, dependency, or Engram runtime
  is required.
- **External-system integration:** cloud provider verification is user-initiated
  and bounded; local diagnostics use the fixed local endpoint only.
- **Deployment sequencing:** merge T1–T4 together or keep them unavailable
  behind the unpublished `/commission` route; perform T5 only after the fresh
  and legacy journeys are green.

## Risks

- Existing provider APIs continue to persist raw keys by user-approved
  temporary exception; the new route must not enlarge their exposure.
- Completion spans settings, providers, and agent configuration rather than one
  database transaction; status-last ordering and idempotent retries limit
  partial-state harm.
- The existing derived `ToolSettings::default()` ignored serde's documented
  offload defaults; the narrow same-concern correction is bundled so a fresh
  commissioned installation starts with the intended safe setting.
- The portable semantic profile will not prove Engram provisioning until the
  upstream integration lands; UI wording must not imply it has completed.

## Changelog

- 2026-07-11: scaffolded; awaiting assumption confirmation.
- 2026-07-11: approved providers.json as temporary credential storage and
  made semantic provisioning explicitly independent from Engram.
- 2026-07-11: added the narrow `ToolSettings` default correction after the T1
  fresh-settings test proved it contradicted its documented safe default.
- 2026-07-11: shipped the gateway-owned commissioning flow, compatibility
  redirect, portable semantic profile, and legacy setup retirement.
- 2026-07-11: removed the boundaries questionnaire; added a local-only user
  profile (name and interests required; hobbies and date of birth optional) and
  an explicit autonomous-but-not-sandboxed execution warning.
