# Plan: MCP ID Normalization

- **Spec:** [`spec.md`](spec.md)
- **Status:** Complete

> **Plan contract:** this is the implementation strategy. Unlike the spec, this
> document is allowed to change as implementation reveals new facts.

## Approach

Normalize optional IDs at the existing `CreateMcpRequest` conversion boundary
so POST and PUT always hand a canonical configuration to `McpService`. Keep the
current dynamic resolver strict: the fix is to generate and persist valid IDs,
not to reintroduce aliases. Make an ID-changing PUT safe by rejecting collisions
and removing OAuth material keyed to the prior ID. Update only the existing
Blender record requested by the user, and prove both boundary and service
behavior with focused unit tests.

## Constraints

- Reuse `gateway/src/http/mcps.rs`; no new endpoint, service, or config
  migration.
- A non-empty explicit ID is a caller-owned stable identifier and must not be
  changed.
- Do not alter the dynamic MCP resolver's exact-ID validation.
- An ID-changing PUT must not create duplicate canonical IDs or orphan OAuth
  secrets/pending authorization state under the prior ID.

## Construction tests

**Integration tests:** none beyond the shared conversion boundary; both POST
and PUT invoke it before persistence.

**Manual verification:** inspect the requested Blender entry in the active
`/home/videogamer/Documents/zbot/config/mcp-servers.json` and use
`blender-mcp` in a subsequent delegated MCP assignment.

## Design (LLD)

### Design decisions

Use a small HTTP-module helper to fill only missing or blank IDs. This keeps
request semantics local to the API boundary and avoids weakening exact dynamic
execution resolution. The service remains the identity-integrity boundary for
PUT collision rejection and old-ID OAuth cleanup. Traces to AC1–AC7.

### Interfaces & contracts

The existing `POST` and `PUT /api/mcps` payloads remain unchanged: `id` is
still optional, but an omitted value now produces a returned and persisted
canonical ID. No repo-level contract is introduced because the repository has
none and adding the top-level contract surface requires a separate RFC. Traces
to AC1–AC4.

### Behavior & rules

For an absent or whitespace-only ID, derive
`name.trim().to_lowercase().replace(' ', "-")`. A non-empty explicit ID wins.
If derivation is required and the trimmed name is empty, return HTTP 400 rather
than create `Some("")`. The same fallible conversion is shared by create and
update. Traces to AC1–AC3.

### Failure, edge cases & resilience

The change does not retry or loosen MCP startup. If a generated or explicit
replacement ID collides during PUT, `McpService` rejects it before mutating
stored configuration and the HTTP route returns a client-error response (400).
An OAuth MCP ID change clears the old ID's OAuth token and pending state because
both are keyed by ID. Existing ID-less records remain readable until manually
updated; only Blender is edited in this work. Traces to AC4–AC6.

## Tasks

### T1: API creation and update derive stable IDs when omitted

**Depends on:** none

**Touches:** `gateway/src/http/mcps.rs`

**Tests:**

- TDD: a request with name `Blender MCP` and `None` ID converts to
  `blender-mcp` (AC1).
- TDD: a blank request ID converts to `blender-mcp`, while a non-empty explicit
  ID remains unchanged (AC1–AC2).
- TDD: omitted or blank IDs with a whitespace-only name are rejected as a
  client error before persistence (AC3).
- TDD: a typed duplicate-ID service error maps to an HTTP 400 response from
  PUT, while a typed internal error remains HTTP 500 (AC4).
- Goal-based check: the focused HTTP module test compiles and passes, proving
  the conversion shared by both create and update routes (AC1–AC3).

**Approach:**

- Add a module-private helper that derives IDs only for absent or blank request
  values.
- Convert `CreateMcpRequest` fallibly so a blank name can reject only a request
  that needs a generated ID.
- Apply the helper in every request variant before constructing
  `McpServerConfig`, and map conversion errors to HTTP 400.
- Match a structured service update error so duplicate IDs map to HTTP 400
  without conflating storage or OAuth failures with client input errors.

**Done when:** requests without IDs consistently return and persist the
canonical hyphenated ID; explicit IDs are untouched.

### T2: Make ID-changing MCP updates collision- and OAuth-safe

**Depends on:** T1

**Touches:** `gateway/gateway-services/src/mcp.rs`

**Tests:**

- TDD: updating one record to the ID of a different existing record returns an
  error and preserves both records (AC4).
- TDD: changing an OAuth MCP's ID clears that prior ID's token and pending
  authorization state, without affecting another MCP's OAuth records (AC5).

**Approach:**

- Before replacing the selected configuration record, reject an ID used by any
  other record with a narrow `DuplicateId` update error distinct from internal
  storage/OAuth failures.
- Treat a canonical-ID change as an OAuth identity change so the existing
  `disconnect_oauth(old_id)` cleanup runs before the replacement is saved.

**Done when:** PUT cannot create duplicate canonical IDs or leave OAuth secret
state keyed to an unconfigured ID.

### T3: Active Blender configuration uses its canonical selected ID

**Depends on:** T1

**Touches:** `/home/videogamer/Documents/zbot/config/mcp-servers.json`

**Tests:**

- Goal-based check: the active Blender entry has exactly
  `"id": "blender-mcp"`, while the remaining config records are unchanged
  (AC6).

**Approach:**

- Add only the requested Blender ID to the active canonical configuration
  file, identified from the running daemon's MCP API response.

**Done when:** a planner/delegation can use the canonical `blender-mcp` ID
without an `unknown_id` rejection.

## Rollout

The active config update takes effect when the daemon restarts or reloads its
MCP configuration. New and updated API-created MCPs receive normalized IDs
without a migration; legacy ID-less records remain unchanged until edited.

## Risks

- Updating an ID-less MCP's name through PUT also changes its derived ID;
  callers that need a stable custom ID must send one explicitly.
- A changed OAuth MCP ID intentionally requires reauthorization because its
  old token and pending state are removed rather than copied to a new identity.
- The requested Blender config update fixes selection identity, but a later
  transport startup failure remains a separate MCP availability condition.

## Changelog

- 2026-07-18: initial plan.
- 2026-07-18: implemented and verified normalization, typed PUT collision
  rejection, old-ID OAuth cleanup, and the active Blender config. The full
  `gateway` unit-test target remains blocked by an unrelated pre-existing
  `ArtifactResponse: Debug` test-compilation error; `gateway-services` tests,
  `gateway` library check, clippy, and disposable live API verification pass.
