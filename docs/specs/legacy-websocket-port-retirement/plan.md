# Plan: Legacy WebSocket Port Retirement

- **Spec:** [`spec.md`](spec.md)
- **Status:** Done

> **Plan contract:** this is the implementation strategy. Unlike the spec, this
> document is allowed to change as you learn. When it changes substantially,
> note why in the changelog at the bottom.

## Approach

Remove the compatibility surface from the outside in: daemon flags and gateway
configuration, conditional startup and raw transport implementation, then UI
legacy-specific behavior and living documentation. Keep the shared
`WebSocketHandler` as the owner of sessions, subscriptions, event routing, and
message forwarding for the Axum `/ws` route. Finish with a literal/config audit
and both Rust and UI gates.

Expected changes are limited to `apps/daemon`, `gateway`, the UI transport and
its fixtures, current infrastructure/documentation references, and this spec.
Frozen historical documents are explicitly outside scope.

Tempted to retain deprecated config fields with ignored serde values;
declining because silent acceptance would conceal misconfiguration and prolong
the removed interface. Tempted to replace the listener with a redirect or
proxy; declining because raw WebSocket clients cannot rely on HTTP redirect
semantics and the goal is one listening port. Tempted to remove all custom
WebSocket URL overrides; declining because arbitrary dev/proxy endpoints are
not legacy-port-specific.

## Constraints

- Follow accepted RFC-0019 and publish its exact external migration path.
- Preserve the established Axum 0.7 `/ws` route and
  `gateway-ws-protocol` message contract.
- Do not change authentication, CORS, LAN exposure, or discovery behavior.
- Do not rewrite frozen historical specs, ADRs, or RFCs.

## Construction tests

- `cargo fmt --all -- --check`
- `cargo clippy --all-targets -- -D warnings`
- `cargo check --workspace`
- `cargo test --workspace`
- From `apps/ui`: `npx tsc --noEmit`, `npm run lint`, `npm test`, and
  `npm run build`.
- A scoped `rg` audit finds no current runtime/config/test/living-doc reference
  to `18790`, `legacy_ws_port_enabled`, `legacy-ws-port-enabled`,
  `DEFAULT_WS_PORT`, or `--ws-port`.

## Design (LLD)

### Design decisions

- `GatewayServer::start` owns one `TcpListener`, the HTTP listener; the Axum
  router upgrades `/ws` connections and reuses the existing handler state.
  Traces to AC2-AC3.
- Removing legacy config keys rather than ignoring them makes stale YAML fail
  only under an explicit unknown-field policy; serde's current default remains
  otherwise unchanged. Traces to AC1.
- Generic UI overrides remain opaque URLs with no port-specific branch.
  Traces to AC4.

### Interfaces & contracts

- Removed CLI interface: `zbotd --ws-port` and
  `zbotd --legacy-ws-port-enabled`.
- Removed configuration interface: `websocket_port` and
  `legacy_ws_port_enabled` in `GatewayConfig`.
- Preserved network interface: `GET /ws` WebSocket upgrade on the configured
  HTTP port, normally `18791`.

### State & control flow

- Startup continues to spawn subscription cleanup and the event router before
  starting the HTTP listener. No conditional secondary accept loop remains.
- Shutdown continues to signal the background tasks and HTTP graceful-shutdown
  receiver through the existing broadcast channel.

### Failure, edge cases & resilience

- A stale command line using either removed flag fails fast through Clap rather
  than starting under false compatibility assumptions.
- External clients using `18790` receive connection failure and must migrate to
  `<http-port>/ws`; the release documentation names that exact replacement.

### Quality attributes (NFRs)

- Network exposure: only the configured HTTP port is bound by the gateway.
- Maintainability: one transport accept path and no gateway dependency on raw
  tokio-tungstenite.

## Tasks

### T1: Gateway and daemon expose only the unified WebSocket endpoint

**Depends on:** none

**Touches:** apps/daemon/src/main.rs, gateway/Cargo.toml, gateway/src/config.rs,
gateway/src/lib.rs, gateway/src/server.rs, gateway/src/http/mod.rs,
gateway/src/websocket/*.rs, gateway/tests/websocket_route.rs, Cargo.lock

**Verification mode:** goal-based check

**Tests:**

- Add a gateway integration smoke test over an ephemeral HTTP listener.
- Run gateway/daemon tests plus workspace check; verify daemon help omits both
  removed flags (AC1-AC3).
- Confirm `/ws` upgrades, emits `Connected`, routes `Ping` to `Pong`, and
  closes cleanly; keep WebSocket handler tests green (AC2).

**Approach:**

- Remove CLI/config fields, address helpers, and conditional server startup.
- Remove the raw tungstenite accept loop and orphaned dependency/imports.
- Update comments and tests around the remaining unified path.

**Done when:** Rust compiles and tests with no legacy listener entry point or
gateway tokio-tungstenite dependency.

### T2: UI and current fixtures contain no legacy-port behavior

**Depends on:** T1

**Touches:** apps/ui/src/services/transport/**, apps/ui/src/test/mocks/handlers.ts

**Verification mode:** goal-based check

**Tests:**

- No TDD stub (removal of a special-case branch while retaining existing
  generic override coverage).
- Run UI TypeScript, lint, Vitest, and build gates (AC4, AC6).
- Verify the override test uses a non-legacy custom endpoint and same-origin
  `/ws` assertions remain green (AC4).

**Approach:**

- Delete the legacy constant and warning function/call sites.
- Replace legacy-only fixture values and align the status fixture/type with the
  backend's current camel-case response.
- Keep the general URL-parameter and window-config override paths.

**Done when:** UI gates pass and no current UI source/test treats `18790`
 specially.

### T3: Living documentation and deployment metadata name only `18791/ws`

**Depends on:** T1, T2

**Touches:** AGENTS.md, apps/AGENTS.md, apps/daemon/AGENTS.md, gateway/AGENTS.md,
gateway/README.md, gateway/src/http/openapi.yaml, docker/Dockerfile,
docker/README.md, e2e/scripts/boot-full-mode.sh, SECURITY.md, docs/architecture/**,
docs/product/changelog.md, docs/specs/legacy-websocket-port-retirement/**,
docs/specs/README.md

**Verification mode:** goal-based check

**Tests:**

- No TDD stub (documentation and repository-audit task).
- Run the scoped legacy-symbol/literal audit and spec-status linter (AC5-AC6).

**Approach:**

- Replace current dual-port descriptions and examples with the unified route.
- Add a changelog migration note for external integrations.
- Mark the spec shipped only after all gates and reviews pass.

**Done when:** current code/tests/living docs have no legacy surface and the
migration target is explicit.

## Rollout

Ship as one breaking-change pull request. Deployment requires no data or
infrastructure migration, but external integrations must change
`ws://<host>:18790` to `ws://<host>:18791/ws` before upgrading. Rollback is the
prior release; no compatibility listener is retained in this release.

## Risks

- Untracked external clients may still use the old port and will fail to
  connect after upgrade; the changelog and current docs provide the migration.
- Accidentally deleting shared routing code could leave `/ws` connected but
  unable to deliver events; existing router/handler tests plus workspace/UI
  gates guard the shared path.
- Stale serialized gateway config keys may be silently ignored by serde's
  existing behavior; this change does not broaden into a global deny-unknown
  configuration policy.

## Changelog

- 2026-07-31: Initial full-mode plan for the user-approved retirement.
- 2026-07-31: Linked accepted RFC-0019 after plan review identified the
  repository's public-interface governance requirement.
