# Spec: MCP OAuth

- **Status:** Done
- **Owner:** phanijapps
- **Plan:** [`plan.md`](plan.md)
- **Constrained by:** none

> **Spec contract:** this document defines what "done" means. The implementing
> PR must match this spec, or update it. Verification must be derivable from it.

## Objective

Let users connect OAuth-protected remote MCP servers, such as Robinhood Trading,
from z-Bot without manually copying bearer tokens into `mcps.json`. A user can
add a Streamable HTTP MCP server, start an interactive browser OAuth flow, return
to z-Bot through a local callback, and have runtime MCP calls include the stored
access token. When a server is not authenticated, z-Bot should report a clear
authentication-required state instead of a generic MCP connection failure.

## Boundaries

The three-tier guard that keeps an implementing agent inside the lines.
*Always do* applies without asking; *Ask first* requires human sign-off before
proceeding; *Never do* is a hard rule, even under time pressure.

### Always do

- Keep MCP server URL/type/name/description in `mcps.json`.
- Store OAuth access tokens, refresh tokens, PKCE verifier state, and expiry
  data outside `mcps.json`.
- Store any dynamic-client `client_secret` outside `mcps.json` with the same
  no-leak and owner-only protections as OAuth tokens.
- Use OAuth Authorization Code with PKCE for interactive user authorization.
- Validate OAuth callback `state` as a single-use, unexpired, MCP-ID-bound value
  before exchanging any authorization code.
- Reject persisted `Authorization` headers on OAuth-protected MCP configs;
  runtime token injection is the only allowed bearer-token source.
- Treat configured, connected, enabled, and assigned-to-agent as separate states.
- Surface authentication-required results distinctly from transport/protocol
  failures.

### Ask first

- Adding a dependency solely for browser launching, OS keychain access, or full
  OAuth client framework behavior.
- Making Robinhood Trading enabled by default during setup or install.
- Supporting non-OAuth remote authentication schemes in this feature.
- Allowing non-HTTPS remote OAuth endpoints outside local test fixtures.

### Never do

- Never persist bearer tokens or refresh tokens inside `mcps.json`.
- Never auto-enable a trading-capable MCP server before the user explicitly
  connects it.
- Never hard-code Robinhood-specific OAuth endpoints when they can be discovered
  from MCP/OAuth metadata.
- Never let agents see OAuth token values through tools, introspection, or API
  responses.
- Never return dynamic-client secrets through status, list, get, callback, or
  test API responses.
- Never exchange an OAuth callback code when `state` is missing, unknown,
  expired, already used, or bound to a different MCP server.
- Never inject expired OAuth access tokens into runtime MCP headers.

## Testing Strategy

- MCP config compatibility: **TDD**. Auth metadata is a compact serialization
  invariant and should round-trip without breaking existing MCP configs.
- OAuth discovery and PKCE URL construction: **TDD**. These are pure or
  mockable protocol invariants.
- Token storage and runtime header injection: **TDD**. Stored tokens must be
  separated from config and only injected as `Authorization` headers at runtime.
- HTTP API behavior: **TDD / goal-based check**. Focused gateway tests should
  cover status/start/disconnect/callback success and error shapes, including
  state validation failures and no-secret responses.
- UI flow: **visual / manual QA plus focused component tests** in a follow-up
  loop. The first backend loop only needs transport types ready for UI wiring.

## OAuth Status Semantics

- `not_configured`: the MCP server has no OAuth metadata and should be tested as
  a normal MCP server.
- `not_connected`: the MCP server has OAuth metadata but no usable stored token.
- `connected`: the MCP server has a non-expired stored access token.
- `reauth_required`: token material exists but cannot be used, including expired
  access token without refresh support, failed refresh, malformed token record,
  or rejected token validation.

## Acceptance Criteria

- [ ] `McpServerConfig` accepts optional OAuth metadata without requiring token
  values in config.
- [ ] Creating or updating an MCP server can mark it as OAuth-protected.
- [ ] `GET /api/mcps/:id/oauth/status` reports `not_configured`,
  `not_connected`, `connected`, or `reauth_required` without returning secrets.
- [ ] `POST /api/mcps/:id/oauth/start` discovers OAuth metadata, creates PKCE
  state, and returns an authorization URL for the browser.
- [ ] `GET /api/mcps/oauth/callback` exchanges a code for tokens and stores
  them outside `mcps.json`.
- [ ] `POST /api/mcps/:id/oauth/disconnect` removes stored OAuth tokens and
  pending state for that MCP server.
- [ ] Runtime MCP startup injects `Authorization: Bearer ...` for connected
  OAuth MCP servers without exposing the token in list/get responses.
- [ ] Robinhood Trading can be represented as a disabled Streamable HTTP MCP
  config with OAuth metadata and URL `https://agent.robinhood.com/mcp/trading`.
- [ ] OAuth callback rejects missing, unknown, mismatched, expired, and replayed
  `state` values before token exchange.
- [ ] OAuth-protected MCP create/update rejects persisted bearer material in
  `headers.Authorization`.
- [ ] Every followed OAuth URL, including protected-resource metadata,
  auth-server metadata, authorization, token, and registration endpoints,
  requires HTTPS except for localhost test fixtures.
- [ ] OAuth token, pending-state, and dynamic-client credential files are
  atomically created with owner-only permissions on Unix; existing
  broader-permission secret files fail closed before read or write.
- [ ] Completing OAuth for Robinhood Trading does not set `enabled: true` and
  does not add the server to any agent's `mcps` list.
- [ ] Dynamic client registration, when advertised, stores returned client
  credentials outside `mcps.json`; when neither registration nor configured
  `client_id` is available, start returns an actionable error.

## Assumptions

- Technical: remote MCP configs currently support `http`, `sse`, and
  `streamable-http` URL plus headers but no first-class OAuth state (source:
  `runtime/agent-runtime/src/mcp/config.rs`;
  `runtime/agent-runtime/src/mcp/http.rs`).
- Technical: gateway MCP CRUD currently stores configs through `McpService` in
  `config/mcps.json` and test-starts servers through `McpManager` (source:
  `gateway/gateway-services/src/mcp.rs`; `gateway/src/http/mcps.rs`).
- Technical: runtime MCP startup receives server configs from `McpService` via
  `build_mcp_manager`, so token injection can happen before configs enter
  `McpManager` (source: `gateway/gateway-execution/src/invoke/executor.rs`).
- Technical: Robinhood documents the Trading MCP as Streamable HTTP at
  `https://agent.robinhood.com/mcp/trading` and expects users to authenticate
  after adding/selecting the server (source: Robinhood Agentic Trading overview,
  fetched 2026-06-15).
- Process: this feature uses the work-loop spec/plan gates because it crosses
  runtime, gateway API, persistence, and UI transport types (source: user
  request 2026-06-15).
- Product: z-Bot should support OAuth-protected MCPs generally, with Robinhood
  as the motivating use-case rather than a hard-coded special path (source:
  user request 2026-06-15).
