"""Generate schema-valid synthetic fixtures for early harness work.

Real fixtures come from record-fixture.py (Task 21). These synthetic ones
let the harness run end-to-end before any live session has been captured.
"""
import hashlib
import json
from pathlib import Path
from typing import Iterable

from pydantic import BaseModel

from e2e.fixtures.types import (
    Execution, LLMResponseRecord, SessionFixture,
    ToolResultRecord, WSEventRecord,
)

SIMPLE_QA_SESSION_ID = "sess-synthetic-simple-qa-0000"
SIMPLE_QA_EXEC_ROOT = "exec-synthetic-root-0000"
SIMPLE_QA_PROMPT = "what is 2+2? one-line answer"
SIMPLE_QA_ANSWER = "4"
SKILLS_MCP_SESSION_ID = "sess-synthetic-skills-mcp-code-research-0000"
SKILLS_MCP_EXEC_ROOT = "exec-synthetic-skills-mcp-root-0000"


def _hash(obj: object) -> str:
    return "sha256:" + hashlib.sha256(
        json.dumps(obj, sort_keys=True).encode()
    ).hexdigest()


def _write_jsonl(path: Path, records: Iterable[BaseModel | dict]) -> None:
    with path.open("w") as f:
        for r in records:
            payload = r.model_dump() if isinstance(r, BaseModel) else r
            f.write(json.dumps(payload) + "\n")


def build_simple_qa_fixture(out_dir: Path) -> None:
    """Root-only scenario: user → agent_started → respond → agent_completed."""
    out_dir.mkdir(parents=True, exist_ok=True)

    session = SessionFixture(
        session_id=SIMPLE_QA_SESSION_ID,
        title="Simple Q+A",
        executions=[
            Execution(
                execution_id=SIMPLE_QA_EXEC_ROOT,
                agent_id="root",
                parent_execution_id=None,
                started_at_offset_ms=0,
                ended_at_offset_ms=1500,
            )
        ],
        artifacts=[],
    )
    (out_dir / "session.json").write_text(session.model_dump_json(indent=2))

    respond_args = {"message": SIMPLE_QA_ANSWER}
    llm_response = {
        "id": "chatcmpl-synthetic",
        "object": "chat.completion",
        "choices": [
            {
                "index": 0,
                "finish_reason": "tool_calls",
                "message": {
                    "role": "assistant",
                    "content": "",
                    "tool_calls": [
                        {
                            "id": "call_1",
                            "type": "function",
                            "function": {
                                "name": "respond",
                                "arguments": json.dumps(respond_args),
                            },
                        }
                    ],
                },
            }
        ],
    }
    llm_records = [
        LLMResponseRecord(
            execution_id=SIMPLE_QA_EXEC_ROOT,
            iteration=0,
            messages_hash=None,
            response=llm_response,
        )
    ]
    _write_jsonl(out_dir / "llm-responses.jsonl", llm_records)

    tool_records = [
        ToolResultRecord(
            execution_id=SIMPLE_QA_EXEC_ROOT,
            tool_index=0,
            tool_name="respond",
            args_hash=_hash(respond_args),
            result=json.dumps({"ok": True}),
        )
    ]
    _write_jsonl(out_dir / "tool-results.jsonl", tool_records)

    ws_events = [
        WSEventRecord(t_offset_ms=0, type="invoke_accepted",
                      payload={"session_id": SIMPLE_QA_SESSION_ID,
                               "conversation_id": "conv-synth"}),
        WSEventRecord(t_offset_ms=50, type="agent_started",
                      payload={"session_id": SIMPLE_QA_SESSION_ID,
                               "execution_id": SIMPLE_QA_EXEC_ROOT,
                               "agent_id": "root"}),
        WSEventRecord(t_offset_ms=200, type="thinking",
                      payload={"session_id": SIMPLE_QA_SESSION_ID,
                               "execution_id": SIMPLE_QA_EXEC_ROOT,
                               "content": "simple arithmetic"}),
        WSEventRecord(t_offset_ms=400, type="tool_call",
                      payload={"session_id": SIMPLE_QA_SESSION_ID,
                               "execution_id": SIMPLE_QA_EXEC_ROOT,
                               "tool_name": "respond",
                               "tool_id": "call_1",
                               "args": respond_args}),
        WSEventRecord(t_offset_ms=450, type="tool_result",
                      payload={"session_id": SIMPLE_QA_SESSION_ID,
                               "execution_id": SIMPLE_QA_EXEC_ROOT,
                               "tool_id": "call_1",
                               "result": json.dumps({"ok": True})}),
        WSEventRecord(t_offset_ms=500, type="turn_complete",
                      payload={"session_id": SIMPLE_QA_SESSION_ID,
                               "execution_id": SIMPLE_QA_EXEC_ROOT,
                               "final_message": ""}),
        WSEventRecord(t_offset_ms=510, type="agent_completed",
                      payload={"session_id": SIMPLE_QA_SESSION_ID,
                               "execution_id": SIMPLE_QA_EXEC_ROOT,
                               "agent_id": "root"}),
    ]
    _write_jsonl(out_dir / "ws-events.jsonl", ws_events)


def build_skills_mcp_code_research_fixture(out_dir: Path) -> None:
    """Scenario for context-capability migration journey coverage."""
    out_dir.mkdir(parents=True, exist_ok=True)

    session = SessionFixture(
        session_id=SKILLS_MCP_SESSION_ID,
        title="Skills MCP Code Research",
        executions=[
            Execution(
                execution_id=SKILLS_MCP_EXEC_ROOT,
                agent_id="root",
                parent_execution_id=None,
                started_at_offset_ms=0,
                ended_at_offset_ms=4200,
            )
        ],
        artifacts=[],
    )
    (out_dir / "session.json").write_text(session.model_dump_json(indent=2))

    calls = [
        ("load_skill", {
            "skill": "coding",
        }, {
            "name": "coding",
            "packet": {
                "summary": "Code analysis workflow",
                "sections": [
                    {
                        "title": "Search",
                        "summary": "Use grep and focused reads before edits.",
                        "resource_uri": "zbot://skills/coding/sections/search",
                        "token_estimate": 16,
                    }
                ],
                "resource_uri": "zbot://skills/coding/SKILL.md",
                "full_body_resource_uri": "zbot://skills/coding/sections/full",
                "token_estimate": 64,
                "render_policy": "summary",
            },
        }),
        ("query_resource", {
            "uri": "mcp://docs/search?query=rust+structured+output",
        }, {
            "resource_uri": "mcp://docs/search?query=rust+structured+output",
            "summary": "MCP docs resource result for structured output research.",
            "metadata": {
                "catalog_source": "context_capability_catalog",
                "raw_mcp_discovery_tool_used": False,
            },
        }),
        ("grep", {
            "pattern": "ContextPacket",
            "path": "gateway",
        }, {
            "matches": [
                {
                    "path": "gateway/gateway-execution/src/recall/mod.rs",
                    "line": 42,
                    "text": "ContextPacket builder",
                }
            ],
        }),
        ("shell", {
            "cmd": "cargo test -p gateway-execution recall --locked",
        }, {
            "exit_code": 0,
            "stdout": "recall packet tests passed",
            "stderr": "",
        }),
        ("respond", {
            "message": "Completed skill-guided MCP research and code analysis with context packets.",
        }, {
            "ok": True,
        }),
    ]

    llm_records = []
    tool_records = []
    ws_events = [
        WSEventRecord(t_offset_ms=0, type="invoke_accepted",
                      payload={"session_id": SKILLS_MCP_SESSION_ID,
                               "conversation_id": "conv-skills-mcp"}),
        WSEventRecord(t_offset_ms=50, type="session_title_changed",
                      payload={"session_id": SKILLS_MCP_SESSION_ID,
                               "title": "Skills MCP Code Research"}),
        WSEventRecord(t_offset_ms=100, type="agent_started",
                      payload={"session_id": SKILLS_MCP_SESSION_ID,
                               "execution_id": SKILLS_MCP_EXEC_ROOT,
                               "agent_id": "root"}),
        WSEventRecord(t_offset_ms=150, type="context_packet",
                      payload={"session_id": SKILLS_MCP_SESSION_ID,
                               "selected_count": 3,
                               "dropped_count": 1,
                               "source_mix": {"memory": 1, "resource": 1, "graph": 1}}),
    ]

    for idx, (tool_name, args, result) in enumerate(calls):
        call_id = f"call_{idx + 1}"
        llm_records.append(LLMResponseRecord(
            execution_id=SKILLS_MCP_EXEC_ROOT,
            iteration=idx,
            messages_hash=None,
            response={
                "id": f"chatcmpl-{SKILLS_MCP_EXEC_ROOT}-{idx}",
                "object": "chat.completion",
                "choices": [{
                    "index": 0,
                    "finish_reason": "tool_calls",
                    "message": {
                        "role": "assistant",
                        "content": "",
                        "tool_calls": [{
                            "id": call_id,
                            "type": "function",
                            "function": {
                                "name": tool_name,
                                "arguments": json.dumps(args),
                            },
                        }],
                    },
                }],
            },
        ))
        tool_records.append(ToolResultRecord(
            execution_id=SKILLS_MCP_EXEC_ROOT,
            tool_index=idx,
            tool_name=tool_name,
            args_hash=_hash(args),
            result=json.dumps(result),
        ))
        ws_events.append(WSEventRecord(
            t_offset_ms=250 + idx * 500,
            type="tool_call",
            payload={"session_id": SKILLS_MCP_SESSION_ID,
                     "execution_id": SKILLS_MCP_EXEC_ROOT,
                     "tool_name": tool_name,
                     "tool_id": call_id,
                     "args": args},
        ))
        ws_events.append(WSEventRecord(
            t_offset_ms=300 + idx * 500,
            type="tool_result",
            payload={"session_id": SKILLS_MCP_SESSION_ID,
                     "execution_id": SKILLS_MCP_EXEC_ROOT,
                     "tool_id": call_id,
                     "result": json.dumps(result)},
        ))

    ws_events.extend([
        WSEventRecord(t_offset_ms=4000, type="turn_complete",
                      payload={"session_id": SKILLS_MCP_SESSION_ID,
                               "execution_id": SKILLS_MCP_EXEC_ROOT,
                               "final_message": "Completed skill-guided MCP research and code analysis with context packets."}),
        WSEventRecord(t_offset_ms=4100, type="agent_completed",
                      payload={"session_id": SKILLS_MCP_SESSION_ID,
                               "execution_id": SKILLS_MCP_EXEC_ROOT,
                               "agent_id": "root"}),
    ])

    _write_jsonl(out_dir / "llm-responses.jsonl", llm_records)
    _write_jsonl(out_dir / "tool-results.jsonl", tool_records)
    _write_jsonl(out_dir / "ws-events.jsonl", ws_events)


if __name__ == "__main__":
    here = Path(__file__).parent
    build_simple_qa_fixture(here / "simple-qa")
    build_skills_mcp_code_research_fixture(here / "skills-mcp-code-research")
    print(f"Wrote synthetic simple-qa fixture to {here / 'simple-qa'}")
    print(
        "Wrote synthetic skills-mcp-code-research fixture to "
        f"{here / 'skills-mcp-code-research'}"
    )
