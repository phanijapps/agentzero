import { describe, it, expect, vi, beforeAll, beforeEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { MemoryRouter, Routes, Route } from "react-router-dom";
import { QuickChat } from "./QuickChat";
import type { QuickChatState } from "./types";
import type { PillState } from "../shared/statusPill";

// A configurable hook mock so tests can drive different UI states.
interface MockHookReturn {
  state: QuickChatState;
  pillState: PillState;
  sendMessage: ReturnType<typeof vi.fn>;
  stopAgent: ReturnType<typeof vi.fn>;
  clearSession: ReturnType<typeof vi.fn>;
}

const mockHookRef: { current: MockHookReturn } = {
  current: makeIdleHook(),
};

function makeIdleHook(): MockHookReturn {
  return {
    state: {
      sessionId: "sess-chat-xyz",
      conversationId: "chat-xyz",
      messages: [],
      status: "idle",
      activeWardName: "stock-analysis",
      artifacts: [],
    },
    pillState: { visible: false, narration: "", suffix: "", category: "neutral", starting: false, swapCounter: 0 },
    sendMessage: vi.fn(),
    stopAgent: vi.fn(),
    clearSession: vi.fn().mockResolvedValue(undefined),
  };
}

vi.mock("./useQuickChat", () => ({
  useQuickChat: () => mockHookRef.current,
}));

// Keep this page-level test focused on opening behavior. The shared viewer's
// network and preview behavior is covered independently.
vi.mock("../chat/ArtifactSlideOut", () => ({
  ArtifactSlideOut: ({ artifact }: { artifact: { fileName: string } }) => (
    <div data-testid="artifact-slideout">{artifact.fileName}</div>
  ),
}));

// jsdom lacks scrollIntoView; polyfill as a no-op so the auto-scroll effect
// doesn't throw during render.
beforeAll(() => {
  if (!Element.prototype.scrollIntoView) {
    Element.prototype.scrollIntoView = () => { /* no-op */ };
  }
});

function renderPage() {
  return render(
    <MemoryRouter initialEntries={["/chat-v2"]}>
      <Routes>
        <Route path="/chat-v2" element={<QuickChat />} />
      </Routes>
    </MemoryRouter>
  );
}

describe("<QuickChat>", () => {
  beforeEach(() => {
    mockHookRef.current = makeIdleHook();
  });

  it("renders the empty state with ward binding", () => {
    renderPage();
    expect(screen.getByText("Quick chat")).toBeTruthy();
    expect(screen.getByText(/bound to stock-analysis/)).toBeTruthy();
  });

  it("shows the ward chip when a ward is active", () => {
    renderPage();
    expect(screen.getAllByText("stock-analysis").length).toBeGreaterThan(0);
  });

  it("does NOT render a ward chip when no ward is active", () => {
    mockHookRef.current = {
      ...makeIdleHook(),
      state: { ...makeIdleHook().state, activeWardName: null },
    };
    const { container } = renderPage();
    expect(container.querySelector(".quick-chat__ward-chip")).toBeNull();
  });

  it("does NOT render a New chat button", () => {
    renderPage();
    expect(screen.queryByText(/New chat/i)).toBeNull();
    expect(screen.queryByTitle(/New chat/i)).toBeNull();
  });

  it("renders conversation messages + inline chips when history is present", () => {
    mockHookRef.current = {
      ...makeIdleHook(),
      state: {
        ...makeIdleHook().state,
        messages: [
          { id: "u1", role: "user", content: "what was z.ai rate limit?", timestamp: 1 },
          {
            id: "a1",
            role: "assistant",
            content: "Per-key semaphore size 3.",
            timestamp: 2,
            chips: [{ id: "c1", kind: "recall", label: "recalled 1" }],
          },
        ],
      },
    };
    renderPage();
    expect(screen.getByText(/z.ai rate limit/)).toBeTruthy();
    expect(screen.getByText(/Per-key semaphore size/)).toBeTruthy();
    expect(screen.getByText("recalled 1")).toBeTruthy();
  });

  it("renders attached files as chips without their server paths", () => {
    mockHookRef.current = {
      ...makeIdleHook(),
      state: {
        ...makeIdleHook().state,
        messages: [{
          id: "u1",
          role: "user",
          content: "Analyze this transcript",
          timestamp: 1,
          attachments: [{
            name: "interview.txt",
            mimeType: "text/plain",
            sizeLabel: "44.7 KB",
          }],
        }],
      },
    };
    renderPage();
    expect(screen.getByTestId("quick-chat-attachment")).toHaveTextContent("interview.txt");
    expect(screen.queryByText(/Documents\/zbot\/temp/)).toBeNull();
  });

  it("shows a Stop button while running and fires stopAgent on click", () => {
    const stopSpy = vi.fn();
    mockHookRef.current = {
      ...makeIdleHook(),
      state: { ...makeIdleHook().state, status: "running" },
      stopAgent: stopSpy,
    };
    renderPage();
    const stopBtn = screen.getByTitle("Stop");
    fireEvent.click(stopBtn);
    expect(stopSpy).toHaveBeenCalled();
  });

  it("renders a visible status pill when pillState.visible is true", () => {
    mockHookRef.current = {
      ...makeIdleHook(),
      pillState: {
        visible: true,
        narration: "Recalling fundamentals",
        suffix: "recall",
        category: "read",
        starting: false,
        swapCounter: 1,
      },
    };
    renderPage();
    expect(screen.getByTestId("status-pill")).toBeTruthy();
    expect(screen.getByText("Recalling fundamentals")).toBeTruthy();
  });

  it("renders a Clear button in the top-right that fires clearSession on confirm", () => {
    const clearSpy = vi.fn().mockResolvedValue(undefined);
    mockHookRef.current = { ...makeIdleHook(), clearSession: clearSpy };
    const confirmStub = vi.spyOn(window, "confirm").mockReturnValue(true);
    renderPage();
    const btn = screen.getByLabelText("Clear chat");
    fireEvent.click(btn);
    expect(confirmStub).toHaveBeenCalled();
    expect(clearSpy).toHaveBeenCalled();
    confirmStub.mockRestore();
  });

  it("Clear button does nothing when the confirm is declined", () => {
    const clearSpy = vi.fn().mockResolvedValue(undefined);
    mockHookRef.current = { ...makeIdleHook(), clearSession: clearSpy };
    const confirmStub = vi.spyOn(window, "confirm").mockReturnValue(false);
    renderPage();
    fireEvent.click(screen.getByLabelText("Clear chat"));
    expect(clearSpy).not.toHaveBeenCalled();
    confirmStub.mockRestore();
  });

  it("Clear button is disabled until sessionId resolves", () => {
    mockHookRef.current = {
      ...makeIdleHook(),
      state: { ...makeIdleHook().state, sessionId: null },
    };
    renderPage();
    const btn = screen.getByLabelText("Clear chat") as HTMLButtonElement;
    expect(btn.disabled).toBe(true);
  });

  it("renders artifact cards with the file name + label", () => {
    mockHookRef.current = {
      ...makeIdleHook(),
      state: {
        ...makeIdleHook().state,
        messages: [{ id: "u1", role: "user", content: "summarize", timestamp: 1 }],
        artifacts: [
          { id: "art-1", fileName: "report.md", fileType: "md", fileSize: 100, label: "summary" },
          { id: "art-2", fileName: "data.csv", fileType: "csv", fileSize: 50 },
        ],
      },
    };
    renderPage();
    expect(screen.getByRole("heading", { name: "Deliverables" })).toBeTruthy();
    expect(screen.getByText("report.md")).toBeTruthy();
    expect(screen.getByText("summary")).toBeTruthy();
    expect(screen.getByText("data.csv")).toBeTruthy();
    expect(screen.getAllByTestId("quick-chat-artifact").length).toBe(2);
  });

  it("clicking an artifact card opens the slide-out viewer", () => {
    mockHookRef.current = {
      ...makeIdleHook(),
      state: {
        ...makeIdleHook().state,
        messages: [{ id: "u1", role: "user", content: "see file", timestamp: 1 }],
        artifacts: [{ id: "art-1", fileName: "open.md", fileType: "md", fileSize: 1 }],
      },
    };
    const { container } = renderPage();
    fireEvent.click(screen.getByTestId("quick-chat-artifact"));
    expect(container.querySelectorAll("[data-testid='quick-chat-artifact']").length).toBe(1);
    expect(screen.getByTestId("artifact-slideout")).toBeTruthy();
  });

  it("does NOT open a slide-out when sessionId is null (defensive guard)", () => {
    mockHookRef.current = {
      ...makeIdleHook(),
      state: {
        ...makeIdleHook().state,
        sessionId: null,
        messages: [{ id: "u1", role: "user", content: "still loading", timestamp: 1 }],
        artifacts: [{ id: "art-1", fileName: "guard.md", fileType: "md", fileSize: 1 }],
      },
    };
    renderPage();
    fireEvent.click(screen.getByTestId("quick-chat-artifact"));
    // Card stays visible, no slideout content beyond the card itself.
    expect(screen.getAllByText("guard.md").length).toBe(1);
  });

  it("renders no Stop button when status is idle", () => {
    renderPage();
    expect(screen.queryByTitle("Stop")).toBeNull();
  });

  it("does not render the artifacts strip when artifacts is empty", () => {
    mockHookRef.current = {
      ...makeIdleHook(),
      state: {
        ...makeIdleHook().state,
        messages: [{ id: "u1", role: "user", content: "hi", timestamp: 1 }],
      },
    };
    const { container } = renderPage();
    expect(container.querySelector("[data-testid='quick-chat-artifacts']")).toBeNull();
  });
});
