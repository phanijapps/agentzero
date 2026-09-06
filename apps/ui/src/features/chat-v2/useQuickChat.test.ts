// =============================================================================
// useQuickChat — hook integration tests against a stubbed Transport.
//
// The hook owns four async paths: bootstrap (init + history + artifacts),
// WS subscribe lifecycle, sendMessage (executeAgent), stopAgent, and
// clearSession. We mock `getTransport` to return a controllable shim and
// drive each path. `useStatusPill` is stubbed to a no-op sink so we don't
// pull the real implementation into the hook test.
// =============================================================================

import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { renderHook, act, waitFor } from "@testing-library/react";

// Mock useStatusPill BEFORE useQuickChat is imported so the hook gets the stub.
vi.mock("../shared/statusPill", () => ({
  useStatusPill: () => ({
    state: { visible: false, narration: "", suffix: "", category: "neutral", starting: false, swapCounter: 0 },
    sink: { push: vi.fn() },
  }),
}));

const transportMock = {
  initChatSession: vi.fn(),
  getSessionMessages: vi.fn(),
  listSessionArtifacts: vi.fn(),
  listSavedSessionSurfaces: vi.fn(),
  subscribeConversation: vi.fn(),
  executeAgent: vi.fn(),
  stopAgent: vi.fn(),
  cancelSession: vi.fn(),
  deleteChatSession: vi.fn(),
};

vi.mock("@/services/transport", async () => {
  const actual = await vi.importActual<Record<string, unknown>>("@/services/transport");
  return {
    ...actual,
    getTransport: async () => transportMock,
  };
});

// Imported lazily after the mocks above are wired.
import { useQuickChat } from "./useQuickChat";

beforeEach(() => {
  for (const fn of Object.values(transportMock)) {
    if (typeof fn === "function" && "mockReset" in fn) (fn as ReturnType<typeof vi.fn>).mockReset();
  }
  // Sensible defaults — individual tests override.
  transportMock.subscribeConversation.mockReturnValue(() => {});
  transportMock.listSessionArtifacts.mockResolvedValue({ success: true, data: [] });
  transportMock.listSavedSessionSurfaces.mockResolvedValue({ success: true, data: [] });
  transportMock.cancelSession.mockResolvedValue({ success: true });
});

afterEach(() => {
  vi.clearAllMocks();
});

describe("useQuickChat — bootstrap", () => {
  it("hydrates with empty messages on a freshly created session", async () => {
    transportMock.initChatSession.mockResolvedValue({
      success: true,
      data: { sessionId: "s1", conversationId: "c1", created: true },
    });

    const { result } = renderHook(() => useQuickChat());

    await waitFor(() => expect(result.current.state.sessionId).toBe("s1"));
    expect(result.current.state.conversationId).toBe("c1");
    expect(result.current.state.messages).toEqual([]);
    // No history fetch on a brand-new session.
    expect(transportMock.getSessionMessages).not.toHaveBeenCalled();
  });

  it("hydrates saved surfaces once for an existing session", async () => {
    // STUB: AC4 — refresh restores the bounded server snapshot.
    transportMock.initChatSession.mockResolvedValue({
      success: true,
      data: { sessionId: "s-saved", conversationId: "c-saved", created: false },
    });
    transportMock.getSessionMessages.mockResolvedValue({ success: true, data: [] });
    const saved = {
      surface_id: "surface-saved",
      catalog_id: "zbot/work-surface/v1",
      components: [],
      data: {},
    };
    transportMock.listSavedSessionSurfaces.mockResolvedValue({
      success: true,
      data: [saved],
    });

    const { result } = renderHook(() => useQuickChat());

    await waitFor(() => expect(result.current.state.sessionId).toBe("s-saved"));
    expect(result.current.surfaces).toEqual([saved]);
    expect(transportMock.listSavedSessionSurfaces).toHaveBeenCalledWith("s-saved");
  });

  it("hydrates from history + artifacts when the session already exists", async () => {
    transportMock.initChatSession.mockResolvedValue({
      success: true,
      data: { sessionId: "s2", conversationId: "c2", created: false },
    });
    transportMock.getSessionMessages.mockResolvedValue({
      success: true,
      data: [
        { id: "m1", role: "user", content: "hi", created_at: "2026-05-05T00:00:00Z" },
        { id: "m2", role: "assistant", content: "hello", created_at: "2026-05-05T00:00:01Z" },
        // filtered out:
        { id: "m3", role: "tool", content: "{}", created_at: "2026-05-05T00:00:02Z" },
        { id: "m4", role: "assistant", content: "[tool calls]", created_at: "2026-05-05T00:00:03Z" },
      ],
    });
    transportMock.listSessionArtifacts.mockResolvedValue({
      success: true,
      data: [{
        id: "art-1",
        fileName: "report.md",
        fileType: "md",
        fileSize: 12,
        label: "summary",
        sessionId: "s2",
        filePath: "report.md",
        isGoalArtifact: true,
        createdAt: "",
      }, {
        id: "art-plan",
        fileName: "working-plan.md",
        sessionId: "s2",
        filePath: "working-plan.md",
        createdAt: "",
        isGoalArtifact: false,
      }, {
        id: "art-legacy",
        fileName: "legacy-output.json",
        sessionId: "s2",
        filePath: "legacy-output.json",
        createdAt: "",
      }],
    });

    const { result } = renderHook(() => useQuickChat());

    await waitFor(() => expect(result.current.state.messages.length).toBe(2));
    expect(result.current.state.messages.map((m) => m.role)).toEqual(["user", "assistant"]);
    expect(result.current.state.artifacts).toEqual([
      expect.objectContaining({ id: "art-1", fileName: "report.md" }),
    ]);
    expect(transportMock.listSessionArtifacts).toHaveBeenCalledWith("s2", {
      goalArtifactsOnly: true,
      limit: 24,
    });
  });

  it("falls back to empty messages when getSessionMessages fails", async () => {
    transportMock.initChatSession.mockResolvedValue({
      success: true,
      data: { sessionId: "s3", conversationId: "c3", created: false },
    });
    transportMock.getSessionMessages.mockResolvedValue({ success: false, error: "boom" });

    const { result } = renderHook(() => useQuickChat());
    await waitFor(() => expect(result.current.state.sessionId).toBe("s3"));
    expect(result.current.state.messages).toEqual([]);
  });

  it("dispatches ERROR status when initChatSession fails", async () => {
    transportMock.initChatSession.mockResolvedValue({ success: false, error: "no init" });

    const { result } = renderHook(() => useQuickChat());
    await waitFor(() => expect(result.current.state.status).toBe("error"));
  });

  it("listSessionArtifacts failure leaves artifacts empty (does not error the hook)", async () => {
    transportMock.initChatSession.mockResolvedValue({
      success: true,
      data: { sessionId: "s4", conversationId: "c4", created: false },
    });
    transportMock.getSessionMessages.mockResolvedValue({ success: true, data: [] });
    transportMock.listSessionArtifacts.mockResolvedValue({ success: false, error: "x" });

    const { result } = renderHook(() => useQuickChat());
    await waitFor(() => expect(result.current.state.sessionId).toBe("s4"));
    expect(result.current.state.artifacts).toEqual([]);
  });
});

describe("useQuickChat — WS subscription lifecycle", () => {
  it("subscribes to the conversationId after hydrate", async () => {
    transportMock.initChatSession.mockResolvedValue({
      success: true,
      data: { sessionId: "s1", conversationId: "c1", created: true },
    });

    renderHook(() => useQuickChat());
    await waitFor(() =>
      expect(transportMock.subscribeConversation).toHaveBeenCalledWith(
        "c1",
        expect.objectContaining({ onEvent: expect.any(Function) }),
      ),
    );
  });

  it("calls the unsubscribe function on unmount", async () => {
    const unsubscribe = vi.fn();
    transportMock.initChatSession.mockResolvedValue({
      success: true,
      data: { sessionId: "s1", conversationId: "c1", created: true },
    });
    transportMock.subscribeConversation.mockReturnValue(unsubscribe);

    const { unmount } = renderHook(() => useQuickChat());
    await waitFor(() => expect(transportMock.subscribeConversation).toHaveBeenCalled());
    unmount();
    await waitFor(() => expect(unsubscribe).toHaveBeenCalled());
  });

  it("replaces an existing surface when surface_updated arrives", async () => {
    transportMock.initChatSession.mockResolvedValue({
      success: true,
      data: { sessionId: "s1", conversationId: "c1", created: true },
    });

    const { result } = renderHook(() => useQuickChat());
    await waitFor(() => expect(transportMock.subscribeConversation).toHaveBeenCalled());
    const subscription = transportMock.subscribeConversation.mock.calls[0][1] as {
      onEvent: (event: Record<string, unknown>) => void;
    };
    const descriptor = {
      surface_id: "surface-1",
      catalog_id: "zbot/work-surface/v1",
      components: [],
      data: { value: 1 },
    };

    act(() => {
      subscription.onEvent({ type: "surface_created", surface: descriptor });
      subscription.onEvent({
        type: "surface_updated",
        surface: { ...descriptor, data: { value: 2 } },
      });
    });

    expect(result.current.surfaces).toEqual([
      expect.objectContaining({ surface: expect.objectContaining({ surface_id: "surface-1", data: { value: 2 } }) }),
    ]);
  });
});

describe("useQuickChat — sendMessage", () => {
  it("calls executeAgent with the right ids + composed prompt", async () => {
    transportMock.initChatSession.mockResolvedValue({
      success: true,
      data: { sessionId: "s1", conversationId: "c1", created: true },
    });
    transportMock.executeAgent.mockResolvedValue({ success: true });

    const { result } = renderHook(() => useQuickChat());
    await waitFor(() => expect(result.current.state.sessionId).toBe("s1"));

    await act(async () => {
      await result.current.sendMessage("hello", []);
    });

    expect(transportMock.executeAgent).toHaveBeenCalledWith("root", "c1", "hello", "s1", "fast");
    expect(result.current.state.messages).toHaveLength(1);
    expect(result.current.state.messages[0]).toMatchObject({ role: "user", content: "hello" });
  });

  it("sets status=error when executeAgent fails", async () => {
    transportMock.initChatSession.mockResolvedValue({
      success: true,
      data: { sessionId: "s1", conversationId: "c1", created: true },
    });
    transportMock.executeAgent.mockResolvedValue({ success: false, error: "rejected" });

    const { result } = renderHook(() => useQuickChat());
    await waitFor(() => expect(result.current.state.sessionId).toBe("s1"));

    await act(async () => {
      await result.current.sendMessage("oops", []);
    });

    await waitFor(() => expect(result.current.state.status).toBe("error"));
  });

  it("is a no-op when text is whitespace only", async () => {
    transportMock.initChatSession.mockResolvedValue({
      success: true,
      data: { sessionId: "s1", conversationId: "c1", created: true },
    });

    const { result } = renderHook(() => useQuickChat());
    await waitFor(() => expect(result.current.state.sessionId).toBe("s1"));

    await act(async () => {
      await result.current.sendMessage("   ", []);
    });
    expect(transportMock.executeAgent).not.toHaveBeenCalled();
    expect(result.current.state.messages).toEqual([]);
  });

  it("is a no-op when there is no sessionId yet", async () => {
    // initChatSession never resolves → state.sessionId stays null.
    transportMock.initChatSession.mockReturnValue(new Promise(() => {}));

    const { result } = renderHook(() => useQuickChat());
    await act(async () => {
      await result.current.sendMessage("hi", []);
    });
    expect(transportMock.executeAgent).not.toHaveBeenCalled();
  });
});

describe("useQuickChat — stopAgent", () => {
  it("cancels the exact running session", async () => {
    transportMock.initChatSession.mockResolvedValue({
      success: true,
      data: { sessionId: "s1", conversationId: "c1", created: true },
    });
    transportMock.executeAgent.mockResolvedValue({ success: true });

    const { result } = renderHook(() => useQuickChat());
    await waitFor(() => expect(result.current.state.sessionId).toBe("s1"));

    // status starts idle → stop is a no-op.
    await act(async () => {
      await result.current.stopAgent();
    });
    expect(transportMock.cancelSession).not.toHaveBeenCalled();

    // Drive into running by simulating an agent_started event through the
    // captured WS handler.
    const onEvent = transportMock.subscribeConversation.mock.calls[0][1].onEvent;
    act(() => {
      onEvent({
        type: "agent_started",
        execution_id: "exec-1",
        conversation_id: "c1",
        agent_id: "root",
      } as unknown as Parameters<typeof onEvent>[0]);
    });
    await waitFor(() => expect(result.current.state.status).toBe("running"));

    await act(async () => {
      await result.current.stopAgent();
    });
    expect(transportMock.cancelSession).toHaveBeenCalledWith("s1", "c1");
  });
});

describe("useQuickChat — clearSession", () => {
  it("deletes + re-bootstraps a fresh session", async () => {
    transportMock.initChatSession
      .mockResolvedValueOnce({
        success: true,
        data: { sessionId: "s1", conversationId: "c1", created: true },
      })
      .mockResolvedValueOnce({
        success: true,
        data: { sessionId: "s2", conversationId: "c2", created: true },
      });
    transportMock.deleteChatSession.mockResolvedValue({ success: true });

    const { result } = renderHook(() => useQuickChat());
    await waitFor(() => expect(result.current.state.sessionId).toBe("s1"));
    const subscription = transportMock.subscribeConversation.mock.calls[0][1] as {
      onEvent: (event: Record<string, unknown>) => void;
    };
    act(() => {
      subscription.onEvent({
        type: "surface_created",
        surface: {
          surface_id: "old-surface",
          catalog_id: "zbot/work-surface/v1",
          components: [],
          data: {},
        },
      });
    });
    expect(result.current.surfaces).toHaveLength(1);

    await act(async () => {
      await result.current.clearSession();
    });

    expect(transportMock.deleteChatSession).toHaveBeenCalled();
    await waitFor(() => expect(result.current.state.sessionId).toBe("s2"));
    expect(result.current.surfaces).toEqual([]);
  });

  it("dispatches ERROR when deleteChatSession fails", async () => {
    transportMock.initChatSession.mockResolvedValue({
      success: true,
      data: { sessionId: "s1", conversationId: "c1", created: true },
    });
    transportMock.deleteChatSession.mockResolvedValue({ success: false, error: "denied" });

    const { result } = renderHook(() => useQuickChat());
    await waitFor(() => expect(result.current.state.sessionId).toBe("s1"));

    await act(async () => {
      await result.current.clearSession();
    });
    await waitFor(() => expect(result.current.state.status).toBe("error"));
  });

  it("dispatches ERROR when bootstrap-after-delete returns null", async () => {
    transportMock.initChatSession
      .mockResolvedValueOnce({
        success: true,
        data: { sessionId: "s1", conversationId: "c1", created: true },
      })
      .mockResolvedValueOnce({ success: false, error: "second init failed" });
    transportMock.deleteChatSession.mockResolvedValue({ success: true });

    const { result } = renderHook(() => useQuickChat());
    await waitFor(() => expect(result.current.state.sessionId).toBe("s1"));

    await act(async () => {
      await result.current.clearSession();
    });
    await waitFor(() => expect(result.current.state.status).toBe("error"));
  });
});
