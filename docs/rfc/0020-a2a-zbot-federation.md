# RFC-0020: A2A zBot Federation

- **Status:** Accepted
- **Author:** phanijapps
- **Approver:** phanijapps
- **Date opened:** 2026-08-06
- **Date closed:** —
- **Related:** `docs/specs/a2a-federation-discovery/`

## The ask

- **Recommendation (BLUF):** Add an opt-in Agent2Agent Protocol 1.0
  HTTP+JSON surface so explicitly trusted zBots can discover one another,
  submit durable work, inspect or cancel that work, and receive its terminal
  result asynchronously. Use LAN mDNS only to discover candidates; never use
  discovery as proof of identity or authorization.
- **Why now (SCQA):** zBot already has durable local task execution and
  same-daemon peer messaging, while its mDNS crate advertises only one local
  daemon. Distributed zBots currently have no interoperable discovery card,
  trust model, or remote task protocol. The decision is whether to extend the
  private peer-message envelope across the network or adopt the stable A2A 1.0
  contract at the daemon boundary while retaining zBot's existing durable
  execution machinery internally.
- **Decision requested:** adopt A2A 1.0 for zBot-to-zBot federation, initially
  on explicitly configured LAN or VPN deployments. Keep ACP, public internet
  directories, brokers, streaming, push notifications, and file transfer in
  separate decisions.

## Problem & goals

Operators want multiple zBots on a network to perform distinct jobs and
collaborate without a central cloud service. A remote zBot must remain an
opaque agent: callers discover its advertised skills and exchange standard A2A
messages and tasks rather than depending on its internal sessions, tools,
memory, or queue schema.

**Goals:**

- Publish a valid A2A 1.0 Agent Card at the registered well-known URI.
- Discover LAN candidates through the existing `_zbot._tcp.local.` service,
  then require an explicit local trust action before any remote call.
- Support the A2A HTTP+JSON operations needed for durable asynchronous work:
  send message, get task, list tasks, and cancel task.
- Let root and ward agents delegate bounded work to a trusted zBot without
  blocking their current execution, and deliver the remote terminal result as
  attributed untrusted peer data.
- Reuse the existing durable work queue, execution state, transcript store,
  cancellation handles, and retry behavior.
- Keep the peer directory and outbound transport behind interfaces so a future
  registry, MQTT, Kafka, or other messaging system can be added without
  changing agent tools or the A2A server contract.

**Non-goals:**

- ACP/A2C or editor-client support.
- Public internet federation, automatic NAT traversal, a hosted directory, or
  anonymous public agents.
- Automatic trust based on mDNS, hostnames, IP addresses, Agent Card text, or
  possession of a task identifier.
- A broker, child broker process, MQTT, Kafka, NATS, or a broker-specific
  envelope.
- A2A streaming, task subscription, push notifications, extended Agent Cards,
  card signing, file/raw/URL parts, structured-data parts, or multi-tenant
  routing in the first delivery.
- Remote access to zBot sessions, internal execution IDs, memory, tools, or
  same-daemon `message_agent` routing.

## Proposal

### Protocol boundary

zBot exposes the A2A 1.0 HTTP+JSON binding under `/a2a` on the existing gateway
listener and publishes `/.well-known/agent-card.json`. The Agent Card advertises
one `HTTP+JSON` interface with protocol version `1.0`, `text/plain` input and
output modes, and explicitly false streaming, push-notification, and extended
card capabilities.

The first server surface implements:

- `POST /a2a/message:send`
- `GET /a2a/tasks/{id}`
- `GET /a2a/tasks`
- `POST /a2a/tasks/{id}:cancel`

Every protocol request requires `A2A-Version: 1.0`,
`Content-Type: application/a2a+json` where a body is present, and an HTTP
Bearer credential associated with one trusted peer. Unsupported optional A2A
operations fail with the standard unsupported-operation response and are not
advertised as capabilities.

The implementation uses the official `a2a-lf` crate for A2A 1.0 types and
wire-compatible serde behavior. It does not use the official server/client
crates in this delivery because those crates introduce Axum 0.8 and Reqwest
0.13 beside zBot's Axum 0.7 and Reqwest 0.12 gateway stack. zBot implements the
small HTTP adapter and client on its established libraries while verifying the
wire surface against the official A2A Technology Compatibility Kit or CLI.

### Identity and trust

mDNS TXT records advertise only bounded public bootstrap data: a random,
non-identifying zBot node ID, A2A protocol version, and Agent Card path. The ID
contains no host, user, ward, or agent name; explicit rotation breaks existing
trust. Browsed services enter an
in-memory candidate registry as **untrusted**. Candidate descriptions and
addresses never grant routing or tool authority and never overwrite a trusted
peer record.

Trust is established locally through CLI operations. An operator creates a
random inbound credential on one zBot and adds that credential plus the exact
peer URL on the other zBot. Bidirectional communication requires reciprocal
trust. Inbound credentials are stored as constant-time comparable hashes with
opaque IDs, 90-day default/365-day maximum expiry, immediate revocation, and at
most two active entries per peer for deliberate rotation; outbound credentials are stored only
in a local owner-readable peer file and are never returned by list APIs, tools,
logs, or Agent Cards.

Trusted peer records bind:

- stable peer node ID;
- normalized A2A origin and Agent Card URL;
- credential material or hash according to direction;
- allowed local A2A agent/skill;
- transport policy, including whether explicitly opted-in private-network HTTP
  is allowed.

HTTPS is required by default. Plain HTTP is accepted only for loopback or when
the operator explicitly enables private-network HTTP for a peer whose traffic
is protected by the deployment boundary, such as an encrypted VPN. Redirects,
URL userinfo, fragments, arbitrary model-supplied URLs, and credential-bearing
discovery records are rejected.

### Inbound execution

An authenticated `SendMessage` accepts exactly one non-empty bounded text part
with role `ROLE_USER`. The authenticated peer identity, never request metadata,
becomes the trusted actor provenance. The request is persisted through the
existing durable generic-agent-task queue before the server returns a submitted
A2A Task.

Inbound A2A work executes with a dedicated `RemotePeer` actor policy. Its only
model-visible tool is `respond`. Its separately built prompt contains a fixed
remote-safety policy, operator-approved public instructions for the advertised
skill, and the remote message; it does not inherit local agent
SOUL/instruction/OS shards, wards, memory, history, provider or tool catalogs,
paths, or credentials. The first delivery excludes delegation, shell,
filesystem access, connector invocation, peer-to-peer forwarding, secret
access, and other side-effecting capabilities. A trusted peer is allowed to
submit work; it is not equivalent to the local user.

A2A task IDs are random opaque correlation IDs independent of work, session,
and execution identifiers. Lookup, listing, cancellation, and result access use
an indexed durable-work predicate that atomically scopes source, kind,
authenticated peer provenance, and correlation. The durable queue gains an
explicit canceled terminal state so a stale worker cannot overwrite peer
cancellation. Task responses expose only A2A identifiers and normalized
states, not zBot session or execution IDs. The terminal assistant transcript row is
projected as a text artifact, reusing the execution runner's canonical terminal
response persistence path so `respond` tool arguments are not lost.

### Outbound execution

Root and ward actors receive a bounded remote-delegation tool that accepts a
trusted peer ID and text, never a URL or credential. The tool persists an
outbound A2A work item and returns immediately. A supervised handler resolves
the trusted peer, sends `SendMessage`, polls the returned task with capped
backoff, and records one stable local work ID through retries and restart
recovery.

When the remote task reaches a terminal state, the handler injects a bounded,
attributed, untrusted A2A result into the originating execution's steering
queue. If the execution is temporarily unavailable, delivery retries under the
existing lease and attempt limits. The peer-result continuation can only
respond; it cannot invoke side effects, connectors, memory writes, or another
delegation. A subsequent local-user turn is required to regain ordinary actor
authority. The content is never system policy, is never logged, and is not
automatically written to semantic memory.

### Discovery and future transports

The discovery crate gains a browser abstraction beside its existing advertiser.
The mDNS implementation is only one candidate source. A registry adapter may
later provide the same candidate records, and an A2A transport adapter may
later run over different infrastructure, without changing the Agent Card,
agent tools, trusted peer identity, or durable work kinds.

This seam deliberately permits future MQTT or Kafka infrastructure but does
not pretend those brokers are A2A protocol bindings. They may distribute
discovery records, wake workers, or carry an internal transport envelope while
the external interoperability boundary remains A2A.

## Security and privacy

- Federation and the A2A execution surface are disabled by default.
- Discovery is untrusted input. DNS names and addresses are bounded,
  normalized, and never become authorization facts.
- Bearer authentication is evaluated before body processing that can enqueue
  work. Tokens use cryptographic randomness, hashes use constant-time
  comparison, and all failures are normalized.
- A2A rejects browser origins outside an exact A2A allowlist and does not
  inherit the gateway's wildcard CORS behavior as authorization.
- Peer configuration is local-only, owner-readable, written atomically, and
  rejects symlinks and unsafe permissions. Secrets are redacted from `Debug`,
  serialization responses, traces, errors, and tool output.
- Outbound URLs are operator-configured and validated to prevent SSRF;
  redirects are disabled and host resolution is checked against the peer's
  configured transport policy.
- Per-peer concurrency, payload, task-list, history, timeout, and retry limits
  prevent one trusted peer from exhausting the daemon.
- Authentication failures are throttled under per-origin and global budgets,
  and all invalid, expired, revoked, and unknown credentials share one bounded
  failure response.
- Remote messages and results remain attributed untrusted data at every prompt
  boundary. The remote actor policy prevents a trusted peer from becoming a
  confused deputy for local user authority.
- Task authorization is object-level: possession or guessing of a task ID does
  not grant read, list, cancel, or result access.
- Logs contain peer IDs, work/task IDs, operation names, durations, and
  normalized reason codes only; no message, artifact, token, raw URL query, or
  provider output is logged.

## Alternatives considered

### Extend `agent.peer-message.v1` across daemons

Rejected. That envelope exposes zBot-specific execution/session semantics and
would create a private federation protocol exactly where A2A supplies a stable
opaque-agent boundary.

### Require MQTT, Kafka, or NATS now

Rejected. A broker adds deployment and operational coupling without solving
Agent Card discovery, task semantics, client interoperability, or application
authorization. The internal transport seam remains open for later adoption.

### Trust every mDNS peer automatically

Rejected. mDNS authenticates neither the advertiser nor its claimed node ID or
endpoint. Automatic trust would permit trivial LAN impersonation and remote
execution.

### Use the complete official Rust server/client SDK

Deferred. The SDK is the right source for the protocol model, but its current
HTTP stack versions are incompatible with the gateway's router types and would
either duplicate major libraries or force an unrelated upgrade. Re-evaluate
when the stacks converge or when zBot intentionally upgrades its gateway.

### Build a custom encrypted transport

Rejected. zBot relies on HTTPS or an explicitly accepted encrypted network
boundary instead of inventing application cryptography.

## Consequences

**Advantages:**

- Independent zBots interoperate through a stable external protocol.
- Remote work inherits durable acceptance, restart recovery, cancellation, and
  terminal-result persistence already present locally.
- Discovery works without a central service while trust remains explicit.
- Protocol, discovery source, durable queue, and future broker are separate
  replaceable layers.
- Remote peers receive less authority than the local user by construction.

**Costs and limitations:**

- Operators initially pair both directions manually and manage private-network
  HTTP opt-in when they do not provide HTTPS.
- The first Agent Card represents a configured remote-safe zBot service rather
  than every local agent and skill.
- Streaming, webhooks, file artifacts, automatic registry enrollment, and
  signed Agent Cards remain unavailable.
- The implementation adds a protocol dependency, a peer configuration file,
  new durable work kinds, and a remote actor policy that must remain aligned
  with tool capability changes.

## Rollout

1. Land the contract, official protocol types, peer config, and discovery
   candidate browser behind `a2a.enabled = false`.
2. Land authenticated inbound A2A tasks and task inspection/cancellation.
3. Land durable outbound delegation and attributed result delivery.
4. Verify two daemon instances with the official A2A CLI/TCK, restart during
   pending work, and confirm cross-peer authorization isolation.
5. Enable only in explicitly configured LAN/VPN deployments; retain immediate
   rollback by disabling A2A without deleting peer state or task history.

## References

- [A2A Protocol 1.0 specification](https://a2a-protocol.org/latest/specification/)
- [A2A agent discovery guidance](https://a2a-protocol.org/latest/topics/agent-discovery/)
- [Official A2A Rust SDK](https://github.com/a2aproject/a2a-rs)
- `docs/specs/durable-generic-agent-tasks/`
- `docs/specs/durable-peer-messaging/`
- `docs/architecture/future-state/2026-05-11-pattern4-peer-messaging-design.md`
