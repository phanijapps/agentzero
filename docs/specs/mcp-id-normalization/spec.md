# Spec: MCP ID Normalization

- **Status:** Shipped
- **Owner:** phanijapps
- **Plan:** [`plan.md`](plan.md)
- **Constrained by:** none
- **Brief:** none
- **Contract:** none
- **Shape:** service

> **Spec contract:** this document defines what "done" means. The implementing
> PR must match this spec, or update it. Verification must be derivable from it.

## Objective

Ensure every MCP created or updated through the existing MCP HTTP API has a
stable canonical ID when the client omits one. The API derives that ID by
lowercasing the MCP name and replacing spaces with hyphens, while preserving a
non-empty client-supplied ID. Explicitly configure the existing Blender MCP as
`blender-mcp` so delegated agents can select it by the canonical ID and receive
its tools. A request that needs a derived ID must supply a non-blank name,
otherwise the API rejects it rather than persisting an empty identity.

## Boundaries

### Always do

- Derive an ID for an absent or blank API request ID from the request name.
- Preserve a non-empty caller-supplied ID without normalization.
- Reject an absent or blank ID paired with a blank or whitespace-only name
  with HTTP 400.
- Reject a PUT that would replace an MCP's canonical ID with another record's
  ID, leaving the stored configuration unchanged.
- When an OAuth MCP's ID changes through PUT, clear the prior ID's OAuth token
  and pending authorization state before persisting the replacement.
- Set the existing Blender MCP configuration ID to `blender-mcp`.

### Ask first

- Renaming or rewriting IDs in existing MCP configuration records other than
  the Blender record explicitly requested here.
- Changing the canonical-ID rules used by dynamic capability resolution.

### Never do

- Never add an MCP endpoint, a persistence migration, or a new top-level
  contract directory for this bug fix.
- Never alter an explicit non-empty MCP ID supplied by an API client.
- Never make dynamic execution accept display-name aliases again.

## Testing Strategy

- **TDD:** request-to-config normalization is a pure HTTP-boundary behavior;
  focused unit tests cover omitted, blank, and explicit IDs across the shared
  conversion used by both create and update routes.
- **Goal-based check:** inspect the persisted Blender configuration to verify
  its explicit `blender-mcp` ID, then run the MCP service tests and affected
  gateway checks.

## Acceptance Criteria

- [x] Given a create or update request with no ID or a blank ID and name
  `Blender MCP`, the resulting configuration ID is `blender-mcp`.
- [x] Given a create or update request with a non-empty explicit ID, that ID
  remains unchanged even when it differs from the name-derived value.
- [x] Given a create or update request with an absent or blank ID and a blank
  or whitespace-only name, the API returns HTTP 400 and persists nothing.
- [x] PUT rejects a generated or explicit replacement ID that already belongs
  to a different MCP with HTTP 400, without changing either record.
- [x] Changing an OAuth MCP ID through PUT removes token and pending state
  associated with the prior ID.
- [x] The active configured Blender MCP persists `id: "blender-mcp"`; no
  other existing MCP record is rewritten.
- [x] Existing dynamic capability resolution remains exact-canonical-ID only,
  and all runnable focused tests, formatting, and relevant library checks
  pass; the unrelated gateway test-compilation blocker is recorded in the
  plan.

## Assumptions

- Technical: the create and update routes share `CreateMcpRequest` conversion,
  so one normalization point covers both (source:
  `gateway/src/http/mcps.rs`).
- Technical: dynamic resolution intentionally accepts exact IDs only, and an
  ID-less config falls back to its name (source:
  `gateway/gateway-services/src/mcp.rs`,
  `runtime/agent-runtime/src/mcp/config.rs`).
- Technical: the running daemon's active canonical MCP configuration is
  `/home/videogamer/Documents/zbot/config/mcp-servers.json`; the legacy
  `/home/videogamer/zbot/config/mcps.json` is not the active daemon's source
  (verified 2026-07-18 via daemon API and on-disk inspection).
- Product: lower-case names with spaces replaced by hyphens are the desired
  derived-ID rule; explicit IDs stay explicit (source: user confirmation
  2026-07-18).
- Product: rejection is preferable to an empty generated identity when a
  request supplies no usable ID or name (derived from the stable-ID objective).
- Process: this is a separate bug-fix spec; no new contract directory is added
  because that would require an RFC (source: `docs/CONVENTIONS.md §4`).
