// =============================================================================
// useResearchSession — R14a/R14f integration tests.
//
// R14a: sendMessage subscribes BEFORE invoke with a client-minted conv_id.
// R14f: snapshot-on-open replaces the previous hydrate-messages-only flow.
// The artifact-polling timer that used to live here is gone; snapshotSession
// pulls artifacts once on open and again on root `agent_completed`.
// =============================================================================

import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { renderHook, act } from "@testing-library/react";
import { createElement, StrictMode, type PropsWithChildren } from "react";
import { MemoryRouter, Routes, Route } from "react-router-dom";
import type { Transport } from "@/services/transport";
import type {
  Artifact,
  ConversationEvent,
  LogSession,
  SessionMessage,
  SessionStatus,
} from "@/services/transport/types";

// ---------------------------------------------------------------------------
// Transport mock — per-test spies for order assertions
// ---------------------------------------------------------------------------

const subscribeConversation = vi.fn<Transport["subscribeConversation"]>();
const executeAgent = vi.fn<Transport["executeAgent"]>();
const stopAgent = vi.fn<Transport["stopAgent"]>();
const getSessionMessages = vi.fn<Transport["getSessionMessages"]>();
const listSessionArtifacts = vi.fn<Transport["listSessionArtifacts"]>();
const listLogSessions = vi.fn<Transport["listLogSessions"]>();
const getSessionState = vi.fn<Transport["getSessionState"]>();
const unsubscribeSpy = vi.fn<() => void>();
// Ordered log of all transport calls to assert subscribe-before-invoke.
const callLog: string[] = [];

vi.mock("@/services/transport", () => ({
  getTransport: async () => ({
    subscribeConversation,
    executeAgent,
    stopAgent,
    getSessionMessages,
    listSessionArtifacts,
    listLogSessions,
    getSessionState,
    // R14h recovery uses this to re-check session_id on reconnect.
    onConnectionStateChange: () => () => undefined,
  }),
}));

vi.mock("sonner", () => ({ toast: { error: vi.fn() } }));

// ---------------------------------------------------------------------------
// Import AFTER the mock is registered.
// ---------------------------------------------------------------------------
import { useResearchSession } from "./useResearchSession";

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

const TEST_INITIAL_PATH = "/research";

function routerWrapper(initialPath: string) {
  return function Wrapper({ children }: PropsWithChildren) {
    return createElement(
      MemoryRouter,
      { initialEntries: [initialPath] },
      createElement(
        Routes,
        null,
        createElement(Route, { path: "/research", element: children }),
        createElement(Route, {
          path: "/research/:sessionId",
          element: children,
        }),
      ),
    );
  };
}

function strictRouterWrapper(initialPath: string) {
  const Inner = routerWrapper(initialPath);
  return function StrictWrapper({ children }: PropsWithChildren) {
    return createElement(
      StrictMode,
      null,
      createElement(Inner, null, children as React.ReactElement),
    );
  };
}

// ---------------------------------------------------------------------------
// Log-row / message fixture factories
// ---------------------------------------------------------------------------

function makeRootRow(sessionId: string, overrides: Partial<LogSession> = {}): LogSession {
  return {
    session_id: `exec-${sessionId}`,
    conversation_id: sessionId,
    agent_id: "root",
    agent_name: "root",
    started_at: "2026-04-19T00:00:00.000Z",
    ended_at: "2026-04-19T00:01:00.000Z",
    status: "completed" as SessionStatus,
    token_count: 0,
    tool_call_count: 0,
    error_count: 0,
    child_session_ids: [],
    title: "Hydrated",
    parent_session_id: undefined,
    ...overrides,
  };
}

function makeUserMessage(sessionId: string): SessionMessage {
  return {
    id: "m-user-1",
    execution_id: `exec-${sessionId}`,
    agent_id: "root",
    delegation_type: "root",
    role: "user",
    content: "old prompt",
    created_at: "2026-04-10T00:00:00Z",
  };
}

beforeEach(() => {
  callLog.length = 0;
  subscribeConversation.mockReset();
  executeAgent.mockReset();
  stopAgent.mockReset();
  getSessionMessages.mockReset();
  listSessionArtifacts.mockReset();
  listLogSessions.mockReset();
  getSessionState.mockReset();
  unsubscribeSpy.mockReset();

  subscribeConversation.mockImplementation((convId: string) => {
    callLog.push(`subscribe:${convId}`);
    return unsubscribeSpy;
  });
  executeAgent.mockImplementation(async (_agent, convId) => {
    callLog.push(`invoke:${convId}`);
    return { success: true, data: { conversationId: convId } };
  });
  stopAgent.mockResolvedValue({ success: true, data: undefined });
  getSessionMessages.mockResolvedValue({ success: true, data: [] });
  listSessionArtifacts.mockResolvedValue({ success: true, data: [] });
  listLogSessions.mockResolvedValue({ success: true, data: [] });
  // Default: benign snapshot state — tests that care about ward info override.
  getSessionState.mockResolvedValue({
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
  });
});

afterEach(() => {
  vi.clearAllMocks();
});

function lastSubscribedConvId(): string {
  const calls = subscribeConversation.mock.calls;
  expect(calls.length).toBeGreaterThan(0);
  return calls[calls.length - 1][0];
}

// ---------------------------------------------------------------------------
// R14a — subscription ordering
// ---------------------------------------------------------------------------

describe("useResearchSession — subscription ordering (R14a)", () => {
  it("sendMessage subscribes BEFORE invoke with the SAME convId", async () => {
    const { result } = renderHook(() => useResearchSession(), {
      wrapper: routerWrapper(TEST_INITIAL_PATH),
    });

    await act(async () => {
      await result.current.sendMessage("hello");
    });

    expect(subscribeConversation).toHaveBeenCalledTimes(1);
    expect(executeAgent).toHaveBeenCalledTimes(1);

    const subConvId = subscribeConversation.mock.calls[0][0];
    const invokeConvId = executeAgent.mock.calls[0][1];
    expect(subConvId).toBe(invokeConvId);

    // Subscribe appears before invoke in the ordered log.
    const subIdx = callLog.findIndex((c) => c.startsWith("subscribe:"));
    const invIdx = callLog.findIndex((c) => c.startsWith("invoke:"));
    expect(subIdx).toBeGreaterThanOrEqual(0);
    expect(invIdx).toBeGreaterThan(subIdx);
  });

  it("uses a research-prefixed client-minted convId for a new session", async () => {
    const { result } = renderHook(() => useResearchSession(), {
      wrapper: routerWrapper(TEST_INITIAL_PATH),
    });
    await act(async () => {
      await result.current.sendMessage("first");
    });
    const convId = lastSubscribedConvId();
    expect(convId).toMatch(/^research-/);
    expect(executeAgent.mock.calls[0][4]).toBe("deep");
  });

  it("second sendMessage on same session does NOT re-subscribe", async () => {
    const { result } = renderHook(() => useResearchSession(), {
      wrapper: routerWrapper(TEST_INITIAL_PATH),
    });
    await act(async () => {
      await result.current.sendMessage("one");
    });
    const firstConvId = lastSubscribedConvId();

    // The hook's "running" status guard blocks back-to-back sends; deliver a
    // terminal error event through the captured onEvent to clear status and
    // unblock the follow-up send. Real lifecycle would clear it via
    // session_complete; error path works the same for this test's purpose.
    const onEvent = subscribeConversation.mock.calls[0][1].onEvent;
    act(() => {
      onEvent({
        type: "error",
        timestamp: Date.now(),
        session_id: "",
        execution_id: "",
        message: "simulated",
      } as ConversationEvent);
    });

    await act(async () => {
      await result.current.sendMessage("two");
    });

    expect(subscribeConversation).toHaveBeenCalledTimes(1);
    expect(executeAgent).toHaveBeenCalledTimes(2);
    // Both invokes used the same convId.
    expect(executeAgent.mock.calls[0][1]).toBe(firstConvId);
    expect(executeAgent.mock.calls[1][1]).toBe(firstConvId);
  });

  it("startNewResearch unsubscribes and the next send subscribes with a NEW convId", async () => {
    const { result } = renderHook(() => useResearchSession(), {
      wrapper: routerWrapper(TEST_INITIAL_PATH),
    });
    await act(async () => {
      await result.current.sendMessage("one");
    });
    const firstConvId = lastSubscribedConvId();

    act(() => {
      result.current.startNewResearch();
    });
    expect(unsubscribeSpy).toHaveBeenCalledTimes(1);

    await act(async () => {
      await result.current.sendMessage("two");
    });
    expect(subscribeConversation).toHaveBeenCalledTimes(2);
    const secondConvId = lastSubscribedConvId();
    expect(secondConvId).not.toBe(firstConvId);
    expect(secondConvId).toMatch(/^research-/);
  });

  it("unmount calls the stored unsubscribe", async () => {
    const { result, unmount } = renderHook(() => useResearchSession(), {
      wrapper: routerWrapper(TEST_INITIAL_PATH),
    });
    await act(async () => {
      await result.current.sendMessage("one");
    });
    expect(unsubscribeSpy).not.toHaveBeenCalled();

    unmount();

    expect(unsubscribeSpy).toHaveBeenCalledTimes(1);
  });

  it("unmount without any sendMessage does NOT crash", () => {
    const { unmount } = renderHook(() => useResearchSession(), {
      wrapper: routerWrapper(TEST_INITIAL_PATH),
    });
    expect(() => unmount()).not.toThrow();
    expect(unsubscribeSpy).not.toHaveBeenCalled();
  });

  it("StrictMode double-mount: subscribe called only once after one send", async () => {
    const { result } = renderHook(() => useResearchSession(), {
      wrapper: strictRouterWrapper(TEST_INITIAL_PATH),
    });
    await act(async () => {
      await result.current.sendMessage("hi");
    });
    // StrictMode double-invokes effects but sendMessage guards on
    // subscribedConvIdRef — the subscribe must fire exactly once.
    expect(subscribeConversation).toHaveBeenCalledTimes(1);
    expect(executeAgent).toHaveBeenCalledTimes(1);
  });

  it("hydrate + sendMessage: subscribe fires with a fresh convId; invoke carries the sessionId", async () => {
    const EXISTING_SESSION = "sess-existing-123";
    listLogSessions.mockResolvedValueOnce({
      success: true,
      data: [makeRootRow(EXISTING_SESSION)],
    });
    getSessionMessages.mockResolvedValueOnce({
      success: true,
      data: [makeUserMessage(EXISTING_SESSION)],
    });

    const { result } = renderHook(() => useResearchSession(), {
      wrapper: routerWrapper(`/research/${EXISTING_SESSION}`),
    });

    // Let hydrate effect flush — snapshotSession fan-out resolves in two ticks.
    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(listLogSessions).toHaveBeenCalled();
    expect(getSessionMessages).toHaveBeenCalledWith(
      EXISTING_SESSION,
      expect.objectContaining({ scope: "all" }),
    );

    await act(async () => {
      await result.current.sendMessage("follow-up");
    });

    // R14g: two subscriptions now — one on the client-minted convId (for
    // conv-id-routed events) and one on sessionId (scope="all" so child
    // tool_calls / thinking reach the top pill + inline tickers; server
    // routes by session_id regardless of scope). Transport seq dedup handles
    // any overlap.
    expect(subscribeConversation).toHaveBeenCalledTimes(2);
    const subscribedKeys = subscribeConversation.mock.calls.map((c) => c[0]);
    const convId = subscribedKeys.find((k) => k.startsWith("research-"));
    expect(convId).toMatch(/^research-/);
    expect(subscribedKeys).toContain(EXISTING_SESSION);

    // executeAgent gets (agentId, convId, message, sessionId, mode).
    const invokeArgs = executeAgent.mock.calls[0];
    expect(invokeArgs[1]).toBe(convId);
    expect(invokeArgs[3]).toBe(EXISTING_SESSION);
    expect(invokeArgs[4]).toBe("deep");
  });

  it("error path: failed invoke dispatches ERROR but keeps the subscription", async () => {
    executeAgent.mockImplementationOnce(async (_agent, convId) => {
      callLog.push(`invoke:${convId}`);
      return { success: false, error: "boom" };
    });
    const { result } = renderHook(() => useResearchSession(), {
      wrapper: routerWrapper(TEST_INITIAL_PATH),
    });
    await act(async () => {
      await result.current.sendMessage("hi");
    });
    expect(result.current.state.status).toBe("error");
    // Subscription is intact so the user can retry/observe.
    expect(unsubscribeSpy).not.toHaveBeenCalled();
  });

  it("subscribe throws: state becomes 'error' and a retry is still possible", async () => {
    // First subscribeConversation throws synchronously; executeAgent must NOT
    // have been called (we bail in the try/catch before reaching invoke).
    subscribeConversation.mockImplementationOnce(() => {
      throw new Error("ws boom");
    });

    const { result } = renderHook(() => useResearchSession(), {
      wrapper: routerWrapper(TEST_INITIAL_PATH),
    });

    await act(async () => {
      await result.current.sendMessage("first");
    });

    // The hook must not be stuck on "running" — it must land on "error".
    expect(result.current.state.status).toBe("error");
    expect(executeAgent).not.toHaveBeenCalled();

    // Idempotency after failure: a fresh send attempts subscribe+invoke again
    // (the second subscribe mock uses the default happy-path implementation).
    await act(async () => {
      await result.current.sendMessage("retry");
    });

    expect(subscribeConversation).toHaveBeenCalledTimes(2);
    expect(executeAgent).toHaveBeenCalledTimes(1);
  });

  it("bumps the ward vault revision after successful write/edit tool results", async () => {
    const { result } = renderHook(() => useResearchSession(), {
      wrapper: routerWrapper(TEST_INITIAL_PATH),
    });
    await act(async () => {
      await result.current.sendMessage("create files");
    });

    const onEvent = subscribeConversation.mock.calls[0][1].onEvent;
    expect(result.current.wardVaultRevision).toBe(0);

    act(() => {
      onEvent({
        type: "tool_call",
        timestamp: Date.now(),
        session_id: "sess-1",
        execution_id: "exec-1",
        tool_call_id: "call-write",
        tool: "write_file",
        args: { path: "report.md" },
      } as ConversationEvent);
      onEvent({
        type: "tool_result",
        timestamp: Date.now(),
        session_id: "sess-1",
        execution_id: "exec-1",
        tool_call_id: "call-write",
        result: "ok",
      } as ConversationEvent);
    });
    expect(result.current.wardVaultRevision).toBe(1);

    act(() => {
      onEvent({
        type: "tool_call",
        timestamp: Date.now(),
        session_id: "sess-1",
        execution_id: "exec-1",
        tool_call_id: "call-read",
        tool: "read",
        args: { path: "report.md" },
      } as ConversationEvent);
      onEvent({
        type: "tool_result",
        timestamp: Date.now(),
        session_id: "sess-1",
        execution_id: "exec-1",
        tool_call_id: "call-read",
        result: "ok",
      } as ConversationEvent);
    });
    expect(result.current.wardVaultRevision).toBe(1);

    act(() => {
      onEvent({
        type: "tool_call",
        timestamp: Date.now(),
        session_id: "sess-1",
        execution_id: "exec-1",
        tool_call_id: "call-edit",
        tool: "edit_file",
        args: { path: "report.md" },
      } as ConversationEvent);
      onEvent({
        type: "tool_result",
        timestamp: Date.now(),
        session_id: "sess-1",
        execution_id: "exec-1",
        tool_call_id: "call-edit",
        result: "",
        error: "missing file",
      } as ConversationEvent);
    });
    expect(result.current.wardVaultRevision).toBe(1);
  });
});

// ---------------------------------------------------------------------------
// R14f — snapshot on open + re-snapshot on agent_completed
// ---------------------------------------------------------------------------

function makeArtifact(id: string, sessionId = "sess-existing-123"): Artifact {
  return {
    id,
    sessionId,
    filePath: `/tmp/${id}.md`,
    fileName: `${id}.md`,
    fileType: "md",
    fileSize: 100,
    createdAt: "2026-04-19T00:00:00Z",
  };
}

describe("useResearchSession — snapshot flow (R14f)", () => {
  const EXISTING_SESSION = "sess-existing-123";

  it("hydrates title + turns + artifacts via snapshotSession on open", async () => {
    listLogSessions.mockResolvedValueOnce({
      success: true,
      data: [makeRootRow(EXISTING_SESSION, { title: "Hydrated title" })],
    });
    getSessionMessages.mockResolvedValueOnce({
      success: true,
      data: [makeUserMessage(EXISTING_SESSION)],
    });
    listSessionArtifacts.mockResolvedValue({
      success: true,
      data: [makeArtifact("a1", EXISTING_SESSION)],
    });
    getSessionState.mockResolvedValueOnce({
      success: true,
      data: {
        session: { id: EXISTING_SESSION, title: null, status: "completed", startedAt: "", durationMs: 0, tokenCount: 0, model: null },
        userMessage: null,
        phase: "completed",
        response: null,
        intentAnalysis: null,
        ward: { name: "stock-analysis" },
        recalledFacts: [],
        plan: [],
        subagents: [],
        isLive: false,
      },
    });

    const { result } = renderHook(() => useResearchSession(), {
      wrapper: routerWrapper(`/research/${EXISTING_SESSION}`),
    });

    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
      await Promise.resolve();
    });

    expect(result.current.state.sessionId).toBe(EXISTING_SESSION);
    expect(result.current.state.title).toBe("Hydrated title");
    expect(result.current.state.wardId).toBe("stock-analysis");
    expect(result.current.state.wardName).toBe("stock-analysis");
    // The single hydrated user message becomes one SessionTurn carrying it.
    expect(result.current.state.turns).toHaveLength(1);
    expect(result.current.state.turns[0].userMessage.content).toBe("old prompt");
    expect(result.current.state.artifacts).toHaveLength(1);
    expect(result.current.state.artifacts[0].id).toBe("a1");
    // Cache populated so ArtifactSlideOut can resolve by id.
    expect(result.current.getFullArtifact("a1")?.fileName).toBe("a1.md");
  });

  it("re-snapshots on root agent_completed to backfill WS-dropped state", async () => {
    // Initial open: idle (a just-created fresh session that the user sends to).
    // Previously-running snapshots don't re-subscribe (documented R14f limitation),
    // so drive sendMessage to create the subscription and capture onEvent.
    listLogSessions.mockResolvedValueOnce({
      success: true,
      data: [makeRootRow(EXISTING_SESSION, { title: "" })],
    });
    getSessionMessages.mockResolvedValueOnce({ success: true, data: [] });
    listSessionArtifacts.mockResolvedValueOnce({ success: true, data: [] });

    const { result } = renderHook(() => useResearchSession(), {
      wrapper: routerWrapper(`/research/${EXISTING_SESSION}`),
    });

    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(result.current.state.title).toBe("");

    // Snapshot resolved to "complete" (rootRow.status defaulted to completed);
    // sendMessage passes the running guard and subscribes.
    await act(async () => {
      await result.current.sendMessage("hello");
    });
    // R14g: dual subscription — the client-minted convId from sendMessage PLUS
    // the session-id subscription (scope="all") that kicks in once status
    // flips to "running". Both feed the same handler.
    expect(subscribeConversation).toHaveBeenCalledTimes(2);
    // Grab the convId handler for event injection (both share the same ctx).
    const convCall = subscribeConversation.mock.calls.find((c) => c[0].startsWith("research-"));
    expect(convCall).toBeTruthy();
    const onEvent = convCall![1].onEvent;

    // Second snapshot returns the finalised title + artifact.
    listLogSessions.mockResolvedValue({
      success: true,
      data: [
        makeRootRow(EXISTING_SESSION, {
          title: "Final title",
          status: "completed" as SessionStatus,
        }),
      ],
    });
    getSessionMessages.mockResolvedValue({ success: true, data: [] });
    listSessionArtifacts.mockResolvedValue({
      success: true,
      data: [makeArtifact("late-artifact", EXISTING_SESSION)],
    });

    // Root agent_completed: parent_execution_id is absent on root.
    act(() => {
      onEvent({
        type: "agent_completed",
        timestamp: Date.now(),
        session_id: EXISTING_SESSION,
        execution_id: `exec-${EXISTING_SESSION}`,
        agent_id: "root",
      } as ConversationEvent);
    });

    // Two ticks let the re-snapshot promise chain resolve.
    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
      await Promise.resolve();
    });

    expect(result.current.state.title).toBe("Final title");
    expect(result.current.state.artifacts.map((a) => a.id)).toContain("late-artifact");
  });

  it("child agent_completed does NOT trigger a re-snapshot", async () => {
    listLogSessions.mockResolvedValue({
      success: true,
      data: [makeRootRow(EXISTING_SESSION)],
    });
    getSessionMessages.mockResolvedValue({ success: true, data: [] });
    listSessionArtifacts.mockResolvedValue({ success: true, data: [] });

    const { result } = renderHook(() => useResearchSession(), {
      wrapper: routerWrapper(`/research/${EXISTING_SESSION}`),
    });
    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
      await Promise.resolve();
    });
    await act(async () => {
      await result.current.sendMessage("hello");
    });

    const callsAfterOpen = listLogSessions.mock.calls.length;
    const onEvent = subscribeConversation.mock.calls[0][1].onEvent;

    act(() => {
      onEvent({
        type: "agent_completed",
        timestamp: Date.now(),
        session_id: EXISTING_SESSION,
        execution_id: "exec-child-1",
        parent_execution_id: `exec-${EXISTING_SESSION}`,
        agent_id: "writer-agent",
      } as ConversationEvent);
    });

    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
    });

    // No extra snapshot calls — children don't retrigger reconcile.
    expect(listLogSessions.mock.calls.length).toBe(callsAfterOpen);
  });

  it("no polling after the snapshot completes", async () => {
    listLogSessions.mockResolvedValue({
      success: true,
      data: [makeRootRow(EXISTING_SESSION)],
    });
    getSessionMessages.mockResolvedValue({ success: true, data: [] });
    listSessionArtifacts.mockResolvedValue({ success: true, data: [] });

    vi.useFakeTimers();
    try {
      renderHook(() => useResearchSession(), {
        wrapper: routerWrapper(`/research/${EXISTING_SESSION}`),
      });
      await act(async () => {
        await Promise.resolve();
        await Promise.resolve();
        await Promise.resolve();
      });
      const initialCalls = listSessionArtifacts.mock.calls.length;
      await act(async () => {
        await vi.advanceTimersByTimeAsync(30_000);
      });
      // Timer is gone — no additional artifact calls purely from time passing.
      expect(listSessionArtifacts.mock.calls.length).toBe(initialCalls);
    } finally {
      vi.useRealTimers();
    }
  });
});
