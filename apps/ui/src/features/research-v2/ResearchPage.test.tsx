import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter, Routes, Route, useLocation } from "react-router-dom";
import { ResearchPage } from "./ResearchPage";
import type { ResearchSessionState } from "./types";
import type { PillState } from "../shared/statusPill";

type DeleteSessionResult =
  | { success: true; data?: void }
  | { success: false; error: string };

const {
  toastErrorMock,
  listArtifactsMock,
  deleteSessionMock,
  listLogSessionsMock,
  getVaultTreeMock,
  searchVaultFilesMock,
  getVaultFileMock,
  getSessionStateMock,
} = vi.hoisted(() => ({
  toastErrorMock: vi.fn(),
  listArtifactsMock: vi.fn(),
  deleteSessionMock: vi.fn<(sessionId: string) => Promise<DeleteSessionResult>>(),
  listLogSessionsMock: vi.fn(),
  getVaultTreeMock: vi.fn(),
  searchVaultFilesMock: vi.fn(),
  getVaultFileMock: vi.fn(),
  getSessionStateMock: vi.fn(),
}));

vi.mock("@/services/transport", async () => {
  const actual = await vi.importActual<Record<string, unknown>>("@/services/transport");
  return {
    ...actual,
    getTransport: async () => ({
      listSessionArtifacts: listArtifactsMock,
      getArtifactContentUrl: () => "about:blank",
      deleteSession: deleteSessionMock,
      listLogSessions: listLogSessionsMock,
      getVaultTree: getVaultTreeMock,
      searchVaultFiles: searchVaultFilesMock,
      getVaultFile: getVaultFileMock,
      getSessionState: getSessionStateMock,
    }),
  };
});

vi.mock("sonner", () => ({
  toast: { error: toastErrorMock },
}));

// ResearchPage owns artifact selection; the shared viewer's content request
// and preview states are covered in ArtifactSlideOut.test.tsx.
vi.mock("../chat/ArtifactSlideOut", () => ({
  ArtifactSlideOut: ({ artifact }: { artifact: { fileName: string } }) => (
    <div data-testid="artifact-slideout">{artifact.fileName}</div>
  ),
}));

interface MockResearchHook {
  state: ResearchSessionState;
  surfaces?: import("@/services/transport/types").SavedSurface[];
  pillState: PillState;
  wardVaultRevision: number;
  sendMessage: ReturnType<typeof vi.fn>;
  stopAgent: ReturnType<typeof vi.fn>;
  startNewResearch: ReturnType<typeof vi.fn>;
  toggleThinking: ReturnType<typeof vi.fn>;
  getFullArtifact: ReturnType<typeof vi.fn>;
}

interface MockListHook {
  sessions: Array<{ id: string; title: string; status: "running" | "complete" | "crashed" | "paused"; wardName: string | null; updatedAt: number }>;
  loading: boolean;
  refresh: ReturnType<typeof vi.fn>;
  deleteSession: ReturnType<typeof vi.fn>;
}

interface UseSessionsListOptionsMock {
  onAfterDelete?: (id: string) => void;
}

const researchRef: { current: MockResearchHook } = { current: makeIdleResearch() };
const listRef: { current: MockListHook } = {
  current: { sessions: [], loading: false, refresh: vi.fn(), deleteSession: vi.fn() },
};
// Captured so tests can simulate "server delete completed, fire onAfterDelete"
// without coupling to the real useSessionsList implementation.
const lastListOptsRef: { current: UseSessionsListOptionsMock } = { current: {} };

function makeIdleResearch(): MockResearchHook {
  const state: ResearchSessionState = {
    sessionId: null,
    conversationId: null,
    title: "",
    status: "idle",
    wardId: null,
    wardName: null,
    rootExecutionId: null,
    turns: [],
    pendingUserTurn: null,
    intentAnalyzing: false,
    intentClassification: null,
    planPath: null,
    artifacts: [],
  };
  const pillState: PillState = {
    visible: false,
    narration: "",
    suffix: "",
    category: "neutral",
    starting: false,
    swapCounter: 0,
  };
  return {
    state,
    surfaces: [],
    pillState,
    wardVaultRevision: 0,
    sendMessage: vi.fn(),
    stopAgent: vi.fn(),
    startNewResearch: vi.fn(),
    toggleThinking: vi.fn(),
    getFullArtifact: vi.fn(),
  };
}

vi.mock("./useResearchSession", () => ({
  useResearchSession: () => researchRef.current,
}));

vi.mock("./useSessionsList", () => ({
  useSessionsList: (opts?: UseSessionsListOptionsMock) => {
    lastListOptsRef.current = opts ?? {};
    return listRef.current;
  },
}));

function LocationProbe() {
  const location = useLocation();
  return <div data-testid="location">{location.pathname + location.search}</div>;
}

function renderPage() {
  return render(
    <MemoryRouter initialEntries={["/research-v2"]}>
      <Routes>
        <Route path="/research-v2" element={<ResearchPage />} />
        <Route path="/research-v2/:id" element={<ResearchPage />} />
        <Route path="/vault" element={<LocationProbe />} />
      </Routes>
    </MemoryRouter>
  );
}

describe("surface timeline attribution", () => {
  function turn(id: string, userText: string, createdAt: string, executionId?: string) {
    return {
      id,
      index: 0,
      executionId,
      userMessage: { id, content: userText, createdAt },
      subagents: [],
      assistantText: `answer ${id}`,
      assistantStreaming: "",
      timeline: [],
      startedAt: createdAt,
      endedAt: null,
      durationMs: null,
      status: "completed" as const,
    };
  }
  function surface(surfaceId: string, title: string, createdAt?: string, executionId = "exec-root") {
    return {
      execution_id: executionId,
      session_id: "sess-1",
      created_at: createdAt,
      surface: {
        surface_id: surfaceId,
        catalog_id: "zbot/work-surface/v1" as const,
        components: [{ id: "c1", type: "Callout" as const, props: { message_path: "/m" } }],
        data: { m: title },
      },
    };
  }

  it("places surfaces by time window when one root execution spans turns", () => {
    // sess-fe38cfc3 shape: both turns share the root exec id (continuations),
    // surfaces created an hour apart must land under their own turns.
    researchRef.current = {
      ...makeIdleResearch(),
      state: {
        ...makeIdleResearch().state,
        sessionId: "sess-1",
        turns: [
          turn("turn-1", "analyze the paper", "2026-08-23T19:06:34Z", "exec-root"),
          turn("turn-2", "give me an example", "2026-08-23T20:10:02Z", "exec-root"),
        ],
      },
      surfaces: [
        surface("repo0", "Repo0 overview", "2026-08-23T19:12:10Z"),
        surface("ebikes", "Ebike example", "2026-08-23T20:13:04Z"),
      ],
    };
    renderPage();
    // The DOM order must be: turn1, surface1, turn2, surface2.
    const col = document.querySelector(".research-page__column")!;
    const classes = [...col.children].map((c) => c.className.toString());
    expect(classes).toEqual(["session-turn", "a2ui-surface", "session-turn", "a2ui-surface"]);
  });

  it("falls back to execution-id matching for legacy rows without created_at", () => {
    researchRef.current = {
      ...makeIdleResearch(),
      state: {
        ...makeIdleResearch().state,
        sessionId: "sess-1",
        turns: [
          turn("turn-1", "first ask", "2026-08-23T19:00:00Z", "exec-a"),
          turn("turn-2", "second ask", "2026-08-23T20:00:00Z", "exec-b"),
        ],
      },
      surfaces: [surface("legacy", "Legacy surface", undefined, "exec-b")],
    };
    renderPage();
    const col = document.querySelector(".research-page__column")!;
    expect(col.textContent).toContain("Legacy surface");
  });
});

describe("<ResearchPage>", () => {
  beforeEach(() => {
    researchRef.current = makeIdleResearch();
    listRef.current = { sessions: [], loading: false, refresh: vi.fn(), deleteSession: vi.fn() };
    lastListOptsRef.current = {};
    toastErrorMock.mockClear();
    listArtifactsMock.mockClear();
    listArtifactsMock.mockResolvedValue({ success: true, data: [] });
    deleteSessionMock.mockClear();
    deleteSessionMock.mockResolvedValue({ success: true });
    listLogSessionsMock.mockClear();
    listLogSessionsMock.mockResolvedValue({ success: true, data: [] });
    getVaultTreeMock.mockClear();
    getVaultTreeMock.mockImplementation((_wardId: string, path = "") => Promise.resolve({
      success: true,
      data: {
        ward_id: "stock-analysis",
        path,
        truncated: false,
        children: path === "" ? [
          {
            ward_id: "stock-analysis",
            path: "reports",
            name: "reports",
            kind: "directory",
            previewable: false,
          },
          {
            ward_id: "stock-analysis",
            path: "deck.ppt",
            name: "deck.ppt",
            kind: "file",
            extension: "ppt",
            size: 128,
            previewable: false,
          },
        ] : [],
      },
    }));
    searchVaultFilesMock.mockClear();
    searchVaultFilesMock.mockResolvedValue({
      success: true,
      data: {
        ward_id: "stock-analysis",
        query: "valuation",
        truncated: false,
        matches: [
          {
            ward_id: "stock-analysis",
            path: "reports/valuation.md",
            name: "valuation.md",
            kind: "file",
            extension: "md",
            size: 11,
            previewable: true,
          },
        ],
      },
    });
    getVaultFileMock.mockClear();
    getVaultFileMock.mockResolvedValue({
      success: true,
      data: {
        kind: "text",
        ward_id: "stock-analysis",
        path: "reports/valuation.md",
        name: "valuation.md",
        extension: "md",
        size: 11,
        content: "# Valuation",
      },
    });
    getSessionStateMock.mockClear();
    getSessionStateMock.mockResolvedValue({ success: true, data: { intentAnalysis: null } });
    // window.confirm is a browser-level primitive — force-accept so the
    // onAfterDelete code path is exercised without hitting jsdom's "confirm
    // is not implemented" stub.
    window.confirm = vi.fn(() => true);
  });

  it("renders the HeroInput landing when session has no content", () => {
    renderPage();
    // HeroInput surfaces the z-Bot brand + the prompt placeholder.
    expect(screen.getByText("z-Bot")).toBeTruthy();
    expect(screen.getByPlaceholderText(/What would you like to work on/i)).toBeTruthy();
  });

  it("has a drawer-toggle button that is accessible by label", () => {
    renderPage();
    expect(screen.getByLabelText("Open sessions")).toBeTruthy();
  });

  it("toggles the drawer open when ☰ is clicked", () => {
    renderPage();
    fireEvent.click(screen.getByLabelText("Open sessions"));
    expect(screen.getByLabelText("Research sessions")).toBeTruthy();
  });

  it("renders the ward chip ONLY when activeWardName is set", () => {
    renderPage();
    // no ward yet → chip absent
    expect(screen.queryByText("stock-analysis")).toBeNull();

    researchRef.current = {
      ...makeIdleResearch(),
      state: {
        ...makeIdleResearch().state,
        wardId: "stock-analysis",
        wardName: "stock-analysis",
      },
    };
    const { container } = renderPage();
    expect(container.textContent).toContain("stock-analysis");
  });

  it("keeps ward context in the embedded explorer instead of a duplicate header link", () => {
    researchRef.current = {
      ...makeIdleResearch(),
      state: {
        ...makeIdleResearch().state,
        wardId: "stock-analysis",
        wardName: "stock-analysis",
      },
    };
    renderPage();
    expect(screen.queryByRole("button", { name: /open ward in vault/i })).toBeNull();
    expect(screen.getByLabelText("Research ward filesystem")).toBeTruthy();
  });

  it("does not render or load the ward Vault explorer before a ward exists", () => {
    researchRef.current = {
      ...makeIdleResearch(),
      state: { ...makeIdleResearch().state, sessionId: "sess-1" },
    };
    renderPage();

    expect(screen.queryByLabelText("Research ward filesystem")).toBeNull();
    expect(getVaultTreeMock).not.toHaveBeenCalled();
    expect(searchVaultFilesMock).not.toHaveBeenCalled();
    expect(getVaultFileMock).not.toHaveBeenCalled();
  });

  it("renders the ward-scoped Vault explorer and keeps sibling wards out of Research", async () => {
    researchRef.current = {
      ...makeIdleResearch(),
      state: {
        ...makeIdleResearch().state,
        sessionId: "sess-1",
        wardId: "stock-analysis",
        wardName: "stock-analysis",
      },
    };

    renderPage();

    expect(await screen.findByLabelText("Research ward filesystem")).toBeTruthy();
    await waitFor(() => expect(getVaultTreeMock).toHaveBeenCalledWith("stock-analysis", ""));
    expect(screen.getAllByText("stock-analysis").length).toBeGreaterThanOrEqual(1);
    expect(screen.queryByText("research-lab")).toBeNull();
  });

  it("keeps a just-created ward pending instead of surfacing the first root 404", async () => {
    getVaultTreeMock.mockResolvedValueOnce({
      success: false,
      error: "HTTP 404: Not Found",
    });
    researchRef.current = {
      ...makeIdleResearch(),
      state: {
        ...makeIdleResearch().state,
        sessionId: "sess-1",
        wardId: "fresh-ward",
        wardName: "fresh-ward",
      },
    };

    renderPage();

    expect(await screen.findByText("Waiting for ward files...")).toBeTruthy();
    expect(screen.queryByText("HTTP 404: Not Found")).toBeNull();
  });

  it("refreshes the ward explorer when the hook reports a vault revision change", async () => {
    const base = makeIdleResearch();
    researchRef.current = {
      ...base,
      state: {
        ...base.state,
        sessionId: "sess-1",
        wardId: "stock-analysis",
        wardName: "stock-analysis",
      },
      wardVaultRevision: 0,
    };
    const { rerender } = renderPage();
    await waitFor(() => expect(getVaultTreeMock).toHaveBeenCalledTimes(1));

    researchRef.current = {
      ...researchRef.current,
      wardVaultRevision: 1,
    };
    rerender(
      <MemoryRouter initialEntries={["/research-v2"]}>
        <Routes>
          <Route path="/research-v2" element={<ResearchPage />} />
          <Route path="/research-v2/:id" element={<ResearchPage />} />
          <Route path="/vault" element={<LocationProbe />} />
        </Routes>
      </MemoryRouter>
    );

    await waitFor(() => expect(getVaultTreeMock).toHaveBeenCalledTimes(2));
    expect(getVaultTreeMock).toHaveBeenLastCalledWith("stock-analysis", "");
  });

  it("fuzzy-searches only the Research ward and opens Markdown in a Vault slide-out", async () => {
    const user = userEvent.setup();
    researchRef.current = {
      ...makeIdleResearch(),
      state: {
        ...makeIdleResearch().state,
        sessionId: "sess-1",
        wardId: "stock-analysis",
        wardName: "stock-analysis",
      },
    };

    renderPage();

    await user.type(await screen.findByRole("searchbox", { name: "Fuzzy search files in stock-analysis" }), "valuation");
    await waitFor(() => expect(searchVaultFilesMock).toHaveBeenCalledWith("stock-analysis", "valuation", 30));
    await user.click(await screen.findByRole("button", { name: /valuation\.md/i }));

    await waitFor(() => expect(getVaultFileMock).toHaveBeenCalledWith("stock-analysis", "reports/valuation.md"));
    expect(await screen.findByRole("heading", { name: "Valuation" })).toBeTruthy();
    expect(screen.getByPlaceholderText("Type a message...")).toBeTruthy();

    await user.click(screen.getByRole("button", { name: "Close preview" }));
    expect(screen.queryByRole("heading", { name: "Valuation" })).toBeNull();
    expect(screen.getByPlaceholderText("Type a message...")).toBeTruthy();
  });

  it("shows non-previewable ward files without reading file content", async () => {
    const user = userEvent.setup();
    researchRef.current = {
      ...makeIdleResearch(),
      state: {
        ...makeIdleResearch().state,
        sessionId: "sess-1",
        wardId: "stock-analysis",
        wardName: "stock-analysis",
      },
    };

    renderPage();

    await user.click(await screen.findByRole("button", { name: /deck\.ppt/i }));
    expect(await screen.findByText("Preview not available for .ppt files.")).toBeTruthy();
    expect(getVaultFileMock).not.toHaveBeenCalled();
  });

  it("collapses and expands the Research ward explorer without losing search state", async () => {
    const user = userEvent.setup();
    researchRef.current = {
      ...makeIdleResearch(),
      state: {
        ...makeIdleResearch().state,
        sessionId: "sess-1",
        wardId: "stock-analysis",
        wardName: "stock-analysis",
      },
    };

    renderPage();

    const search = await screen.findByRole("searchbox", { name: "Fuzzy search files in stock-analysis" });
    await user.type(search, "valuation");
    await user.click(screen.getByRole("button", { name: "Collapse vault explorer" }));
    await user.click(screen.getByRole("button", { name: /expand ward filesystem/i }));

    expect(screen.getByRole("searchbox", { name: "Fuzzy search files in stock-analysis" })).toHaveValue("valuation");
  });

  it("keeps collapsed ward controls, the thread, artifacts, and composer in document flow", async () => {
    const user = userEvent.setup();
    researchRef.current = {
      ...makeIdleResearch(),
      state: {
        ...makeIdleResearch().state,
        sessionId: "sess-1",
        wardId: "stock-analysis",
        wardName: "stock-analysis",
        artifacts: [{ id: "artifact-1", fileName: "handoff.md", fileType: "md" }],
        turns: [{
          id: "turn-1",
          index: 0,
          userMessage: { id: "message-1", content: "Keep this long research thread reachable", createdAt: "2026-07-15T00:00:00.000Z" },
          subagents: [],
          assistantText: "The active thread remains available after the filesystem rail is collapsed.",
          assistantStreaming: "",
          timeline: [],
          status: "completed",
          startedAt: "2026-07-15T00:00:00.000Z",
          endedAt: "2026-07-15T00:01:00.000Z",
          durationMs: 60_000,
        }],
      },
    };

    const { container } = renderPage();
    await user.click(await screen.findByRole("button", { name: "Collapse vault explorer" }));

    const body = container.querySelector(".research-page__body");
    const expand = screen.getByRole("button", { name: /expand ward filesystem/i });
    const column = container.querySelector(".research-page__column");
    expect(body).toBeTruthy();
    expect(column).toBeTruthy();
    expect(expand.compareDocumentPosition(column as Node) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(screen.getAllByText("Keep this long research thread reachable").length).toBeGreaterThanOrEqual(1);
    expect(screen.getByRole("button", { name: "Open artifact handoff.md" })).toBeTruthy();
    expect(screen.getByPlaceholderText("Type a message...")).toBeTruthy();
  });

  it("shows Stop button only while running and fires stopAgent on click", () => {
    // Idle: no Stop button
    renderPage();
    expect(screen.queryByTitle("Stop")).toBeNull();

    const stopSpy = vi.fn();
    researchRef.current = {
      ...makeIdleResearch(),
      state: { ...makeIdleResearch().state, status: "running" },
      stopAgent: stopSpy,
    };
    renderPage();
    const btn = screen.getByTitle("Stop");
    fireEvent.click(btn);
    expect(stopSpy).toHaveBeenCalled();
  });

  it("New research button fires startNewResearch", () => {
    const newSpy = vi.fn();
    researchRef.current = {
      ...makeIdleResearch(),
      startNewResearch: newSpy,
      state: { ...makeIdleResearch().state, sessionId: "sess-1" },
    };
    renderPage();
    // /New research/ also appears as the title placeholder — scope to the button.
    fireEvent.click(screen.getByRole("button", { name: /New research/ }));
    expect(newSpy).toHaveBeenCalled();
  });

  it("renders a user-query bubble when state.turns has an open turn", () => {
    researchRef.current = {
      ...makeIdleResearch(),
      state: {
        ...makeIdleResearch().state,
        sessionId: "sess-1",
        turns: [
          {
            id: "turn-m1",
            index: 0,
            userMessage: {
              id: "m1",
              content: "what's the Q4 outlook?",
              createdAt: "2026-04-19T00:00:00.000Z",
            },
            subagents: [],
            assistantText: null,
            assistantStreaming: "",
            timeline: [],
            status: "running",
            startedAt: "2026-04-19T00:00:00.000Z",
            endedAt: null,
            durationMs: null,
          },
        ],
      },
    };
    renderPage();
    // Bubble and derived title both carry the user message — 2 matches expected.
    expect(screen.getAllByText(/Q4 outlook/).length).toBeGreaterThanOrEqual(1);
  });

  it("renders the status pill when pillState.visible is true", () => {
    researchRef.current = {
      ...makeIdleResearch(),
      state: { ...makeIdleResearch().state, sessionId: "sess-1" },
      pillState: {
        visible: true,
        narration: "Running shell",
        suffix: "ls -la ~",
        category: "read",
        starting: false,
        swapCounter: 1,
      },
    };
    renderPage();
    expect(screen.getByTestId("status-pill")).toBeTruthy();
    expect(screen.getByText("Running shell")).toBeTruthy();
  });

  it("uses the compact active-goal masthead instead of a Research metric strip", () => {
    researchRef.current = {
      ...makeIdleResearch(),
      state: {
        ...makeIdleResearch().state,
        sessionId: "sess-1234567890abcdef",
        wardId: "stock-analysis",
        wardName: "stock-analysis",
      },
      pillState: {
        visible: true,
        narration: "Reading ward files",
        suffix: "find docs -maxdepth 2",
        category: "read",
        starting: false,
        swapCounter: 1,
      },
    };

    const { container } = renderPage();
    const header = container.querySelector(".research-page__header");
    expect(header).toBeTruthy();
    expect(within(header as HTMLElement).getByText("Research / active goal")).toBeTruthy();
    expect(within(header as HTMLElement).getByText(/Ward: stock-analysis/)).toBeTruthy();
    expect(within(header as HTMLElement).getByTestId("status-pill")).toBeTruthy();
    expect(container.querySelector(".research-page__pill-strip")).toBeNull();
    expect(screen.queryByText("Your attention")).toBeNull();
  });

  it("shows intent analysing muted line when intentAnalyzing=true", () => {
    researchRef.current = {
      ...makeIdleResearch(),
      state: {
        ...makeIdleResearch().state,
        sessionId: "sess-1",
        intentAnalyzing: true,
      },
    };
    renderPage();
    const intentToggle = screen.getByRole("button", { name: /collapse intent analysis/i });
    expect(intentToggle).toHaveAttribute("aria-expanded", "true");
    expect(screen.getByText(/analyzing intent/i)).toBeTruthy();
  });

  it("renders nested subagent cards under their parent (root → child → grandchild)", () => {
    const now = 1000;
    const child = {
      id: "child-1",
      agentId: "planner-agent",
      parentExecutionId: "root-1",
      startedAt: now + 200,
      completedAt: now + 300,
      status: "completed" as const,
      wardId: "w1",
      request: "Plan this goal.",
      timeline: [{
        id: "history-recall",
        at: now + 210,
        kind: "tool_call" as const,
        text: "recall",
        toolName: "recall",
      }],
      tokenCount: 10,
      respond: "child body",
      respondStreaming: "",
      thinkingExpanded: false,
      errorMessage: null,
    };
    const grandchild = {
      ...child,
      id: "grand-1",
      agentId: "builder-agent",
      parentExecutionId: "child-1",
      startedAt: now + 400,
      respond: "grand body",
      request: "Run builder step.",
    };
    researchRef.current = {
      ...makeIdleResearch(),
      state: {
        ...makeIdleResearch().state,
        sessionId: "sess-1",
        rootExecutionId: "root-1",
        turns: [
          {
            id: "turn-u1",
            index: 0,
            userMessage: {
              id: "u1",
              content: "do the thing",
              createdAt: "2026-04-19T00:00:00.000Z",
            },
            subagents: [child, grandchild],
            assistantText: "root body",
            assistantStreaming: "",
            timeline: [],
            status: "completed",
            startedAt: "2026-04-19T00:00:00.000Z",
            endedAt: "2026-04-19T00:01:00.000Z",
            durationMs: 60_000,
          },
        ],
      },
    };
    const { container } = renderPage();

    // Root assistant area renders inside the SessionTurnBlock.
    expect(container.querySelector(".research-msg--assistant")).toBeTruthy();

    // Both subagent cards present, and grand-child is nested under child.
    const planner = container.querySelector('.subagent-card[data-parent="root-1"]');
    expect(planner).toBeTruthy();
    const builder = container.querySelector('.subagent-card[data-parent="child-1"]');
    expect(builder).toBeTruthy();
    expect(container.querySelectorAll('.subagent-card[data-parent="child-1"]')).toHaveLength(1);

    // Root's reply is always visible on the SessionTurn's assistant block.
    expect(container.textContent).toContain("root body");
    // Subagent cards are auto-collapsed on completion; expand them and
    // verify their responses then become visible.
    for (const btn of Array.from(container.querySelectorAll("button.subagent-card__toggle"))) {
      fireEvent.click(btn);
    }
    expect(container.textContent).toContain("child body");
    expect(container.textContent).toContain("grand body");
  });

  it("shows intent classification line when set", () => {
    researchRef.current = {
      ...makeIdleResearch(),
      state: {
        ...makeIdleResearch().state,
        sessionId: "sess-1",
        intentClassification: "research",
        wardName: "maritime",
      },
    };
    renderPage();
    expect(screen.getByText(/Intent: research/)).toBeTruthy();
  });

  it("moves intent analysis and delegated agents into the labelled Research context inspector", async () => {
    const now = 1000;
    const child = {
      id: "child-1",
      agentId: "planner-agent",
      parentExecutionId: "root-1",
      startedAt: now + 200,
      completedAt: now + 300,
      status: "completed" as const,
      wardId: "w1",
      request: "Plan this goal.",
      timeline: [{
        id: "history-recall",
        at: now + 210,
        kind: "tool_call" as const,
        text: "recall",
        toolName: "recall",
      }],
      tokenCount: 10,
      respond: "child body",
      respondStreaming: "",
      thinkingExpanded: false,
      errorMessage: null,
    };
    getSessionStateMock.mockResolvedValue({
      success: true,
      data: {
        intentAnalysis: {
          primary_intent: "research task",
          execution_strategy: { approach: "parallel" },
        },
      },
    });
    researchRef.current = {
      ...makeIdleResearch(),
      state: {
        ...makeIdleResearch().state,
        sessionId: "sess-1",
        rootExecutionId: "root-1",
        intentClassification: "research",
        turns: [{
          id: "turn-u1",
          index: 0,
          userMessage: { id: "u1", content: "do the thing", createdAt: "2026-04-19T00:00:00.000Z" },
          subagents: [child],
          assistantText: "root body",
          assistantStreaming: "",
          timeline: [],
          status: "completed",
          startedAt: "2026-04-19T00:00:00.000Z",
          endedAt: "2026-04-19T00:01:00.000Z",
          durationMs: 60_000,
        }],
      },
    };

    const { container } = renderPage();
    const inspector = screen.getByLabelText("Research context");
    const thread = container.querySelector(".research-page__column");

    expect(within(inspector).getByText("planner-agent")).toBeTruthy();
    const intentToggle = within(inspector).getByRole("button", { name: /expand intent analysis/i });
    expect(intentToggle).toHaveAttribute("aria-expanded", "false");
    expect(within(inspector).getByText(/intent: research/i)).toBeTruthy();
    expect(within(inspector).queryByText("research task")).toBeNull();
    fireEvent.click(within(inspector).getByRole("button", { name: /expand subagent/i }));
    expect(within(inspector).getByText("recall")).toBeTruthy();
    expect(within(thread as HTMLElement).queryByText("planner-agent")).toBeNull();
    expect(within(thread as HTMLElement).getByText("root body")).toBeTruthy();
    fireEvent.click(intentToggle);
    expect(intentToggle).toHaveAttribute("aria-expanded", "true");
    await waitFor(() => {
      expect(within(inspector).getByText("research task")).toBeTruthy();
    });
    fireEvent.click(intentToggle);
    expect(intentToggle).toHaveAttribute("aria-expanded", "false");
    expect(within(inspector).queryByText("research task")).toBeNull();
    expect(getSessionStateMock).toHaveBeenCalledWith("sess-1");
  });

  it("does not render a Research context inspector or fetch intent details for a session without confirmed context", () => {
    researchRef.current = {
      ...makeIdleResearch(),
      state: { ...makeIdleResearch().state, sessionId: "sess-1" },
    };
    renderPage();

    expect(screen.queryByLabelText("Research context")).toBeNull();
    expect(getSessionStateMock).not.toHaveBeenCalled();
  });

  // ------- R14d: artifact strip + slide-out wiring -------

  it("does not render the artifact strip when state.artifacts is empty", () => {
    renderPage();
    expect(screen.queryByRole("list", { name: /session artifacts/i })).toBeNull();
  });

  it("renders a chip per artifact when state.artifacts has entries", () => {
    researchRef.current = {
      ...makeIdleResearch(),
      state: {
        ...makeIdleResearch().state,
        sessionId: "sess-1",
        artifacts: [
          { id: "a1", fileName: "plan.md", fileType: "md" },
          { id: "a2", fileName: "data.csv", fileType: "csv" },
        ],
      },
    };
    renderPage();
    expect(screen.getByRole("list", { name: /session artifacts/i })).toBeTruthy();
    expect(screen.getByRole("button", { name: /Open artifact plan\.md/ })).toBeTruthy();
    expect(screen.getByRole("button", { name: /Open artifact data\.csv/ })).toBeTruthy();
  });

  it("clicking a chip opens the slide-out for the matching artifact (cached path)", async () => {
    const cached = {
      id: "a1",
      sessionId: "sess-1",
      filePath: "/tmp/plan.md",
      fileName: "plan.md",
      fileType: "md",
      fileSize: 100,
      createdAt: "2026-04-19T00:00:00Z",
    };
    researchRef.current = {
      ...makeIdleResearch(),
      state: {
        ...makeIdleResearch().state,
        sessionId: "sess-1",
        artifacts: [{ id: "a1", fileName: "plan.md", fileType: "md" }],
      },
      getFullArtifact: vi.fn().mockReturnValue(cached),
    };
    renderPage();
    fireEvent.click(screen.getByRole("button", { name: /Open artifact plan\.md/ }));
    await waitFor(() => {
      expect(researchRef.current.getFullArtifact).toHaveBeenCalledWith("a1");
    });
    // Slide-out header shows the filename.
    await waitFor(() => {
      expect(screen.getAllByText(/plan\.md/).length).toBeGreaterThan(0);
    });
  });

  it("falls back to listSessionArtifacts when the cache miss returns undefined", async () => {
    const remote = {
      id: "a1",
      sessionId: "sess-1",
      filePath: "/tmp/plan.md",
      fileName: "plan.md",
      fileType: "md",
      fileSize: 100,
      isGoalArtifact: true,
      createdAt: "2026-04-19T00:00:00Z",
    };
    listArtifactsMock.mockResolvedValueOnce({ success: true, data: [remote] });
    researchRef.current = {
      ...makeIdleResearch(),
      state: {
        ...makeIdleResearch().state,
        sessionId: "sess-1",
        artifacts: [{ id: "a1", fileName: "plan.md", fileType: "md" }],
      },
      getFullArtifact: vi.fn().mockReturnValue(undefined),
    };
    renderPage();
    fireEvent.click(screen.getByRole("button", { name: /Open artifact plan\.md/ }));
    await waitFor(() => {
      expect(listArtifactsMock).toHaveBeenCalledWith("sess-1", {
        goalArtifactsOnly: true,
        limit: 24,
      });
    });
  });

  // ---------------------------------------------------------------------------
  // R19 — onAfterDelete → startNewResearch wiring
  // ---------------------------------------------------------------------------

  it("onAfterDelete fires startNewResearch when the current session is the one deleted", () => {
    const startNewResearch = vi.fn();
    researchRef.current = {
      ...makeIdleResearch(),
      state: { ...makeIdleResearch().state, sessionId: "sess-1" },
      startNewResearch,
    };
    renderPage();
    // ResearchPage handed the hook an onAfterDelete — simulate the hook calling
    // it (what a real transport.deleteSession + refresh() would do).
    lastListOptsRef.current.onAfterDelete?.("sess-1");
    expect(startNewResearch).toHaveBeenCalledTimes(1);
  });

  it("onAfterDelete does NOT fire startNewResearch for a different session id", () => {
    const startNewResearch = vi.fn();
    researchRef.current = {
      ...makeIdleResearch(),
      state: { ...makeIdleResearch().state, sessionId: "sess-1" },
      startNewResearch,
    };
    renderPage();
    lastListOptsRef.current.onAfterDelete?.("sess-999");
    expect(startNewResearch).not.toHaveBeenCalled();
  });

  // ---------------------------------------------------------------------------
  // Regression: chat-mode sessions must not leak into the research landing
  // hero's "Recent" cards. Before the session-kind fix, chat-v2 turns
  // rendered as suggestion chips under the research composer.
  // ---------------------------------------------------------------------------

  it("excludes chat-mode sessions from the research empty-state recent cards", async () => {
    listLogSessionsMock.mockResolvedValue({
      success: true,
      data: [
        {
          session_id: "exec-chat-1",
          conversation_id: "sess-chat-abc",
          agent_id: "root",
          agent_name: "root",
          title: "hi there",
          started_at: "2026-04-24T08:00:00Z",
          status: "completed",
          token_count: 100,
          tool_call_count: 0,
          error_count: 0,
          child_session_ids: [],
          mode: "fast",
        },
        {
          session_id: "exec-research-1",
          conversation_id: "sess-research-xyz",
          agent_id: "root",
          agent_name: "root",
          title: "Analyze Q4 market data",
          started_at: "2026-04-24T09:00:00Z",
          status: "completed",
          token_count: 4200,
          tool_call_count: 12,
          error_count: 0,
          child_session_ids: [],
          mode: null,
        },
      ],
    });
    renderPage();
    // Research card visible.
    await waitFor(() => {
      expect(screen.getByText(/Analyze Q4 market data/)).toBeTruthy();
    });
    // Chat card's title ("hi there") must not appear as a recent card.
    expect(screen.queryByText(/^hi there$/)).toBeNull();
  });
});
