# Plan: Goal Artifacts

- **Spec:** [`spec.md`](spec.md)
- **Status:** Shipped

> **Plan contract:** this is the implementation strategy. It may change as the
> implementation reveals facts; material changes are recorded below.

## Approach

Add one backwards-compatible `is_goal_artifact` bit to the existing artifact
declaration and persistence flow, while making the existing artifact pipeline
safe to expose as a user-facing deliverable surface. A declaration is accepted
only for a bounded regular file canonically inside the active ward root; the
content request is bound to the owning session and revalidates that confinement.
The list API remains the manifest for all session artifacts but returns the bit
and no resolved `filePath`; Quick Chat calls its bounded, goal-only query and
renders it as Deliverables. The schema migration sets existing rows to `0`, so
no historical artifact is guessed visible.

## Constraints

- Follow [`spec.md`](spec.md) and the
  [`goal-artifact contract`](../../../contracts/openapi/goal-artifacts.yaml).
- Reuse the existing `artifacts` table, `/api/sessions/:session_id/artifacts`
  endpoint, `ArtifactSlideOut`, and UI transport. No dependency, endpoint,
  or service boundary is introduced.
- The runtime has no reference architecture document; conform to the existing
  Rust/SQLite gateway and React/TypeScript UI patterns.
- Treat model-provided paths, labels, and the display boolean as untrusted
  input. Only the existing gateway artifact pipeline authorizes filesystem
  access; display metadata grants no capability.
- Use a platform's standard/OS no-follow file-open capability when available;
  a platform without it must reject artifact persistence/serving rather than
  use an unsafe check-then-read fallback. No new general dependency is added.

## Construction tests

**Integration tests:** gateway artifact persistence plus the existing list and
content endpoints must prove true/default-false values, an atomic 24-goal
session limit, path confinement, session binding, bounded goal-only listing,
response redaction, and oversized-content rejection.

**Manual verification:** create one deliverable and one working file in a
Quick Chat turn; verify the former alone appears under Deliverables and opens
in the existing preview. Confirm Research still renders its complete artifact
strip.

## Design (LLD)

### Design decisions

`is_goal_artifact` is a persisted boolean, not a path rule or a broader role
registry. It is opt-in: `false` is safe for omitted and legacy declarations.
The bit is never an authorization signal. The gateway accepts only constrained
ward files and rechecks them at serve time. The existing list route gains an
optional `goal_artifacts_only=true&limit=24` query; only Quick Chat uses it.
This satisfies AC 1–9 and avoids classifying arbitrary working files.

### Data & schema

`artifacts.is_goal_artifact INTEGER NOT NULL DEFAULT 0` becomes schema version
23. `ArtifactDeclaration`, `execution_state::Artifact`, and
`ArtifactResponse` carry the same boolean. The public camelCase JSON field is
`isGoalArtifact`; resolved `file_path` stays server-only. The OpenAPI contract
defines the list and session-bound content response. `create_artifact` gains a
transactional goal-artifact insertion path whose count and insert occur in one
SQLite transaction, so concurrent executions cannot exceed 24. This satisfies
AC 1–9.

### Interfaces & contracts

The existing `respond` tool accepts optional `is_goal_artifact` per artifact.
The actual propagation chain is `respond` → `RespondAction` →
`StreamEvent::ActionRespond` → `handle_artifact_declarations` →
`process_artifact_declarations` → store/API/UI; `GatewayEvent::Respond` does
not carry artifact declarations. The existing artifact-list endpoint returns
the required camelCase field without `filePath`. Its existing content endpoint
requires the matching session ID and serves only a revalidated, owner-bound
record read from the same no-follow handle used for validation. Direct HTML and
SVG responses force a non-executable attachment content type and `nosniff`; the
slide-out fetches text into its script-disabled sandbox. The session parameter
is an ownership/correlation binding in the
single-owner local deployment, not multi-user authorization. No new endpoint
or event type is introduced. This satisfies AC 1, 2, 5, and 6 and implements
the goal-artifact contract.

### Component / module decomposition

`useQuickChat` maps transport artifacts to lightweight refs after filtering
for `isGoalArtifact === true`; it requests at most 24 from the endpoint and
holds at most 24 defensively. `QuickChat` only receives displayable refs and
renders them below a **Deliverables** label with existing `ArtifactCard` and
`ArtifactSlideOut`, including a 24-per-session limit hint. Research does not
reuse this filter. This satisfies AC 2–9.

### State & control flow

On bootstrap and after a turn becomes idle, Quick Chat requests
`goal_artifacts_only=true&limit=24`; the hook defensively discards
false/omitted values. Research uses the existing unfiltered request. The UI has
no separate cache or polling path. This satisfies AC 2–5 and 7.

### Failure, edge cases & resilience

Missing values from an older server or legacy row are false. Invalid, escaped,
symlinked, overlong, over-budget, or no-ward declarations are skipped before
persistence; a content request with an owner-session mismatch is rejected.
Existing artifact fetch failures remain non-throwing and yield no Quick Chat
cards. Existing oversized rows are rejected at content serve time and display
a safe preview-unavailable state without a download action. No artifact record
or file is removed. This satisfies AC 3, 5, 6, and 9.

### Quality attributes (NFRs)

The change adds no request and makes the existing Quick Chat fetch bounded to
24 goal artifacts. The shared preview limits artifact bytes and renders HTML/SVG
without scripts; direct active content is download-only. Office documents remain
download-only because browser-side decompression of untrusted ZIP containers is
not an acceptable preview boundary. Existing card semantics and preview are preserved for
accessible keyboard use. This satisfies AC 2, 7, 8, and 9.

### Dependencies & integration

The established chain is `respond` tool → `RespondAction` →
`StreamEvent::ActionRespond` → `handle_artifact_declarations` →
`process_artifact_declarations` → execution-state SQLite → existing artifact
endpoint → UI transport → Quick Chat. This satisfies AC 1–9.

## Tasks

### T1: Goal designation survives declaration and storage

**Depends on:** none

**Touches:** `runtime/agent-primitives/src/event.rs`,
`runtime/agent-runtime/src/tools/respond.rs`,
`gateway/gateway-execution/src/artifacts.rs`,
`services/execution-state/src/{types.rs,repository.rs}`,
`stores/zbot-runtime-sqlite/src/schema.rs`

**Tests:**

- TDD: an omitted declaration stores `is_goal_artifact == false`; an explicitly
  marked declaration stores true (AC 1–2).
- TDD: a version-22 database migrates to version 23 with old rows at `0` (AC 5).
- TDD: the response-tool schema accepts the optional boolean and parsing
  preserves it (AC 1–2).
- TDD: declarations for `/etc/hosts`, traversal, outside-root symlinks,
  missing ward context, overlong metadata, and excess response/session limits
  are skipped without reading or persisting their target (AC 6–7).
- TDD: parallel goal-artifact inserts for the same session persist no more
  than 24 rows, and a 5 MiB-plus file is rejected before storage (AC 7, 9).
- TDD: a symlink replacement after path resolution cannot alter bytes served:
  the no-follow opened handle is both validated and read (AC 9).

**Approach:**

- Add the opt-in field to `ArtifactDeclaration` and the respond JSON schema
  with guidance limiting true to a final user-facing goal deliverable.
- Treat declaration path and label as untrusted data; canonicalize a regular,
  non-symlink file inside `vault/wards/<active-ward>` before persisting it.
  Cap at 8 declarations per response, 24 designated deliverables per session,
  1,024 path characters, 160 label characters, and 5 MiB per file.
- Add the boolean to the execution-state type and a transactional repository
  method that counts and inserts a goal artifact under one SQLite transaction,
  then increment the runtime SQLite schema to 23 with a safe
  `ALTER TABLE ... DEFAULT 0` migration and fresh-schema column.
- Propagate the field through `process_artifact_declarations`; update affected
  struct fixtures and focused Rust tests.

**Done when:** T1 tests prove true and omitted declarations have durable,
compatible values in SQLite.

### T2: Artifact manifest exposes the designation

**Depends on:** T1

**Touches:** `gateway/src/http/artifacts.rs`,
`apps/ui/src/services/transport/{types.ts,http.ts,interface.ts}`,
`apps/ui/src/features/chat/ArtifactSlideOut.tsx`,
`apps/ui/src/services/transport/http.embeddings.test.ts`,
`apps/ui/tests/e2e/quick-chat.spec.ts`, `contracts/openapi/goal-artifacts.yaml`

**Tests:**

- TDD: `ArtifactResponse` serializes `isGoalArtifact` for true and false
  artifacts, but omits `filePath` (AC 1–2, 5, 8).
- TDD: the content request rejects an artifact ID owned by a different session
  and validates/reads one no-follow canonical regular-file handle (AC 6, 9).
- TDD: the goal-only list query returns no more than its valid limit, rejects
  invalid limits with 400, and a 5 MiB-plus persisted legacy file returns the
  safe too-large response (AC 7, 9).
- TDD: direct HTML/HTM/SVG content carries non-executable `Content-Type`,
  `X-Content-Type-Options: nosniff`, and attachment disposition; every shared
  preview type shows one 413 unavailable state without a download action (AC 9).
- TDD: parse the checked-in OpenAPI YAML with the workspace `serde_yaml`,
  assert a serialized artifact has exactly its declared properties and all
  required keys, and assert a manifest missing `isGoalArtifact` fails the
  shape check without adding a validator dependency (AC 7–8).
- TDD: TypeScript transport type accepts the endpoint field while omitted
  values from an older server remain safely hidden by Quick Chat (AC 5).

**Approach:**

- Author the OpenAPI contract directly because no contract-authoring skill is
  installed; keep its `x-spec` back-link current.
- Extend the existing response and UI transport shapes without adding an
  endpoint. Add optional bounded goal-only list parameters, remove resolved
  paths from the response, and make the existing content URL carry its owning
  session ID for server-side ownership/correlation and confinement checks.
- Revalidate file size before the server reads content, return a distinct safe
  too-large response, and render HTML/SVG in the shared slide-out with scripts
  disabled. Direct HTML/HTM/SVG routes are download-only with `nosniff`; no route
  validates a pathname then reads it later. The shared preview behavior changes
  for both Quick Chat and Research as required by AC 9.

**Done when:** the existing list/content endpoints are documented, tested for
safe ownership/confinement, and all current consumers typecheck without a
browser-visible resolved path.

### T3: Quick Chat presents goal outputs as Deliverables

**Depends on:** T2

**Touches:** `apps/ui/src/features/chat-v2/{useQuickChat.ts,QuickChat.tsx,types.ts,quick-chat.css}`, associated tests

**Tests:**

- TDD: the Quick Chat hook keeps only `isGoalArtifact === true` refs, treats
  absent as false, requests `goal_artifacts_only=true&limit=24`, and never
  holds more than 24 (AC 2–5, 7).
- TDD: the rendered **Deliverables** list includes a true `.py` or `.json`
  artifact and excludes a false `.md` plan; click opens the existing preview
  (AC 2–4).
- Visual/manual QA: complete the two-file Quick Chat turn from the
  construction tests (AC 7).

**Approach:**

- Request and defensively filter the existing manifest at the Quick Chat
  boundary, treating absent as false and retaining at most 24; retain the
  existing safe refresh/preview path.
- Rename the presentation semantics to **Deliverables** without changing the
  Research components or their artifact list.

**Done when:** Quick Chat only shows explicit bounded goal artifacts under
**Deliverables**, while Research's tests still assert its unfiltered strip.

### T4: Agents receive precise deliverable guidance

**Depends on:** T1

**Touches:** `gateway/templates/shards/tooling_skills.md`,
`runtime/agent-runtime/src/tools/respond.rs`, associated tests

**Tests:**

- TDD: the respond tool parameter schema describes `is_goal_artifact` and
  defaults omitted declarations to false; its prompt text states that path and
  label are untrusted data and the boolean grants no file/tool authority (AC 1, 6).
- Goal-based: the shipped tooling shard instructs agents to mark only final,
  useful goal outputs and rejects extension-based wording (AC 3–4).

**Approach:**

- Align the runtime tool description and seeded tooling guidance on the same
  explicit-final-deliverable rule, path confinement, and no-extension-heuristic
  rule.

**Done when:** model-visible guidance makes the opt-in rule clear without a
file-type allowlist.

## Rollout

- **Delivery:** coordinated UI and gateway release. The required content-route
  session parameter intentionally makes a stale UI incompatible; release the
  gateway only with the matching static UI, and restart both during local
  development. Current UI still treats a missing `isGoalArtifact` as false.
- **Infrastructure:** none.
- **Deployment sequencing:** migrate schema/runtime/gateway and matching UI as
  one release. A new UI against an old gateway hides all cards safely; do not
  run an old UI against the new gateway.
- **Rollback:** revert the UI presentation safely; persisted boolean values and
  column are additive and require no data deletion. Do not roll back file
  confinement or script-disabled preview safeguards.

## Risks

- Agent output may under- or over-designate a file. Clear model-visible tool
  guidance and an opt-in default reduce noise; automatic classification is
  deliberately out of scope.
- An incomplete schema migration would break artifact listing on existing
  conversation databases. T1 tests both fresh initialization and v22 upgrade.
- Different app/gateway versions can temporarily disagree; missing values are
  treated as false, which favors hiding noise over exposing working files. The
  content-route binding requires a coordinated release to avoid stale UI
  preview failures.
- Model output could attempt host-file disclosure or overload a session. The
  server, not prompt guidance, confines paths, revalidates on read, atomically
  bounds goal artifacts, and bounds the Quick Chat manifest query.
- The local single-owner session correlation check is not multi-user
  authorization. A future LAN/multi-user deployment must add an authenticated
  session-access policy before relying on this route for isolation.

## Changelog

- 2026-07-14: Initial plan after user confirmed Quick Chat-only scope,
  explicit semantic designation, legacy hiding, and session-wide presentation.
- 2026-07-14: Security review expanded the shared artifact safety contract:
  confined no-follow reads, bounded goal-only manifest, non-executable active content,
  and a coordinated UI/gateway release now require user sign-off before code.
- 2026-07-14: User approved the complete safety scope and coordinated release.
- 2026-07-15: Post-ship regression correction: Research no longer synthesizes
  clickable artifact IDs from `respond` paths when the persisted manifest is
  empty. The session-bound content route correctly rejects those paths, so the
  manifest is now the sole source of previewable Research artifacts.
