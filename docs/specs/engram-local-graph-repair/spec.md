# Spec: Engram Local Graph Repair

Mode: full (destructive local data repair)

- **Status:** Shipped
- **Owner:** zbot maintainers
- **Plan:** [`plan.md`](plan.md)
- **Constrained by:** RFC-0011
- **Brief:** none
- **Contract:** none
- **Shape:** data

## Objective

The local Engram graph repair utility removes relationship-only `unknown` entity
stubs and relationship predicates outside z-Bot's built-in vocabulary from a
specified SQLite graph database. It previews every repair by default and takes
a timestamped SQLite backup before any mutation.

## Boundaries

### Always do

- Require an explicit database path and `--apply` before mutation.
- Create a verified SQLite backup before changing the target database.
- Delete relationships before deleting their unknown endpoint entities.
- Run SQLite integrity verification after an applied repair.

### Ask first

- Merging, renaming, or deleting non-`unknown` entities.
- Changing the predicate allowlist or repairing any database other than the
  explicit command-line target.

### Never do

- Mutate the graph in default/dry-run mode.
- Remove memory facts, beliefs, documents, chunks, ontology, or taxonomy data.
- Add a dependency, schema migration, new service, or top-level module.

## Testing Strategy

- **TDD:** a temporary SQLite fixture proves dry-run makes no changes, apply
  creates a backup, unsupported edges and unknown stubs are removed, valid
  topology remains, and a repeat apply is idempotent.
- **Manual QA:** run dry-run then apply against the user-authorized local
  database and record pre/post counts plus `PRAGMA integrity_check`.

## Acceptance Criteria

- [x] The script requires `--apply` to mutate a database.
- [x] An applied repair creates a timestamped backup before mutation.
- [x] Only entities whose normalized type is `unknown` are removed.
- [x] Only relationships outside the built-in `RelationshipType::BUILTIN_IDS`
  vocabulary are removed.
- [x] Relationships connected to removed unknown entities are deleted before
  those entities.
- [x] Dry-run, apply, repeat-apply, and integrity verification succeed against
  the local database.

## Assumptions

- Technical: the repair target is
  `/home/videogamer/Documents/zbot/data/engram/engram_data.db` and exposes
  `kg_entities` / `kg_relationships` (read-only SQLite probe, 2026-08-07).
- Technical: z-Bot's built-in relationship vocabulary is defined by
  `RelationshipType::BUILTIN_IDS` in `services/knowledge-graph/src/types.rs`.
- Product: conservative cleanup removes only `unknown` stubs and non-built-in
  predicates; possible duplicate aliases remain untouched (user confirmation
  2026-08-07).
- Process: direct SQLite repair is explicitly authorized for this local graph
  (user confirmation 2026-08-07).
