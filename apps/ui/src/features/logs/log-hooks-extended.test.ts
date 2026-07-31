// ============================================================================
// log-hooks — extended coverage for useSessionDetail and useAutoRefresh
// ============================================================================

import { describe, it, expect, vi, beforeEach } from "vitest";
import { renderHook, waitFor, act } from "@testing-library/react";
import type { Transport } from "@/services/transport";
import type { LogSession, SessionDetail } from "@/services/transport/types";

const listLogSessions = vi.fn<Transport["listLogSessions"]>();
const getLogSession = vi.fn<Transport["getLogSession"]>();

vi.mock("@/services/transport", () => ({
  getTransport: async () => ({ listLogSessions, getLogSession }),
}));

import { useLogSessions, useSessionDetail, useAutoRefresh } from "./log-hooks";

function makeSession(overrides: Partial<LogSession> = {}): LogSession {
  return {
    session_id: "sess-1",
    conversation_id: "conv-1",
    agent_id: "root",
    agent_name: "root",
    title: null,
    started_at: new Date().toISOString(),
    ended_at: null,
    status: "completed",
    token_count: 0,
    tool_call_count: 0,
    error_count: 0,
    duration_ms: 100,
    parent_session_id: undefined,
    child_session_ids: [],
    mode: null,
    ...overrides,
  } as LogSession;
}

function makeDetail(overrides: Partial<SessionDetail> = {}): SessionDetail {
  return {
    session: makeSession(),
    logs: [],
    children: [],
    ...overrides,
  } as unknown as SessionDetail;
}

// ─────────────────────────────────────────────────────────────────────────────
// useLogSessions — error and loading behaviour
// ─────────────────────────────────────────────────────────────────────────────

describe("useLogSessions", () => {
  beforeEach(() => {
    listLogSessions.mockReset();
    getLogSession.mockReset();
  });

  it("starts in loading state", async () => {
    listLogSessions.mockResolvedValue({ success: true, data: [] });
    const { result } = renderHook(() => useLogSessions());
    expect(result.current.loading).toBe(true);
  });

  it("sets error when transport returns failure", async () => {
    listLogSessions.mockResolvedValue({
      success: false,
      error: "Server error",
    });
    const { result } = renderHook(() => useLogSessions());
    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(result.current.error).toBe("Server error");
    expect(result.current.sessions).toHaveLength(0);
  });

  it("sets error fallback message when transport returns no error string", async () => {
    listLogSessions.mockResolvedValue({ success: false });
    const { result } = renderHook(() => useLogSessions());
    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(result.current.error).toBe("Failed to load sessions");
  });

  it("sets error on thrown exception", async () => {
    listLogSessions.mockRejectedValue(new Error("Network failure"));
    const { result } = renderHook(() => useLogSessions());
    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(result.current.error).toBe("Network failure");
  });

  it("sets error 'Unknown error' for non-Error thrown values", async () => {
    listLogSessions.mockRejectedValue("oops");
    const { result } = renderHook(() => useLogSessions());
    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(result.current.error).toBe("Unknown error");
  });

  it("loads sessions successfully", async () => {
    listLogSessions.mockResolvedValue({
      success: true,
      data: [makeSession({ session_id: "s1" }), makeSession({ session_id: "s2" })],
    });
    const { result } = renderHook(() => useLogSessions());
    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(result.current.sessions).toHaveLength(2);
  });

  it("refetch increments tick causing reload", async () => {
    listLogSessions.mockResolvedValue({ success: true, data: [] });
    const { result } = renderHook(() => useLogSessions());
    await waitFor(() => expect(result.current.loading).toBe(false));

    const callsBefore = listLogSessions.mock.calls.length;
    act(() => result.current.refetch());
    await waitFor(() => {
      expect(listLogSessions.mock.calls.length).toBeGreaterThan(callsBefore);
    });
  });

  it("passes filters to transport", async () => {
    listLogSessions.mockResolvedValue({ success: true, data: [] });
    const filter = { agent_id: "agent-1", level: "error" as const, limit: 50, root_only: true };
    const { result } = renderHook(() => useLogSessions(filter));
    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(listLogSessions).toHaveBeenCalledWith({
      agent_id: "agent-1",
      level: "error",
      limit: 50,
      root_only: true,
    });
  });
});

// ─────────────────────────────────────────────────────────────────────────────
// useSessionDetail
// ─────────────────────────────────────────────────────────────────────────────

describe("useSessionDetail", () => {
  beforeEach(() => {
    getLogSession.mockReset();
  });

  it("returns null detail and loading=false when sessionId is null", () => {
    const { result } = renderHook(() => useSessionDetail(null));
    expect(result.current.detail).toBeNull();
    expect(result.current.loading).toBe(false);
  });

  it("fetches and returns detail when sessionId is set", async () => {
    const detail = makeDetail();
    getLogSession.mockResolvedValue({ success: true, data: detail });

    const { result } = renderHook(() => useSessionDetail("sess-1"));
    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(result.current.detail).toBe(detail);
  });

  it("leaves detail null when transport returns failure", async () => {
    getLogSession.mockResolvedValue({ success: false, error: "not found" });
    const { result } = renderHook(() => useSessionDetail("sess-missing"));
    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(result.current.detail).toBeNull();
  });

  it("handles thrown exception gracefully", async () => {
    getLogSession.mockRejectedValue(new Error("boom"));
    const { result } = renderHook(() => useSessionDetail("sess-boom"));
    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(result.current.detail).toBeNull();
  });

  it("clears detail when sessionId changes to null", async () => {
    const detail = makeDetail();
    getLogSession.mockResolvedValue({ success: true, data: detail });

    const initialProps: { id: string | null } = { id: "sess-1" };
    const { result, rerender } = renderHook(
      ({ id }: { id: string | null }) => useSessionDetail(id),
      { initialProps },
    );
    await waitFor(() => expect(result.current.detail).not.toBeNull());

    rerender({ id: null });
    expect(result.current.detail).toBeNull();
  });
});

// ─────────────────────────────────────────────────────────────────────────────
// useAutoRefresh
// ─────────────────────────────────────────────────────────────────────────────

describe("useAutoRefresh", () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });

  it("does not poll when no sessions are running", () => {
    const refetch = vi.fn();
    const sessions = [makeSession({ status: "completed" })];
    renderHook(() => useAutoRefresh(sessions, refetch));

    vi.advanceTimersByTime(10000);
    expect(refetch).not.toHaveBeenCalled();
  });

  it("polls every 5s when a session has status 'running'", () => {
    const refetch = vi.fn();
    const sessions = [makeSession({ status: "running" })];
    renderHook(() => useAutoRefresh(sessions, refetch));

    vi.advanceTimersByTime(5000);
    expect(refetch).toHaveBeenCalledTimes(1);

    vi.advanceTimersByTime(5000);
    expect(refetch).toHaveBeenCalledTimes(2);
  });

  it("stops polling when sessions no longer have running status", () => {
    const refetch = vi.fn();
    const { rerender } = renderHook(
      ({ sessions }: { sessions: LogSession[] }) => useAutoRefresh(sessions, refetch),
      { initialProps: { sessions: [makeSession({ status: "running" })] } },
    );

    vi.advanceTimersByTime(5000);
    expect(refetch).toHaveBeenCalledTimes(1);

    rerender({ sessions: [makeSession({ status: "completed" })] });
    vi.advanceTimersByTime(5000);
    // Still 1 — polling stopped
    expect(refetch).toHaveBeenCalledTimes(1);
  });

  it("cleans up interval on unmount", () => {
    const refetch = vi.fn();
    const sessions = [makeSession({ status: "running" })];
    const { unmount } = renderHook(() => useAutoRefresh(sessions, refetch));

    unmount();
    vi.advanceTimersByTime(10000);
    expect(refetch).not.toHaveBeenCalled();
  });
});
