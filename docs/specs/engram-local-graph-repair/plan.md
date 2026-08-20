# Plan: Engram Local Graph Repair

- **Spec:** [`spec.md`](spec.md)
- **Status:** Done

## Approach

Provide a dependency-free Python utility at `tools/engram_local_graph_repair.py`.
It uses SQLite's backup API and a single transaction: remove unsupported
relationships, remove relationships connected to `unknown` entities, then
remove those entities. The default path is read-only dry-run; `--apply` is the
explicit destructive mode.

Tempted to merge aliases automatically; declining because canonical identity
requires semantic judgment. Tempted to repair every graph table; declining
because this task is limited to `kg_entities` and `kg_relationships`.

## Design (LLD)

- **Components:** the script owns argument validation, inspection, backup,
  transactional deletion, and post-apply integrity verification.
- **Data:** built-in predicate IDs mirror the z-Bot `RelationshipType` contract.
- **Failure behavior:** any backup, transaction, or integrity failure exits
  non-zero and reports no successful repair.

## Tasks

### T1: Add an auditable local graph repair utility

**Status:** Done on 2026-08-07.

**Depends on:** none

**Touches:** `tools/engram_local_graph_repair.py`,
`tools/test_engram_local_graph_repair.py`,
`docs/specs/engram-local-graph-repair/**`, `docs/specs/README.md`

**Mode:** TDD + manual QA

**Tests:**

- Red/green fixture tests prove dry-run non-mutation, backup-before-apply,
  conservative deletion, preservation of standalone unknown entities,
  collision-proof backups, rollback on integrity failure, preserved valid
  records, and idempotent repeat apply.
- Manual target verification records dry-run and apply summaries, then checks
  `PRAGMA integrity_check` after the authorized repair.

**Approach:**

- Use parameterized SQLite queries and SQLite's `Connection.backup` API.
- Store the backup next to the explicit target with a timestamped suffix.
- Refuse mutation without `--apply`; print JSON summaries for auditability.

**Done when:** fixture tests pass and the target database repair is applied,
verified, and repeatable without further mutation.

## Rollout

Run dry-run, inspect the reported counts, then run `--apply` against the exact
authorized path. The backup is the rollback artifact.

## Risks

- The selected noise rules intentionally leave alias/duplicate cleanup for a
  later human-reviewed repair.

## Changelog

- 2026-08-07: initial conservative repair plan after user authorization.
