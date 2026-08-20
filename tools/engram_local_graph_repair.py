#!/usr/bin/env python3
"""Conservatively repair noisy local Engram graph topology.

Default operation is a non-mutating dry run. `--apply` creates a timestamped
SQLite backup before removing only relationship-only `unknown` entity stubs and
relationship predicates outside z-Bot's built-in relationship vocabulary.
"""

from __future__ import annotations

import argparse
import dataclasses
import datetime as dt
import json
import os
import pathlib
import sqlite3
import sys
from typing import Iterable


BUILTIN_RELATIONSHIP_TYPES = frozenset({"works_for", "located_in", "related_to", "created", "uses", "part_of", "mentions", "before", "after", "during", "concurrent_with", "succeeded_by", "preceded_by", "president_of", "founder_of", "member_of", "author_of", "held_role", "employed_by", "held_at", "born_in", "died_in", "caused", "enabled", "prevented", "triggered_by", "contains", "instance_of", "subtype_of"})


@dataclasses.dataclass(frozen=True)
class RepairReport:
    database: str
    applied: bool
    unknown_entities: int
    unsupported_relationships: int
    stub_relationships: int
    deleted_entities: int
    deleted_relationships: int
    backup_path: str | None
    integrity_check: str | None


def repair_database(database: pathlib.Path, *, apply: bool) -> RepairReport:
    """Inspect or conservatively repair one explicit local graph database."""
    database = database.expanduser().resolve()
    if not database.is_file():
        raise ValueError(f"database does not exist or is not a file: {database}")
    with sqlite3.connect(f"file:{database}?mode=ro", uri=True) as connection:
        _validate_schema(connection)
        unknown_entities = _count_unknown_entities(connection)
        unsupported_relationships = _count_unsupported_relationships(connection)
        stub_relationships = _count_stub_relationships(connection)
    if not apply:
        return RepairReport(str(database), False, unknown_entities, unsupported_relationships, stub_relationships, 0, 0, None, None)

    backup_path = _backup_database(database)
    with sqlite3.connect(database) as connection:
        _validate_schema(connection)
        connection.execute("BEGIN IMMEDIATE")
        stub_entity_ids = _relationship_unknown_entity_ids(connection)
        deleted_unsupported = _delete_unsupported_relationships(connection)
        deleted_stub_edges = _delete_stub_relationships(connection, stub_entity_ids)
        deleted_entities = _delete_unknown_entities(connection, stub_entity_ids)
        integrity_check = _post_repair_integrity_check(connection)
        if integrity_check != "ok":
            raise RuntimeError(f"integrity check failed after repair: {integrity_check}")
        connection.commit()
    return RepairReport(str(database), True, unknown_entities, unsupported_relationships, stub_relationships, deleted_entities, deleted_unsupported + deleted_stub_edges, str(backup_path), integrity_check)


def _validate_schema(connection: sqlite3.Connection) -> None:
    tables = {row[0] for row in connection.execute("SELECT name FROM sqlite_master WHERE type = 'table'")}
    missing = sorted({"kg_entities", "kg_relationships"} - tables)
    if missing:
        raise ValueError(f"database is missing required graph tables: {', '.join(missing)}")


def _unknown_entity_ids(connection: sqlite3.Connection) -> list[str]:
    return [row[0] for row in connection.execute("SELECT id FROM kg_entities WHERE lower(trim(entity_type)) = 'unknown'")]


def _relationship_unknown_entity_ids(connection: sqlite3.Connection) -> list[str]:
    return [
        row[0]
        for row in connection.execute(
            """
            SELECT DISTINCT entity.id
            FROM kg_entities AS entity
            JOIN kg_relationships AS relationship
              ON relationship.source_entity_id = entity.id
              OR relationship.target_entity_id = entity.id
            WHERE lower(trim(entity.entity_type)) = 'unknown'
            """
        )
    ]


def _relationship_placeholders() -> str:
    return ", ".join("?" for _ in BUILTIN_RELATIONSHIP_TYPES)


def _count_unknown_entities(connection: sqlite3.Connection) -> int:
    return len(_unknown_entity_ids(connection))


def _count_unsupported_relationships(connection: sqlite3.Connection) -> int:
    query = "SELECT COUNT(*) FROM kg_relationships WHERE relationship_type NOT IN (" + _relationship_placeholders() + ")"
    return int(connection.execute(query, tuple(BUILTIN_RELATIONSHIP_TYPES)).fetchone()[0])


def _count_stub_relationships(connection: sqlite3.Connection) -> int:
    unknown_ids = _unknown_entity_ids(connection)
    if not unknown_ids:
        return 0
    placeholders = ", ".join("?" for _ in unknown_ids)
    query = (
        "SELECT COUNT(*) FROM kg_relationships WHERE "
        "(source_entity_id IN (" + placeholders + ") OR target_entity_id IN (" + placeholders + ")) "
        "AND relationship_type IN (" + _relationship_placeholders() + ")"
    )
    params = tuple(unknown_ids) * 2 + tuple(BUILTIN_RELATIONSHIP_TYPES)
    return int(connection.execute(query, params).fetchone()[0])


def _delete_unsupported_relationships(connection: sqlite3.Connection) -> int:
    query = "DELETE FROM kg_relationships WHERE relationship_type NOT IN (" + _relationship_placeholders() + ")"
    return connection.execute(query, tuple(BUILTIN_RELATIONSHIP_TYPES)).rowcount


def _delete_stub_relationships(connection: sqlite3.Connection, stub_entity_ids: list[str]) -> int:
    if not stub_entity_ids:
        return 0
    placeholders = ", ".join("?" for _ in stub_entity_ids)
    query = "DELETE FROM kg_relationships WHERE source_entity_id IN (" + placeholders + ") OR target_entity_id IN (" + placeholders + ")"
    return connection.execute(query, tuple(stub_entity_ids) * 2).rowcount


def _delete_unknown_entities(connection: sqlite3.Connection, stub_entity_ids: list[str]) -> int:
    if not stub_entity_ids:
        return 0
    placeholders = ", ".join("?" for _ in stub_entity_ids)
    return connection.execute(
        "DELETE FROM kg_entities WHERE id IN (" + placeholders + ") AND lower(trim(entity_type)) = 'unknown'",
        tuple(stub_entity_ids),
    ).rowcount


def _backup_database(database: pathlib.Path) -> pathlib.Path:
    timestamp = dt.datetime.now(tz=dt.UTC).strftime("%Y%m%dT%H%M%S.%fZ")
    backup = _reserve_backup_path(database, timestamp)
    try:
        with sqlite3.connect(f"file:{database}?mode=ro", uri=True) as source:
            with sqlite3.connect(backup) as destination:
                source.backup(destination)
    except Exception:
        backup.unlink(missing_ok=True)
        raise
    with sqlite3.connect(f"file:{backup}?mode=ro", uri=True) as connection:
        if connection.execute("PRAGMA integrity_check").fetchone()[0] != "ok":
            raise RuntimeError(f"backup integrity check failed: {backup}")
    return backup


def _reserve_backup_path(database: pathlib.Path, timestamp: str) -> pathlib.Path:
    for suffix in range(1000):
        discriminator = "" if suffix == 0 else f"-{suffix}"
        candidate = database.with_name(
            f"{database.name}.before-graph-repair-{timestamp}{discriminator}.bak"
        )
        try:
            descriptor = os.open(candidate, os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o600)
        except FileExistsError:
            continue
        os.close(descriptor)
        return candidate
    raise RuntimeError("could not reserve a unique graph-repair backup path")


def _post_repair_integrity_check(connection: sqlite3.Connection) -> str:
    return str(connection.execute("PRAGMA integrity_check").fetchone()[0])


def _parse_args(argv: Iterable[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--db", required=True, type=pathlib.Path, help="explicit SQLite database path")
    parser.add_argument("--apply", action="store_true", help="create a backup and apply repair; omit for dry-run")
    return parser.parse_args(argv)


def main(argv: Iterable[str] | None = None) -> int:
    args = _parse_args(sys.argv[1:] if argv is None else argv)
    try:
        report = repair_database(args.db, apply=args.apply)
    except (OSError, RuntimeError, sqlite3.Error, ValueError) as error:
        print(f"engram local graph repair failed: {error}", file=sys.stderr)
        return 1
    print(json.dumps(dataclasses.asdict(report), sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
