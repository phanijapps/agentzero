"""Synthetic fixture generator emits schema-valid bundles."""
import json
from pathlib import Path

from e2e.fixtures.seed_synthetic import (
    build_simple_qa_fixture,
    build_skills_mcp_code_research_fixture,
)
from e2e.fixtures.types import SessionFixture, ToolResultRecord, WSEventRecord


def test_simple_qa_emits_four_files(tmp_path: Path):
    out = tmp_path / "simple-qa"
    build_simple_qa_fixture(out)
    assert (out / "session.json").exists()
    assert (out / "llm-responses.jsonl").exists()
    assert (out / "tool-results.jsonl").exists()
    assert (out / "ws-events.jsonl").exists()


def test_simple_qa_session_json_validates(tmp_path: Path):
    out = tmp_path / "simple-qa"
    build_simple_qa_fixture(out)
    raw = json.loads((out / "session.json").read_text())
    fixture = SessionFixture(**raw)
    assert fixture.session_id.startswith("sess-")
    assert len(fixture.executions) == 1
    assert fixture.executions[0].agent_id == "root"


def test_simple_qa_ws_events_include_invoke_and_respond(tmp_path: Path):
    out = tmp_path / "simple-qa"
    build_simple_qa_fixture(out)
    events = [
        WSEventRecord(**json.loads(line))
        for line in (out / "ws-events.jsonl").read_text().splitlines()
        if line.strip()
    ]
    types = [e.type for e in events]
    assert "invoke_accepted" in types
    assert "agent_started" in types
    assert "agent_completed" in types


def test_skills_mcp_code_research_fixture_uses_context_capability_path(tmp_path: Path):
    out = tmp_path / "skills-mcp-code-research"
    build_skills_mcp_code_research_fixture(out)

    raw = json.loads((out / "session.json").read_text())
    fixture = SessionFixture(**raw)
    assert fixture.title == "Skills MCP Code Research"

    tool_records = [
        ToolResultRecord(**json.loads(line))
        for line in (out / "tool-results.jsonl").read_text().splitlines()
        if line.strip()
    ]
    tool_names = [record.tool_name for record in tool_records]
    assert tool_names == ["load_skill", "query_resource", "grep", "shell", "respond"]
    assert "list_skills" not in tool_names
    assert "list_mcps" not in tool_names
    assert "set_session_title" not in tool_names

    skill_result = json.loads(tool_records[0].result)
    assert "packet" in skill_result
    assert "instructions" not in skill_result
    assert skill_result["packet"]["resource_uri"].startswith("zbot://skills/")

    mcp_result = json.loads(tool_records[1].result)
    assert mcp_result["resource_uri"].startswith("mcp://")
    assert mcp_result["metadata"]["raw_mcp_discovery_tool_used"] is False

    events = [
        WSEventRecord(**json.loads(line))
        for line in (out / "ws-events.jsonl").read_text().splitlines()
        if line.strip()
    ]
    assert any(event.type == "context_packet" for event in events)
