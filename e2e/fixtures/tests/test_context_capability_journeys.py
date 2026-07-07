"""Journey fixture assertions for the context capability migration."""
import json
from collections import Counter
from pathlib import Path

from e2e.fixtures.types import ToolResultRecord, WSEventRecord


FIXTURES = Path(__file__).resolve().parents[1]


def _tool_records(name: str) -> list[ToolResultRecord]:
    path = FIXTURES / name / "tool-results.jsonl"
    return [
        ToolResultRecord(**json.loads(line))
        for line in path.read_text().splitlines()
        if line.strip()
    ]


def _ws_events(name: str) -> list[WSEventRecord]:
    path = FIXTURES / name / "ws-events.jsonl"
    return [
        WSEventRecord(**json.loads(line))
        for line in path.read_text().splitlines()
        if line.strip()
    ]


def test_aapl_fixture_records_baseline_counts_for_parity():
    counts = Counter(record.tool_name for record in _tool_records("aapl-peer-valuation"))

    assert counts["shell"] >= 1
    assert counts["memory"] >= 1
    assert counts["set_session_title"] == 1
    assert counts["load_skill"] == 0
    assert counts["list_mcps"] == 0
    assert counts["list_skills"] == 0


def test_synthetic_mcp_research_fixture_uses_target_surface():
    counts = Counter(record.tool_name for record in _tool_records("skills-mcp-code-research"))

    assert counts == Counter({
        "load_skill": 1,
        "query_resource": 1,
        "grep": 1,
        "shell": 1,
        "respond": 1,
    })
    events = _ws_events("skills-mcp-code-research")
    packet_events = [event for event in events if event.type == "context_packet"]
    assert packet_events
    assert packet_events[0].payload["selected_count"] == 3
