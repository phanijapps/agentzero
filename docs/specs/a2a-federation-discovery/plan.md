# Plan: A2A Federation and Discovery

- **Spec:** [`spec.md`](spec.md)
- **Status:** Done

## Approach

Add one `gateway-a2a` sub-crate that owns the official A2A wire types, peer
configuration, Agent Card construction, client transport, task projections,
and protocol-neutral discovery registry. Extend `discovery` with a browser
port and mDNS implementation, while keeping advertisements and trust distinct.
The gateway shell owns Axum routes and composes inbound A2A requests with the
existing durable generic-task service; `gateway-execution` owns the new
remote-actor policy and model-visible outbound tools. Implement server
acceptance before outbound agent delegation so each layer has a runnable
contract and the two-daemon journey is the final integration step.

The highest-risk path is authority translation: a bearer-authenticated peer
must become bounded host provenance, not root-user authority, and every task
read/cancel/result must remain scoped to that peer. The second risk is durable
correlation across local work IDs, remote A2A task IDs, retries, restarts, and
terminal steering delivery. Tests therefore lead each task and use temporary
SQLite stores plus fake discovery/HTTP transports before the final real-daemon
smoke.

## Constraints

- Follow [RFC-0020](../../rfc/0020-a2a-zbot-federation.md) and the exact
  [A2A 1.0 specification](https://a2a-protocol.org/latest/specification/).
- `contracts/openapi/a2a-federation.yaml` is the in-repo supported-surface
  contract. `a2a-lf` 0.3.0 is the typed wire-model source; applicable A2A
  CLI/TCK checks are the external interoperability oracle.
- Preserve the one-listener gateway decision from
  [RFC-0019](../../rfc/0019-retire-legacy-websocket-port.md); A2A routes share
  port `18791`.
- Preserve the authoritative durable-work and broker-neutral transport seams
  from `docs/specs/durable-work-queue/` and
  `docs/specs/durable-generic-agent-tasks/`.
- Preserve same-daemon `agent.peer-message.v1` semantics and public HTTP/WS
  behavior outside the new A2A routes and tools.
- No API-contract authoring skill is installed; the OpenAPI file is authored
  directly, linted as YAML/OpenAPI, and cross-checked against typed Rust
  fixtures and the official protocol.

## Construction tests

**Integration tests:**

- Router tests build the real Axum A2A routes with temporary peer/work/state
  and conversation stores, proving auth, persist-before-response, peer-scoped
  task projection, cancellation, and normalized errors end to end.
- Durable outbound tests run the real work worker against a fake A2A HTTP peer,
  stop/recreate the worker between submit and terminal polling, and assert one
  stable local result plus attributed steering delivery.

**Deferred external verification (AC17):**

- [a2a-external-conformance](../../backlog.md#a2a-external-conformance) will
  build `zbotd` and `zbot`, start two isolated daemons, discover and pair them,
  ask zBot A to delegate a generic research task to zBot B, continue interacting
  with A, restart B while work is pending, and observe the attributed terminal
  result at A.
- That follow-up will run the official `a2a-cli` or applicable A2A TCK checks
  against B's Agent Card and non-streaming HTTP+JSON surface and record command,
  exit status, and supported/unsupported operation results.

## Design (LLD)

### Design decisions

- Use `a2a-lf` only, not `a2a-server-lf` or `a2a-client-lf`; this retains the
  normative 1.0 model without introducing incompatible Axum/Reqwest router
  versions. Traces to: AC1, AC2, AC7, AC10, AC17 ·
  `contracts/openapi/a2a-federation.yaml`.
- Use A2A tasks, not the local peer-message envelope, at the daemon boundary.
  Traces to: AC7–AC14 · `contracts/openapi/a2a-federation.yaml`.
- Represent discovery candidates and trusted peers as separate types and
  stores. There is no promotion-by-observation path. Traces to: AC3, AC6.
- Add `RuntimeActorKind::RemotePeer` instead of treating an authenticated peer
  as root or ward authority. Traces to: AC8, AC15.
- Use the existing gateway listener and explicit HTTPS/private-network policy;
  do not add a TLS listener or bespoke encryption. Traces to: AC5, AC16.

### Data & schema

`gateway-a2a` defines:

- `DiscoveredPeerCandidate { node_id, instance_name, addresses, port,
  agent_card_path, protocol_version, expires_at }`, containing no trust state.
- `TrustedPeer { node_id, display_name, origin, target_agent_id,
  allow_private_http, outbound_credential, inbound_credentials, created_at }`,
  serialized through a redacted DTO and validated on load. Each inbound entry
  carries only `{ credential_id, credential_hash, created_at, expires_at,
  revoked_at }`; issuance defaults to 90 days, caps at 365 days, and at most
  two may be active to permit deliberate rotation.
- `A2aInboundTask`, `A2aOutboundDispatchV1`, and `A2aOutboundPollV1` durable
  payloads with canonical peer, local work, remote task, originating
  session/execution, and dedupe IDs.
- `RemoteTaskCorrelation` crosses the immutable-work boundary as a second
  durable poll item enqueued before dispatch completion, rather than an
  unaudited in-memory map or an in-place payload mutation.
- Inbound A2A work uses a random opaque A2A task ID as the durable envelope's
  correlation ID. Its work ID, execution ID, and session ID remain internal.
  `WorkScope` queries bind source, kind, provenance actor, and correlation ID so
  authorization and lookup are one database predicate rather than a read then
  authorize race.

The peer file lives at `config/a2a-peers.json`, is versioned, strict, bounded,
atomically replaced, owner-readable, and rejected if it is a symlink or has
unsafe ownership/permissions on platforms that expose those checks. Inbound
tokens are SHA-256 hashes compared with a constant-time helper; outbound tokens
are write-only in responses and redacted from debug output. The durable work
schema gains a `canceled` terminal state plus an index supporting peer-scoped
`(source, kind, provenance_actor_id, correlation_id, created_at, id)` queries.
The generic `WorkStore` gains bounded scoped find/list/cancel operations; no
A2A-only ownership table or duplicated task state is introduced.

Traces to: AC3–AC5, AC7, AC9, AC13, AC15.

### Interfaces & contracts

The public interface is the OpenAPI surface in
`contracts/openapi/a2a-federation.yaml`:

- `GET /.well-known/agent-card.json` is public and cacheable.
- `/a2a/*` uses Bearer authentication, `A2A-Version: 1.0`, and
  `application/a2a+json`.
- The Agent Card declares the same HTTP Bearer scheme and requirement; the
  value remains out-of-band. Known optional operations and the `/a2a/*`
  fallback return standard unsupported-operation envelopes.
- Implemented operations are send, get, list, and cancel; streaming,
  subscription, push configuration, and extended card capabilities are false
  or unsupported.

The model-facing interface adds `list_zbots` and `delegate_to_zbot`. The
delegation tool accepts only `{ peer_id, content }`; trusted context supplies
source execution/session/actor and the peer store supplies URL/token. The tool
returns `{ status: "queued", task_id }` only after durable insertion.

The internal interfaces are `PeerStore`, `CandidateRegistry`, `A2aTransport`,
`A2aInboundRuntime`, `A2aDelegationService`, and the generic durable
`WorkScope` query/control surface. MQTT/Kafka/registry adapters can implement a
seam later without entering these public contracts.

Traces to: AC1–AC3, AC7, AC9, AC12, AC17 ·
`contracts/openapi/a2a-federation.yaml`.

### Component / module decomposition

- `gateway/gateway-a2a/src/{card,client,config,model,registry,task}.rs` owns the
  standard model adapters and federation domain. It depends downward on
  `a2a-lf`, serde, Reqwest 0.12, and small utility crates, not on the gateway
  shell or runtime.
- `discovery/src/{browser,mdns_browser}.rs` owns candidate event production and
  cleanup, beside the existing advertiser.
- `gateway/src/http/a2a.rs` authenticates and maps Axum HTTP requests into the
  domain service, rejects browser origins outside an exact A2A allowlist, and
  owns the A2A unsupported-operation fallback. `gateway/src/a2a_tasks.rs`
  bridges protocol tasks to existing work/state/transcript services.
- `gateway/gateway-execution/src/{a2a,tools/list_zbots,tools/delegate_to_zbot}.rs`
  owns trusted execution context, remote-result prompt envelopes, actor policy,
  the isolated public-only remote prompt builder, and model tools behind an
  injected service trait.
- `apps/cli/src/peers.rs` provides local discovery/list/token/add/remove
  commands by using the same strict peer-store library; it never sends secrets
  through the daemon's unauthenticated admin HTTP surface.
- `gateway/src/server.rs` wires browser, peer store, A2A services, exact durable
  handlers, router state, and supervised shutdown once.

Traces to: AC3, AC6–AC8, AC12–AC16.

### State & control flow

Inbound flow:

1. Axum reads the latest peer snapshot, authenticates Bearer token to `peer_id`
   before accepting the body, and applies per-origin/global throttling.
2. Strict A2A/version/media/text validation produces a host-owned request.
3. The gateway generates an opaque A2A task ID, persists an
   `agent.a2a-inbound.v1` task carrying it as correlation with peer provenance,
   and returns a submitted A2A Task.
4. The existing worker starts/resumes one remote-safe execution.
5. Get/list/cancel re-read durable state and re-authorize peer ownership.
6. Terminal projection reads the canonical assistant transcript row and emits
   a bounded A2A text artifact.

Outbound flow:

1. An authorized local root/ward tool call resolves `peer_id`, derives source
   context, persists `agent.a2a-outbound.v1`, and returns queued.
2. The dispatch handler resolves the trusted peer, sends idempotent
   `SendMessage`, persists a correlated poll item, then completes dispatch.
3. Retry/restart resumes the poll item; a crash before poll persistence may
   replay send with the same message ID. zBot peers deduplicate that identity;
   other trusted A2A servers may observe at-least-once submission.
4. Terminal state becomes a bounded attributed steering envelope for the
   originating execution; the stable local work ID is the dedupe token.

Traces to: AC7, AC9–AC14, AC16.

### Behavior & rules

- Message bodies require `returnImmediately: true` and allow one `ROLE_USER`
  text part only. Continuation/task IDs, references, extensions, metadata, all
  other part kinds, empty/multiple text parts, omitted/false immediate return,
  and oversized values fail before enqueue.
- A peer may access only durable A2A tasks selected by one exact `WorkScope`
  whose source and kind are A2A-owned, provenance actor is
  `a2a:<peer_id>`, and correlation is the opaque A2A task ID; missing and
  foreign IDs share one normalized not-found response.
- Candidate expiry affects discovery display only. Trusted routes change only
  through a local peer-store mutation, and every inbound/outbound operation
  reads the current atomic snapshot so CLI changes take effect without restart.
- Idempotency uses authenticated peer ID plus client message ID. Same payload
  returns the same task; different payload with the same identity conflicts.
- Remote actors cannot see or invoke outbound A2A tools, preventing federation
  loops in this contract. Their complete model inventory is exactly `respond`;
  peer-result continuations use that same singleton inventory until a
  subsequent local-user turn.

Traces to: AC2–AC9, AC12, AC15.

### Failure, edge cases & resilience

- Peer-store uncertainty fails startup of the A2A surface without disabling the
  rest of zBot; malformed or unsafe files are never repaired silently.
- Discovery startup failure degrades to configured trusted peers and emits one
  normalized health signal; it does not disable explicitly configured A2A.
- Authentication, authorization, and stored-payload uncertainty fail closed.
- Outbound connect/timeout/5xx errors retry with existing capped work backoff;
  4xx/auth/protocol/ownership failures dead-letter with normalized reason codes.
- The outbound client disables redirects and bounds DNS, connect, response,
  body, and total task-poll durations.
- A2A middleware rejects disallowed browser origins before authentication even
  when the outer gateway has wildcard CORS enabled.
- The steer/complete crash gap remains at-least-once. Remote result envelopes
  carry the stable local work ID and instruct the recipient to ignore duplicate
  IDs already handled.

Traces to: AC2–AC6, AC10–AC16.

### Quality attributes (NFRs)

- The spec's canonical limits table is the single source of truth. Config
  parsing rejects values above each hard maximum; unit tests exercise the
  default, boundary, and one-over-maximum value for every configurable limit.
- Maximum inbound message: 1,000 Unicode code points, 4,000 UTF-8 bytes;
  maximum serialized request: 65,536 bytes.
- Agent Card and task list responses use bounded counts and response-size
  limits; list defaults to 50, caps at 100, orders by update time and task ID
  descending, and omits artifacts unless explicitly requested.
- Every outbound HTTP stage has a timeout and every per-peer outstanding and
  concurrent task count has a configured bounded default.
- Secrets and content are absent from diagnostic channels; tests install a
  capture subscriber and inspect emitted fields.
- A2A-disabled startup registers no remote routes that accept work, starts no
  browser/handler, and adds no model-visible tools.

Traces to: AC1, AC4, AC7, AC9, AC15–AC17.

### Dependencies & integration

- New runtime dependency: `a2a-lf = 0.3.0` for A2A 1.0 wire types.
- Small security dependencies may be added only for OS randomness, secret
  zeroization/redaction, and constant-time comparison after lockfile/audit
  verification; do not implement these primitives ad hoc.
- Existing dependencies: Axum 0.7, Reqwest 0.12 rustls, Tokio, serde,
  `mdns-sd`, SQLite work/state/transcript stores, and the steering registry.
- External verification: official `a2a-cli` or applicable A2A TCK.

Traces to: AC1–AC5, AC17.

## Tasks

### T1: The A2A 1.0 domain crate and supported HTTP contract agree on every wire fixture

**Depends on:** none

**Touches:** `Cargo.toml`, `Cargo.lock`, `gateway/gateway-a2a/**`, `contracts/openapi/a2a-federation.yaml`

**Verification mode:** TDD plus goal-based contract check (AC1, AC2, AC7,
AC9, AC10, AC17).

**Construction artifacts:** `stub: draft (uncompiled)` — the Rust crate and
typed surface do not exist at PLAN, so the work-loop validation rule degrades
instead of inventing an uncompilable import. T1 starts by creating
`gateway/gateway-a2a/tests/wire_contract.rs` with
`agent_card_declares_bearer_auth`, `send_requires_immediate_text`,
`task_projection_matches_contract`, and `standard_errors_are_bounded`, plus
`gateway/gateway-a2a/tests/openapi_contract.rs` with
`openapi_matches_supported_surface`, before production modules.

**Tests:**

- Rust fixtures serialize/deserialize the supported Agent Card, text message,
  submitted/completed task, pagination, cancellation, and standard error shapes
  exactly as the OpenAPI and official A2A 1.0 examples require.
- Validators reject multiple/unsupported parts, roles, unknown required
  extensions, bad versions/media types, missing required fields, and all size
  boundaries.
- Goal check parses the OpenAPI YAML, resolves every `$ref`, verifies its
  `x-spec` backlink, and compares advertised paths/capabilities with fixtures.

**Approach:**

- Add `gateway-a2a` as a workspace member and pin `a2a-lf` 0.3.0.
- Implement strict wrapper validation around the official forward-compatible
  serde types and redacted error/debug types.
- Build Agent Card/task/error projections and contract fixtures before any
  network handler.

**Done when:** crate tests and contract checks are green and `cargo tree` shows
no `a2a-server-lf`, `a2a-client-lf`, Axum 0.8, or Reqwest 0.13 introduction.

### T2: Discovery browsing produces expiring untrusted candidates without changing trusted peers

**Depends on:** T1

**Touches:** `discovery/src/{lib,browser,mdns_browser}.rs`, `discovery/tests/**`, `gateway/gateway-a2a/src/registry.rs`

**Verification mode:** TDD plus goal-based loopback integration (AC3, AC6,
AC16).

**Construction artifacts:** `stub: draft (uncompiled)` — the browser trait does
not exist at PLAN. T2 starts by creating `discovery/tests/a2a_browser.rs` with
`candidate_events_are_untrusted_and_expire`,
`random_node_id_contains_no_local_identity`, and
`loopback_goodbye_removes_candidate` before the implementation.

**Tests:**

- Fake browser events add/update/remove bounded candidates, expire stale
  candidates, reject malformed TXT records, and cannot mutate a trusted record.
- A loopback mDNS advertiser/browser pair resolves the A2A TXT fields and sends
  goodbye removal without relying on the friendly alias.
- Generated advertised node IDs are random/non-identifying; explicit rotation
  is trust-breaking and neither display metadata nor TXT records reuse local
  user, host, ward, or agent identifiers.
- Browser startup/shutdown is single-owner and disabled configuration starts no
  background thread/task.

**Approach:**

- Add `Browser`, `BrowseHandle`, and candidate-event types parallel to the
  existing `Advertiser` abstractions.
- Implement `MdnsBrowser` with the existing `mdns-sd` daemon and feed a bounded
  in-memory candidate registry.
- Add A2A bootstrap TXT records to production advertisement only when A2A is
  enabled.

**Done when:** discovery unit/integration tests prove candidate lifecycle and
the existing advertisement tests remain green.

### T3: Local peer commands persist redacted explicit trust with safe transport policy

**Depends on:** T1

**Touches:** `gateway/gateway-a2a/src/{config,peers,secrets,url_policy}.rs`, `apps/cli/src/{main,peers}.rs`, `apps/cli/Cargo.toml`

**Verification mode:** TDD plus manual CLI artifact exercise (AC3–AC5, AC15).

**Construction artifacts:** `stub: draft (uncompiled)` — neither the peer-store
types nor CLI command exist at PLAN. T3 starts by creating
`gateway/gateway-a2a/tests/{peer_store,url_policy}.rs` and
`apps/cli/tests/a2a_peers.rs` with exact cases for issue/rotate/revoke,
permissions/redaction, live snapshot reload, and URL resolution before the
implementation.

**Tests:**

- Temporary peer files prove strict schema, bounds, atomic replace, symlink and
  unsafe-permission rejection, owner-only permissions, hash verification,
  constant-time helper use, and no secret in debug/list/error output.
- URL policy covers HTTPS, loopback HTTP, explicit private HTTP, redirects,
  userinfo/query/fragment, DNS rebinding, public/private/link-local/metadata
  targets, IPv4/IPv6, and peer-ID-only resolution.
- CLI parser/golden tests cover discover/list/issue/rotate/revoke/add/remove,
  bounded expiry, two-active-token cap, and never echo an existing stored
  secret. A running peer-store reader observes add/remove/revoke on its next
  operation without restart.

**Approach:**

- Implement a versioned `config/a2a-peers.json` store shared by daemon and CLI.
- Generate 256-bit credentials with an OS-backed RNG; store inbound hashes and
  display new plaintext once.
- Add local peer subcommands that operate on the data directory directly,
  avoiding the daemon's browser-facing admin routes; daemon readers parse the
  latest atomic snapshot for every security-sensitive operation.

**Done when:** the real `zbot peers token`, `add`, `list`, and `remove` commands
work against a temporary data directory and permissions/redaction are observed.

### T4: Authenticated inbound A2A tasks are durable, remote-safe, peer-scoped, and cancelable

**Depends on:** T1, T3

**Touches:** `services/execution-state/src/work.rs`, `stores/zbot-runtime-sqlite/src/schema.rs`, `stores/zbot-runtime-sqlite/tests/durable_work_store.rs`, `gateway/src/{a2a_tasks,server,state/mod}.rs`, `gateway/src/http/{a2a,mod}.rs`, `gateway/gateway-execution/src/invoke/{executor,mod}.rs`, `gateway/src/services/runtime.rs`, `gateway/Cargo.toml`

**Verification mode:** TDD and router integration (AC2, AC7–AC11, AC15,
AC16).

**Construction artifacts:** `stub: draft (uncompiled)` — the router/service and
RemotePeer actor types do not exist at PLAN. T4 starts by creating
`gateway/tests/a2a_http.rs` with
`auth_precedes_body_and_enqueue`, `send_rejects_non_immediate_and_context`,
`task_scope_prevents_cross_peer_access`, `cancel_fences_stale_worker`,
`list_is_update_descending_and_artifacts_opt_in`,
`wildcard_cors_does_not_reach_a2a`, and
`optional_routes_use_a2a_unsupported`; add
`remote_peer_inventory_is_exactly_respond` and
`remote_peer_prompt_excludes_local_context_and_canaries` beside existing
executor inventory tests before production handlers.

**Tests:**

- Authentication happens before enqueue/body effects; valid send persists before
  submitted response; retry with the same peer/message is idempotent and a
  conflicting payload fails closed.
- Omitted/false immediate return, continuation/reference/extension/metadata
  fields, disallowed browser origins, and optional operations fail before any
  queue side effect with their specified bounded response.
- `RemotePeer` and peer-result continuation actor inventories equal
  `{ respond }` even when the configured target agent normally has more tools;
  all existing actor inventory snapshots remain unchanged.
- Remote execution prompt fixtures include only fixed remote safety policy,
  approved public skill instructions, and remote text; canaries placed in every
  local prompt shard, ward file, memory result, provider/tool catalog, and prior
  conversation are absent from the model request and persisted transcript.
- Peer A cannot get/list/cancel peer B's tasks, unknown and foreign IDs are
  indistinguishable, pagination bounds hold, and task responses expose no local
  provenance IDs.
- Existing databases migrate to the canceled work state and peer-scope index;
  scoped find/list/cancel use one ownership predicate, cursor order is stable,
  and a stale leased worker cannot settle canceled work.
- Pending/running cancellation, restart recovery, terminal state mapping, and
  canonical assistant artifact projection cover streamed text and `respond`
  argument-only completion.

**Approach:**

- Add Bearer authentication and A2A Axum routes on the existing listener.
- Generalize durable agent-task creation minimally to accept trusted remote
  provenance and `RemotePeer` actor kind without weakening local Research.
- Extend the generic work store with bounded `WorkScope` find/list/cancel and a
  canceled terminal transition, then implement A2A projections over those
  operations and the existing execution handles.

**Done when:** the real router passes temporary-SQLite end-to-end tests and an
unauthenticated/foreign peer produces no observable task side effect.

### T5: Local agents delegate through durable outbound A2A work and receive attributed results

**Depends on:** T1, T3, T4

**Touches:** `gateway/gateway-a2a/src/client.rs`, `gateway/src/a2a_tasks.rs`, `gateway/gateway-execution/src/{a2a,tools/mod,tools/list_zbots,tools/delegate_to_zbot,invoke/executor}.rs`, `contracts/jsonschema/a2a-outbound-task.schema.json`

**Verification mode:** TDD plus fake-peer integration (AC12–AC15).

**Construction artifacts:** `stub: draft (uncompiled)` — the outbound adapter,
durable payloads, and tool types do not exist at PLAN. T5 starts by creating
`gateway/tests/a2a_outbound.rs` with
`tool_queue_is_nonblocking`, `dispatch_handoff_survives_restart`,
`zbot_peer_deduplicates_replayed_message_id`,
`external_peer_replay_is_reported_at_least_once`, and
`peer_result_continuation_has_no_side_effect_tools`; add exact root/ward/remote
inventory assertions beside the existing unique-tool tests.

**Tests:**

- Tool inventories expose `list_zbots`/`delegate_to_zbot` only to root/ward,
  keep names unique, and accept no URL/token/source identity arguments.
- Queue acceptance is persist-before-queued and non-blocking; stored provenance
  matches the trusted caller and peer config; the dispatch-to-poll handoff is
  durable before dispatch completion.
- Fake A2A transport proves exact headers/media/body, no redirects, timeout and
  retry classification, remote correlation persistence before polling, stable
  message-ID replay with zBot deduplication, honest at-least-once behavior for
  other peers, cancellation, and terminal mapping.
- Result delivery is bounded, attributed, untrusted, at-least-once with stable
  ID, redacted from logs, and unavailable to semantic-memory ingestion.

**Approach:**

- Define/inject `A2aDelegationService` at the gateway-execution boundary and add
  two Rig tools using the existing trusted execution context pattern.
- Register `agent.a2a-outbound.v1` with the existing exact-handler worker.
- Implement the Reqwest 0.12 transport from trusted peer records and deliver
  terminal results through the steering registry.

**Done when:** a local test execution queues remote work, continues, survives a
worker restart, and later receives exactly the specified attributed envelope.

### T6: Production wiring and repository verification close the delivered operator journey

**Depends on:** T2, T4, T5

**Touches:** `gateway/src/{server,state/mod}.rs`, `apps/daemon/src/main.rs`, `e2e/**`, `docs/architecture/**`, `docs/product/changelog.md`, `docs/specs/{durable-work-queue,durable-queue-worker-runtime,durable-generic-agent-tasks}/spec.md`, `docs/specs/README.md`, `docs/rfc/README.md`

**Verification mode:** goal-based gates plus repository integration QA
(AC1–AC16). External A2A CLI/TCK and the full two-daemon operator journey
(AC17) remain tracked as
[`a2a-external-conformance`](../../backlog.md#a2a-external-conformance) and in
`workspace.toml`.

**Construction artifacts:** `stub: draft (uncompiled)` — the production router
and process fixture do not exist at PLAN. T6 starts by creating the repository
integration coverage and
`docs/specs/a2a-federation-discovery/verification.md`; external CLI/TCK and
two-daemon evidence are explicitly deferred to
[`a2a-external-conformance`](../../backlog.md#a2a-external-conformance).

**Tests:**

- Production composition registers browser and exact inbound/outbound handlers
  once, rejects duplicates, disables all A2A behavior by default, and drains on
  shutdown.
- Repository integration tests prove pairing and trust boundaries,
  send/get/list, cross-peer denial, cancel and restart-state behavior,
  asynchronous local continuation, and terminal result delivery.
- The deferred
  [`a2a-external-conformance`](../../backlog.md#a2a-external-conformance) work
  item owns official A2A CLI/TCK validation and the documented two-daemon
  pair/delegate/get/cancel/restart journey.
- Workspace fmt, clippy, check, tests, spec lint, dependency audit, and existing
  tool-footprint/unique-name gates pass.

**Approach:**

- Wire services and lifecycle under the default-off config gate.
- Record repository-level verification commands/output while keeping the
  outstanding external process validation visible in the durable backlog.
- Update current-state architecture, changelog, RFC/spec indexes, and operator
  setup/troubleshooting guidance without duplicating the spec contract. Amend
  the three shipped durable-work specs with the new generic scoped-query and
  canceled-state contract plus regression evidence.

**Done when:** AC1–AC16 and their mechanical gates pass, and the unexecuted
external conformance scope is durably recorded as
[`a2a-external-conformance`](../../backlog.md#a2a-external-conformance) rather
than claimed as completed evidence.

## Rollout

- **Delivery:** A2A is disabled by default. Enabling requires local peer
  configuration and daemon restart. Disabling is the rollback and leaves peer
  configuration and durable history intact.
- **Infrastructure:** no broker or new listener. Deployments provide HTTPS, an
  encrypted VPN boundary, or an explicit private-network HTTP acceptance per
  peer.
- **External-system integration:** `a2a-lf` is pinned; the official CLI/TCK is
  verification tooling, not a daemon runtime dependency.
- **Deployment sequencing:** protocol/types and peer store land first;
  discovery and inbound server can dark-launch disabled; outbound tools land
  only after peer-scoped inbound behavior is verified. Repository documentation
  ships with AC17 deferred to
  [`a2a-external-conformance`](../../backlog.md#a2a-external-conformance), where
  the external two-daemon and CLI/TCK output will be recorded.

## Risks

- Adding `RemotePeer` to exhaustive actor-policy matches can accidentally
  broaden or hide tools; inventory snapshots and default-deny matches are
  mandatory.
- Reusing generic-agent work can leak local identifiers or local-user authority
  if projection/authentication is misplaced; peer-scoped integration tests must
  use two credentials and overlapping guessed IDs.
- Adding cancel and scoped-list semantics to the generic durable store can
  regress worker settlement or pagination; migration, stale-lease, and existing
  queue conformance tests are mandatory.
- The official Rust SDK is young despite implementing stable A2A 1.0; pin the
  core crate and validate wire behavior externally before release.
- Owner-only file semantics differ across Unix and Windows; use platform-native
  checks and fail closed where guarantees cannot be established.
- A private-network HTTP opt-in transfers confidentiality responsibility to the
  deployment boundary; warnings and peer status must make that posture visible.
- Restart between remote send and correlation persistence can create duplicate
  remote work unless the A2A client message ID is stable and server idempotency
  is verified.

## Declined patterns

- Complete SDK server/client adoption: declined because it introduces
  incompatible gateway-library majors in this change.
- Automatic mDNS pairing: declined because discovery does not authenticate.
- A generic arbitrary-URL A2A tool: declined because it creates SSRF and secret
  exfiltration paths.
- Root-authority inbound executions: declined because peer trust is not local
  user authority.
- New TLS listener: declined to preserve the one-listener gateway; HTTPS is
  supplied by the deployment boundary and insecure private HTTP is explicit.
- Broker abstraction implementation: declined because the existing transport
  ports already keep that future open.

## Resolve-vs-surface record

- Resolved in plan: A2A version/binding, supported operations, official Rust
  type source, trust bootstrap, transport defaults, remote actor authority,
  durable task mapping, discovery/trust separation, and protocol exclusions.
- Surface before implementation if: the official core crate cannot compile
  without the excluded HTTP stack; remote-safe execution cannot be expressed as
  a default-deny actor intersection; platform peer-file permissions cannot fail
  closed; or applicable A2A TCK behavior contradicts the authored contract.

## Changelog

- 2026-08-06: initial full-risk plan for A2A 1.0 LAN/VPN zBot federation.
