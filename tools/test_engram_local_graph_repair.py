"""Construction tests for the conservative local Engram graph repair."""

from __future__ import annotations

import pathlib
import sqlite3
import sys
import tempfile
import unittest
from unittest import mock

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))

import engram_local_graph_repair as repair


class EngramLocalGraphRepairTests(unittest.TestCase):
    def setUp(self) -> None:
        self.tempdir = tempfile.TemporaryDirectory()
        self.db_path = pathlib.Path(self.tempdir.name) / "engram_data.db"
        with sqlite3.connect(self.db_path) as connection:
            connection.executescript(
                """
                CREATE TABLE kg_entities (id TEXT PRIMARY KEY, entity_type TEXT NOT NULL);
                CREATE TABLE kg_relationships (
                    id TEXT PRIMARY KEY,
                    source_entity_id TEXT NOT NULL,
                    target_entity_id TEXT NOT NULL,
                    relationship_type TEXT NOT NULL
                );
                INSERT INTO kg_entities VALUES ('known-a', 'organization');
                INSERT INTO kg_entities VALUES ('known-b', 'tool');
                INSERT INTO kg_entities VALUES ('unknown-a', 'unknown');
                INSERT INTO kg_entities VALUES ('unknown-standalone', 'unknown');
                INSERT INTO kg_relationships VALUES ('valid', 'known-a', 'known-b', 'uses');
                INSERT INTO kg_relationships VALUES ('custom', 'known-a', 'known-b', 'partnerof');
                INSERT INTO kg_relationships VALUES ('stub-edge', 'unknown-a', 'known-b', 'uses');
                INSERT INTO kg_relationships VALUES ('overlap-edge', 'unknown-a', 'known-b', 'partnerof');
                """
            )

    def tearDown(self) -> None:
        self.tempdir.cleanup()

    def test_dry_run_does_not_mutate(self) -> None:
        report = repair.repair_database(self.db_path, apply=False)

        self.assertEqual(report.unknown_entities, 2)
        self.assertEqual(report.unsupported_relationships, 2)
        self.assertEqual(report.stub_relationships, 1)
        self.assertIsNone(report.backup_path)
        self.assertEqual(self._count("kg_entities"), 4)
        self.assertEqual(self._count("kg_relationships"), 4)

    def test_apply_backs_up_removes_only_noise_and_is_idempotent(self) -> None:
        first = repair.repair_database(self.db_path, apply=True)

        self.assertIsNotNone(first.backup_path)
        self.assertTrue(pathlib.Path(first.backup_path).is_file())
        self.assertEqual(self._count("kg_entities"), 3)
        self.assertEqual(self._count("kg_relationships"), 1)
        self.assertEqual(self._scalar("SELECT relationship_type FROM kg_relationships"), "uses")
        self.assertEqual(
            self._scalar("SELECT entity_type FROM kg_entities WHERE id = 'unknown-standalone'"),
            "unknown",
        )
        self.assertEqual(self._scalar("PRAGMA integrity_check"), "ok")

        second = repair.repair_database(self.db_path, apply=True)
        self.assertEqual(second.deleted_entities, 0)
        self.assertEqual(second.deleted_relationships, 0)
        self.assertNotEqual(first.backup_path, second.backup_path)
        with sqlite3.connect(first.backup_path) as first_backup:
            self.assertEqual(first_backup.execute("SELECT COUNT(*) FROM kg_entities").fetchone()[0], 4)
            self.assertEqual(first_backup.execute("SELECT COUNT(*) FROM kg_relationships").fetchone()[0], 4)

    def test_integrity_failure_rolls_back_and_retains_backup(self) -> None:
        with mock.patch.object(repair, "_post_repair_integrity_check", return_value="corrupt"):
            with self.assertRaisesRegex(RuntimeError, "integrity check failed"):
                repair.repair_database(self.db_path, apply=True)

        self.assertEqual(self._count("kg_entities"), 4)
        self.assertEqual(self._count("kg_relationships"), 4)
        backups = list(self.db_path.parent.glob("*.before-graph-repair-*.bak"))
        self.assertEqual(len(backups), 1)
        with sqlite3.connect(backups[0]) as backup:
            self.assertEqual(backup.execute("PRAGMA integrity_check").fetchone()[0], "ok")
            self.assertEqual(backup.execute("SELECT COUNT(*) FROM kg_entities").fetchone()[0], 4)

    def _count(self, table: str) -> int:
        return int(self._scalar(f"SELECT COUNT(*) FROM {table}"))

    def _scalar(self, query: str) -> object:
        with sqlite3.connect(self.db_path) as connection:
            return connection.execute(query).fetchone()[0]


if __name__ == "__main__":
    unittest.main()
