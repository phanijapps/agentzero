# Spec: A2A Federation and Discovery

- **Status:** Shipped
- **Owner:** phanijapps
- **Plan:** [`plan.md`](plan.md)
- **Constrained by:** [RFC-0020](../../rfc/0020-a2a-zbot-federation.md),
  Durable Work Queue,
  Durable Queue Worker Runtime,
  Durable Generic Agent Tasks
- **Brief:** none
- **Discovery:** none
- **Contract:** [`contracts/openapi/a2a-federation.yaml`](../../../contracts/openapi/a2a-federation.yaml), [`contracts/jsonschema/a2a-outbound-task.schema.json`](../../../contracts/jsonschema/a2a-outbound-task.schema.json)
- **Shape:** mixed

> **Spec contract:** this document defines what "done" means. The implementing
> PR must match this spec, or update it. Verification must be derivable from it.

## Objective

An operator can explicitly pair zBots on a LAN or VPN, after which root and ward
agents can delegate bounded text work to a trusted remote zBot without blocking
their current execution. The remote zBot exposes an interoperable A2A 1.0 Agent
Card and durable HTTP+JSON task surface, runs the request under remote-safe
authority, and returns an attributed terminal result that survives daemon
restart and does not expose either zBot's internal sessions, credentials,
memory, or queue schema.

## Boundaries

### Always do

- Keep federation disabled by default and require an explicit local trust
  action before any discovered candidate can receive or submit work.
- Use the A2A 1.0 HTTP+JSON data model, media type, version header, error model,
  Agent Card path, and task semantics; use the official Rust core types as the
  wire-model source.
- Authenticate before enqueue, derive peer identity from trusted credential
  context, and re-authorize every get, list, cancel, retry, and result operation
  against the owning peer.
- Persist inbound and outbound work before returning acceptance, retain stable
  task/work IDs across retries, and reuse the existing durable queue and
  canonical terminal assistant persistence path.
- Treat mDNS records, Agent Cards, remote messages, and remote results as
  untrusted data. Bound, validate, attribute, and redact them at network,
  persistence, log, and prompt boundaries.
- Run inbound work with a remote-specific actor policy whose effective
  capabilities are the intersection of the configured target agent and the
  remote-safe allowlist.
- Require HTTPS unless the operator explicitly accepts private-network HTTP for
  a specific peer; reject redirects and model-supplied URLs or credentials.

### Ask first

- Add an A2A binding other than HTTP+JSON, enable streaming/subscription, push
  notifications, extended cards, signed cards, files, URL parts, structured
  parts, multi-tenancy, or public-internet federation.
- Allow a remote peer to delegate locally, invoke connectors, execute shell,
  write files, access secrets, forward to another peer, or receive any other
  side-effecting capability.
- Add automatic trust, automatic endpoint mutation for trusted peers, a hosted
  registry, or a broker implementation such as MQTT, Kafka, or NATS.
- Change same-daemon peer-message semantics, public UI/WS contracts, the
  conversation schema, or the existing durable queue envelope.

### Never do

- Treat discovery presence, hostname, IP address, Agent Card content, task ID,
  model arguments, or routing labels as authentication or authorization.
- Put bearer credentials in Agent Cards, mDNS TXT records, model-visible tools,
  URLs, logs, errors, traces, transcripts, semantic memory, or API responses.
- Send credentials or task content over plain HTTP unless that peer has an
  explicit private-network HTTP opt-in; never silently downgrade HTTPS.
- Execute remote content as system policy, interpolate it into SQL/shell/log
  templates, or grant a remote actor the local user's authority.
- Claim exactly-once delivery, exactly-once interpretation, automatic WAN
  safety, or A2A conformance for capabilities the Agent Card does not advertise.
- Introduce ACP/A2C, a second daemon listener, a broker, or a second private
  cross-daemon message protocol in this feature.

## Testing Strategy

- **Protocol model and HTTP binding:** TDD with contract fixtures and router
  integration tests for Agent Card serialization, version/media negotiation,
  request/response shapes, standard errors, and disabled optional operations.
- **Trust, credential, and URL policy:** TDD with filesystem and HTTP fakes,
  proving owner-only atomic persistence, one-time credential display, hashed
  inbound verification, secret redaction, HTTPS defaults, explicit private HTTP,
  redirect rejection, and SSRF-resistant peer resolution.
- **Discovery:** TDD against a fake browser event stream plus a loopback mDNS
  integration check, proving candidates remain untrusted and cannot overwrite
  trusted routing state.
- **Inbound durable tasks:** TDD with temporary SQLite state/work/transcript
  stores, proving persist-before-submit, peer-scoped task access, cancellation,
  remote actor capability restriction, restart recovery, and terminal artifact
  projection.
- **Outbound delegation:** TDD with a fake A2A transport plus temporary durable
  work state, proving non-blocking acceptance, standard client requests, capped
  retry/polling, stable IDs, restart recovery, and attributed steering delivery.
- **Interoperability and operator journey:** goal-based checks plus manual QA
  using two daemon instances and the official A2A CLI or TCK: discover, pair,
  delegate, continue local work, restart one daemon, receive the remote result,
  cancel another task, and verify cross-peer task access is denied.
- **Regression gates:** goal-based workspace formatting, clippy, check, and test
  gates, plus the existing unique-tool-inventory and prompt-footprint tests for
  every actor whose tool surface changes.

## Acceptance Criteria

### Canonical limits

| Limit | Default | Hard maximum |
|---|---:|---:|
| Serialized inbound request | 65,536 bytes | 65,536 bytes |
| Text content | 1,000 Unicode code points / 4,000 UTF-8 bytes | same |
| Trusted peers | 64 | 256 |
| Active inbound credentials per peer | 2 | 2 |
| Credential lifetime | 90 days | 365 days |
| Concurrent inbound executions per peer | 2 | 8 |
| Accepted non-terminal tasks per peer | 100 | 1,000 |
| List page size | 50 | 100 |
| Auth-failure origin bucket | capacity 10, refill 10/minute | fixed |
| Auth-failure global bucket | capacity 100, refill 100/minute | fixed |
| Outbound connect timeout | 5 seconds | 30 seconds |
| Outbound request timeout | 30 seconds | 120 seconds |
| Transport retry attempts | 8 | 20 |
| Retry/poll backoff | 1-second base, 30-second cap | fixed cap |
| Remote task deadline | 60 minutes | 24 hours |
| A2A response body | 1 MiB | 1 MiB |

Configuration may lower any configurable value. Raising one above its hard
maximum is a startup validation error. Auth buckets are keyed by the normalized
transport origin address before authentication; successful authentication does
not refund tokens, and a global bucket prevents distributed-origin bypass.

- [x] **AC1 — standard Agent Card:** when A2A is enabled, `GET
  /.well-known/agent-card.json` returns a valid A2A 1.0 Agent Card with one
  preferred `HTTP+JSON` interface under `/a2a`, bounded public identity/skill
  metadata, `text/plain` modes, a required public HTTP Bearer security scheme
  and matching security requirement, and false streaming, push-notification,
  and extended-card capabilities. It contains no credential, internal path,
  session/execution ID, provider secret, or untrusted discovered-card content.
- [x] **AC2 — protocol negotiation:** every `/a2a` operation enforces
  `A2A-Version: 1.0`; body operations enforce
  `application/a2a+json`; malformed, unsupported-version, unsupported-extension,
  and unsupported-operation requests return bounded standard A2A errors without
  enqueueing work or leaking parser/auth details. Known optional operations and
  every unmatched `/a2a/*` path use the bounded A2A unsupported-operation error
  rather than the gateway's generic 404. A2A routes reject browser `Origin`
  requests unless the origin is on an exact A2A-specific allowlist; gateway
  wildcard CORS never applies as peer authorization.
- [x] **AC3 — explicit trust:** an mDNS candidate has no routing or execution
  authority until a local CLI trust action binds its canonical node ID, exact
  normalized origin, credential direction, allowed local agent/skill, and
  transport policy. Every authentication, outbound dispatch, list tool, and
  task operation reads the current atomically replaceable peer-store snapshot;
  removing trust therefore blocks new inbound and outbound calls in a running
  daemon without deleting historical tasks or waiting for restart.
- [x] **AC4 — secret lifecycle and handling:** generated credentials contain at
  least 256 bits of OS randomness, have an opaque credential ID, default to
  90-day expiry with a configurable maximum of 365 days, and are displayed
  once. Inbound credentials persist only as hashes
  compared in constant time; outbound credentials live only in an atomically
  replaced owner-readable local file. A peer may have at most two active
  inbound credentials for rotation, and local CLI operations can issue, list
  metadata for, and immediately revoke either credential without revealing its
  value. Expired and revoked credentials fail closed. No list command, API,
  Agent Card, tool, `Debug`, trace, error, transcript, or memory record reveals
  credential material.
- [x] **AC5 — safe endpoint resolution:** trusted endpoints accept HTTPS by
  default, permit HTTP only for loopback or an explicit per-peer
  private-network opt-in, contain no userinfo/query/fragment, do not follow
  redirects, and are resolved under an outbound policy that rejects prohibited
  destinations and rebinding. Model-visible tools accept peer IDs only, never
  URLs, headers, or credentials.
- [x] **AC6 — untrusted discovery:** the discovery browser emits bounded
  candidate records for `_zbot._tcp.local.` advertisements carrying `nodeId`,
  `a2aVersion`, and `agentCardPath`. Malformed records are dropped with
  identifier-only diagnostics; expiry removes candidates; candidate changes
  never mutate trusted peer origins or credentials. A2A discovery is off until
  explicitly enabled. Advertised node IDs are randomly generated, contain no
  hostname, user, ward, or agent name, and are stable only within one operator
  identity; explicit rotation invalidates existing peer trust. Display names
  and skills are bounded, local-operator-authored public metadata.
- [x] **AC7 — authenticated durable send:** an authenticated `POST
  /a2a/message:send` accepts exactly one non-empty text part with `ROLE_USER`,
  bounded to 1,000 Unicode code points, 4,000 UTF-8 bytes, and a 65,536-byte
  serialized body, and requires `configuration.returnImmediately: true` with
  `text/plain` output. Omitted/false immediate return, request/task context,
  `taskId`, `contextId`, `referenceTaskIds`, message/request/part metadata, and
  extensions are rejected before enqueue because continuation and extensions
  are outside this slice. It generates an opaque A2A task ID independent of zBot work,
  session, and execution identifiers, persists that ID and authorized durable
  work before returning an A2A Task in `TASK_STATE_SUBMITTED`; duplicates with
  the same peer/message ID are idempotent and conflicting reuse fails closed.
- [x] **AC8 — remote-safe execution:** inbound work runs only as the peer-bound
  target agent under `RemotePeer` actor policy. Its model-visible capabilities
  contain exactly the built-in `respond` tool. Delegation, shell, filesystem,
  connectors, memory, graph, skills, multimodal, goals, plans, wards, peer
  forwarding, trust management, dynamically configured MCP/connector tools,
  and every capability outside that singleton allowlist are absent; existing
  local actor inventories remain unchanged except for the outbound A2A tools
  explicitly described here. Its prompt/context profile contains only a fixed
  remote-safety policy, operator-approved public instructions for the advertised
  A2A skill, and the remote user's text. It excludes local SOUL, INSTRUCTIONS,
  OS and prompt shards, ward files, memory/recall, conversation history,
  provider/tool/catalog metadata, credentials, paths, and all non-public host
  context.
- [x] **AC9 — peer-scoped task access:** `GET /a2a/tasks/{id}`, `GET
  /a2a/tasks`, and `POST /a2a/tasks/{id}:cancel` expose only tasks owned by the
  authenticated peer. Ownership and task correlation are rechecked by a
  durable indexed query, not by parsing the task ID or trusting request
  metadata. Unknown, foreign, malformed, and inaccessible task IDs are
  indistinguishable; listing is ordered by status update time descending then
  opaque task ID descending, uses an opaque peer-bound cursor, defaults to 50,
  caps at 100, and never exposes zBot session/execution/work provenance.
  Artifacts are omitted by default and included only when explicitly requested.
- [x] **AC10 — honest state and result projection:** submitted, working,
  completed, failed, canceled, and rejected durable/runtime states map
  deterministically to A2A Task states. A completed task includes one bounded
  text artifact built from the canonical persisted terminal assistant row,
  including terminal content carried by `respond` tool arguments; failures
  expose normalized codes rather than raw runtime/provider errors.
- [x] **AC11 — cancellation:** an authorized cancel request fences pending work
  by durably transitioning it to canceled, or atomically fences leased work and
  signals only the matching running execution. It is idempotent once canceled,
  cannot cancel completed, foreign, or unrelated local executions, and a stale
  worker cannot overwrite the canceled terminal state. Unknown, foreign,
  malformed, and inaccessible task IDs take the same normalized 404 path;
  only an ownership-proven terminal task may return not-cancelable. Restart
  preserves the canceled terminal state.
- [x] **AC12 — non-blocking outbound delegation:** `list_zbots` and
  `delegate_to_zbot` are available only to root and ward actors. Delegation
  accepts a trusted peer ID and bounded text, persists one outbound durable work
  item, and returns its stable local task ID without waiting for remote
  completion; ordinary delegated, reviewer, and remote-peer actors cannot start
  cross-node work.
- [x] **AC13 — durable remote completion:** the outbound worker sends a
  standards-compliant A2A request, stores the remote task correlation, polls
  with capped exponential backoff and per-request timeouts, survives daemon
  restart using the same message ID, and converts remote terminal state into
  one stable local terminal result. Conforming zBot peers deduplicate replay to
  one logical task; for other explicitly trusted A2A implementations the
  crash-before-correlation window is honestly at-least-once remote submission
  rather than an exactly-once claim.
- [x] **AC14 — prompt boundary:** a remote terminal result reaches the
  originating execution as a bounded, attributed `agent.peer`-style steering
  envelope naming peer ID and stable task ID, explicitly labeling content as
  untrusted remote data and permitting at-least-once duplicate delivery. It is
  never system authority and is not automatically persisted to semantic memory.
  The continuation consuming a peer-tainted result receives only `respond` and
  host-internal non-model operations; it cannot invoke side-effecting tools,
  connectors, memory writes, or local/remote delegation. Further action
  requires a subsequent local-user-authored turn under ordinary policy.
- [x] **AC15 — isolation and limits:** authentication precedes enqueue and task
  lookup; every accepted path re-authorizes at delivery. Per-peer concurrency,
  request size, task-list, timeout, retry, and outstanding-task limits are
  enforced. Authentication failures are throttled by bounded per-origin and
  global budgets without revealing whether a peer or credential exists, and
  successful authentication does not reset another origin's budget.
  Diagnostics contain only canonical IDs, operation names, durations, states,
  and normalized reason codes.
- [x] **AC16 — production lifecycle:** production starts and supervises the
  discovery browser plus exact inbound/outbound durable handlers once, rejects
  duplicate handler registration, and drains them through normal daemon
  shutdown. Disabling A2A stops discovery browsing and remote acceptance while
  preserving peer configuration and durable history.
- [ ] **AC17 — interoperability (deferred: a2a-external-conformance):** two
  isolated zBot daemons pass the documented
  pair/delegate/get/cancel/restart journey, and the advertised non-streaming
  HTTP+JSON surface passes the applicable official A2A CLI/TCK checks. Existing
  local peer messaging, Research, Quick Chat, HTTP/WS clients, and tool inventory
  limits remain green.

## Assumptions

- Technical: A2A 1.0 is the current stable protocol and defines
  `/.well-known/agent-card.json`, `HTTP+JSON`, `A2A-Version`, messages, tasks,
  and Agent Cards. (source: https://a2a-protocol.org/latest/specification/)
- Technical: official `a2a-lf` 0.3.0 implements A2A 1.0 core types and requires
  Rust 1.85, while zBot runs Rust 1.94.1. (source: `cargo info a2a-lf`; `rustc
  --version` probe 2026-08-06)
- Technical: the complete official A2A server/client crates currently use Axum
  0.8 and Reqwest 0.13, while the gateway uses Axum 0.7 and Reqwest 0.12.
  (source: official `a2a-rs` workspace `Cargo.toml`; `gateway/Cargo.toml`)
- Technical: zBot already has authoritative durable generic-agent work,
  cancellation state, transcript persistence, same-daemon peer delivery, and
  mDNS advertisement, but the durable work store has no task-list query or
  cancel transition and there is no cross-daemon router or discovery browser.
  (source:
  `gateway/src/durable_agent_tasks.rs`;
  `gateway/gateway-execution/src/peer_messaging.rs`; `discovery/src/`)
- Product: the delivery is LAN/VPN federation with explicit pairing; public
  internet federation, registries, streaming, webhooks, file artifacts, MQTT,
  and Kafka are outside this feature. (source: user confirmation 2026-08-06)
- Product: ACP/A2C is a separate RFC and does not share this feature's surface.
  (source: user confirmation 2026-08-06)
- Process: a cross-package external interface and security-boundary change uses
  an RFC, full work-loop, contract artifact, adversarial review, and secure-design
  review before implementation. (source: `docs/CONVENTIONS.md`;
  `/home/videogamer/projects/agentzero/.agents/skills/work-loop/SKILL.md`)
- Process: no OpenAPI authoring skill is installed, so the contract is authored
  directly and checked mechanically without type-specific rule enforcement.
  (source: available skill roster 2026-08-06)
