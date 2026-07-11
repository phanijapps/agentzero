# Spec: SQLite Runtime Store Split

- **Status:** Shipped
- **Owner:** zbot maintainers
- **Plan:** [`plan.md`](plan.md)
- **Constrained by:** none
- **Brief:** none
- **Contract:** none
- **Shape:** mixed

## Objective

Separate zbot's runtime SQLite persistence from legacy semantic SQLite so the
active application continues to use the same `conversations.db` filename and
path while durable memory, graph, belief, recall, and semantic storage stay
behind the Engram provider and its compatibility adapter. The workspace becomes
clearer without a user data migration or a change to runtime session behavior.

## Boundaries

### Always do

- Preserve `conversations.db` and every existing user-facing data path.
- Keep runtime conversation, session, execution, log, checkpoint, and outbox
  persistence available through a dedicated zbot runtime SQLite crate.
- Keep semantic persistence behind `zbot-engram-adapter` in active production
  composition; retain legacy semantic SQLite only for migration/parity work.

### Ask first

- Removing a legacy semantic SQLite repository that still has a production
  caller.
- Changing persistence schema, filenames, or data-root layout.
- Moving the quarantined legacy crate outside the workspace.

### Never do

- Never rename or move `conversations.db`.
- Never require users to export, import, or recreate runtime conversations.
- Never let gateway/runtime code restore direct production access to legacy
  semantic SQLite types after the split.

## Testing Strategy

- **TDD:** crate exports, dependency direction, and compatibility shims have
  precise compile-time invariants.
- **Goal-based integration:** `conversations.db` path preservation and the
  absence of active semantic SQLite imports are verified by focused commands.
- **Manual smoke:** a fresh and an existing data directory can start the daemon
  and retain a session after restart.

## Acceptance Criteria

- [x] Runtime persistence is provided by a dedicated crate without changing
  the `conversations.db` filename or data-root location.
- [x] Gateway, execution, daemon, and CLI runtime persistence imports resolve
  through the dedicated runtime SQLite crate.
- [x] Active memory, graph, belief, recall, and governance composition reaches
  Engram only through `zbot-engram-adapter`.
- [x] Legacy semantic SQLite implementations remain available only to explicit
  migration/parity fixtures and have no production gateway/runtime callers.
- [x] Existing runtime session, execution, log, checkpoint, and outbox tests
  pass against the unchanged database path.
- [x] Runtime-owned sources physically live under `zbot-runtime-sqlite`, with
  no `#[path = "../../zbot-stores-sqlite/..."]` bridge modules.
- [x] The workspace dependency graph documents the runtime/legacy boundary and
  rejects new production imports of the quarantined semantic crate.

## Assumptions

- Technical: `zbot-stores-sqlite` currently combines runtime and semantic
  storage (`stores/zbot-stores-sqlite/Cargo.toml`, `src/lib.rs`).
- Technical: the Engram adapter uses `EngramConfig` and `bootstrap_provider`
  (`stores/zbot-engram-adapter/src/bootstrap.rs`).
- Process: the existing backlog requires a focused crate-boundary spec
  (`docs/backlog.md`).
- Product: preserve `conversations.db` paths and filenames; no automatic data
  migration (`user confirmation 2026-07-10`).
- Product: retain legacy semantic SQLite in-workspace for migration/parity;
  thin zbot's active memory layer (`user confirmation 2026-07-10`).
