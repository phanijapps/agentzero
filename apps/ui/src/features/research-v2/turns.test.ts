// =============================================================================
// turns — pure-function turn builder tests
//
// Mirrors the rollup algorithm spec in
// memory-bank/future-state/2026-05-05-research-multi-turn-design.md.
// =============================================================================

import { describe, it, expect } from "vitest";
import type {
  LogSession,
  SessionMessage,
  SessionStatus,
} from "@/services/transport/types";
import {
  bucketSubagents,
  buildSessionTurns,
  extractAssistantReplyForTurn,
  extractDelegationTasksInWindow,
  extractToolActivityForTurn,
  findTurnBoundaries,
  type TurnBoundary,
} from "./turns";

// -----------------------------------------------------------------------------
// Fixture factories
// -----------------------------------------------------------------------------

function userMsg(id: string, createdAt: string, content = `user ${id}`): SessionMessage {
  return {
    id,
    execution_id: "root-exec",
    agent_id: "root",
    delegation_type: "root",
    role: "user",
    content,
    created_at: createdAt,
  };
}

function asstMsg(
  id: string,
  createdAt: string,
  content: string,
  toolCalls?: unknown,
): SessionMessage {
  return {
    id,
    execution_id: "root-exec",
    agent_id: "root",
    delegation_type: "root",
    role: "assistant",
    content,
    created_at: createdAt,
    tool_calls: toolCalls,
  };
}

function child(
  sessionId: string,
  startedAt: string,
  endedAt: string,
  status: SessionStatus = "completed",
): LogSession {
  return {
    session_id: sessionId,
    conversation_id: "sess-X",
    agent_id: "builder-agent",
    agent_name: "builder-agent",
    parent_session_id: "root-exec",
    started_at: startedAt,
    ended_at: endedAt,
    status,
    token_count: 0,
    tool_call_count: 0,
    error_count: 0,
    child_session_ids: [],
    title: "",
  } as unknown as LogSession;
}

// -----------------------------------------------------------------------------
// findTurnBoundaries
// -----------------------------------------------------------------------------

describe("findTurnBoundaries", () => {
  it("returns one boundary per user message in chronological order", () => {
    const msgs = [
      userMsg("u1", "2026-05-03T13:05:34Z"),
      asstMsg("a1", "2026-05-03T13:12:50Z", "first reply"),
      userMsg("u2", "2026-05-05T13:11:30Z"),
    ];
    const boundaries = findTurnBoundaries(msgs, null);
    expect(boundaries).toHaveLength(2);
    expect(boundaries[0].userMessage.id).toBe("u1");
    expect(boundaries[0].startedAt).toBe("2026-05-03T13:05:34Z");
    expect(boundaries[0].endedAt).toBe("2026-05-05T13:11:30Z");
    expect(boundaries[1].userMessage.id).toBe("u2");
    expect(boundaries[1].endedAt).toBeNull(); // last turn open
  });

  it("leaves the last turn's endedAt null even on a completed session — trailing respond() messages may land microseconds past root.ended_at", () => {
    const msgs = [userMsg("u1", "2026-05-03T13:05:34Z")];
    const boundaries = findTurnBoundaries(msgs, "2026-05-03T13:12:50Z");
    expect(boundaries[0].endedAt).toBeNull();
  });

  it("returns empty when no user messages yet", () => {
    expect(findTurnBoundaries([], null)).toEqual([]);
    expect(
      findTurnBoundaries([asstMsg("a1", "2026-05-03T13:00:00Z", "x")], null),
    ).toEqual([]);
  });

  it("sorts by created_at even if input is reversed", () => {
    const msgs = [
      userMsg("u2", "2026-05-05T13:11:30Z"),
      userMsg("u1", "2026-05-03T13:05:34Z"),
    ];
    const boundaries = findTurnBoundaries(msgs, null);
    expect(boundaries.map((b) => b.userMessage.id)).toEqual(["u1", "u2"]);
  });

  it("turns persisted attachment metadata into safe display data", () => {
    const [boundary] = findTurnBoundaries([
      userMsg(
        "u1",
        "2026-05-03T13:05:34Z",
        "Analyze this\n\n**Attached files:**\n| File | Type | Size | Path |\n|------|------|------|------|\n| interview.txt | text/plain | 44.7 KB | /private/transcript.txt |",
      ),
    ], null);

    expect(boundary.userMessage.content).toBe("Analyze this");
    expect(boundary.userMessage.attachments).toEqual([
      { name: "interview.txt", mimeType: "text/plain", sizeLabel: "44.7 KB" },
    ]);
  });
});

// -----------------------------------------------------------------------------
// bucketSubagents
// -----------------------------------------------------------------------------

describe("bucketSubagents", () => {
  const boundaries: TurnBoundary[] = [
    {
      userMessage: { id: "u1", content: "", createdAt: "2026-05-03T13:05:34Z" },
      startedAt: "2026-05-03T13:05:34Z",
      endedAt: "2026-05-05T13:11:30Z",
    },
    {
      userMessage: { id: "u2", content: "", createdAt: "2026-05-05T13:11:30Z" },
      startedAt: "2026-05-05T13:11:30Z",
      endedAt: null,
    },
  ];

  it("buckets each child into the turn whose [startedAt, endedAt) contains its started_at", () => {
    const childA = child("c-A", "2026-05-03T13:05:49Z", "2026-05-03T13:08:39Z"); // turn 1
    const childB = child("c-B", "2026-05-05T13:11:35Z", "2026-05-05T13:14:58Z"); // turn 2
    const buckets = bucketSubagents(boundaries, [childB, childA]);
    expect(buckets.get(0)?.map((c) => c.session_id)).toEqual(["c-A"]);
    expect(buckets.get(1)?.map((c) => c.session_id)).toEqual(["c-B"]);
  });

  it("subagent at exact boundary belongs to the new turn (half-open >= start, < end)", () => {
    const onBoundary = child(
      "c",
      "2026-05-05T13:11:30Z",
      "2026-05-05T13:11:35Z",
    );
    const buckets = bucketSubagents(boundaries, [onBoundary]);
    expect(buckets.get(0) ?? []).toEqual([]);
    expect(buckets.get(1)?.map((c) => c.session_id)).toEqual(["c"]);
  });

  it("subagent before any user message is dropped (defensive)", () => {
    const earlyChild = child(
      "c",
      "2026-05-03T13:00:00Z",
      "2026-05-03T13:00:30Z",
    );
    const buckets = bucketSubagents(boundaries, [earlyChild]);
    expect(buckets.size).toBe(0);
  });

  it("last open turn (endedAt=null) accepts arbitrarily late children", () => {
    const lateChild = child("c", "2099-01-01T00:00:00Z", "2099-01-01T01:00:00Z");
    const buckets = bucketSubagents(boundaries, [lateChild]);
    expect(buckets.get(1)?.map((c) => c.session_id)).toEqual(["c"]);
  });
});

// -----------------------------------------------------------------------------
// extractAssistantReplyForTurn
// -----------------------------------------------------------------------------

describe("extractAssistantReplyForTurn", () => {
  it("returns the latest plain-text assistant message in the window", () => {
    const win = [
      asstMsg("a1", "2026-05-03T13:06:00Z", "intermediate"),
      asstMsg("a2", "2026-05-03T13:12:50Z", "final answer"),
    ];
    expect(extractAssistantReplyForTurn(win)).toBe("final answer");
  });

  it("falls back to respond() tool call when no plain text exists", () => {
    const win = [
      asstMsg(
        "a1",
        "2026-05-03T13:06:00Z",
        "[tool calls]",
        JSON.stringify([{ tool_name: "respond", args: { message: "via respond" } }]),
      ),
    ];
    expect(extractAssistantReplyForTurn(win)).toBe("via respond");
  });

  it("prefers plain text over respond() when both exist", () => {
    const win = [
      asstMsg(
        "a1",
        "2026-05-03T13:06:00Z",
        "[tool calls]",
        JSON.stringify([{ tool_name: "respond", args: { message: "via respond" } }]),
      ),
      asstMsg("a2", "2026-05-03T13:12:50Z", "plain text reply"),
    ];
    expect(extractAssistantReplyForTurn(win)).toBe("plain text reply");
  });

  it("uses a later respond() result over earlier progress text", () => {
    const win = [
      asstMsg("a1", "2026-05-03T13:06:00Z", "I'll research that now."),
      asstMsg(
        "a2",
        "2026-05-03T13:12:50Z",
        "[tool calls]",
        JSON.stringify([{ tool_name: "respond", args: { message: "final report" } }]),
      ),
    ];
    expect(extractAssistantReplyForTurn(win)).toBe("final report");
  });

  it("returns null on an empty window", () => {
    expect(extractAssistantReplyForTurn([])).toBeNull();
  });

  it("ignores the [tool calls] placeholder when no respond() call accompanies it", () => {
    const win = [asstMsg("a1", "2026-05-03T13:06:00Z", "[tool calls]")];
    expect(extractAssistantReplyForTurn(win)).toBeNull();
  });
});

// -----------------------------------------------------------------------------
// extractDelegationTasksInWindow
// -----------------------------------------------------------------------------

describe("extractDelegationTasksInWindow", () => {
  it("returns delegation task strings in chronological order", () => {
    const win = [
      asstMsg(
        "a1",
        "2026-05-03T13:05:39Z",
        "[tool calls]",
        JSON.stringify([
          { tool_name: "delegate_to_agent", args: { task: "extract docx" } },
        ]),
      ),
      asstMsg(
        "a2",
        "2026-05-03T13:08:48Z",
        "[tool calls]",
        JSON.stringify([
          { tool_name: "delegate_to_agent", args: { task: "fetch web sources" } },
        ]),
      ),
    ];
    expect(extractDelegationTasksInWindow(win)).toEqual([
      "extract docx",
      "fetch web sources",
    ]);
  });

  it("ignores non-delegation tool calls", () => {
    const win = [
      asstMsg(
        "a1",
        "2026-05-03T13:05:39Z",
        "[tool calls]",
        JSON.stringify([
          { tool_name: "shell", args: { command: "ls" } },
          { tool_name: "delegate_to_agent", args: { task: "build" } },
        ]),
      ),
    ];
    expect(extractDelegationTasksInWindow(win)).toEqual(["build"]);
  });
});

// -----------------------------------------------------------------------------
// extractToolActivityForTurn
// -----------------------------------------------------------------------------

describe("extractToolActivityForTurn", () => {
  it("retains chronological non-response tool names without arguments", () => {
    const entries = extractToolActivityForTurn([
      asstMsg(
        "a1",
        "2026-05-03T13:05:39Z",
        "[tool calls]",
        JSON.stringify([
          { tool_name: "recall", args: { query: "private query" } },
          { tool_name: "respond", args: { message: "answer" } },
        ]),
      ),
      asstMsg(
        "a2",
        "2026-05-03T13:05:40Z",
        "[tool calls]",
        JSON.stringify([{ tool_name: "write_file", args: { path: "/home/private" } }]),
      ),
    ]);

    expect(entries.map((entry) => entry.toolName)).toEqual(["recall", "write_file"]);
    expect(entries.every((entry) => entry.toolArgsPreview === undefined)).toBe(true);
  });
});

// -----------------------------------------------------------------------------
// buildSessionTurns end-to-end
// -----------------------------------------------------------------------------

describe("buildSessionTurns", () => {
  it("recreates the recorded sess-9f40dc55 shape (2 turns, 4 subagents)", () => {
    const delegate = (task: string) =>
      JSON.stringify([{ tool_name: "delegate_to_agent", args: { task } }]);
    const rootMessages: SessionMessage[] = [
      userMsg("u1", "2026-05-03T13:05:34Z", "help me"),
      asstMsg("d1", "2026-05-03T13:05:48Z", "[tool calls]", delegate("extract docx")),
      asstMsg("d2", "2026-05-03T13:08:47Z", "[tool calls]", delegate("fetch sources")),
      asstMsg("d3", "2026-05-03T13:10:59Z", "[tool calls]", delegate("write answers")),
      asstMsg("a1", "2026-05-03T13:12:50Z", "The assignment is fully complete!"),
      userMsg("u2", "2026-05-05T13:11:30Z", "can you make it into a presentation"),
      asstMsg("d4", "2026-05-05T13:11:34Z", "[tool calls]", delegate("build slides")),
      asstMsg(
        "a2",
        "2026-05-05T13:14:58Z",
        "Done! I've created a 22-slide HTML presentation",
      ),
    ];
    const childRows: LogSession[] = [
      child("c1", "2026-05-03T13:05:49Z", "2026-05-03T13:08:39Z"),
      child("c2", "2026-05-03T13:08:48Z", "2026-05-03T13:10:55Z"),
      child("c3", "2026-05-03T13:11:00Z", "2026-05-03T13:12:42Z"),
      child("c4", "2026-05-05T13:11:35Z", "2026-05-05T13:14:58Z"),
    ];
    const turns = buildSessionTurns({
      rootSessionId: "root-exec",
      rootEndedAt: "2026-05-05T13:15:02Z",
      rootStatus: "completed",
      rootMessages,
      childRows,
    });
    expect(turns).toHaveLength(2);
    expect(turns[0].userMessage.content).toBe("help me");
    expect(turns[0].subagents.map((s) => s.id)).toEqual(["c1", "c2", "c3"]);
    expect(turns[0].subagents.map((s) => s.request)).toEqual([
      "extract docx",
      "fetch sources",
      "write answers",
    ]);
    expect(turns[0].assistantText).toBe("The assignment is fully complete!");
    expect(turns[0].timeline.map((entry) => entry.toolName)).toEqual([
      "delegate_to_agent",
      "delegate_to_agent",
      "delegate_to_agent",
    ]);
    expect(turns[0].status).toBe("completed");
    expect(turns[0].index).toBe(0);
    expect(turns[1].userMessage.content).toBe("can you make it into a presentation");
    expect(turns[1].subagents.map((s) => s.id)).toEqual(["c4"]);
    expect(turns[1].subagents.map((s) => s.request)).toEqual(["build slides"]);
    expect(turns[1].assistantText).toBe("Done! I've created a 22-slide HTML presentation");
    expect(turns[1].status).toBe("completed");
    expect(turns[1].index).toBe(1);
  });

  it("last turn has status='running' when the root execution is still active and no reply yet", () => {
    const turns = buildSessionTurns({
      rootSessionId: "root-exec",
      rootEndedAt: null,
      rootStatus: "running",
      rootMessages: [userMsg("u1", "2026-05-05T13:00:00Z")],
      childRows: [],
    });
    expect(turns[0].status).toBe("running");
    expect(turns[0].assistantText).toBeNull();
    expect(turns[0].endedAt).toBeNull();
    expect(turns[0].durationMs).toBeNull();
  });

  it("derives last-turn durationMs from rootEndedAt when the boundary edge is null", () => {
    const turns = buildSessionTurns({
      rootSessionId: "root-exec",
      rootEndedAt: "2026-05-03T13:06:00Z",
      rootStatus: "completed",
      rootMessages: [
        userMsg("u1", "2026-05-03T13:05:00Z"),
        asstMsg("a1", "2026-05-03T13:05:30Z", "ok"),
      ],
      childRows: [],
    });
    expect(turns[0].endedAt).toBeNull();
    expect(turns[0].durationMs).toBe(60_000);
  });

  it("captures a respond() that lands microseconds past root.ended_at (regression: turn 2 stuck on waiting…)", () => {
    // Real Pi data: root.ended_at = 13:15:02.838, but the respond()
    // tool call arrived at 13:15:02.841 — 3 ms later. With a closed
    // upper bound the message fell outside any turn's window.
    const turns = buildSessionTurns({
      rootSessionId: "root-exec",
      rootEndedAt: "2026-05-05T13:15:02.838Z",
      rootStatus: "completed",
      rootMessages: [
        userMsg("u1", "2026-05-05T13:11:30Z"),
        asstMsg(
          "a1",
          "2026-05-05T13:15:02.841Z",
          "[tool calls]",
          JSON.stringify([
            { tool_name: "respond", args: { message: "Done!" } },
          ]),
        ),
      ],
      childRows: [],
    });
    expect(turns[0].assistantText).toBe("Done!");
  });
});
