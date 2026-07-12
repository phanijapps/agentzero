# Plan: Vault Layout Standardization

- **Spec:** [`spec.md`](spec.md)
- **Status:** Completed

> **Plan contract:** this is the implementation strategy. Unlike the spec, this
> document is allowed to change as we learn.

## Approach

Introduce the canonical layout in the existing `VaultPaths` boundary, then
migrate configuration files in an idempotent, copy-first fashion. Update all
services and templates to resolve canonical paths through that boundary, make
optional folders lazy, and stop loading the dormant OKF prompt/hook material.
Keep the physical workspace roots `wards/`, `skills/`, and `plugins/` stable
in this cutover; moving them is broader than naming cleanup and has a large
runtime blast radius.

## Constraints

- No new crate, database, storage backend, or Engram integration.
- Existing `providers.json` remains named and located as-is by user direction.
- Legacy copies are preserved; no automatic deletion or user-content cleanup.
- `SOUL.md`, `INSTRUCTIONS.md`, `OS.md`, and `AGENTS.md` retain exact casing.

## Construction tests

**Integration tests:** a legacy vault resolves providers, MCPs, schedules,
OAuth state, templates, and prompts after migration; a canonical/legacy
conflict does not overwrite either input.

**Manual verification:** copy the current vault to a temporary location, start
the daemon, and confirm Settings, a configured provider, a schedule, and a
ward still appear while optional directories remain absent until used.

## Design (LLD)

### Design decisions

- `VaultPaths` owns canonical and legacy-relative paths plus the one-time
  migration plan. Services never build configuration filenames themselves.
- Migration is copy-first and idempotent: canonical files are preferred; legacy
  files remain as compatibility copies until a future explicit purge.
- Layout names use lowercase kebab-case. Reserved agent-contract filenames are
  intentionally uppercase and are the only casing exception.
- Runtime environments and transient work areas remain in their current
  compatibility locations for this cutover, but are created lazily. Moving
  their physical roots is deferred because it changes tool/runtime contracts.

### Data & schema

- Canonical configuration: `mcp-servers.json`, `schedules.json`,
  `auth/mcp/{pending,tokens}.json`, `agent/{SOUL,INSTRUCTIONS,OS}.md`, and
  `agent-prompts/*.md`.
- Existing JSON document shapes and key casing do not change.
- Migration returns and logs copied files and detected canonical/legacy
  conflicts; no additional state marker is needed for an idempotent copy-first
  migration.

### Component / module decomposition

- `gateway/gateway-services/src/paths.rs`: canonical locations, layout
  migration, directory classifications, and tests.
- `gateway/src/state/mod.rs` and `gateway/gateway-execution/*`: rename
  vault-root variables and consume `VaultPaths` rather than literal joins.
- `gateway/gateway-services/src/{mcp,mcp_oauth}.rs`,
  `gateway/gateway-cron`, `gateway/gateway-templates`: resolve canonical paths.
- `gateway` startup/templates: stop loading or seeding OKF-only prompt/hook
  material.

### State & control flow

1. Startup constructs `VaultPaths` and creates only required roots.
2. Migration copies a legacy file only if its canonical destination is absent.
3. If both exist, the canonical file is retained and a conflict is recorded.
4. Services read/write canonical locations; legacy files remain rollback copies.
5. Optional roots are created by the service/tool that first uses them.

### Failure, edge cases & resilience

- A copy failure leaves the legacy source intact and returns an error.
- Case-fold collisions are detected before a casing-only path migration; no
  automatic merge is attempted.
- Malformed legacy data remains untouched and is reported rather than copied.

### Quality attributes (NFRs)

- Safety: copy-first migration and canonical-wins conflicts prevent silent data
  loss; OAuth/provider secrets remain in their existing document formats.
- Maintainability: all canonical filenames are represented once in `VaultPaths`.
- Operability: startup logs migration actions and conflicts with relative paths.

## Tasks

### T1: Canonical vault paths and safe migration are deterministic

**Depends on:** none

**Touches:** `gateway/gateway-services/src/paths.rs`, path tests

**Tests:**
- TDD: legacy-only, canonical-only, and conflicting files produce the expected
  canonical resolution without overwriting data. Verifies AC1, AC3, AC4.
- TDD: required roots are created while optional roots are absent until an
  owning service requests them. Verifies AC5.

**Approach:**
- Add canonical lower-kebab path accessors, copy-first migration helpers, and
  an explicit `ensure_optional_dir` API.

**Done when:** temporary-vault tests prove deterministic, non-destructive path
resolution and directory creation.

### T2: Runtime and service code use the vault root and canonical config paths

**Depends on:** T1

**Touches:** `gateway/src/state/mod.rs`, `gateway/gateway-execution/*`,
`gateway/gateway-services/src/{mcp,mcp_oauth,providers,settings}.rs`,
`gateway/gateway-cron/*`, `gateway/gateway-connectors/*`

**Tests:**
- TDD: service fixtures read canonical files after a legacy migration.
- Goal-based: repository search finds no stale direct configuration filename
  joins outside `VaultPaths`. Verifies AC1-AC4.

**Approach:**
- Rename vault-root variables from `config_dir` to `vault_dir`.
- Replace direct filesystem joins with `VaultPaths` accessors and keep provider
  storage unchanged at `config/providers.json`.

**Done when:** all config-owning services read canonical paths and no public
JSON/API shape changes.

### T3: Agent contracts, prompts, and runtime folders are consistently named

**Depends on:** T1

**Touches:** `gateway/gateway-templates/*`, `gateway/gateway-execution/*`,
`gateway/src/http/{settings,commissioning,customization}.rs`,
`runtime/agent-tools/*`

**Tests:**
- TDD: agent contracts retain uppercase names while legacy root files migrate
  to `config/agent/`; prompts migrate from `shards/` to `agent-prompts/`.
- Goal-based: all ordinary new filenames match lowercase kebab-case. Verifies
  AC2 and AC3.

**Approach:**
- Centralize SOUL/INSTRUCTIONS/OS and agent-prompt accessors.
- Stop eager creation of Python, Node, temp, plugin, skill, and language
  configuration folders. Their owning service creates them only when needed.

**Done when:** fresh and migrated vaults load identical instruction content and
runtime directories are not created before use.

### T4: Dormant OKF material is retired without touching user workspaces

**Depends on:** T1, T3

**Touches:** startup prompt loading, seeded templates, hook configuration
handling, `docs/guides/` or architecture reference

**Tests:**
- Goal-based: active prompt sources contain no `okf(` tool instruction and no
  startup path loads `hooks/hooks.yaml`. Verifies AC7.
- Integration: existing wards are byte-for-byte unchanged by startup migration.
  Verifies AC6.

**Approach:**
- Stop loading/seeded-referencing the OKF shard and mark legacy `config/okf`
  and `hooks/` as inactive compatibility content without deleting them.

**Done when:** z-Bot no longer advertises unavailable OKF capabilities and
user-owned ward files are untouched.

### T5: Document the canonical layout and migration lifecycle

**Depends on:** T1-T4

**Touches:** `docs/architecture/`, root/component `AGENTS.md`, `README.md`,
`docs/specs/README.md`

**Tests:**
- Goal-based: documented tree matches `VaultPaths` test fixtures and all links
  resolve. Verifies AC1-AC8.

**Approach:**
- Document required, lazy, user-managed, and deprecated paths; explain
  uppercase exceptions and the explicit future-purge process.

**Done when:** a new contributor can identify every supported vault path and
understand which empty folders are intentional.

## Rollout

- **Delivery:** copy-first migration at startup with canonical-path writes.
- **Infrastructure:** no new service or storage.
- **External-system integration:** none; Engram remains untouched.
- **Deployment sequencing:** land T1 before any service uses canonical names;
  keep legacy files through at least one release before an explicit purge.

## Risks

- Existing direct joins may bypass `VaultPaths`; goal-based search and service
  tests are required before claiming migration completeness.
- Copy-first migration temporarily duplicates sensitive configuration. File
  permissions and old files are retained as existing local-user data, never
  logged or transmitted.
- Case-only moves behave differently on case-insensitive filesystems; conflicts
  must be detected, not merged.
- The current Engram integration build can block workspace compilation without
  implying a vault-layout regression.

## Changelog

- 2026-07-11: initial implementation plan following user-approved naming and
  cleanup direction.
- 2026-07-11: kept runtime roots in place for this cutover; made creation lazy
  instead of moving tool-owned paths without a dedicated runtime contract.
