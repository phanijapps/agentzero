# Spec: Vault Layout Standardization

- **Status:** Shipped
- **Owner:** phanijapps
- **Plan:** [`plan.md`](plan.md)
- **Constrained by:** [`RFC-0011`](../../rfc/0011-engram-memory-engine-cutover.md); [`RFC-0012`](../../rfc/0012-engram-upstream-risk-reduction.md)
- **Brief:** none
- **Contract:** none
- **Shape:** mixed

> **Spec contract:** this document defines what "done" means. The implementing
> PR must match this spec, or update it. Verification must be derivable from it.

## Objective

Make a z-Bot vault understandable, stable, and honest. Configuration uses one
canonical lowercase-kebab filesystem vocabulary, with only the reserved agent
contracts `SOUL.md`, `INSTRUCTIONS.md`, `OS.md`, and `AGENTS.md` remaining
uppercase. Startup creates only required folders; optional integrations and
runtime environments are lazy. Existing vaults migrate safely with a
compatibility path. Dormant OKF prompts, templates, and hooks no longer claim
that a non-existent z-Bot capability is available. This changes z-Bot vault
layout only and does not introduce an Engram dependency or semantic backend.

## Boundaries

### Always do

- Make `VaultPaths` the canonical owner of vault locations and rename internal
  variables that call the vault root a `config_dir`.
- Use lowercase kebab-case for ordinary directories and file names; preserve
  the four exact uppercase agent-contract names.
- Migrate a legacy path only when its canonical destination is absent; retain a
  rollback-compatible source copy until a later explicit cleanup release.
- Keep user ward, wiki, skill, and plugin content untouched by layout cleanup.
- Retire active OKF instructions and hook configuration because z-Bot has no
  registered `okf` tool or external hook runner.

### Ask first

- Deleting legacy migration copies, user-authored `config/okf` content, a
  user-created empty directory, or any ward content.
- Renaming public REST paths, agent IDs, provider IDs, JSON field names, or
  moving Engram-managed data.
- Adding a configuration database, secrets vault, or dependency to implement
  this layout.

### Never do

- Never delete a user-configured provider, connector, MCP, OAuth token,
  plugin, ward, or skill during migration.
- Never silently choose between a legacy and canonical file when both exist;
  canonical wins and the conflict is reported.
- Never make OKF a new z-Bot runtime dependency or leave prompts that advertise
  an unavailable `okf(...)` tool.
- Never change the casing of `SOUL.md`, `INSTRUCTIONS.md`, `OS.md`, or
  `AGENTS.md`.

## Testing Strategy

- **TDD:** path migration and layout classification are deterministic filesystem
  state transitions, so temporary-vault tests cover absent, legacy, canonical,
  and conflicting paths.
- **Goal-based checks:** repository searches prove direct path literals and
  active OKF prompts/hooks were removed from runtime loading.
- **Integration tests:** settings, provider, MCP, cron, templates, and runner
  construction resolve the same canonical paths after migration.
- **Manual QA:** start z-Bot against a copied legacy vault and verify Settings,
  a provider, a schedule, and a ward remain available without empty optional
  directories reappearing.

## Acceptance Criteria

- [x] `VaultPaths` exposes canonical configuration, agent-contract, runtime,
  and workspace paths; code no longer calls the vault root `config_dir`.
- [x] Ordinary vault paths use lowercase kebab-case and the only uppercase
  filenames introduced or retained by the layout are the four reserved agent
  contracts.
- [x] A vault with legacy `mcps.json`, `cron_jobs.json`, flat MCP OAuth files,
  root agent prompts, or `shards/` is migrated safely to canonical paths while
  preserving legacy copies for rollback.
- [x] When a legacy and canonical path both exist, the canonical file is used,
  neither file is overwritten, and the conflict is observable in logs/tests.
- [x] Fresh startup creates required directories only; plugins, language ward
  config, runtime environments, and transient work areas are created lazily.
- [x] Existing wiki structure and all user wards remain unchanged.
- [x] Active z-Bot prompt loading does not mention `okf(...)`; no z-Bot startup
  code reads the dormant external-hook YAML or invokes its script.
- [x] Engram has no new import, storage path, migration, or runtime dependency
  in this feature.

## Assumptions

- Technical: `VaultPaths` currently owns most config file locations but
  `AppState.config_dir` is documented as a legacy vault-root alias (source:
  `gateway/gateway-services/src/paths.rs`; `gateway/src/state/mod.rs`).
- Technical: `ensure_dirs_exist` eagerly creates config, data, logs, agents,
  skills, plugins, wards, scratch, temp, traces, and language config folders
  (source: `gateway/gateway-services/src/paths.rs`).
- Technical: live vault OKF templates, a prompt shard, and a posttool hook
  exist, while indexed runtime source has no registered `okf` tool or loader
  for `hooks/hooks.yaml` (source: read-only vault inspection and codegraph
  search, 2026-07-11).
- Product: ordinary names use lowercase kebab-case; uppercase is reserved for
  `SOUL.md`, `INSTRUCTIONS.md`, `OS.md`, and `AGENTS.md` (source: user
  confirmation 2026-07-11).
- Product: inactive OKF is retired from active z-Bot loading without deleting
  user ward content (source: user direction 2026-07-11).
- Process: this is a full work-loop change because it moves persisted files,
  affects secrets/configuration, and has dependent tasks (source:
  `work-loop` risk triggers).

## Changelog

- 2026-07-11: shipped canonical vault paths, copy-first legacy migration,
  lazy optional-folder creation, canonical customization paths, and inactive
  OKF compatibility handling. No user vault content was deleted.
