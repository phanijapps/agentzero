#!/usr/bin/env python3
"""Verify context-capability terminal cleanup deny lists.

This is intentionally targeted. Whole-repo greps are too noisy because old
conversation fixtures, historical log replay, UI CRUD route names, MCP protocol
methods, tests, and RFC/spec text legitimately mention retired names.
"""

from __future__ import annotations

import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]


def read(rel: str) -> str:
    return (ROOT / rel).read_text(encoding="utf-8")


def fail(message: str) -> None:
    print(f"cleanup check failed: {message}", file=sys.stderr)
    sys.exit(1)


def assert_absent(rel: str, needles: list[str], *, before_tests: bool = False) -> None:
    text = read(rel)
    if before_tests:
        text = text.split("#[cfg(test)]", 1)[0]
    for needle in needles:
        if needle in text:
            fail(f"{needle!r} still present in {rel}")


def assert_missing(rel: str) -> None:
    if (ROOT / rel).exists():
        fail(f"retired file still exists: {rel}")


def main() -> None:
    retired_tools = [
        "ListSkillsTool",
        "ListMcpsTool",
        "ListToolsTool",
        "SetSessionTitleTool",
        "TodoTool",
        "list_skills",
        "list_mcps",
        "list_tools",
        "set_session_title",
        "todos",
        "GrepTool",
        '"grep"',
    ]
    retired_sqlite_runtime = [
        "pub knowledge_db",
        "state.knowledge_db",
        "KnowledgeDatabase",
        "embedding_reindex",
        "kg_backfill",
        "memory_facts_index",
        "sqlite-vec",
        "zbot_stores_sqlite::REQUIRED_VEC_TABLES",
    ]

    prompt_paths = [
        "gateway/templates/default_policies.json",
        "gateway/templates/chat_instructions.md",
        "gateway/templates/shards/planning_autonomy.md",
        "gateway/templates/shards/tooling_skills.md",
        "gateway/templates/shards/first_turn_protocol.md",
        "gateway/templates/agents/solution-agent.md",
        "gateway/templates/agents/builder-agent.md",
        "gateway/templates/agents/planner-agent.md",
        "runtime/AGENTS.md",
    ]
    for rel in prompt_paths:
        assert_absent(rel, retired_tools)

    assert_absent("runtime/agent-tools/src/lib.rs", retired_tools)
    assert_absent("runtime/agent-tools/src/tools/mod.rs", retired_tools + ["introspection"])
    assert_absent("runtime/agent-tools/src/tools/execution/mod.rs", retired_tools)
    assert_absent("runtime/agent-tools/src/tools/execution/skills.rs", ["list_skills"])
    assert_absent("runtime/agent-tools/src/tools/memory.rs", ["memory_facts_index", "vec0"])

    assert_missing("runtime/agent-tools/src/tools/introspection.rs")
    assert_missing("runtime/agent-tools/src/tools/execution/session_title.rs")
    assert_missing("runtime/agent-tools/src/tools/execution/todos.rs")

    assert_absent(
        "gateway/gateway-execution/src/invoke/executor.rs",
        [
            "ListSkillsTool",
            "ListMcpsTool",
            "ListToolsTool",
            "SetSessionTitleTool",
            "TodoTool",
            "ToolCapability::SkillList",
            "ToolCapability::McpList",
            "ToolCapability::SessionTitleWrite",
        ],
        before_tests=True,
    )
    assert_absent(
        "gateway/src/state/mod.rs",
        retired_sqlite_runtime,
        before_tests=True,
    )
    assert_absent("gateway/src/http/embeddings.rs", retired_sqlite_runtime)
    assert_missing("gateway/gateway-execution/src/sleep/embedding_reindex.rs")
    assert_missing("gateway/gateway-execution/src/sleep/kg_backfill.rs")

    print("context capability cleanup checks passed")


if __name__ == "__main__":
    main()
