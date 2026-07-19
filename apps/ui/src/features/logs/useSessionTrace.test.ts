// ============================================================================
// useSessionTrace — unit tests
// ============================================================================

import { describe, it, expect, vi, beforeEach } from "vitest";
import { renderHook, waitFor, act } from "@testing-library/react";
import type { Transport } from "@/services/transport";

const getLogSession = vi.fn<Transport["getLogSession"]>();
const getSessionMessages = vi.fn<Transport["getSessionMessages"]>();

vi.mock("@/services/transport", () => ({
  getTransport: async () => ({ getLogSession, getSessionMessages }),
}));

import { useSessionTrace } from "./useSessionTrace";

// ─── Helpers ─────────────────────────────────────────────────────────────────

function makeSession(
  id: string,
  agentId = "root",
  status = "completed",
  childIds: string[] = [],
) {
  return {
    session_id: id,
    conversation_id: id.startsWith("exec-") ? "sess-1" : id,
    agent_id: agentId,
    agent_name: agentId,
    title: `Title for ${id}`,
    status,
    started_at: "2024-01-01T00:00:00Z",
    duration_ms: 100,
    token_count: 50,
    child_session_ids: childIds,
  };
}

function makeLog(
  id: string,
  category: string,
  opts: Record<string, unknown> = {},
) {
  return {
    id,
    agent_id: "root",
    session_id: "s1",
    category,
    message: (opts.message as string) ?? `log ${id}`,
    level: (opts.level as string) ?? "info",
    timestamp: "2024-01-01T00:00:01Z",
    duration_ms: 10,
    metadata: (opts.metadata as Record<string, unknown>) ?? null,
  };
}

function makeDetail(sessionId: string, logs = [], childIds: string[] = []) {
  return {
    success: true as const,
    data: {
      session: makeSession(sessionId, "root", "completed", childIds),
      logs,
      executions: [],
    },
  };
}

beforeEach(() => {
  getLogSession.mockReset();
  getSessionMessages.mockReset();
  getSessionMessages.mockResolvedValue({ success: true, data: [] });
});

// ─── Tests ────────────────────────────────────────────────────────────────────

describe("useSessionTrace", () => {
  it("returns loading=false, trace=null when sessionId is null", () => {
    const { result } = renderHook(() => useSessionTrace(null));
    expect(result.current.loading).toBe(false);
    expect(result.current.trace).toBeNull();
  });

  it("loads trace for a session with no children or logs", async () => {
    getLogSession.mockResolvedValue(makeDetail("s1"));
    const { result } = renderHook(() => useSessionTrace("s1"));
    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(result.current.trace).not.toBeNull();
    expect(result.current.trace?.id).toBe("s1");
    expect(result.current.trace?.type).toBe("root");
    expect(result.current.trace?.children).toHaveLength(0);
  });

  it("sets trace=null when sessionId changes to null", async () => {
    getLogSession.mockResolvedValue(makeDetail("s1"));
    const { result, rerender } = renderHook(({ id }) => useSessionTrace(id), {
      initialProps: { id: "s1" as string | null },
    });
    await waitFor(() => expect(result.current.trace).not.toBeNull());
    rerender({ id: null });
    await waitFor(() => expect(result.current.trace).toBeNull());
  });

  it("handles failed getLogSession gracefully", async () => {
    getLogSession.mockResolvedValue({ success: false, error: "not found" });
    const { result } = renderHook(() => useSessionTrace("s1"));
    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(result.current.trace).toBeNull();
  });

  it("handles thrown exception gracefully", async () => {
    getLogSession.mockRejectedValue(new Error("network error"));
    const { result } = renderHook(() => useSessionTrace("s1"));
    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(result.current.trace).toBeNull();
  });

  it("refetch triggers a new load", async () => {
    getLogSession
      .mockResolvedValueOnce(makeDetail("s1"))
      .mockResolvedValueOnce(makeDetail("s1"));
    const { result } = renderHook(() => useSessionTrace("s1"));
    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(getLogSession).toHaveBeenCalledTimes(1);

    act(() => result.current.refetch());
    await waitFor(() => expect(getLogSession).toHaveBeenCalledTimes(2));
  });

  it("includes tool_call log as child node", async () => {
    const toolLog = makeLog("l1", "tool_call", {
      metadata: { tool_name: "shell", tool_id: "tid1" },
    });
    const resultLog = makeLog("l2", "tool_result", {
      metadata: { tool_id: "tid1" },
    });
    getLogSession.mockResolvedValue({
      success: true,
      data: {
        session: makeSession("s1", "root", "completed", []),
        logs: [toolLog, resultLog],
        executions: [],
      },
    });
    const { result } = renderHook(() => useSessionTrace("s1"));
    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(result.current.trace?.children.length).toBeGreaterThan(0);
    expect(result.current.trace?.children[0].type).toBe("tool_call");
  });

  it("enriches slim tool logs with input and output from session messages", async () => {
    const toolLog = makeLog("l1", "tool_call", {
      metadata: { tool_name: "shell", tool_id: "tid1" },
    });
    const resultLog = makeLog("l2", "tool_result", {
      metadata: { tool_id: "tid1" },
    });
    getLogSession.mockResolvedValue(makeDetail("exec-1", [toolLog, resultLog]));
    getSessionMessages.mockResolvedValue({
      success: true,
      data: [
        {
          id: "m1",
          execution_id: "exec-1",
          agent_id: "root",
          delegation_type: "root",
          role: "assistant",
          content: "[tool calls]",
          created_at: "2024-01-01T00:00:01Z",
          tool_calls: [
            { tool_id: "tid1", tool_name: "shell", args: { command: "pwd" } },
          ],
        },
        {
          id: "m2",
          execution_id: "exec-1",
          agent_id: "root",
          delegation_type: "root",
          role: "tool",
          content: '{"stdout":"/tmp"}',
          created_at: "2024-01-01T00:00:02Z",
          tool_call_id: "tid1",
        },
      ],
    });

    const { result } = renderHook(() => useSessionTrace("exec-1"));
    await waitFor(() => expect(result.current.loading).toBe(false));

    expect(getSessionMessages).toHaveBeenCalledWith("sess-1", { scope: "all" });
    const tool = result.current.trace?.children[0];
    expect(tool?.args).toBe('{"command":"pwd"}');
    expect(tool?.result).toBe('{"stdout":"/tmp"}');
  });

  it("includes error log as child node", async () => {
    const errLog = makeLog("l1", "error", {
      message: "something broke",
      level: "error",
    });
    getLogSession.mockResolvedValue({
      success: true,
      data: {
        session: makeSession("s1", "root", "completed", []),
        logs: [errLog],
        executions: [],
      },
    });
    const { result } = renderHook(() => useSessionTrace("s1"));
    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(result.current.trace?.children[0].type).toBe("error");
    expect(result.current.trace?.children[0].summary).toBe("something broke");
  });

  it("includes delegation log and fetches child session", async () => {
    const delegLog = makeLog("l1", "delegation", {
      message: "Delegating to code-agent",
      metadata: { child_agent: "code-agent", task: "write code" },
    });
    // Root session has 1 child
    getLogSession
      .mockResolvedValueOnce({
        success: true,
        data: {
          session: makeSession("s1", "root", "completed", ["s2"]),
          logs: [delegLog],
          executions: [],
        },
      })
      // Child session
      .mockResolvedValueOnce({
        success: true,
        data: {
          session: makeSession("s2", "code-agent", "completed", []),
          logs: [],
          executions: [],
        },
      });

    const { result } = renderHook(() => useSessionTrace("s1"));
    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(result.current.trace?.children[0].type).toBe("delegation");
    expect(result.current.trace?.children[0].agentId).toBe("code-agent");
  });

  it("skips internal tool calls (they should not appear as children)", async () => {
    // analyze_intent is internal; shell is not
    const internalLog = makeLog("l1", "tool_call", {
      metadata: { tool_name: "analyze_intent", tool_id: "tid1" },
    });
    const externalLog = makeLog("l2", "tool_call", {
      metadata: { tool_name: "shell", tool_id: "tid2" },
    });
    getLogSession.mockResolvedValue({
      success: true,
      data: {
        session: makeSession("s1"),
        logs: [internalLog, externalLog],
        executions: [],
      },
    });
    const { result } = renderHook(() => useSessionTrace("s1"));
    await waitFor(() => expect(result.current.loading).toBe(false));
    // Only external tool should appear
    const children = result.current.trace?.children ?? [];
    expect(children.every((c) => c.label !== "analyze_intent")).toBe(true);
    expect(children.some((c) => c.label === "shell")).toBe(true);
  });

  it("maps session statuses to TraceNode statuses correctly", async () => {
    for (const [status, expected] of [
      ["running", "running"],
      ["completed", "completed"],
      ["error", "error"],
      ["stopped", "error"],
      ["crashed", "error"],
      ["unknown", "completed"],
    ] as const) {
      getLogSession.mockResolvedValueOnce({
        success: true,
        data: {
          session: makeSession("s1", "root", status, []),
          logs: [],
          executions: [],
        },
      });
      const { result } = renderHook(() => useSessionTrace("s1"));
      await waitFor(() => expect(result.current.loading).toBe(false));
      expect(result.current.trace?.status).toBe(expected);
    }
  });
});
