# Plan: Commissioning Memory Profile

- **Spec:** [`spec.md`](spec.md)
- **Status:** Complete

> **Plan contract:** this is the implementation strategy. Unlike the spec, this
> document is allowed to change as we learn.

## Approach

Extend the existing commissioning request with one required, versioned memory
profile enum. Put the canonical memory preset and serialization contract in
`gateway-memory`, keep fixed-path profile provisioning in the gateway
commissioning boundary, and reuse `VaultPaths` plus the existing settings
service. Add a fourth user-visible commissioning step for explicit consent.
Persist a fixed pending-restart marker for the full profile, then finalize
commissioning during the next boot immediately before boot-bound memory
consumers are constructed from the new configuration. Implement profile logic and
file transitions with TDD, then run the complete fresh-vault journey and
workspace gates before review and publication.

## Constraints

- RFC-0011 keeps Engram behind z-Bot-owned settings and adapter boundaries.
- The shipped Agent Commissioning behavior retains provider verification,
  local-origin enforcement, write-only credentials, and completion-last state.
- Canonical profile files use fixed `VaultPaths`; the request carries only an
  enum and cannot influence paths or file contents.
- Existing `http::vault` peer-locality and symlink-component checks are reused
  rather than reimplemented; no new security module is introduced.
- No new crate, npm dependency, module boundary, commissioning-time download, or
  semantic database operation is introduced.

Tempted to add a general profile registry; declining because one versioned
enum and constructor are sufficient. Tempted to add a generic transactional
filesystem abstraction; declining because bounded fixed-file provisioning can
reuse a small helper in the existing commissioning module. Tempted to expose
all memory tuning in the UI; declining because first-run setup needs one clear
choice, while advanced tuning remains file/settings owned.

## Construction tests

**Integration tests:** a temporary-vault commissioning fixture proves that a
verified provider plus the full profile yields settings, recall, governance,
and complete state together; safe baseline yields none of the profile files.

**Manual verification:** open `/commission`, confirm neither profile is
preselected, select each profile in turn, and verify the displayed consequence
and submitted behavior. No external deployment action is part of this spec.

## Design (LLD)

### Design decisions

- `CommissioningMemoryProfile` is a two-value request enum. It is not persisted
  as a second source of truth; resulting settings/files are authoritative.
- `MemorySettings::zbot_recommended_v1()` and
  `RecallConfig::zbot_recommended_v1()` deserialize complete, reviewed,
  checked-in canonical JSON fixtures. Constructors never inherit mutable Rust
  defaults, and serialization tests require exact fixture parity.
- Bundled governance JSON lives with gateway templates and is embedded at
  compile time; the runtime never reads the development reference directory.
- The V1 fixture persists the exact model string reported by the built-in
  client and compared by runtime stores: `bge-small-en-v1.5`, 384 dimensions.
  This intentionally differs from the current provider-qualified default and
  avoids a commissioning-only equivalence rule that store checks cannot see.
  Commissioning never opens or reindexes storage.
- Full-profile activation is a two-boot state machine. The request persists
  settings/files and a fixed pending marker while leaving setup incomplete;
  the next boot verifies exact inputs, saves commissioning complete, removes
  the marker, and then follows normal memory construction. Completion is
  externally observable only after successful gateway startup. Safe baseline
  remains single-boot.

### Data & schema

- OpenAPI and TypeScript add required `memoryProfile`:
  `safe_baseline | zbot_recommended_v1`.
- Full-profile memory is the complete pinned JSON fixture, including fields
  whose values currently match defaults. This prevents future default drift.
- Profile files are `config/recall-config.json`,
  `config/governance/base-ontology.json`, and
  `config/governance/base-taxonomy.json`.
- The fixed activation marker is `config/.zbot-memory-profile-v1-pending`, has
  bundled exact V1 bytes, is never request-controlled, and participates in the
  same symlink/regular-file/conflict/no-overwrite rules as the other targets.

### Interfaces & contracts

- `POST /api/commissioning/complete` consumes the amended request in
  `contracts/openapi/commissioning.yaml`; API 2.0 adds the required request
  field and additively extends status responses with `restartRequired`.
- Omission/unknown enum fails serde/request validation before handler effects.
- `CommissioningRequest` and `ProviderSelection` use Serde
  `deny_unknown_fields` to enforce OpenAPI `additionalProperties: false`.
- A parts-only `LocalCommissioningRequest` extractor runs before the body
  extractor, reads `GatewayConfig`, `ConnectInfo<SocketAddr>`, and Origin, and
  reuses `http::vault::is_local_request`; malformed remote JSON therefore still
  receives the finite 403 before body deserialization or effects.
- The completion response adds `restartRequired`; the full-profile persistence
  response uses state `needs_attention`, recovery code
  `memory_profile_restart_required`, and `restartRequired: true`. Status after
  successful boot returns complete with `restartRequired: false`.
- Provisioning/settings/embedding conflicts map to finite
  `memory_profile_conflict`; other I/O
  failures map to the existing redacted internal error.

### Component / module decomposition

- `gateway/gateway-memory/src/lib.rs` and `gateway/gateway-memory/templates/*`:
  versioned memory/recall fixtures, constructors, and exact parity tests.
- `gateway/templates/governance/*.json`: reviewed base definitions.
- `gateway/src/http/commissioning.rs`: enum, fixed-path idempotent provisioning,
  conflict preflight, pending-restart response, and temporary-vault tests.
- `gateway/src/state/mod.rs`: boot-time pending-profile verification,
  save-before-marker-removal finalization, and cleanup before normal memory
  construction.
- `apps/ui/src/features/commissioning/*`: explicit memory step and component
  tests.
- `apps/ui/src/services/transport/types.ts` and OpenAPI: request parity.

### State & control flow

1. UI requires focus, intelligence, memory profile, and personal profile.
2. Gateway authorizes the Origin/peer before deserialization-driven work,
   validates the request, and verifies the selected provider without saving it.
3. Full profile preflights all fixed targets, canonical parents, symlink
   components, regular-file status, and byte conflicts before any mutation.
4. Safe baseline leaves existing memory/embedding configuration untouched.
   Full profile also preflights serialized `execution.memory` (only fresh
   default or exact V1 is accepted) and the persisted/live embedding backend
   (only internal/384 with the canonical model mapping is accepted).
5. Full profile creates absent files with exclusive no-overwrite semantics,
   assigns the versioned settings, persists the fixed activation marker, and
   remains `needs_attention` with `setupComplete = false`.
6. Provider/default selection and root/SOUL changes occur only after every
   conflict-prone preflight. Safe baseline commits completion last; full
   profile returns the restart-required recovery state.
7. On the next daemon boot, before memory construction, boot accepts the marker
   only when commissioning is restart-pending and exact V1 memory, embedding,
   recall, governance, and marker bytes all match. It saves `setupComplete =
   true` plus commissioning `complete` first, then removes the marker. The
   existing startup path then constructs all memory consumers from the pinned
   inputs. Engram import/bootstrap remains outside commissioning state: the UI
   can observe completion only after the whole gateway subsequently starts.
8. If completion save fails, the marker remains. If cleanup fails or a crash
   follows the save, completed-state plus an identical marker is a cleanup-only
   case on the next boot; a stale/conflicting marker never activates anything.

### Behavior & rules

- No profile button starts selected. Full profile carries the Recommended
  label and states that background memory processing can consume LLM-provider
  usage/cost, cloud providers can receive memory-derived content, and the
  built-in model may need a one-time local download on first use.
- Full-profile files are create-if-absent and compare-if-present. Different
  content is never replaced.
- Fixed-file writes canonicalize the vault/config parents, reject every symlink
  component and non-regular target, write a create-new temporary sibling, and
  atomically hard-link it into an absent destination so the final name is never
  followed or overwritten. Temporary siblings are removed on every result.
- The marker uses that identical helper and exact-byte retry comparison.
- The selected LLM provider is independent of the built-in embedding backend.
- After full-profile persistence the UI shows a dedicated restart-required
  screen and does not navigate into the application. The user restarts zbotd;
  reconnect/status polling observes completion only after the new daemon is
  serving with the profile active.

### Failure, edge cases & resilience

- Invalid profile input never reaches provider testing or filesystem writes.
- Unknown request fields fail deserialization, and the established peer-aware
  local-access guard runs before validation or effects.
- A conflicting file fails closed with a finite error and incomplete
  commissioning state. Preflight occurs before provider/default/SOUL/settings
  mutation. Identical files make client-controlled retries idempotent; the
  server adds no automatic retry loop.
- A file I/O failure returns a redacted error; no absolute path is exposed.
- Conflicts and I/O failures emit structured private logs with finite stage and
  event codes but no file contents, credentials, or absolute paths.
- Partial files from prior external/manual edits are conflicts, not repair
  candidates. The user retains ownership of existing configuration.
- Existing customized `execution.memory`, non-internal embeddings, or an
  embedding dimension other than 384 are conflicts. No hot backend swap or
  semantic reindex is attempted by commissioning.
- Marker activation additionally requires the restart-pending commissioning
  state and exact V1 artifacts/settings. Save-completion precedes marker
  removal; failure injection covers settings-save failure, marker-removal
  failure, and crashes on either side of that boundary.
- A later lazy FastEmbed model load/download failure does not roll back the
  already-active non-vector memory profile: vector recall degrades through the
  existing embedding health/error path, and the restart screen directs the
  user to Settings > Advanced > Embeddings to retry or diagnose it.

### Quality attributes (NFRs)

- Security: fixed paths, no path input, local-origin guard before mutation,
  peer-aware originless handling, symlink/regular-file confinement, redacted
  errors, and no credential changes on preflight failure.
- Maintainability: one enum, two fixture-backed constructors, five bundled
  assets total (memory, recall, ontology, taxonomy, marker), and no
  general registry or dependency.
- Accessibility: profile choices use buttons with `aria-pressed`, visible
  labels, and keyboard-native behavior.

### Dependencies & integration

- Reuses `gateway-memory`, `gateway-services::VaultPaths`, settings/provider
  services, Axum/Serde, React, and Vitest already in the workspace.
- The memory provider remains configured through z-Bot settings and is opened
  later by the existing adapter-first bootstrap.

## Tasks

### T1: Contract and typed request require an explicit memory profile

**Depends on:** none

**Touches:** `contracts/openapi/commissioning.yaml`, `gateway/src/http/commissioning.rs`, `apps/ui/src/services/transport/types.ts`

**Mode:** TDD for Rust request validation; goal-based for OpenAPI/TypeScript.

**Tests:**
- Rust serde tests reject omitted and unknown profiles and accept both enum
  values before side effects (AC1). `stub: true`
- Goal-based: parse the OpenAPI YAML and run the UI TypeScript build; no stub
  (goal-based).
- Rust tests reject unknown top-level and nested provider fields (AC1).
  `stub: true`

**Approach:**
- Add the required OpenAPI property and backlink this spec.
- Add matching Rust and TypeScript enums without changing response types.

**Done when:** request fixtures and contract/build checks accept only the two
approved values.

### T2: Versioned preset serializes the approved built-in memory behavior

**Depends on:** none

**Touches:** `gateway/gateway-memory/src/lib.rs`, `gateway/gateway-memory/templates/*.json`, `gateway/templates/governance/*.json`

**Mode:** TDD.

**Tests:**
- Exact byte/JSON parity pins every V1 memory field and proves FastEmbed
  identity, model, prompt profile, and 384 dimensions without inheriting
  defaults (AC4). `stub: true`
- Recall fixture deserialization and direct bundled-byte provisioning pin
  deterministic inspectable V1 bytes despite runtime `HashMap` ordering (AC5).
  `stub: true`
- Bundled ontology/taxonomy parse and expose the expected kind and IDs (AC5).
  `stub: true`

**Approach:**
- Add the two V1 constructors by deserializing canonical embedded fixtures.
- Add reviewed governance templates copied from the approved reference.
- Assert exact identity equality between the V1 fixture and built-in client.

**Done when:** `cargo test -p gateway-memory` passes the exact profile contract.

### T3: Commissioning provisions full profile idempotently and fails closed

**Depends on:** T1, T2

**Touches:** `gateway/src/http/commissioning.rs`, `gateway/src/state/*`, `gateway/tests/*`

**Mode:** TDD.

**Tests:**
- Safe baseline keeps default memory and creates none of the three files (AC3).
  `stub: true`
- Full profile writes settings and all files beneath a temporary vault (AC4,
  AC5). `stub: true`
- Identical retry succeeds; conflicting content remains byte-identical and
  returns `memory_profile_conflict` without complete state (AC6). `stub: true`
- Existing origin, credential-redaction, provider, semantic-profile, and
  legacy-status tests remain green (AC7). `stub: true`
- Symlinked parent/final targets and non-regular files fail without writes or
  provider/default/SOUL/settings changes (AC7). `stub: true`
- LAN-bound originless remote peers receive 403 with zero effects; loopback
  peers remain accepted (AC8). `stub: true`
- Malformed JSON from a non-local peer is rejected by the parts-only guard with
  `403 commissioning_origin_denied` before JSON extraction. `stub: true`
- Customized existing memory settings and non-internal/unconfigured/Ollama
  embedding settings fail before provider/default/SOUL/settings/file mutation;
  exact V1 retry state is accepted. `stub: true`
- Full-profile success persists the pending marker, returns the finite
  restart-required response, and leaves both setup and commissioning
  incomplete; a boot fixture validates V1 inputs, saves completion, then
  removes the marker before normal construction. Save failure retains recovery
  state and marker; cleanup failure leaves completed state plus marker.
  `stub: true`
- Marker symlink/non-regular/conflict/stale-state cases fail without mutation;
  identical pending retry succeeds. Failure injection proves save-before-remove
  ordering and completed-plus-marker cleanup. `stub: true`
- Failure tests snapshot provider/default, SOUL, settings, profile bytes, and
  completion state before and after every conflict class and assert redacted
  public errors. `stub: true`

**Approach:**
- Expose the existing `http::vault` locality/symlink predicates at `pub(super)`
  scope and reuse them from commissioning.
- Add fixed `preflight_memory_profile` and `provision_memory_profile` helpers
  using `VaultPaths`, canonical-parent checks, create-new temporary siblings,
  and atomic no-overwrite hard links.
- Run preflight after provider verification but before any persistent mutation;
  compare existing regular-file bytes and never overwrite conflicts.
- Add an early boot finalizer that validates all pending inputs, saves complete,
  then removes the marker before existing boot-bound memory construction. No
  commissioning state depends on Engram import/bootstrap; completion is only
  externally observable if the gateway finishes startup.

**Done when:** temporary-vault tests prove completion-last and idempotency.

### T4: Commissioning UI obtains explicit informed memory consent

**Depends on:** T1

**Touches:** `apps/ui/src/features/commissioning/*`, `apps/ui/src/styles/components.css`

**Mode:** TDD plus visual/manual QA.

**Tests:**
- Component test sees two unselected choices and disabled Continue, then sees
  the recommended/background-processing copy and enabled Continue after a
  click (AC2). `stub: true`
- Component copy states that cloud-provider selection may send memory-derived
  content to that provider, incur provider usage/cost, and require a one-time
  local embedding-model download (AC2). `stub: true`
- Submission test asserts the selected enum is sent and API-key clearing is
  unchanged (AC2, AC7). `stub: true`
- Full success renders restart-required recovery instead of navigating; status
  completion after restart resumes normal navigation. `stub: true`
- Manual QA: keyboard selection, responsive layout, and error announcement;
  no stub (manual QA).

**Approach:**
- Add a fourth `memory` step and selection state using existing semantic CSS
  classes/modifiers.
- Include `memoryProfile` in the existing single completion request.

**Done when:** Vitest drives the visible flow and `npm run build` passes.

### T5: Integrated fresh-vault journey and release gates are clean

**Depends on:** T3, T4

**Touches:** `docs/specs/commissioning-memory-profile/*`, `docs/specs/agent-commissioning/spec.md`, `docs/specs/README.md`, `docs/product/changelog.md`, relevant integration tests

**Mode:** Goal-based integration and manual QA.

**Tests:**
- Temporary-vault end-to-end commissioning verifies full and safe outputs;
  no stub (goal-based integration).
- Run fmt, clippy, focused tests, workspace check, UI tests/build, spec lint,
  and manual `/commission` QA (AC8); no stub (goal-based/manual).

**Approach:**
- Exercise both profile paths across the actual service boundary.
- Bump the breaking commissioning OpenAPI contract to `2.0.0`, document that local
  definition materialization is complete while Engram import/bootstrap remains
  deferred, and add an Unreleased changelog entry for the required request
  field and restart behavior.
- Update spec status/criteria and current documentation only after gates pass.

**Done when:** all mechanical gates and required reviewers are clean.

## Rollout

- **Delivery:** additive required request field ships with the updated bundled
  UI and gateway together. Rollback is the feature commit; no existing file is
  overwritten and no database migration occurs.
- **Infrastructure:** none.
- **External-system integration:** none; built-in embeddings replace the
  reference installation's Ollama embedding dependency.
- **Deployment sequencing:** gateway and bundled UI are released as one z-Bot
  artifact. Older third-party callers must add the required enum.

## Risks

- Making the request field required is intentionally breaking for external
  commissioning clients; the OpenAPI version/backlink and release notes must
  make that visible.
- Full memory enables LLM-backed background workers, increasing provider usage;
  explicit consent and clear UI copy are mandatory.
- A crash between individual fixed-file writes can leave a retry-visible
  partial profile; compare-if-present makes retry safe, while conflicting
  external edits fail closed.
- The implementation must preserve the established local-access behavior for
  loopback/native clients while closing the originless-LAN gap before effects.

## Changelog

- 2026-07-18: initial plan after explicit profile-choice confirmation.
- 2026-07-18: tightened peer authorization, conflict preflight, request
  strictness, consent copy, structured errors, and symlink-safe provisioning
  after secure-design review.
- 2026-07-18: added fixture-pinned V1 bytes, legacy settings/backend conflict
  rules, explicit embedding identity mapping, and a boot-verified
  restart-required activation state after adversarial plan review.
- 2026-07-18: implemented the approved profile, added durable reload recovery
  and exact pending-state validation after implementation review, and passed
  the workspace Rust/UI gates.
