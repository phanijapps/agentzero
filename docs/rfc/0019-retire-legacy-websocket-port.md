# RFC-0019: Retire the Legacy WebSocket Port

- **Status:** Accepted
- **Author:** phanijapps
- **Approver:** phanijapps
- **Date opened:** 2026-07-31
- **Date closed:** 2026-07-31
- **Related:** `docs/specs/legacy-websocket-port-retirement/`

## The ask

- **Recommendation (BLUF):** Remove the deprecated, opt-in standalone
  WebSocket listener on port `18790` in the next release. Keep the supported
  WebSocket protocol unchanged at `ws://<host>:18791/ws` and publish that
  migration in the changelog.
- **Why now (SCQA):** z-Bot has served WebSockets through the HTTP listener
  since April 2026, while the old listener has remained disabled by default
  behind an explicit compatibility flag. Its documented one-release grace
  period has passed, and retaining it preserves a second network surface,
  dependency, config shape, and client special case. The decision is whether
  to complete that announced retirement now or extend compatibility again.
- **Decision requested:** retire `18790` in the next release, remove its daemon
  flags and gateway configuration, and direct external integrations to
  `18791/ws`. Recommended and approved by `phanijapps` on 2026-07-31.

## Problem & goals

The gateway currently has two WebSocket accept paths for one wire protocol.
The supported Axum path shares the HTTP listener at `/ws`; the disabled-by-
default raw tungstenite path exists only for integrations hardcoded to the old
port. Keeping both paths makes network exposure, startup, configuration,
documentation, tests, and dependencies describe behavior normal installs do
not use.

**Goals:**

- Make the configured HTTP port the gateway's only listener and `/ws` its only
  client event-stream WebSocket endpoint.
- Remove the legacy flags, serialized config fields, raw accept loop, and
  port-specific UI behavior.
- Give external consumers one explicit migration from
  `ws://<host>:18790` to `ws://<host>:18791/ws`.

**Non-goals:**

- Changing the WebSocket message protocol, authentication, CORS, LAN exposure,
  discovery, or generic custom endpoint overrides.
- Changing the separate `/bridge/ws` worker transport, which already shares
  the HTTP listener and serves a different protocol and consumer.
- Adding a redirect, proxy, compatibility daemon, or configurable second
  listener.
- Rewriting frozen historical artifacts that accurately describe earlier
  behavior.

## Proposal

Delete `--ws-port` and `--legacy-ws-port-enabled` from `zbotd`; delete
`websocket_port`, `legacy_ws_port_enabled`, the default-port constant, and the
standalone address helper from `GatewayConfig`; and remove the conditional
startup branch and raw tokio-tungstenite connection driver. The existing
`WebSocketHandler` continues to own shared sessions, subscriptions, message
dispatch, cleanup, and event routing for the Axum `/ws` handler.

Remove only the UI's port-`18790` warning and fixtures. Preserve opaque
`gateway_ws` and `TransportConfig.wsUrl` overrides for development and custom
reverse proxies. Update current documentation, deployment metadata, OpenAPI
text, and the Unreleased changelog. Existing external clients must change the
port and add `/ws` before installing the release containing this change.

## Options considered

The exhaustive axis is **retirement timing**: the compatibility endpoint is
either unscheduled, scheduled after another grace release, or removed in the
next release.

| Option | Trade-off | Decision |
| --- | --- | --- |
| Keep indefinitely with no removal date | Avoids breaking unknown clients but permanently retains the second listener, config surface, dependency, and documentation burden. | Rejected |
| Announce a new sunset and retain it for another release | Gives hardcoded clients another window, following the staged-deprecation pattern, but repeats a grace period already documented as one release with no evidence of an in-repo consumer. | Rejected |
| Remove in the next release and publish the migration | Completes the existing deprecation and reduces exposed/configurable surface; unknown external clients must migrate before upgrading. | **Accepted** |

This timing model follows established deprecation practice: first announce the
future removal, then make the removal and migration conspicuous. The repository
already completed the announcement stage and has shipped releases since.

## Risks & what would make this wrong

- **Pre-mortem:** an external integration still connects directly to `18790`
  and stops receiving events after upgrade. Mitigation: a prominent Removed
  changelog entry with the exact replacement URL; rollback is the prior
  release.
- **Pre-mortem:** shared event-routing code is deleted with the raw listener,
  producing successful `/ws` connections without streamed events. Mitigation:
  preserve unconditional background-task startup and run gateway/UI protocol
  tests.
- **Falsifiable assumption:** no in-repository production client enables or
  requires the standalone listener. Engram impact analysis and a scoped source
  audit found none; discovering one before merge invalidates immediate removal.
- **Falsifiable assumption:** the one-release grace period has elapsed. Git
  history dates unified-port introduction to 2026-04-24 and the compatibility
  comment to 2026-05-15; repository tags include later releases through
  `v2026.7.30`.
- **Drawback:** stale CLI invocations and YAML configurations lose a
  compatibility option. This is the intended breaking change, not silently
  emulated behavior.

## Evidence & prior art

- **Spike / de-risk result:** Engram found the primary `18791/ws` path and the
  opt-in legacy path but no caller relationship for the standalone
  `WebSocketHandler::run`. A repository search found no production client that
  passes either legacy flag; the UI defaults to same-origin `/ws`.
- **Repo precedent:** commits `02d56fb3` and `5474a1f8` introduced the unified
  path and made shared event routing independent of the legacy accept loop.
  Commit `5c9cb278` documented a one-release compatibility period. Current
  architecture documentation identifies `18791/ws` as primary and `18790` as
  disabled and slated for removal.
- **External prior art:** [RFC 8594](https://www.rfc-editor.org/rfc/rfc8594.html)
  models service sunset as a point after which a resource may become
  unavailable and recommends linking migration information. [Keep a
  Changelog](https://keepachangelog.com/en/1.1.0/) distinguishes Deprecated
  from Removed and recommends making deprecations, removals, and breaking
  changes conspicuous to upgraders. [Semantic Versioning
  2.0.0](https://semver.org/) classifies incompatible public-API changes as
  breaking; z-Bot's CalVer release process still needs to communicate that
  compatibility impact explicitly.

## Open questions

None. The approver accepted immediate next-release removal on 2026-07-31.

## Follow-on artifacts

- Spec: `docs/specs/legacy-websocket-port-retirement/`
- Living architecture, contributor, security, OpenAPI, deployment, and product
  changelog updates in the implementation PR.
