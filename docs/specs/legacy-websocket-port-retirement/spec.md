# Spec: Legacy WebSocket Port Retirement

- **Status:** Shipped
- **Owner:** phanijapps
- **Plan:** [`plan.md`](plan.md)
- **Constrained by:** RFC-0019
- **Brief:** none
- **Contract:** none
- **Shape:** mixed

> **Spec contract:** this document defines what "done" means. The implementing
> PR must match this spec, or update it. Verification must be derivable from it.

## Objective

Retire the deprecated standalone WebSocket listener on port `18790` so z-Bot
has one network entry point: HTTP and WebSocket upgrades on port `18791`, with
WebSockets available at `/ws`. Remove the legacy daemon flags, gateway config,
runtime listener, client warning, and living-document references while giving
external integrations an explicit migration target of
`ws://<host>:18791/ws`.

## Boundaries

### Always do

- Preserve the existing WebSocket protocol, sessions, subscriptions, event
  router, and `/ws` Axum upgrade behavior.
- Keep generic explicit `gateway_ws` and `TransportConfig.wsUrl` overrides so
  development and custom reverse-proxy endpoints remain supported.
- Update living documentation and examples to name `18791/ws` as the only
  supported client event-stream WebSocket endpoint.

### Ask first

- Change the WebSocket message protocol, authentication posture, or CORS
  policy.
- Add a compatibility proxy, redirect, feature flag, or replacement listener.
- Remove generic custom WebSocket URL configuration.
- Change the separate `/bridge/ws` worker transport, which remains on the
  shared HTTP listener and is not a replacement client event endpoint.

### Never do

- Leave any runtime path capable of binding the standalone `18790` listener.
- Add a dependency, module boundary, or second network entry point.
- Rewrite frozen ADRs, RFCs, or shipped specs to erase historical references.

## Testing Strategy

- **Goal-based Rust checks:** gateway and daemon compilation/tests prove the
  removed fields, flags, listener, and dependency have no remaining caller.
- **Gateway route smoke test:** an ephemeral HTTP listener upgrades `/ws`,
  receives the initial `Connected` frame, routes `Ping` to `Pong`, and closes
  cleanly through the production Axum route.
- **Goal-based UI checks:** TypeScript, lint, Vitest, and the production build
  prove generic endpoint overrides and the same-origin `/ws` default remain
  intact after removing legacy-specific behavior.
- **Goal-based repository audit:** a scoped search proves runtime, current
  tests, configuration, and living docs no longer advertise or reference port
  `18790` or its removed flags.

## Acceptance Criteria

- [x] The daemon no longer accepts `--ws-port` or
  `--legacy-ws-port-enabled`, and `GatewayConfig` no longer serializes or
  exposes the corresponding legacy fields/helpers.
- [x] `GatewayServer` starts only its HTTP listener, and WebSocket traffic
  continues through `ws://<host>:<http-port>/ws` with background routing and
  shutdown behavior preserved.
- [x] The raw standalone tungstenite listener and its gateway-only dependency
  are removed.
- [x] The UI defaults to same-origin `/ws`, retains arbitrary explicit
  `gateway_ws` overrides, and contains no special handling for port `18790`.
- [x] Current tests, OpenAPI text, container metadata, security documentation,
  architecture documentation, and contributor instructions describe the
  unified `/ws` route as the sole client event-stream WebSocket endpoint.
- [x] Rust formatting, clippy, workspace type-check/tests, UI TypeScript,
  lint, tests, and production build pass.

## Assumptions

- Technical: Engram identifies `18791/ws` as the primary WebSocket endpoint and
  the `18790` listener as opt-in legacy compatibility slated for removal
  (source: Engram context retrieval, 2026-07-31).
- Technical: no in-repository production client enables or depends on the
  standalone listener (source: Engram impact analysis plus scoped repository
  search, 2026-07-31).
- Process: RFC-0019 accepts completing the already-documented deprecation in
  the next release (source: `docs/rfc/0019-retire-legacy-websocket-port.md`).
- Product: breaking external integrations still hardcoded to `18790` is an
  accepted consequence; their migration target is `18791/ws` (source: user
  confirmation 2026-07-31).

## Verification Evidence

- `cargo fmt --all -- --check`, `cargo clippy --all-targets -- -D warnings`,
  `cargo check --workspace`, and `cargo test --workspace` passed.
- `cargo test -p gateway --test websocket_route` passed the real HTTP upgrade,
  initial connection frame, routed ping/pong, and clean-close smoke test.
- `npx tsc --noEmit`, `npm run lint`, all 1,352 Vitest tests, and
  `npm run build` passed; lint reported 20 pre-existing warnings and no errors.
- `zbotd --help` exposes `--http-port` and neither removed WebSocket flag.
- The scoped current-source audit found no retired port, flag, field, constant,
  or address helper outside the frozen RFC/spec migration record.
