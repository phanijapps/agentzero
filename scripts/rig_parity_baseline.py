#!/usr/bin/env python3
"""Generate sanitized Rig migration parity baseline artifacts.

This script is intentionally conservative: it reads the conversation database
in SQLite read-only mode, emits only structural signatures, and fails closed on
unknown schema fields or sanitizer errors before writing committed artifacts.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import sqlite3
import subprocess
import sys
import tempfile
from collections import Counter, defaultdict
from pathlib import Path
from typing import Any


REPO_ROOT = Path(__file__).resolve().parents[1]
DEFAULT_OUTPUT_DIR = REPO_ROOT / "docs/specs/rig-engine-migration/fixtures"
DEFAULT_LOCAL_MANIFEST = REPO_ROOT / ".rig-parity/local_manifest.json"
DEFAULT_LOCAL_DB_SIGNATURE = REPO_ROOT / ".rig-parity/old_engine_db_signature.local.json"
DEFAULT_ALLOWED_ROOTS = [Path.home() / "Documents"]
DEFAULT_FALLBACKS = [
    Path.home() / "Documents/zbot/data/conversations.db",
    Path.home() / "Documents/agentzero/conversations.db",
]
SOURCE_LABEL = "current-conversation-db"
SENTINEL = "SENTINEL_SECRET_SHOULD_NOT_LEAK"

PUBLIC_ENUM_VALUES: dict[tuple[str, str], set[str]] = {
    ("sessions", "status"): {"pending", "running", "completed", "cancelled", "canceled", "stopped", "failed", "crashed"},
    ("sessions", "source"): {"api", "cli", "cron", "daemon", "desktop", "ui", "web", "websocket"},
    ("sessions", "mode"): {"auto", "fast", "normal", "review", "build", "chat"},
    ("messages", "role"): {"assistant", "developer", "system", "tool", "user"},
    ("agent_executions", "status"): {"pending", "running", "completed", "cancelled", "canceled", "stopped", "failed", "crashed"},
    ("agent_executions", "delegation_type"): {"root", "parallel", "sequential", "delegated", "reviewer", "executor"},
    ("execution_logs", "level"): {"trace", "debug", "info", "warn", "warning", "error"},
    ("execution_logs", "category"): {
        "delegation",
        "error",
        "intent",
        "response",
        "session",
        "tool_call",
        "tool_result",
        "runtime",
        "context",
    },
}

JSON_ALLOWED_KEYS: dict[tuple[str, str], set[str]] = {
    ("sessions", "metadata"): {
        "source",
        "status",
        "summary",
        "type",
    },
    ("messages", "tool_calls"): {
        "action",
        "agent_id",
        "args",
        "artifacts",
        "category",
        "command",
        "confidence",
        "content",
        "count",
        "cwd",
        "detail",
        "file",
        "format",
        "freshness",
        "inline_references",
        "key",
        "label",
        "limit",
        "maximum_number_of_snippets",
        "maximum_number_of_tokens",
        "message",
        "mode",
        "name",
        "new_text",
        "offset",
        "old_text",
        "parallel",
        "path",
        "pattern",
        "plan",
        "prompt",
        "query",
        "search_lang",
        "skill",
        "skills",
        "source",
        "status",
        "step",
        "task",
        "timeout_seconds",
        "title",
        "tool_id",
        "tool_name",
        "type",
        "wait_for_result",
    },
    ("messages", "tool_results"): {"content", "error", "result", "status", "tool_call_id", "tool_id", "tool_name"},
    ("agent_executions", "checkpoint"): {"status", "step", "summary", "tool_calls", "turn"},
    ("execution_logs", "metadata"): {
        "action",
        "agent_id",
        "approach",
        "args",
        "artifacts",
        "blocked_by_hook",
        "category",
        "child_agent",
        "command",
        "confidence",
        "content",
        "count",
        "cwd",
        "detail",
        "error",
        "execution_strategy",
        "explanation",
        "file",
        "format",
        "graph",
        "hidden_intents",
        "key",
        "label",
        "limit",
        "message",
        "name",
        "new_text",
        "offset",
        "old_text",
        "path",
        "pattern",
        "plan",
        "primary_intent",
        "prompt",
        "recommended_agents",
        "recommended_skills",
        "reason",
        "result",
        "rewritten_prompt",
        "source",
        "status",
        "step",
        "structure",
        "subdirectory",
        "task",
        "timeout_seconds",
        "title",
        "tool_id",
        "tool_name",
        "type",
        "ward_name",
        "ward_recommendation",
    },
    ("bridge_outbox", "payload"): {"capability", "request_id", "status", "type"},
}


CLASSIFIED_SCHEMA: dict[str, dict[str, str]] = {
    "agent_executions": {
        "id": "id",
        "session_id": "id",
        "agent_id": "label_hash",
        "parent_execution_id": "id_optional",
        "delegation_type": "enum",
        "task": "sensitive_text",
        "status": "enum",
        "started_at": "timestamp",
        "completed_at": "timestamp",
        "tokens_in": "int",
        "tokens_out": "int",
        "checkpoint": "json_shape",
        "error": "sensitive_text",
        "log_path": "sensitive_path",
        "child_session_id": "id_optional",
    },
    "artifacts": {
        "id": "id",
        "session_id": "id",
        "ward_id": "label_hash",
        "execution_id": "id_optional",
        "agent_id": "label_hash",
        "file_path": "sensitive_path",
        "file_name": "sensitive_text",
        "file_type": "enum",
        "file_size": "int",
        "label": "sensitive_text",
        "created_at": "timestamp",
    },
    "bridge_outbox": {
        "id": "id",
        "adapter_id": "label_hash",
        "capability": "enum",
        "payload": "json_shape",
        "status": "enum",
        "session_id": "id_optional",
        "thread_id": "id_optional",
        "agent_id": "label_hash",
        "created_at": "timestamp",
        "sent_at": "timestamp",
        "error": "sensitive_text",
        "retry_count": "int",
        "retry_after": "timestamp",
    },
    "distillation_runs": {
        "id": "id",
        "session_id": "id",
        "status": "enum",
        "facts_extracted": "int",
        "entities_extracted": "int",
        "relationships_extracted": "int",
        "episode_created": "int",
        "error": "sensitive_text",
        "retry_count": "int",
        "duration_ms": "int",
        "created_at": "timestamp",
    },
    "execution_logs": {
        "id": "id",
        "session_id": "id",
        "conversation_id": "id_optional",
        "agent_id": "label_hash",
        "parent_session_id": "id_optional",
        "timestamp": "timestamp",
        "level": "enum",
        "category": "enum",
        "message": "sensitive_text",
        "metadata": "json_shape",
        "duration_ms": "int",
    },
    "messages": {
        "id": "id",
        "execution_id": "id_optional",
        "session_id": "id_optional",
        "role": "enum",
        "content": "sensitive_text",
        "created_at": "timestamp",
        "token_count": "int",
        "tool_calls": "json_shape",
        "tool_results": "json_shape",
        "tool_call_id": "id_optional",
    },
    "recall_log": {
        "session_id": "id",
        "fact_key": "label_hash",
        "recalled_at": "timestamp",
    },
    "schema_version": {
        "version": "int",
    },
    "sessions": {
        "id": "id",
        "status": "enum",
        "source": "enum",
        "root_agent_id": "label_hash",
        "title": "sensitive_text",
        "created_at": "timestamp",
        "started_at": "timestamp",
        "completed_at": "timestamp",
        "total_tokens_in": "int",
        "total_tokens_out": "int",
        "metadata": "json_shape",
        "pending_delegations": "int",
        "continuation_needed": "int",
        "ward_id": "label_hash",
        "parent_session_id": "id_optional",
        "thread_id": "id_optional",
        "connector_id": "label_hash",
        "respond_to": "sensitive_text",
        "archived": "int",
        "mode": "enum",
    },
}


def sha256_text(value: str) -> str:
    return hashlib.sha256(value.encode("utf-8")).hexdigest()


def stable_json(value: Any) -> str:
    return json.dumps(value, sort_keys=True, separators=(",", ":"))


def bucket_number(value: int | None) -> str:
    if value is None:
        return "null"
    if value == 0:
        return "0"
    if value <= 16:
        return "1-16"
    if value <= 128:
        return "17-128"
    if value <= 1024:
        return "129-1024"
    if value <= 8192:
        return "1025-8192"
    return "8193+"


def coerce_counter(counter: Counter[str]) -> dict[str, int]:
    return {key: counter[key] for key in sorted(counter)}


def enum_token(table: str, column: str, value: Any) -> str:
    if value is None:
        return "null"
    text = str(value)
    if text in PUBLIC_ENUM_VALUES.get((table, column), set()):
        return text
    raise ValueError(f"Unhandled enum value for {table}.{column}")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--db", help="Conversation DB path. Defaults to ZBOT_PARITY_DB or known current DB path.")
    parser.add_argument("--out-dir", type=Path, default=DEFAULT_OUTPUT_DIR, help="Committed sanitized artifact directory.")
    parser.add_argument("--local-manifest", type=Path, default=DEFAULT_LOCAL_MANIFEST, help="Gitignored exact local provenance manifest.")
    parser.add_argument("--allowed-root", action="append", type=Path, help="Allowed canonical root for DB input. Defaults to ~/Documents.")
    parser.add_argument("--self-test", action="store_true", help="Run sanitizer sentinel tests.")
    parser.add_argument("--write", action="store_true", help="Write artifacts instead of printing signature JSON.")
    return parser.parse_args()


def canonical_roots(args: argparse.Namespace) -> list[Path]:
    roots = args.allowed_root or DEFAULT_ALLOWED_ROOTS
    return [root.expanduser().resolve(strict=True) for root in roots]


def is_relative_to(path: Path, root: Path) -> bool:
    try:
        path.relative_to(root)
        return True
    except ValueError:
        return False


def resolve_db_path(args: argparse.Namespace) -> tuple[Path, str]:
    raw = args.db or os.environ.get("ZBOT_PARITY_DB")
    roots = canonical_roots(args)
    if raw:
        path = Path(raw).expanduser().resolve(strict=True)
        source = "env-or-cli"
    else:
        existing = [candidate.expanduser().resolve(strict=True) for candidate in DEFAULT_FALLBACKS if candidate.expanduser().exists()]
        unique = sorted(set(existing))
        if len(unique) != 1:
            rendered = ", ".join(str(p) for p in unique) or "none"
            raise SystemExit(
                "Unable to select parity DB unambiguously. Set ZBOT_PARITY_DB. "
                f"Fallback matches: {rendered}"
            )
        path = unique[0]
        source = "default-fallback"

    if not any(is_relative_to(path, root) for root in roots):
        allowed = ", ".join(str(root) for root in roots)
        raise SystemExit(f"Refusing DB outside allowed roots: {path}. Allowed roots: {allowed}")
    if not path.is_file():
        raise SystemExit(f"Parity DB is not a regular file: {path}")
    return path, source


def connect_readonly(path: Path) -> sqlite3.Connection:
    conn = sqlite3.connect(f"{path.as_uri()}?mode=ro", uri=True)
    conn.row_factory = sqlite3.Row
    return conn


def table_names(conn: sqlite3.Connection) -> list[str]:
    rows = conn.execute("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name").fetchall()
    return [row["name"] for row in rows]


def validate_schema(conn: sqlite3.Connection) -> dict[str, list[str]]:
    found = table_names(conn)
    unknown_tables = sorted(set(found) - set(CLASSIFIED_SCHEMA))
    missing_tables = sorted(set(CLASSIFIED_SCHEMA) - set(found))
    if unknown_tables:
        raise ValueError(f"Unhandled tables: {unknown_tables}")
    if missing_tables:
        raise ValueError(f"Missing expected tables: {missing_tables}")

    columns_by_table: dict[str, list[str]] = {}
    for table in found:
        rows = conn.execute(f"PRAGMA table_info({table})").fetchall()
        cols = [row["name"] for row in rows]
        unknown_cols = sorted(set(cols) - set(CLASSIFIED_SCHEMA[table]))
        if unknown_cols:
            raise ValueError(f"Unhandled columns for {table}: {unknown_cols}")
        columns_by_table[table] = cols
    return columns_by_table


def validate_json_columns(conn: sqlite3.Connection, columns_by_table: dict[str, list[str]]) -> None:
    for table, columns in columns_by_table.items():
        for column in columns:
            if CLASSIFIED_SCHEMA[table][column] != "json_shape":
                continue
            rows = conn.execute(f"SELECT {column} FROM {table} WHERE {column} IS NOT NULL AND {column} != ''").fetchall()
            for row in rows:
                json_shape(table, column, row[column])


def validate_storage_types(conn: sqlite3.Connection, columns_by_table: dict[str, list[str]]) -> None:
    for table, columns in columns_by_table.items():
        for column in columns:
            klass = CLASSIFIED_SCHEMA[table][column]
            allowed = allowed_storage_types(klass)
            rows = conn.execute(f"SELECT DISTINCT typeof({column}) AS storage_type FROM {table}").fetchall()
            observed = {row["storage_type"] for row in rows}
            unsupported = sorted(observed - allowed)
            if unsupported:
                raise ValueError(f"Unsupported storage type for {table}.{column}: {unsupported}")


def allowed_storage_types(klass: str) -> set[str]:
    if klass == "int":
        return {"integer", "null"}
    if klass in {
        "enum",
        "id",
        "id_optional",
        "json_shape",
        "label_hash",
        "sensitive_path",
        "sensitive_text",
        "timestamp",
    }:
        return {"text", "null"}
    raise ValueError(f"Unhandled classified schema type: {klass}")


def json_shape(table: str, column: str, value: str | None) -> Any:
    parsed = parse_json_value(table, column, value)
    if parsed is None:
        return None
    return shape_of(parsed)


def parse_json_value(table: str, column: str, value: str | None) -> Any:
    if value is None or value == "":
        return None
    parsed = json.loads(value)
    validate_json_keys(table, column, parsed)
    return parsed


def json_array_length(table: str, column: str, value: str | None) -> int:
    parsed = parse_json_value(table, column, value)
    if parsed is None:
        return 0
    if not isinstance(parsed, list):
        raise ValueError(f"Unhandled JSON shape for {table}.{column}")
    return len(parsed)


def validate_json_keys(table: str, column: str, value: Any) -> None:
    allowed = JSON_ALLOWED_KEYS.get((table, column))
    if allowed is None:
        raise ValueError(f"No JSON allowlist for {table}.{column}")
    expected_top = "list" if (table, column) in {("messages", "tool_calls"), ("messages", "tool_results")} else "object"
    if expected_top == "list" and not isinstance(value, list):
        raise ValueError(f"Unhandled JSON shape for {table}.{column}")
    if expected_top == "object" and not isinstance(value, dict):
        raise ValueError(f"Unhandled JSON shape for {table}.{column}")
    validate_json_keys_inner(table, column, value, allowed)


def validate_json_keys_inner(table: str, column: str, value: Any, allowed: set[str]) -> None:
    if isinstance(value, dict):
        for key, inner in value.items():
            if str(key) not in allowed:
                raise ValueError(f"Unhandled JSON key for {table}.{column}")
            validate_json_keys_inner(table, column, inner, allowed)
    elif isinstance(value, list):
        for item in value:
            validate_json_keys_inner(table, column, item, allowed)
    elif value is None or isinstance(value, (bool, int, float, str)):
        return
    else:
        raise TypeError(f"Unsupported JSON value type: {type(value).__name__}")


def shape_of(value: Any) -> Any:
    top = coarse_json_type(value)
    if value is None:
        return {"top": top}
    if isinstance(value, (bool, int, float, str)):
        return {"top": top, "serialized_length": bucket_number(len(stable_json(value)))}
    if isinstance(value, list):
        item_shapes = sorted(stable_json(shape_of(item)) for item in value)
        return {
            "top": top,
            "length": bucket_number(len(value)),
            "item_type_counts": coerce_counter(Counter(coarse_json_type(item) for item in value)),
            "item_shapes": [json.loads(item_shape) for item_shape in item_shapes],
        }
    if isinstance(value, dict):
        fields = {str(key): shape_of(value[key]) for key in sorted(value)}
        return {
            "top": top,
            "key_count": bucket_number(len(value)),
            "keys": sorted(str(key) for key in value),
            "fields": fields,
            "value_type_counts": coerce_counter(Counter(coarse_json_type(inner) for inner in value.values())),
        }
    raise TypeError(f"Unsupported JSON value type: {type(value).__name__}")


def coarse_json_type(value: Any) -> str:
    if value is None:
        return "null"
    if isinstance(value, bool):
        return "bool"
    if isinstance(value, (int, float)):
        return "number"
    if isinstance(value, str):
        return "string"
    if isinstance(value, list):
        return "list"
    if isinstance(value, dict):
        return "object"
    raise TypeError(f"Unsupported JSON value type: {type(value).__name__}")


def provenance(path: Path) -> tuple[dict[str, Any], dict[str, Any]]:
    stat = path.stat()
    exact = {
        "path": str(path),
        "size_bytes": stat.st_size,
        "mtime_ns": stat.st_mtime_ns,
    }
    committed = {
        "label": SOURCE_LABEL,
        "size_bucket": bucket_number(stat.st_size),
    }
    return committed, exact


def validate_local_manifest_path(path: Path) -> Path:
    resolved = path.expanduser().resolve(strict=False)
    expected = DEFAULT_LOCAL_MANIFEST.resolve(strict=False)
    if resolved != expected:
        raise ValueError(f"Exact provenance manifest must be written to {expected}")

    result = subprocess.run(
        ["git", "check-ignore", "-q", str(expected)],
        cwd=REPO_ROOT,
        check=False,
    )
    if result.returncode != 0:
        raise ValueError(f"Exact provenance manifest is not gitignored: {expected}")
    return expected


def validate_local_db_signature_path() -> Path:
    expected = DEFAULT_LOCAL_DB_SIGNATURE.resolve(strict=False)
    result = subprocess.run(
        ["git", "check-ignore", "-q", str(expected)],
        cwd=REPO_ROOT,
        check=False,
    )
    if result.returncode != 0:
        raise ValueError(f"Exact DB signature is not gitignored: {expected}")
    return expected


def count_rows(conn: sqlite3.Connection, table: str) -> int:
    return int(conn.execute(f"SELECT COUNT(*) AS n FROM {table}").fetchone()["n"])


def shape_hash(value: Any) -> str:
    return sha256_text(stable_json(value))[:24]


def build_event_signatures(conn: sqlite3.Connection) -> dict[str, Any]:
    session_events: dict[str, list[str]] = defaultdict(list)
    session_terminal_events: dict[str, str] = {}
    event_variants = Counter()
    lifecycle_counts = Counter()
    tool_pair_counts = Counter()
    delegation_transitions = Counter()

    for row in conn.execute("SELECT id, status FROM sessions ORDER BY created_at, id"):
        session_id = str(row["id"])
        events = session_events[session_id]
        events.append("AgentStarted")
        event_variants["AgentStarted"] += 1
        status = enum_token("sessions", "status", row["status"])
        lifecycle_counts[f"session_status:{status}"] += 1
        if status == "completed":
            session_terminal_events[session_id] = "AgentCompleted"
            event_variants["AgentCompleted"] += 1
        elif status in {"cancelled", "canceled", "stopped"}:
            session_terminal_events[session_id] = "AgentStopped"
            event_variants["AgentStopped"] += 1
        elif status in {"failed", "crashed"}:
            session_terminal_events[session_id] = "Error"
            event_variants["Error"] += 1

    tool_counts_by_session: dict[str, Counter[str]] = defaultdict(Counter)
    for row in conn.execute(
        "SELECT session_id, role, content, tool_calls, tool_results FROM messages ORDER BY session_id, created_at, id"
    ):
        session_id = str(row["session_id"])
        events = session_events[session_id]
        role = enum_token("messages", "role", row["role"])
        events.append("MessageAdded")
        event_variants["MessageAdded"] += 1
        lifecycle_counts[f"message_role:{role}"] += 1

        tool_call_count = json_array_length("messages", "tool_calls", row["tool_calls"])
        tool_result_count = json_array_length("messages", "tool_results", row["tool_results"])
        if role == "tool" and tool_result_count == 0:
            tool_result_count = 1
        for _ in range(tool_call_count):
            events.append("ToolCall")
            event_variants["ToolCall"] += 1
        for _ in range(tool_result_count):
            events.append("ToolResult")
            event_variants["ToolResult"] += 1
        tool_counts_by_session[session_id]["calls"] += tool_call_count
        tool_counts_by_session[session_id]["results"] += tool_result_count
        if role == "assistant" and row["content"]:
            events.append("Token")
            events.append("TurnComplete")
            event_variants["Token"] += 1
            event_variants["TurnComplete"] += 1

    for session_id, counts in tool_counts_by_session.items():
        tool_pair_counts[
            f"calls:{bucket_number(counts['calls'])}|results:{bucket_number(counts['results'])}"
        ] += 1
        if counts["calls"] == counts["results"]:
            tool_pair_counts["balanced"] += 1
        else:
            tool_pair_counts["unbalanced"] += 1

    for row in conn.execute("SELECT status, delegation_type, parent_execution_id, child_session_id FROM agent_executions"):
        status = enum_token("agent_executions", "status", row["status"])
        delegation_type = enum_token("agent_executions", "delegation_type", row["delegation_type"])
        lifecycle_counts[f"execution_status:{status}"] += 1
        delegation_transitions[
            f"type:{delegation_type}|parent:{bool(row['parent_execution_id'])}|child:{bool(row['child_session_id'])}|status:{status}"
        ] += 1
        if row["child_session_id"]:
            event_variants["DelegationStarted"] += 1
        if row["parent_execution_id"] and status == "completed":
            event_variants["DelegationCompleted"] += 1
        if status in {"failed", "crashed"}:
            event_variants["Error"] += 1
        elif status in {"cancelled", "canceled", "stopped"}:
            event_variants["AgentStopped"] += 1

    for session_id, terminal_event in session_terminal_events.items():
        session_events[session_id].append(terminal_event)

    event_sequence_hashes = Counter(shape_hash(sequence) for sequence in session_events.values())
    event_length_buckets = Counter(bucket_number(len(sequence)) for sequence in session_events.values())
    return {
        "event_variant_counts": coerce_counter(event_variants),
        "event_sequence_hashes": coerce_counter(event_sequence_hashes),
        "event_sequence_length_buckets": coerce_counter(event_length_buckets),
        "tool_pair_counts": coerce_counter(tool_pair_counts),
        "lifecycle_counts": coerce_counter(lifecycle_counts),
        "delegation_transition_counts": coerce_counter(delegation_transitions),
    }


def build_signature(conn: sqlite3.Connection, db_path: Path, source_kind: str) -> tuple[dict[str, Any], dict[str, Any]]:
    columns_by_table = validate_schema(conn)
    validate_storage_types(conn, columns_by_table)
    validate_json_columns(conn, columns_by_table)
    committed_provenance, exact_provenance = provenance(db_path)

    counts = {table: count_rows(conn, table) for table in sorted(columns_by_table)}
    schema_hashes = {
        table: shape_hash({"columns": columns_by_table[table], "classes": CLASSIFIED_SCHEMA[table]})
        for table in sorted(columns_by_table)
    }

    session_status = Counter()
    session_source = Counter()
    session_mode = Counter()
    session_pending = Counter()
    for row in conn.execute("SELECT status, source, mode, pending_delegations, continuation_needed, archived FROM sessions"):
        session_status[enum_token("sessions", "status", row["status"])] += 1
        session_source[enum_token("sessions", "source", row["source"])] += 1
        session_mode[enum_token("sessions", "mode", row["mode"])] += 1
        session_pending[
            f"pending:{bucket_number(row['pending_delegations'])}|continuation:{bucket_number(row['continuation_needed'])}|archived:{bucket_number(row['archived'])}"
        ] += 1

    role_counts = Counter()
    content_buckets = Counter()
    token_buckets = Counter()
    role_sequences: dict[str, list[str]] = defaultdict(list)
    tool_shape_hashes = Counter()
    for row in conn.execute("SELECT session_id, role, content, token_count, tool_calls, tool_results FROM messages ORDER BY session_id, created_at, id"):
        role = enum_token("messages", "role", row["role"])
        role_counts[role] += 1
        role_sequences[str(row["session_id"])].append(role)
        content_buckets[bucket_number(len(row["content"] or ""))] += 1
        token_buckets[bucket_number(row["token_count"])] += 1
        for field in ("tool_calls", "tool_results"):
            if row[field]:
                tool_shape_hashes[f"{field}:{shape_hash(json_shape('messages', field, row[field]))}"] += 1

    sequence_hashes = Counter()
    for sequence in role_sequences.values():
        sequence_hashes[shape_hash(sequence)] += 1

    execution_status = Counter()
    delegation_types = Counter()
    execution_token_buckets = Counter()
    parent_child = Counter()
    for row in conn.execute(
        "SELECT status, delegation_type, parent_execution_id, child_session_id, tokens_in, tokens_out FROM agent_executions"
    ):
        execution_status[enum_token("agent_executions", "status", row["status"])] += 1
        delegation_types[enum_token("agent_executions", "delegation_type", row["delegation_type"])] += 1
        parent_child[f"parent:{bool(row['parent_execution_id'])}|child:{bool(row['child_session_id'])}"] += 1
        execution_token_buckets[f"in:{bucket_number(row['tokens_in'])}|out:{bucket_number(row['tokens_out'])}"] += 1

    log_levels = Counter()
    log_categories = Counter()
    log_message_buckets = Counter()
    log_metadata_shapes = Counter()
    for row in conn.execute("SELECT level, category, message, metadata FROM execution_logs"):
        log_levels[enum_token("execution_logs", "level", row["level"])] += 1
        log_categories[enum_token("execution_logs", "category", row["category"])] += 1
        log_message_buckets[bucket_number(len(row["message"] or ""))] += 1
        if row["metadata"]:
            log_metadata_shapes[shape_hash(json_shape("execution_logs", "metadata", row["metadata"]))] += 1

    version_row = conn.execute("SELECT MAX(version) AS version FROM schema_version").fetchone()
    signature = {
        "artifact_schema": 1,
        "source": committed_provenance | {"selection": source_kind},
        "comparison_dimensions": [
            "table_counts",
            "schema_hashes",
            "session_status_counts",
            "message_role_counts",
            "role_sequence_hashes",
            "tool_json_shape_hashes",
            "delegation_type_counts",
            "execution_status_counts",
            "parent_child_transition_counts",
            "execution_log_category_counts",
            "db_derived_event_variant_counts",
            "db_derived_event_sequence_hashes",
            "db_derived_tool_pair_counts",
            "db_derived_delegation_transition_counts",
        ],
        "schema_version": version_row["version"],
        "table_counts": counts,
        "schema_hashes": schema_hashes,
        "sessions": {
            "status_counts": coerce_counter(session_status),
            "source_counts": coerce_counter(session_source),
            "mode_counts": coerce_counter(session_mode),
            "state_counts": coerce_counter(session_pending),
        },
        "messages": {
            "role_counts": coerce_counter(role_counts),
            "content_length_buckets": coerce_counter(content_buckets),
            "token_count_buckets": coerce_counter(token_buckets),
            "role_sequence_hashes": coerce_counter(sequence_hashes),
            "tool_shape_hashes": coerce_counter(tool_shape_hashes),
        },
        "executions": {
            "status_counts": coerce_counter(execution_status),
            "delegation_type_counts": coerce_counter(delegation_types),
            "parent_child_counts": coerce_counter(parent_child),
            "token_buckets": coerce_counter(execution_token_buckets),
        },
        "execution_logs": {
            "level_counts": coerce_counter(log_levels),
            "category_counts": coerce_counter(log_categories),
            "message_length_buckets": coerce_counter(log_message_buckets),
            "metadata_shape_hashes": coerce_counter(log_metadata_shapes),
        },
        "db_derived_event_signatures": build_event_signatures(conn),
    }
    local_manifest = {
        "artifact_schema": 1,
        "source": SOURCE_LABEL,
        "selection": source_kind,
        "exact_provenance": exact_provenance,
        "committed_provenance": committed_provenance,
    }
    return signature, local_manifest


def coarsen_counts(counts: dict[str, int]) -> dict[str, str]:
    return {key: bucket_number(value) for key, value in sorted(counts.items())}


def coarsen_signature(signature: dict[str, Any]) -> dict[str, Any]:
    event_signatures = signature["db_derived_event_signatures"]
    return {
        "artifact_schema": signature["artifact_schema"],
        "source": signature["source"],
        "comparison_dimensions": [
            "table_count_buckets",
            "schema_hashes",
            "session_status_count_buckets",
            "message_role_count_buckets",
            "message_content_length_bucket_presence",
            "delegation_type_count_buckets",
            "execution_status_count_buckets",
            "db_derived_event_variant_count_buckets",
            "db_derived_tool_pair_count_buckets",
            "db_derived_delegation_transition_count_buckets",
        ],
        "schema_version": signature["schema_version"],
        "table_count_buckets": coarsen_counts(signature["table_counts"]),
        "schema_hashes": signature["schema_hashes"],
        "sessions": {
            "status_count_buckets": coarsen_counts(signature["sessions"]["status_counts"]),
            "source_count_buckets": coarsen_counts(signature["sessions"]["source_counts"]),
            "mode_count_buckets": coarsen_counts(signature["sessions"]["mode_counts"]),
            "state_count_buckets": coarsen_counts(signature["sessions"]["state_counts"]),
        },
        "messages": {
            "role_count_buckets": coarsen_counts(signature["messages"]["role_counts"]),
            "content_length_bucket_presence": sorted(signature["messages"]["content_length_buckets"].keys()),
            "token_count_bucket_presence": sorted(signature["messages"]["token_count_buckets"].keys()),
            "distinct_role_sequence_count_bucket": bucket_number(len(signature["messages"]["role_sequence_hashes"])),
            "distinct_tool_shape_count_bucket": bucket_number(len(signature["messages"]["tool_shape_hashes"])),
        },
        "executions": {
            "status_count_buckets": coarsen_counts(signature["executions"]["status_counts"]),
            "delegation_type_count_buckets": coarsen_counts(signature["executions"]["delegation_type_counts"]),
            "parent_child_count_buckets": coarsen_counts(signature["executions"]["parent_child_counts"]),
            "token_bucket_presence": sorted(signature["executions"]["token_buckets"].keys()),
        },
        "execution_logs": {
            "level_count_buckets": coarsen_counts(signature["execution_logs"]["level_counts"]),
            "category_count_buckets": coarsen_counts(signature["execution_logs"]["category_counts"]),
            "message_length_bucket_presence": sorted(signature["execution_logs"]["message_length_buckets"].keys()),
            "distinct_metadata_shape_count_bucket": bucket_number(len(signature["execution_logs"]["metadata_shape_hashes"])),
        },
        "db_derived_event_signatures": {
            "event_variant_count_buckets": coarsen_counts(event_signatures["event_variant_counts"]),
            "event_sequence_length_bucket_presence": sorted(event_signatures["event_sequence_length_buckets"].keys()),
            "distinct_event_sequence_count_bucket": bucket_number(len(event_signatures["event_sequence_hashes"])),
            "tool_pair_count_buckets": coarsen_counts(event_signatures["tool_pair_counts"]),
            "lifecycle_count_buckets": coarsen_counts(event_signatures["lifecycle_counts"]),
            "delegation_transition_count_buckets": coarsen_counts(event_signatures["delegation_transition_counts"]),
        },
    }


def synthetic_cases(signature: dict[str, Any], event_signature: dict[str, Any] | None = None) -> dict[str, Any]:
    events = signature["db_derived_event_signatures"]["event_variant_counts"]
    executions = signature["executions"]["status_counts"]
    roles = signature["messages"]["role_counts"]
    cases: list[dict[str, Any]] = []
    skipped: list[dict[str, str]] = []

    if roles.get("user", 0) and roles.get("assistant", 0):
        cases.append(
            {
                "name": "simple_chat",
                "source_pattern": {
                    "user_messages": bucket_number(roles["user"]),
                    "assistant_messages": bucket_number(roles["assistant"]),
                },
                "messages": [
                    {"role": "user", "content": "<redacted-user-message>"},
                    {"role": "assistant", "content": "<redacted-assistant-response>"},
                ],
                "expected_events": ["AgentStarted", "Token", "TurnComplete", "AgentCompleted"],
            }
        )
    else:
        skipped.append({"name": "simple_chat", "reason": "source DB has no user/assistant message pattern"})

    if events.get("ToolCall", 0) and events.get("ToolResult", 0):
        cases.append(
            {
                "name": "tool_call_result",
                "source_pattern": {
                    "tool_calls": bucket_number(events["ToolCall"]),
                    "tool_results": bucket_number(events["ToolResult"]),
                },
                "messages": [
                    {"role": "user", "content": "<redacted-user-message>"},
                    {"role": "assistant", "tool_calls": [{"name": "<redacted-observed-tool>", "arguments_shape": {"top": "object"}}]},
                    {"role": "tool", "tool_call_id": "synthetic-tool-call", "content": "<redacted-tool-result>"},
                    {"role": "assistant", "content": "<redacted-assistant-response>"},
                ],
                "expected_events": ["ToolCall", "ToolResult", "TurnComplete"],
            }
        )
    else:
        skipped.append({"name": "tool_call_result", "reason": "source DB has no tool call/result pattern"})

    if events.get("DelegationStarted", 0) or events.get("DelegationCompleted", 0):
        cases.append(
            {
                "name": "delegation_continuation",
                "source_pattern": {
                    "delegation_started": bucket_number(events.get("DelegationStarted", 0)),
                    "delegation_completed": bucket_number(events.get("DelegationCompleted", 0)),
                },
                "execution_states": [
                    {"delegation_type": "root", "status": "running"},
                    {"delegation_type": "delegated", "status": "completed", "parent": True},
                    {"continuation": "SessionContinuationReady"},
                ],
                "expected_events": ["DelegationStarted", "DelegationCompleted"],
                "internal_transitions": ["SessionContinuationReady"],
            }
        )
    else:
        skipped.append({"name": "delegation_continuation", "reason": "source DB has no delegation pattern"})

    if executions.get("crashed", 0) or executions.get("failed", 0):
        cases.append(
            {
                "name": "error",
                "source_pattern": {
                    "crashed": bucket_number(executions.get("crashed", 0)),
                    "failed": bucket_number(executions.get("failed", 0)),
                },
                "execution_states": [{"status": "crashed", "error": "<redacted-error>"}],
                "expected_events": ["Error"],
            }
        )
    else:
        skipped.append({"name": "error", "reason": "source DB has no failed/crashed execution pattern"})

    if (
        events.get("AgentStopped", 0)
        or signature["sessions"]["status_counts"].get("cancelled", 0)
        or event_signature_has_server_variant(event_signature, "agent_stopped")
    ) and event_signature_has_direct_server_variant(event_signature, "session_cancelled"):
        cases.append(
            {
                "name": "stop_cancel",
                "source_pattern": {
                    "agent_stopped": bucket_number(events.get("AgentStopped", 0)),
                    "cancelled_sessions": bucket_number(signature["sessions"]["status_counts"].get("cancelled", 0)),
                    "old_engine_wire_agent_stopped": event_signature_has_server_variant(event_signature, "agent_stopped"),
                    "old_engine_direct_session_cancelled": event_signature_has_direct_server_variant(event_signature, "session_cancelled"),
                },
                "execution_states": [{"status": "cancelled"}],
                "expected_events": ["AgentStopped", "SessionCancelled"],
            }
        )
    else:
        skipped.append({"name": "stop_cancel", "reason": "source DB has no stop/cancel pattern; fixture must be added when observed or simulated by old-engine capture"})

    return {
        "artifact_schema": 1,
        "source": "sanitized-db-derived-cases",
        "cases": cases,
        "skipped_cases": skipped,
    }


def event_signature_has_server_variant(signature: dict[str, Any] | None, variant: str) -> bool:
    if not signature:
        return False
    for section in ("stream_scenarios", "gateway_scenarios"):
        for scenario in signature.get(section, []):
            for record in scenario.get("records", []):
                message = record.get("server_message")
                if isinstance(message, dict) and message.get("variant") == variant:
                    return True
    return False


def event_signature_has_direct_server_variant(signature: dict[str, Any] | None, variant: str) -> bool:
    if not signature:
        return False
    for record in signature.get("direct_server_messages", []):
        if isinstance(record, dict) and record.get("variant") == variant:
            return True
    return False


def assert_no_sentinel(value: Any) -> None:
    rendered = stable_json(value)
    if SENTINEL in rendered:
        raise AssertionError("sentinel leaked into sanitized artifact")


def write_json(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_suffix(path.suffix + ".tmp")
    tmp.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    os.replace(tmp, path)


def capture_old_engine_event_signature() -> dict[str, Any]:
    with tempfile.TemporaryDirectory() as tmpdir:
        path = Path(tmpdir) / "old_engine_event_signature.json"
        subprocess.run(
            [
                "cargo",
                "run",
                "-q",
                "-p",
                "gateway",
                "--features",
                "rig-parity-capture",
                "--example",
                "rig_parity_event_capture",
                "--",
                str(path),
            ],
            cwd=REPO_ROOT,
            check=True,
        )
        return json.loads(path.read_text(encoding="utf-8"))


def run_self_test() -> None:
    with tempfile.TemporaryDirectory() as tmpdir:
        db_path = Path(tmpdir) / "conversations.db"
        conn = sqlite3.connect(db_path)
        try:
            create_test_schema(conn)
            seed_test_data(conn)
            conn.commit()
        finally:
            conn.close()
        ro = connect_readonly(db_path)
        try:
            signature, manifest = build_signature(ro, db_path.resolve(), "self-test")
            fixtures = synthetic_cases(signature)
        finally:
            ro.close()
        assert_no_sentinel(signature)
        assert_no_sentinel(fixtures)
        assert_no_sentinel(manifest["committed_provenance"])
        assert_unknown_enum_fails_closed(db_path)
        assert_unknown_json_fails_closed()
        assert_bare_json_fails_closed()
        assert_blob_fails_closed()
        assert_shape_and_count_invariants()


def assert_unknown_enum_fails_closed(db_path: Path) -> None:
    conn = sqlite3.connect(db_path)
    try:
        conn.execute("UPDATE sessions SET status = ?", (SENTINEL,))
        conn.commit()
    finally:
        conn.close()

    ro = connect_readonly(db_path)
    try:
        try:
            build_signature(ro, db_path.resolve(), "self-test")
        except ValueError as exc:
            if "Unhandled enum value" not in str(exc):
                raise
        else:
            raise AssertionError("unknown enum value did not fail closed")
    finally:
        ro.close()


def assert_unknown_json_fails_closed() -> None:
    with tempfile.TemporaryDirectory() as tmpdir:
        db_path = Path(tmpdir) / "unknown-json.db"
        conn = sqlite3.connect(db_path)
        try:
            create_test_schema(conn)
            seed_test_data(conn)
            conn.execute("UPDATE execution_logs SET metadata = ?", (json.dumps({"connector_secret": SENTINEL}),))
            conn.commit()
        finally:
            conn.close()

        ro = connect_readonly(db_path)
        try:
            try:
                build_signature(ro, db_path.resolve(), "self-test")
            except ValueError as exc:
                if "Unhandled JSON key" not in str(exc):
                    raise
            else:
                raise AssertionError("unknown JSON key did not fail closed")
        finally:
            ro.close()


def assert_bare_json_fails_closed() -> None:
    with tempfile.TemporaryDirectory() as tmpdir:
        db_path = Path(tmpdir) / "bare-json.db"
        conn = sqlite3.connect(db_path)
        try:
            create_test_schema(conn)
            seed_test_data(conn)
            conn.execute("UPDATE sessions SET metadata = ?", (json.dumps(SENTINEL),))
            conn.commit()
        finally:
            conn.close()

        ro = connect_readonly(db_path)
        try:
            try:
                build_signature(ro, db_path.resolve(), "self-test")
            except ValueError as exc:
                if "Unhandled JSON shape" not in str(exc):
                    raise
            else:
                raise AssertionError("bare JSON value did not fail closed")
        finally:
            ro.close()


def assert_blob_fails_closed() -> None:
    with tempfile.TemporaryDirectory() as tmpdir:
        db_path = Path(tmpdir) / "blob.db"
        conn = sqlite3.connect(db_path)
        try:
            create_test_schema(conn)
            seed_test_data(conn)
            conn.execute("UPDATE messages SET content = ?", (sqlite3.Binary(SENTINEL.encode("utf-8")),))
            conn.commit()
        finally:
            conn.close()

        ro = connect_readonly(db_path)
        try:
            try:
                build_signature(ro, db_path.resolve(), "self-test")
            except ValueError as exc:
                if "Unsupported storage type" not in str(exc):
                    raise
            else:
                raise AssertionError("BLOB value did not fail closed")
        finally:
            ro.close()


def assert_shape_and_count_invariants() -> None:
    first_shape = shape_hash(
        json_shape(
            "messages",
            "tool_calls",
            json.dumps([{"tool_id": "first", "tool_name": "tool"}]),
        )
    )
    second_shape = shape_hash(
        json_shape(
            "messages",
            "tool_calls",
            json.dumps([{"status": "ok", "tool_name": "tool"}]),
        )
    )
    if first_shape == second_shape:
        raise AssertionError("JSON shape hashing is not key-sensitive")

    with tempfile.TemporaryDirectory() as tmpdir:
        db_path = Path(tmpdir) / "shape-counts.db"
        conn = sqlite3.connect(db_path)
        try:
            create_test_schema(conn)
            seed_test_data(conn)
            conn.execute(
                "INSERT INTO messages (id, session_id, role, content, created_at, token_count, tool_calls, tool_results) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
                (
                    "message-2",
                    "session-1",
                    "assistant",
                    "",
                    "2026-06-27T00:00:01Z",
                    0,
                    json.dumps(
                        [
                            {"tool_id": "call-1", "tool_name": "tool", "args": {"command": "redacted"}},
                            {"tool_id": "call-2", "tool_name": "tool", "args": {"path": "redacted"}},
                        ]
                    ),
                    json.dumps([]),
                ),
            )
            conn.execute(
                "INSERT INTO messages (id, session_id, role, content, created_at, token_count, tool_calls, tool_results) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
                (
                    "message-3",
                    "session-1",
                    "assistant",
                    "",
                    "2026-06-27T00:00:02Z",
                    0,
                    json.dumps([]),
                    json.dumps([]),
                ),
            )
            conn.execute(
                "INSERT INTO messages (id, session_id, role, content, created_at, token_count, tool_calls, tool_results) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
                (
                    "message-4",
                    "session-1",
                    "tool",
                    "redacted",
                    "2026-06-27T00:00:03Z",
                    0,
                    None,
                    json.dumps([]),
                ),
            )
            conn.commit()
        finally:
            conn.close()

        ro = connect_readonly(db_path)
        try:
            signature, _ = build_signature(ro, db_path.resolve(), "self-test")
        finally:
            ro.close()

    events = signature["db_derived_event_signatures"]["event_variant_counts"]
    if events.get("ToolCall") != 3:
        raise AssertionError("tool call event counts do not reflect JSON array entries")
    if events.get("ToolResult") != 2:
        raise AssertionError("tool result event counts do not handle arrays and tool-role messages")


def create_test_schema(conn: sqlite3.Connection) -> None:
    for table, cols in CLASSIFIED_SCHEMA.items():
        col_defs = []
        for col, klass in cols.items():
            sql_type = "INTEGER" if klass == "int" else "TEXT"
            col_defs.append(f"{col} {sql_type}")
        conn.execute(f"CREATE TABLE {table} ({', '.join(col_defs)})")


def seed_test_data(conn: sqlite3.Connection) -> None:
    conn.execute(
        "INSERT INTO sessions (id, status, source, root_agent_id, title, created_at, metadata, pending_delegations, continuation_needed, archived) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        ("session-1", "completed", "web", "root", SENTINEL, "2026-06-27", json.dumps({"summary": SENTINEL}), 0, 0, 0),
    )
    conn.execute(
        "INSERT INTO messages (id, session_id, role, content, created_at, token_count, tool_calls, tool_results) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        (
            "message-1",
            "session-1",
            "assistant",
            SENTINEL,
            "2026-06-27",
            10,
            json.dumps([{"tool_name": SENTINEL, "tool_id": SENTINEL, "args": {"command": SENTINEL}}]),
            json.dumps([{"tool_name": SENTINEL, "tool_id": SENTINEL, "result": SENTINEL}]),
        ),
    )
    conn.execute(
        "INSERT INTO agent_executions (id, session_id, agent_id, delegation_type, task, status, checkpoint, error, tokens_in, tokens_out) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        ("exec-1", "session-1", "agent", "root", SENTINEL, "completed", json.dumps({"status": SENTINEL}), SENTINEL, 1, 2),
    )
    conn.execute(
        "INSERT INTO execution_logs (id, session_id, agent_id, timestamp, level, category, message, metadata) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        ("log-1", "session-1", "agent", "2026-06-27", "error", "runtime", SENTINEL, json.dumps({"result": SENTINEL})),
    )


def main() -> int:
    args = parse_args()
    if args.self_test:
        run_self_test()
        if not args.write:
            return 0

    db_path, source_kind = resolve_db_path(args)
    conn = connect_readonly(db_path)
    try:
        exact_signature, local_manifest = build_signature(conn, db_path, source_kind)
    finally:
        conn.close()

    local_manifest_path = validate_local_manifest_path(args.local_manifest) if args.write else None
    local_db_signature_path = validate_local_db_signature_path() if args.write else None
    event_signature = capture_old_engine_event_signature() if args.write else None
    committed_signature = coarsen_signature(exact_signature)
    fixtures = synthetic_cases(exact_signature, event_signature)
    local_manifest["exact_db_signature_path"] = str(local_db_signature_path) if local_db_signature_path else None

    assert_no_sentinel(exact_signature)
    assert_no_sentinel(committed_signature)
    assert_no_sentinel(fixtures)
    if event_signature is not None:
        assert_no_sentinel(event_signature)
    if args.write:
        write_json(local_db_signature_path, exact_signature)
        write_json(args.out_dir / "old_engine_event_signature.json", event_signature)
        write_json(args.out_dir / "old_engine_signature.json", committed_signature)
        write_json(args.out_dir / "synthetic_e2e_cases.json", fixtures)
        write_json(local_manifest_path, local_manifest)
    else:
        print(json.dumps(committed_signature, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (json.JSONDecodeError, sqlite3.Error, ValueError, TypeError, OSError) as exc:
        print(f"rig_parity_baseline: failed closed: {exc}", file=sys.stderr)
        raise SystemExit(1)
