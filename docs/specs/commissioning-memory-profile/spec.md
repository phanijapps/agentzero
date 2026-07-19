# Spec: Commissioning Memory Profile

- **Status:** Approved
- **Owner:** phanijapps
- **Plan:** [`plan.md`](plan.md)
- **Constrained by:** [`RFC-0011`](../../rfc/0011-engram-memory-engine-cutover.md); [`agent-commissioning`](../agent-commissioning/spec.md)
- **Brief:** none
- **Contract:** [`contracts/openapi/commissioning.yaml`](../../../contracts/openapi/commissioning.yaml)
- **Shape:** mixed

> **Spec contract:** this document defines what "done" means. The implementing
> PR must match this spec, or update it. Verification must be derivable from it.

## Objective

Let a first-time user explicitly choose either the safe baseline or a
versioned Full Zbot memory profile during Agent Commissioning. The full profile
must reproduce the approved local z-Bot memory, governance, and recall behavior
using z-Bot's built-in embedding backend, then leave the resulting configuration
inspectable under the installation's canonical `config/` directory. Provider
verification and the existing local-only commissioning boundary remain intact.
Because memory stores, recall gates, and background workers are boot-bound, a
full-profile commission remains in a finite restart-required state until the
daemon restarts and activates the persisted profile; only that boot reports the
commission complete.

## Boundaries

### Always do

- Require an explicit memory-profile selection; visually recommend the full
  profile but never infer consent from a default form value.
- Use the built-in FastEmbed embedding space for the full profile and persist
  the exact runtime/store identity `bge-small-en-v1.5` with 384 dimensions.
- Provision recall and governance files only under canonical `VaultPaths`
  locations; reject symlinked path components and non-regular targets;
  preserve identical retry artifacts; atomically refuse conflicting existing
  content; and mark commissioning complete last.
- Keep API keys write-only and preserve the existing local-origin requirement
  for commissioning completion.
- Reuse the gateway's peer-aware local-access policy: originless clients are
  allowed only on a loopback bind or from a loopback peer, and authorization
  runs before validation, provider testing, or file/settings effects.
- Preflight every profile-file conflict before changing provider credentials,
  provider default selection, SOUL, settings, or profile files.
- Treat a full-profile restart as part of commissioning: persist a fixed
  activation marker, keep `setupComplete` false, show a restart-required
  recovery screen, and finalize durable completion only on the next daemon
  boot after exact inputs are verified. The completed state is externally
  observable only if that boot then constructs memory services and starts the
  gateway successfully.
- Protect the pending marker exactly like every profile artifact: its fixed V1
  bytes are bundled, symlinks/non-regular/conflicting content fail closed, and
  only byte-identical retries are accepted. Boot may activate only when both
  commissioning is in the restart-required state and every V1 artifact,
  setting, and persisted embedding value matches.

### Ask first

- Changing the approved Full Zbot profile values, embedding model, dimensions,
  governance IDs, or recall defaults after `zbot_recommended_v1` ships.
- Adding automatic model downloads, external embedding services, or a third
  commissioning memory choice.
- Overwriting or migrating an existing user's memory, recall, ontology, or
  taxonomy configuration.
- Replacing an existing Ollama, unconfigured, or otherwise non-internal
  embedding backend, or reindexing an existing semantic store.

### Never do

- Never read `~/Documents/zbot/config` at runtime or treat a developer's home
  directory as a production template source.
- Never silently enable background memory workers for a user who chose the safe
  baseline or who did not submit a valid explicit profile choice.
- Never import Engram crates, open a semantic database, or perform semantic
  writes from the commissioning handler.
- Never accept a model-controlled or user-supplied filesystem path for profile
  provisioning, and never introduce a new top-level module or dependency.
- Never silently accept unknown commissioning or nested provider request
  fields; the runtime contract must match OpenAPI `additionalProperties: false`.

## Testing Strategy

- Profile values and identity alignment: **TDD** with Rust serialization
  snapshots and exact assertions because the versioned preset is a compact,
  deterministic contract.
- Provisioning, conflicts, retries, and completion ordering: **TDD** with
  temporary-vault tests because these file and state transitions must fail
  closed without overwriting user data.
- OpenAPI and TypeScript request parity: **goal-based checks** using contract
  parsing and TypeScript build/typecheck because schema shape is the observable.
- Commissioning choice and submission: **TDD plus user-visible component
  tests** that drive the buttons and assert the memory step, required consent,
  submitted enum, and recovery error.
- Fresh-install journey: **goal-based integration** using a temporary vault,
  followed by manual QA of `/commission` for the final visual state.

Stub tally at PLAN: TDD tasks T1-T3 require compilable red stubs; contract/build
and manual checks record `no stub (mode)`.

## Acceptance Criteria

- [ ] The commissioning contract requires `memoryProfile` with only
  `safe_baseline` and `zbot_recommended_v1`; unknown or omitted values are
  rejected before any provider or filesystem side effect, and unknown
  top-level or nested provider fields are also rejected.
- [ ] The commissioning UI presents both profiles, marks Full Zbot memory as
  recommended, explains that it enables background memory processing and may
  incur selected-provider usage/cost, and keeps Continue disabled until the
  user explicitly selects one. When a cloud provider is selected, the same
  consent surface states that memory-derived content may be sent to that
  provider. It also discloses that the built-in embedding model may need a
  one-time local download on first use.
- [ ] Selecting `safe_baseline` preserves the installation's existing memory
  and embedding settings and creates no recall or governance profile files; on
  a fresh vault this is `MemorySettings::default()` behavior.
- [ ] Selecting `zbot_recommended_v1` persists the approved memory tuning with
  built-in FastEmbed identity `bge-small-en-v1.5`, 384 dimensions, and query
  prompt profile, matching the exact identity reported by the internal client
  and used by store compatibility checks. It proceeds only when existing memory
  settings are the fresh defaults or byte-equivalent V1 settings and the
  persisted/live embedding backend is already internal/384; customized memory
  settings or any other embedding backend return a finite conflict without
  mutation.
- [ ] The complete V1 memory and recall documents are checked-in canonical JSON
  assets; constructors deserialize those assets and tests require exact
  serialized parity, so mutable Rust defaults cannot change V1 behavior or
  retry bytes.
- [ ] Full-profile completion materializes an inspectable
  `config/recall-config.json` matching the approved versioned recall snapshot
  and provisions `zbot.base:v1` plus `zbot.general:v1` definitions beneath
  `config/governance/`.
- [ ] Retrying full-profile completion accepts byte-identical existing profile
  files, rejects different existing content with a finite redacted error code,
  never overwrites it, and does not report commissioning complete on failure.
- [ ] A successful full-profile request returns
  `memory_profile_restart_required`, leaves `setupComplete` false, and does not
  expose the normal application as active. On the next daemon boot, memory
  settings and fixed artifacts are verified before memory construction,
  commissioning is durably marked complete, and only then is the activation
  marker removed. The normal gateway is observable only if the remainder of
  startup constructs memory services from that profile successfully. A failed
  completion-state save retains the marker; completed state plus a leftover
  marker is cleaned up idempotently on the next boot.
- [ ] A symlink in `config`, `governance`, or any final profile-file position,
  and a non-regular existing target, fail closed without writing outside the
  canonical vault or changing provider, SOUL, settings, or commissioning state.
- [ ] A browser request from a non-local Origin and an originless request from
  a non-loopback peer receive `403 commissioning_origin_denied` before any
  validation, provider test, credential, filesystem, SOUL, or settings effect;
  loopback browser and native/CLI calls remain supported.
- [ ] Existing provider verification, local-origin enforcement, credential
  redaction, semantic-profile persistence, and legacy readiness behavior remain
  covered and unchanged.
- [ ] Rust formatting, clippy, relevant Rust tests, UI tests, UI build, and the
  workspace check pass before the branch is published.

## Assumptions

- Technical: memory-provider defaults already identify built-in FastEmbed as
  `BAAI/bge-small-en-v1.5` with 384 dimensions, but the full commissioning
  fixture intentionally pins the exact runtime/store-compatible basename
  `bge-small-en-v1.5` (source:
  `gateway/gateway-memory/src/lib.rs`).
- Technical: the internal `EmbeddingClient` reports `bge-small-en-v1.5`, and
  runtime store compatibility checks compare model strings exactly; the V1
  fixture therefore persists that same basename rather than relying on an
  equivalence predicate (sources: `runtime/agent-runtime/src/llm/local_embedding.rs`;
  `stores/zbot-engram-adapter/src/stores/memory_facts.rs`).
- Technical: memory-provider construction, recall configuration, query gating,
  belief/hierarchy wiring, and sleep-worker configuration are read at daemon
  boot and cannot truthfully be reported active immediately after a settings
  write (source: `gateway/src/state/mod.rs`).
- Technical: commissioning currently persists provider/root/semantic choices
  but does not configure `execution.memory` (source:
  `gateway/src/http/commissioning.rs`).
- Technical: missing `config/recall-config.json` currently selects compiled
  recall defaults and the loader never auto-creates the file (source:
  `gateway/gateway-memory/src/lib.rs`).
- Process: the existing commissioning request is an OpenAPI contract under
  `contracts/openapi/commissioning.yaml`; this feature modifies that contract
  directly because no `api-contract` skill is installed (source: repository
  contract and available-skills roster).
- Process: the feature remains adapter-first and does not add an Engram
  dependency to commissioning (source: RFC-0011 and agent-commissioning spec).
- Product: first-time setup should reproduce the approved Zbot memory and
  recall behavior while using built-in embeddings (source: user confirmation
  2026-07-18).
- Product: Full Zbot memory is an explicit recommended choice rather than an
  automatic default (source: user confirmation 2026-07-18).
- Product: the full preset is intended for a fresh installation. Incomplete
  legacy installations retain ownership of customized memory and embedding
  configuration and receive a conflict instead of an implicit migration.
- Security: the established peer-aware local-access rule permits an originless
  call only when the daemon binds to loopback or the connection peer is
  loopback (source: `gateway/src/http/vault.rs`).
- Security: the filesystem threat model includes accidental or adversarial
  symlink/conflict state in the vault, but not a malicious same-user process
  racing every syscall; such a process can already rewrite the user's complete
  z-Bot configuration (source: spec-stage secure-design review 2026-07-18).
