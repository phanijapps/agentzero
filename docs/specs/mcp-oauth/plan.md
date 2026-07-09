# Plan: MCP OAuth

- **Spec:** [`spec.md`](spec.md)
- **Status:** Done

> **Plan contract:** this is the implementation strategy. Unlike the spec, this
> document is allowed to change as you learn. When it changes substantially (a
> different approach, not just a re-ordering), note why in the changelog at the
> bottom.

## Approach

Add OAuth support as an extension of the existing MCP config and gateway service
path. First, add non-secret auth metadata to `McpServerConfig` and make
`McpService` able to report auth status without leaking secrets. Then add a
file-backed token/pending-state store and OAuth discovery/PKCE service used by
new gateway endpoints. Finally, inject bearer tokens into runtime MCP configs
before `McpManager` starts remote servers, leaving the HTTP MCP client itself as
a header-based transport.

## Constraints

- OAuth tokens and PKCE state must stay outside `mcps.json`.
- Robinhood Trading is a preset/use-case, not a hard-coded OAuth endpoint path.
- No new OAuth framework or keychain dependency in the first backend loop.
- Existing stdio/http/sse/streamable-http configs must continue to deserialize.

## Construction tests

**Integration tests:**
- `cargo test -p agent-runtime mcp_oauth`
- `cargo test -p gateway-services mcp_oauth`
- `cargo test -p gateway mcp_oauth`

**Manual verification:**
- Add a Streamable HTTP MCP with OAuth metadata through the API.
- Start OAuth and verify the returned URL is a browser authorization URL.
- Confirm `config/mcps.json` does not contain token material after callback.

## Tasks

### T1: MCP configs carry non-secret OAuth metadata

**Depends on:** none

**Touches:** `runtime/agent-runtime/src/mcp/config.rs`,
`gateway/src/http/mcps.rs`, `apps/ui/src/services/transport/types.ts`

**Tests:**
- TDD: existing MCP JSON without `auth` still deserializes.
- TDD: `streamable-http` with `auth.type = "oauth2"` round-trips without
  tokens.
- TDD: create/update rejects `headers.Authorization` when `auth.type =
  "oauth2"`.

**Approach:**
- Add `McpAuthConfig` and `McpAuthType`.
- Add optional `auth` fields to remote MCP config variants.
- Extend create/update request and frontend transport types with optional auth.
- Add request validation that rejects persisted bearer material for OAuth MCPs.

**Done when:** config and API request types can represent OAuth-protected MCPs
without token values.

### T2: OAuth token and pending-state storage is separate from config

**Depends on:** T1

**Touches:** `gateway/gateway-services/src/mcp.rs`

**Tests:**
- TDD: saving a token creates a separate token store and leaves `mcps.json`
  unchanged.
- TDD: disconnect removes token and pending state for one MCP ID only.
- TDD: Unix token and pending files are created with `0600` permissions, or the
  operation fails closed.
- TDD: existing token, pending, or dynamic-client credential files with broader
  Unix permissions fail closed before read or write.
- TDD: status returns `not_connected`, `connected`, and `reauth_required` based
  on token presence, expiry, and malformed records.

**Approach:**
- Add private `mcp_oauth_tokens.json` and `mcp_oauth_pending.json` helpers under
  the config directory.
- Create secret files atomically with owner-only permissions on Unix; reject
  existing secret files that are readable or writable by group/other.
- Keep token values out of summaries and config responses.

**Done when:** token lifecycle operations are file-backed and isolated from MCP
config serialization.

### T3: OAuth discovery and PKCE start/callback work

**Depends on:** T2

**Touches:** `gateway/gateway-services/src/mcp_oauth.rs`,
`gateway/gateway-services/src/lib.rs`

**Tests:**
- TDD: PKCE verifier/challenge generation produces URL-safe S256 challenges.
- TDD: protected resource metadata discovery uses `WWW-Authenticate`
  `resource_metadata` when present and falls back to well-known URLs.
- TDD: token exchange stores access token, refresh token, and expiry.
- TDD: begin authorization rejects non-HTTPS authorization/token endpoints
  except localhost fixtures.
- TDD: discovery rejects non-HTTPS protected-resource metadata, auth-server
  metadata, authorization, token, and dynamic registration endpoints except
  localhost fixtures.
- TDD: callback rejects missing, unknown, mismatched, expired, and replayed
  `state` before token exchange.
- TDD: dynamic client registration stores returned client credentials outside
  `mcps.json`; absence of both registration and configured `client_id` returns
  an actionable error.
- TDD: returned dynamic-client `client_secret` uses the same secret-file
  no-leak and owner-only permission contract as OAuth tokens.

**Approach:**
- Add an async `McpOAuthService`.
- Implement `begin_authorization`, `complete_callback`, `status`, and
  `disconnect`.
- Support dynamic client registration when a registration endpoint is
  advertised; otherwise require configured `client_id`.
- Bind pending state to MCP ID, redirect URI, PKCE verifier, resource URL, and
  expiry; delete it before or during successful exchange so replay fails.
- Treat DCR `client_secret` as secret material: store outside `mcps.json`,
  never return through API responses, and only use it during token exchange.

**Done when:** the service can produce an auth URL and complete a callback
against mocked OAuth endpoints.

### T4: Gateway exposes OAuth MCP endpoints

**Depends on:** T3

**Touches:** `gateway/src/http/mcps.rs`, `gateway/src/http/mod.rs`,
`gateway/src/http/openapi.yaml`

**Tests:**
- TDD: status endpoint returns no secrets.
- TDD: start endpoint returns `authUrl` for OAuth MCPs and a clear error for
  non-OAuth MCPs.
- TDD: disconnect endpoint is idempotent.
- TDD: callback route returns browser-readable success and error responses,
  including state failures, without secrets.

**Approach:**
- Add status/start/disconnect/callback handlers.
- Wire routes under `/api/mcps`.
- Keep callback response minimal and browser-readable.

**Done when:** gateway HTTP API can drive the backend OAuth flow.

### T5: Runtime MCP startup injects connected OAuth bearer tokens

**Depends on:** T2

**Touches:** `gateway/gateway-services/src/mcp.rs`,
`gateway/gateway-execution/src/invoke/executor.rs`

**Tests:**
- TDD: `get_multiple`/runtime config resolution injects an Authorization
  header only when an OAuth access token exists.
- TDD: expired or malformed token records do not inject Authorization and report
  `reauth_required`.
- TDD: list/get summary responses still do not include token values.

**Approach:**
- Add a token-aware config resolution path in `McpService`.
- Use it from executor MCP startup.
- Keep `HttpMcpClient` unchanged: it already accepts headers.

**Done when:** authenticated remote MCPs execute through the existing HTTP
client with bearer headers.

### T6: Robinhood preset and first UI transport hooks

**Depends on:** T1-T4

**Touches:** `gateway/templates/default_mcps.json`,
`apps/ui/src/services/transport/interface.ts`,
`apps/ui/src/services/transport/http.ts`,
`apps/ui/src/services/transport/types.ts`

**Tests:**
- Goal-based: Robinhood preset is disabled and has OAuth metadata.
- Goal-based: Robinhood preset is not assigned to any default agent and OAuth
  completion does not toggle `enabled`.
- Goal-based: TypeScript accepts OAuth transport methods and types.

**Approach:**
- Add disabled Robinhood Trading preset.
- Add transport functions for OAuth status/start/disconnect.
- Leave full Integrations panel UX as the next loop unless this loop has room.

**Done when:** setup/default data can surface Robinhood safely and the UI
transport has methods for the future connect button.

## Rollout

Ship backend support as backward-compatible config/API additions. Existing MCPs
continue to work. OAuth-protected MCPs remain disabled/not connected until the
user explicitly starts and completes authorization.

## Risks

- OAuth servers differ in metadata and dynamic client registration support; the
  first implementation must return actionable errors when discovery is
  incomplete.
- File-backed token storage is not as strong as OS keychain storage; do not
  overstate it as secure secret management.
- Runtime bearer injection through config headers must never leak back through
  list/get API responses.

## Changelog

- 2026-06-15: initial work-loop plan for backend-first MCP OAuth support.
