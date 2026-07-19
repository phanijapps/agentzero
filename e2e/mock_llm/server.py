"""FastAPI app speaking /v1/chat/completions, replaying recorded fixtures.

The real gateway identifies which execution is making an LLM request via
a synthetic field `zbot_execution_id` in the POST body. (When the real
gateway makes the call, our adapter injects that field. For unit tests
we set it directly.)
"""
import hashlib
import json
from pathlib import Path

from fastapi import FastAPI, Request
from fastapi.responses import JSONResponse, StreamingResponse

from e2e.mock_llm.replay import ReplayStore


class AppState:
    def __init__(self, store: ReplayStore) -> None:
        self.store = store


def _messages_hash(messages: list[dict]) -> str:
    payload = json.dumps(messages, sort_keys=True)
    return "sha256:" + hashlib.sha256(payload.encode()).hexdigest()


def _is_intent_analysis_request(messages: list[dict]) -> bool:
    """Heuristic: zerod's intent-analysis middleware ships a dedicated
    system prompt whose first line is 'You are an intent analyzer'.
    Match the prefix only — the main agent's much larger system prompt
    mentions 'intent analysis' but never begins with that sentence.
    """
    if not messages or messages[0].get("role") != "system":
        return False
    content = str(messages[0].get("content", "")).lstrip().lower()
    return content.startswith("you are an intent analyzer")


def _is_handoff_summary_request(messages: list[dict]) -> bool:
    """Heuristic for the sleep handoff summarizer.

    Full-mode fixtures record the agent turn itself. Handoff summarization is
    post-turn bookkeeping, so return a stable stub instead of consuming replay
    records and creating drift.
    """
    if not messages or messages[0].get("role") != "system":
        return False
    content = str(messages[0].get("content", "")).lstrip().lower()
    return content.startswith("you are a concise session summarizer")


def _stream_chunk(completion_id: str, delta: dict, finish_reason=None) -> str:
    """Render one SSE `data:` line holding an OpenAI streaming chunk."""
    payload = {
        "id": completion_id,
        "object": "chat.completion.chunk",
        "choices": [{"index": 0, "delta": delta, "finish_reason": finish_reason}],
    }
    return f"data: {json.dumps(payload)}\n\n"


async def _stream_response(full: dict):
    """Convert a non-streaming OpenAI chat.completion into SSE chunks."""
    completion_id = full.get("id", "chatcmpl-stream")
    choice = full.get("choices", [{}])[0]
    message = choice.get("message", {})
    finish_reason = choice.get("finish_reason")

    # Chunk 1: role announcement.
    yield _stream_chunk(completion_id, {"role": "assistant"})

    # Content streaming (if any non-empty content).
    content = message.get("content") or ""
    if content:
        yield _stream_chunk(completion_id, {"content": content})

    # Tool-call streaming: emit each as a complete delta so zerod's
    # accumulator has everything it needs in one chunk per call.
    tool_calls = message.get("tool_calls") or []
    for i, tc in enumerate(tool_calls):
        fn = tc.get("function", {})
        yield _stream_chunk(completion_id, {
            "tool_calls": [{
                "index": i,
                "id": tc.get("id") or f"call_{i}",
                "type": "function",
                "function": {
                    "name": fn.get("name", ""),
                    "arguments": fn.get("arguments", "{}"),
                },
            }],
        })

    # Final chunk with finish_reason.
    yield _stream_chunk(completion_id, {}, finish_reason=finish_reason)
    yield "data: [DONE]\n\n"


def _empty_intent_response() -> dict:
    """Minimal OpenAI-shaped response matching zerod's intent schema.
    The middleware tolerates parse failures as non-fatal, but returning
    the full schema keeps logs clean and avoids any downstream branches
    that might depend on intent fields being present.
    """
    intent_json = json.dumps({
        "primary_intent": "research",
        "hidden_intents": [],
        "recommended_skills": [],
        "recommended_agents": ["root"],
        "ward_recommendation": {
            "action": "create_new",
            "ward_name": "e2e-mock",
            "subdirectory": None,
            "reason": "e2e mock-llm stub",
        },
        "execution_strategy": {
            "approach": "simple",
            "explanation": "single-turn Q+A",
        },
    })
    return {
        "id": "chatcmpl-intent-stub",
        "object": "chat.completion",
        "choices": [
            {
                "index": 0,
                "finish_reason": "stop",
                "message": {"role": "assistant", "content": intent_json},
            }
        ],
    }


def _handoff_summary_response() -> dict:
    return {
        "id": "chatcmpl-handoff-stub",
        "object": "chat.completion",
        "choices": [
            {
                "index": 0,
                "finish_reason": "stop",
                "message": {
                    "role": "assistant",
                    "content": "The session answered the user's simple arithmetic question with 4.",
                },
            }
        ],
    }


def create_app(
    fixture_dir: Path, *, strict_hashing: bool = False
) -> FastAPI:
    app = FastAPI()
    state = AppState(store=ReplayStore.from_fixture(
        fixture_dir, strict_hashing=strict_hashing
    ))
    app.state.replay = state

    @app.get("/health")
    def health() -> dict:
        return {"status": "ok", "fixture": str(fixture_dir)}

    @app.get("/v1/models")
    def list_models() -> dict:
        return {
            "object": "list",
            "data": [
                {"id": "gpt-4", "object": "model"},
                {"id": "gpt-4o-mini", "object": "model"},
                {"id": "glm-4-plus", "object": "model"},
            ],
        }

    @app.post("/v1/chat/completions")
    async def chat_completions(request: Request):
        body = await request.json()
        messages = body.get("messages", [])
        streaming = bool(body.get("stream"))
        exec_id = body.get("zbot_execution_id") or request.headers.get(
            "x-zbot-execution-id"
        )
        store = app.state.replay.store
        if _is_intent_analysis_request(messages):
            # Bypass the fixture: intent-analysis is a preamble middleware
            # the fixture doesn't record. Serve an empty intent so zerod
            # falls through to the real agent turn.
            response = _empty_intent_response()
            if streaming:
                return StreamingResponse(
                    _stream_response(response), media_type="text/event-stream",
                )
            return JSONResponse(response)
        if _is_handoff_summary_request(messages):
            response = _handoff_summary_response()
            if streaming:
                return StreamingResponse(
                    _stream_response(response), media_type="text/event-stream",
                )
            return JSONResponse(response)
        if exec_id:
            result = store.next_response(
                exec_id=exec_id, messages_hash=_messages_hash(messages),
            )
        else:
            # Mode Full: real zerod sends plain OpenAI bodies. Fall back to
            # FIFO record order across all executions in fixture order.
            exec_id = "<fifo>"
            result = store.next_any()
        if result.ok:
            if streaming:
                return StreamingResponse(
                    _stream_response(result.response),
                    media_type="text/event-stream",
                )
            return JSONResponse(result.response)
        if result.reason == "exhausted":
            return JSONResponse(
                {"error": "fixture exhausted", "exec_id": exec_id},
                status_code=410,
            )
        if result.reason == "hash_mismatch":
            return JSONResponse(
                {
                    "error": "drift",
                    "expected_hash": result.expected_hash,
                    "received_hash": result.received_hash,
                    "exec_id": exec_id,
                },
                status_code=409,
            )
        return JSONResponse(
            {"error": result.reason, "exec_id": exec_id}, status_code=500,
        )

    @app.get("/__replay/status")
    def replay_status() -> dict:
        return app.state.replay.store.status()

    return app
