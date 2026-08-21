// =============================================================================
// session-snapshot — R14f unit tests (post multi-turn refactor).
//
// Coverage targets (new shape):
// - null returns on listLogSessions failure and missing root row
// - completed session: title, SessionTurn rollup with subagents per turn
// - running session: status + conversationId:null documented limitation
// - turnFromLogRow status mapping (still emits AgentTurn for subagent rows)
// - children sorted by startedAt; leftover children get request:null
// - only persisted /artifacts records become clickable artifact refs
// - isRootRow ignores empty/undefined parent_session_id
//
// The old `extractRespondByExecId` / `extractDelegationTasks` helpers were
// deleted as part of the refactor; their per-turn equivalents
// (`extractAssistantReplyForTurn`, `extractDelegationTasksInWindow`) live in
// `turns.test.ts`.
// =============================================================================

import { describe, it, expect, vi, beforeEach } from "vitest";
import type { Transport } from "@/services/transport";
import type {
  Artifact,
  LogSession,
  SessionMessage,
  SessionStatus,
} from "@/services/transport/types";

const listLogSessions = vi.fn<Transport["listLogSessions"]>();
const getSessionMessages = vi.fn<Transport["getSessionMessages"]>();
const listSessionArtifacts = vi.fn<Transport["listSessionArtifacts"]>();
const getSessionState = vi.fn<Transport["getSessionState"]>();
const listSavedSessionSurfaces = vi.fn<Transport["listSavedSessionSurfaces"]>();

function makeTransport(): Transport {
  // getSessionState defaults to a benign no-ward response; individual
  // tests can override to assert ward-bearing snapshots.
  getSessionState.mockImplementation(async () => ({
    success: true,
    data: {
      session: { id: "", title: null, status: "completed", startedAt: "", durationMs: 0, tokenCount: 0, model: null },
      userMessage: null,
      phase: "completed",
      response: null,
      intentAnalysis: null,
      ward: null,
      recalledFacts: [],
      plan: [],
      subagents: [],
      isLive: false,
    },
  }));
  return {
    listLogSessions,
    getSessionMessages,
    listSessionArtifacts,
    getSessionState,
    listSavedSessionSurfaces,
  } as unknown as Transport;
}

import {
  isRootRow,
  snapshotSession,
  turnFromLogRow,
} from "./session-snapshot";

// -----------------------------------------------------------------------------
// Fixture factories
// -----------------------------------------------------------------------------

const SESSION_ID = "sess-ABC";
const ROOT_EXEC = "exec-root";
const CHILD_EXEC_1 = "exec-child-1";
const CHILD_EXEC_2 = "exec-child-2";

function makeRow(overrides: Partial<LogSession> = {}): LogSession {
  return {
    session_id: ROOT_EXEC,
    conversation_id: SESSION_ID,
    agent_id: "root",
    agent_name: "root",
    started_at: "2026-04-19T00:00:00.000Z",
    ended_at: "2026-04-19T00:01:00.000Z",
    status: "completed" as SessionStatus,
    token_count: 0,
    tool_call_count: 0,
    error_count: 0,
    child_session_ids: [],
    title: "",
    ...overrides,
  };
}

function makeMessage(overrides: Partial<SessionMessage> = {}): SessionMessage {
  return {
    id: `msg-${Math.random().toString(36).slice(2)}`,
    execution_id: ROOT_EXEC,
    agent_id: "root",
    delegation_type: "root",
    role: "assistant",
    content: "",
    created_at: "2026-04-19T00:00:00.000Z",
    ...overrides,
  };
}

function makeToolCallMessage(
  execId: string,
  toolCalls: Array<{ tool_name: string; args: Record<string, unknown>; tool_id?: string }>,
  createdAt: string = "2026-04-19T00:00:30.000Z",
): SessionMessage {
  return makeMessage({
    execution_id: execId,
    role: "assistant",
    content: "[tool calls]",
    created_at: createdAt,
    // Wire uses camelCase for this column; our parser accepts both.
    tool_calls: JSON.stringify(toolCalls),
  });
}

function makeArtifact(id: string, overrides: Partial<Artifact> = {}): Artifact {
  return {
    id,
    sessionId: SESSION_ID,
    filePath: `/tmp/${id}.md`,
    fileName: `${id}.md`,
    fileType: "md",
    fileSize: 100,
    isGoalArtifact: true,
    createdAt: "2026-04-19T00:00:00Z",
    ...overrides,
  };
}

// -----------------------------------------------------------------------------
// Setup
// -----------------------------------------------------------------------------

beforeEach(() => {
  listLogSessions.mockReset();
  getSessionMessages.mockReset();
  listSessionArtifacts.mockReset();
  getSessionState.mockReset();
  listSavedSessionSurfaces.mockReset();
  listSavedSessionSurfaces.mockResolvedValue({ success: true, data: [] });
});

// -----------------------------------------------------------------------------
// Null paths
// -----------------------------------------------------------------------------

describe("snapshotSession — null returns", () => {
  it("returns null when listLogSessions fails", async () => {
    listLogSessions.mockResolvedValueOnce({ success: false, error: "offline" });
    getSessionMessages.mockResolvedValueOnce({ success: true, data: [] });
    listSessionArtifacts.mockResolvedValueOnce({ success: true, data: [] });

    const result = await snapshotSession(makeTransport(), SESSION_ID);
    expect(result).toBeNull();
  });

  it("returns null when no root row matches the sessionId", async () => {
    const unrelated = makeRow({
      conversation_id: "sess-DIFFERENT",
      parent_session_id: undefined,
    });
    listLogSessions.mockResolvedValueOnce({ success: true, data: [unrelated] });
    getSessionMessages.mockResolvedValueOnce({ success: true, data: [] });
    listSessionArtifacts.mockResolvedValueOnce({ success: true, data: [] });

    const result = await snapshotSession(makeTransport(), SESSION_ID);
    expect(result).toBeNull();
  });
});

// -----------------------------------------------------------------------------
// Completed-session happy path
// -----------------------------------------------------------------------------

describe("snapshotSession — completed session", () => {
  it("includes saved work surfaces in the hydration snapshot", async () => {
    // STUB: AC4 — Research restore uses the same bounded server snapshot.
    listLogSessions.mockResolvedValueOnce({
      success: true,
      data: [makeRow()],
    });
    getSessionMessages.mockResolvedValueOnce({ success: true, data: [] });
    listSessionArtifacts.mockResolvedValueOnce({ success: true, data: [] });
    const saved = {
      surface_id: "surface-research",
      catalog_id: "zbot/work-surface/v1" as const,
      components: [],
      data: {},
    };
    listSavedSessionSurfaces.mockResolvedValueOnce({
      success: true,
      data: [{ execution_id: "exec-1", surface: saved }],
    });

    const snapshot = await snapshotSession(makeTransport(), SESSION_ID);

    expect(snapshot?.surfaces).toEqual([{ execution_id: "exec-1", surface: saved }]);
    expect(listSavedSessionSurfaces).toHaveBeenCalledWith(SESSION_ID);
  });

  it("carries the recorded primary intent into the hydrated snapshot", async () => {
    const rootRow = makeRow({ parent_session_id: undefined });
    listLogSessions.mockResolvedValueOnce({ success: true, data: [rootRow] });
    getSessionMessages.mockResolvedValueOnce({ success: true, data: [] });
    listSessionArtifacts.mockResolvedValueOnce({ success: true, data: [] });

    const transport = makeTransport();
    getSessionState.mockResolvedValueOnce({
      success: true,
      data: {
        session: { id: SESSION_ID, title: null, status: "completed", startedAt: "", durationMs: 0, tokenCount: 0, model: null },
        userMessage: null,
        phase: "completed",
        response: null,
        intentAnalysis: { primary_intent: "architecture review" },
        ward: null,
        recalledFacts: [],
        plan: [],
        subagents: [],
        isLive: false,
      },
    });

    const snap = await snapshotSession(transport, SESSION_ID);

    expect(snap).not.toBeNull();
    expect(snap!.intentClassification).toBe("architecture review");
    expect(snap!.intentAnalyzing).toBe(false);
  });

  it("rehydrates nested delegated executions with their safe tool activity", async () => {
    const rootRow = makeRow({
      session_id: ROOT_EXEC,
      parent_session_id: undefined,
    });
    const parent = makeRow({
      session_id: CHILD_EXEC_1,
      agent_id: "planner-agent",
      parent_session_id: ROOT_EXEC,
      started_at: "2026-04-19T00:00:10.000Z",
    });
    const grandchild = makeRow({
      session_id: CHILD_EXEC_2,
      agent_id: "builder-agent",
      parent_session_id: CHILD_EXEC_1,
      started_at: "2026-04-19T00:00:20.000Z",
    });
    const user = makeMessage({
      role: "user",
      content: "Plan this work.",
      created_at: "2026-04-19T00:00:00.000Z",
    });
    const rootDelegation = makeToolCallMessage(
      ROOT_EXEC,
      [{ tool_name: "delegate_to_agent", args: { task: "Make the plan." } }],
      "2026-04-19T00:00:05.000Z",
    );
    const parentRecall = makeToolCallMessage(
      CHILD_EXEC_1,
      [{ tool_name: "recall", args: {} }],
      "2026-04-19T00:00:15.000Z",
    );
    const parentReply = makeMessage({
      execution_id: CHILD_EXEC_1,
      role: "assistant",
      content: "Plan complete.",
      created_at: "2026-04-19T00:00:30.000Z",
    });
    const grandchildWrite = makeToolCallMessage(
      CHILD_EXEC_2,
      [{ tool_name: "write_file", args: {} }],
      "2026-04-19T00:00:25.000Z",
    );
    const grandchildReply = makeMessage({
      execution_id: CHILD_EXEC_2,
      role: "assistant",
      content: "File written.",
      created_at: "2026-04-19T00:00:35.000Z",
    });

    listLogSessions.mockResolvedValueOnce({ success: true, data: [rootRow, parent, grandchild] });
    getSessionMessages.mockResolvedValueOnce({
      success: true,
      data: [user, rootDelegation, parentRecall, parentReply, grandchildWrite, grandchildReply],
    });
    listSessionArtifacts.mockResolvedValueOnce({ success: true, data: [] });

    const snap = await snapshotSession(makeTransport(), SESSION_ID);
    const agents = snap!.turns[0].subagents;
    const hydratedParent = agents.find((agent) => agent.id === CHILD_EXEC_1)!;
    const hydratedGrandchild = agents.find((agent) => agent.id === CHILD_EXEC_2)!;

    expect(hydratedParent.respond).toBe("Plan complete.");
    expect(hydratedParent.timeline.map((entry) => entry.toolName)).toEqual(["recall"]);
    expect(hydratedGrandchild.parentExecutionId).toBe(CHILD_EXEC_1);
    expect(hydratedGrandchild.respond).toBe("File written.");
    expect(hydratedGrandchild.timeline.map((entry) => entry.toolName)).toEqual(["write_file"]);
  });

  it("recovers delegated agents from a conversation-scoped history query", async () => {
    const rootRow = makeRow({ session_id: ROOT_EXEC, parent_session_id: undefined });
    const child = makeRow({
      session_id: CHILD_EXEC_1,
      agent_id: "research-agent",
      parent_session_id: ROOT_EXEC,
      started_at: "2026-04-19T00:00:10.000Z",
    });
    const user = makeMessage({ role: "user", content: "Research this." });
    const delegate = makeToolCallMessage(
      ROOT_EXEC,
      [{ tool_name: "delegate_to_agent", args: { task: "Find the evidence." } }],
      "2026-04-19T00:00:05.000Z",
    );

    // The logs endpoint normally returns only the newest global executions.
    // A session snapshot must scope its request so a session's child execution
    // is not dropped merely because unrelated sessions ran after it.
    listLogSessions.mockImplementation(async (filter) => ({
      success: true,
      data: filter?.conversation_id === SESSION_ID ? [rootRow, child] : [rootRow],
    }));
    getSessionMessages.mockResolvedValueOnce({ success: true, data: [user, delegate] });
    listSessionArtifacts.mockResolvedValueOnce({ success: true, data: [] });

    const snapshot = await snapshotSession(makeTransport(), SESSION_ID);

    expect(snapshot?.turns[0].subagents).toEqual(
      expect.arrayContaining([
        expect.objectContaining({
          id: CHILD_EXEC_1,
          agentId: "research-agent",
          request: "Find the evidence.",
        }),
      ]),
    );
  });

  it("builds a SessionTurn carrying user message + subagents + assistant reply", async () => {
    const rootRow = makeRow({
      session_id: ROOT_EXEC,
      title: "Q4 market analysis",
      status: "completed" as SessionStatus,
      parent_session_id: undefined,
    });
    const childRow1 = makeRow({
      session_id: CHILD_EXEC_1,
      agent_id: "planner-agent",
      parent_session_id: ROOT_EXEC,
      started_at: "2026-04-19T00:00:10.000Z",
      status: "completed" as SessionStatus,
    });
    const childRow2 = makeRow({
      session_id: CHILD_EXEC_2,
      agent_id: "writer-agent",
      parent_session_id: ROOT_EXEC,
      started_at: "2026-04-19T00:00:20.000Z",
      status: "completed" as SessionStatus,
    });

    const userMsg = makeMessage({
      execution_id: ROOT_EXEC,
      role: "user",
      content: "What's the Q4 outlook?",
      created_at: "2026-04-19T00:00:00.000Z",
    });
    const rootDelegate1 = makeToolCallMessage(
      ROOT_EXEC,
      [{ tool_name: "delegate_to_agent", args: { agent_id: "planner-agent", task: "Plan the analysis." } }],
      "2026-04-19T00:00:05.000Z",
    );
    const rootDelegate2 = makeToolCallMessage(
      ROOT_EXEC,
      [{ tool_name: "delegate_to_agent", args: { agent_id: "writer-agent", task: "Draft the response." } }],
      "2026-04-19T00:00:15.000Z",
    );
    const rootRespond = makeToolCallMessage(
      ROOT_EXEC,
      [{ tool_name: "respond", args: { message: "Final answer." } }],
      "2026-04-19T00:00:50.000Z",
    );
    // Subagent respond rows aren't on the root execution; they don't appear
    // in the per-turn assistant-reply window. Their respond text comes from
    // the WS stream during a live run; for snapshot reload, the subagents
    // simply land with respond=null (turnFromLogRow doesn't backfill it).
    // What we DO assert is that the per-turn rollup zips delegation tasks
    // onto the matching subagent in chronological order.

    listLogSessions.mockResolvedValueOnce({
      success: true,
      data: [rootRow, childRow1, childRow2],
    });
    getSessionMessages.mockResolvedValueOnce({
      success: true,
      data: [userMsg, rootDelegate1, rootDelegate2, rootRespond],
    });
    listSessionArtifacts.mockResolvedValueOnce({ success: true, data: [] });

    const snap = await snapshotSession(makeTransport(), SESSION_ID);
    expect(snap).not.toBeNull();
    expect(snap!.title).toBe("Q4 market analysis");
    expect(snap!.status).toBe("complete");
    expect(snap!.rootExecutionId).toBe(ROOT_EXEC);

    // One user message → one SessionTurn carrying both subagents.
    expect(snap!.turns).toHaveLength(1);
    const turn = snap!.turns[0];
    expect(turn.userMessage.content).toBe("What's the Q4 outlook?");
    expect(turn.assistantText).toBe("Final answer.");
    expect(turn.subagents).toHaveLength(2);

    // Subagents sorted by started_at; delegation tasks zipped in order.
    const [c1, c2] = turn.subagents;
    expect(c1.id).toBe(CHILD_EXEC_1);
    expect(c1.parentExecutionId).toBe(ROOT_EXEC);
    expect(c1.request).toBe("Plan the analysis.");
    expect(c2.id).toBe(CHILD_EXEC_2);
    expect(c2.parentExecutionId).toBe(ROOT_EXEC);
    expect(c2.request).toBe("Draft the response.");
  });

  it("returns conversationId:null — documented limitation for re-attach", async () => {
    const runningRow = makeRow({
      session_id: ROOT_EXEC,
      status: "running" as SessionStatus,
      parent_session_id: undefined,
      ended_at: undefined,
    });
    listLogSessions.mockResolvedValueOnce({ success: true, data: [runningRow] });
    getSessionMessages.mockResolvedValueOnce({ success: true, data: [] });
    listSessionArtifacts.mockResolvedValueOnce({ success: true, data: [] });
    // Session-level truth must also say running — the snapshot status is
    // now sourced from /state.isLive first, with /logs/sessions as a
    // fallback (see session-snapshot.ts for why).
    getSessionState.mockResolvedValueOnce({
      success: true,
      data: {
        session: { id: "", title: null, status: "running", startedAt: "", durationMs: 0, tokenCount: 0, model: null },
        userMessage: null,
        phase: "executing",
        response: null,
        intentAnalysis: null,
        ward: null,
        recalledFacts: [],
        plan: [],
        subagents: [],
        isLive: true,
      },
    });

    const snap = await snapshotSession(makeTransport(), SESSION_ID);
    expect(snap).not.toBeNull();
    expect(snap!.status).toBe("running");
    expect(snap!.conversationId).toBeNull();
  });

  it("live mid-flight session: /state.isLive=true wins even when root row shows completed", async () => {
    // Regression: reopened session where the root execution completed its
    // first pass but subagents + continuation are still in flight.
    // /logs/sessions reports the root row's "completed" status; /state
    // reports isLive=true. The WS subscribe guard keys on
    // snap.status === "running", so choosing the wrong source silenced
    // live updates on reopen.
    const completedRootRow = makeRow({
      session_id: ROOT_EXEC,
      status: "completed" as SessionStatus,
      parent_session_id: undefined,
    });
    listLogSessions.mockResolvedValueOnce({ success: true, data: [completedRootRow] });
    getSessionMessages.mockResolvedValueOnce({ success: true, data: [] });
    listSessionArtifacts.mockResolvedValueOnce({ success: true, data: [] });
    getSessionState.mockResolvedValueOnce({
      success: true,
      data: {
        session: { id: "", title: null, status: "running", startedAt: "", durationMs: 0, tokenCount: 0, model: null },
        userMessage: null,
        phase: "executing",
        response: null,
        intentAnalysis: null,
        ward: null,
        recalledFacts: [],
        plan: [],
        subagents: [],
        isLive: true,
      },
    });

    const snap = await snapshotSession(makeTransport(), SESSION_ID);
    expect(snap).not.toBeNull();
    expect(snap!.status).toBe("running");
  });
});

// -----------------------------------------------------------------------------
// turnFromLogRow — status mapping
// -----------------------------------------------------------------------------

describe("turnFromLogRow", () => {
  it("maps backend status strings into AgentTurnStatus", () => {
    const completed = turnFromLogRow(makeRow({ status: "completed" as SessionStatus }), null);
    expect(completed.status).toBe("completed");

    const running = turnFromLogRow(makeRow({ status: "running" as SessionStatus, ended_at: undefined }), null);
    expect(running.status).toBe("running");
    expect(running.completedAt).toBeNull();

    const stopped = turnFromLogRow(makeRow({ status: "stopped" as SessionStatus }), null);
    expect(stopped.status).toBe("stopped");

    const crashed = turnFromLogRow(makeRow({ status: "error" as SessionStatus }), null);
    expect(crashed.status).toBe("error");
  });

  it("carries parent execution id through", () => {
    const t = turnFromLogRow(makeRow({ session_id: CHILD_EXEC_1 }), ROOT_EXEC);
    expect(t.parentExecutionId).toBe(ROOT_EXEC);
    expect(t.id).toBe(CHILD_EXEC_1);
  });
});

// -----------------------------------------------------------------------------
// Zip tasks → children (per-turn)
// -----------------------------------------------------------------------------

describe("snapshotSession — children zip + sort", () => {
  it("leftover subagents get request:null when delegation count < subagent count", async () => {
    const rootRow = makeRow({ session_id: ROOT_EXEC, parent_session_id: undefined });
    const childA = makeRow({
      session_id: "child-A",
      parent_session_id: ROOT_EXEC,
      started_at: "2026-04-19T00:00:10.000Z",
    });
    const childB = makeRow({
      session_id: "child-B",
      parent_session_id: ROOT_EXEC,
      started_at: "2026-04-19T00:00:20.000Z",
    });
    const userMsg = makeMessage({
      execution_id: ROOT_EXEC,
      role: "user",
      content: "do the thing",
      created_at: "2026-04-19T00:00:00.000Z",
    });
    // Only one delegation task, two children — second child is orphan.
    const delegate = makeToolCallMessage(
      ROOT_EXEC,
      [{ tool_name: "delegate_to_agent", args: { task: "only task" } }],
      "2026-04-19T00:00:05.000Z",
    );

    listLogSessions.mockResolvedValueOnce({ success: true, data: [rootRow, childB, childA] });
    getSessionMessages.mockResolvedValueOnce({ success: true, data: [userMsg, delegate] });
    listSessionArtifacts.mockResolvedValueOnce({ success: true, data: [] });

    const snap = await snapshotSession(makeTransport(), SESSION_ID);
    expect(snap!.turns).toHaveLength(1);
    const [first, second] = snap!.turns[0].subagents;
    // Subagents sorted by started_at ascending.
    expect(first.id).toBe("child-A");
    expect(second.id).toBe("child-B");
    expect(first.request).toBe("only task");
    expect(second.request).toBeNull();
  });
});

// -----------------------------------------------------------------------------
// Artifacts merge
// -----------------------------------------------------------------------------

describe("snapshotSession — artifacts", () => {
  it("keeps only explicitly designated goal deliverables from a mixed manifest", async () => {
    const rootRow = makeRow({ session_id: ROOT_EXEC, parent_session_id: undefined });
    const workingScript = makeArtifact("scratch", {
      fileName: "scratch.py",
      fileType: "py",
      isGoalArtifact: false,
    });
    const requestedDeliverable = makeArtifact("report", {
      fileName: "research-report.py",
      fileType: "py",
      isGoalArtifact: true,
    });

    listLogSessions.mockResolvedValueOnce({ success: true, data: [rootRow] });
    getSessionMessages.mockResolvedValueOnce({ success: true, data: [] });
    listSessionArtifacts.mockResolvedValueOnce({
      success: true,
      data: [workingScript, requestedDeliverable],
    });

    const snap = await snapshotSession(makeTransport(), SESSION_ID);

    expect(listSessionArtifacts).toHaveBeenCalledWith(SESSION_ID, {
      goalArtifactsOnly: true,
      limit: 24,
    });
    expect(snap!.artifacts).toEqual([
      expect.objectContaining({ id: "report", fileName: "research-report.py" }),
    ]);
  });

  it("/artifacts endpoint wins when it returns data", async () => {
    const rootRow = makeRow({ session_id: ROOT_EXEC, parent_session_id: undefined });
    const respondWithHints = makeToolCallMessage(ROOT_EXEC, [
      {
        tool_name: "respond",
        args: { message: "done", artifacts: [{ path: "/tmp/hint.md", label: "Hint" }] },
      },
    ]);
    const realArtifact = makeArtifact("real-1", {
      fileName: "real.md",
      isGoalArtifact: true,
    });

    listLogSessions.mockResolvedValueOnce({ success: true, data: [rootRow] });
    getSessionMessages.mockResolvedValueOnce({ success: true, data: [respondWithHints] });
    listSessionArtifacts.mockResolvedValueOnce({ success: true, data: [realArtifact] });

    const snap = await snapshotSession(makeTransport(), SESSION_ID);
    expect(snap!.artifacts).toHaveLength(1);
    expect(snap!.artifacts[0].id).toBe("real-1");
    expect(snap!.artifacts[0].fileName).toBe("real.md");
  });

  it("does not turn an unpersisted respond artifact path into a broken clickable ref", async () => {
    const rootRow = makeRow({ session_id: ROOT_EXEC, parent_session_id: undefined });
    const respondWithHints = makeToolCallMessage(ROOT_EXEC, [
      {
        tool_name: "respond",
        args: { message: "done", artifacts: [{ path: "/tmp/hint.md", label: "Plan" }] },
      },
    ]);

    listLogSessions.mockResolvedValueOnce({ success: true, data: [rootRow] });
    getSessionMessages.mockResolvedValueOnce({ success: true, data: [respondWithHints] });
    listSessionArtifacts.mockResolvedValueOnce({ success: true, data: [] });

    const snap = await snapshotSession(makeTransport(), SESSION_ID);
    expect(snap!.artifacts).toEqual([]);
  });
});

// -----------------------------------------------------------------------------
// isRootRow
// -----------------------------------------------------------------------------

describe("isRootRow", () => {
  it("true when parent_session_id is undefined", () => {
    expect(isRootRow(makeRow({ parent_session_id: undefined }))).toBe(true);
  });
  it("true when parent_session_id is empty string", () => {
    expect(isRootRow(makeRow({ parent_session_id: "" }))).toBe(true);
  });
  it("false when parent_session_id is non-empty", () => {
    expect(isRootRow(makeRow({ parent_session_id: "exec-parent" }))).toBe(false);
  });
});
