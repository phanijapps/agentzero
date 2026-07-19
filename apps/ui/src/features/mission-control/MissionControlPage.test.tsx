import { describe, expect, it, vi, beforeEach } from "vitest";
import { fireEvent, render, screen } from "@/test/utils";
import type { LogSession } from "@/services/transport/types";

const mockUseMissionControlSessions = vi.fn();

vi.mock("./useMissionControlSessions", () => ({
  useMissionControlSessions: (...args: unknown[]) => mockUseMissionControlSessions(...args),
}));

vi.mock("./SessionDetailPane", () => ({
  SessionDetailPane: ({ session }: { session: LogSession }) => (
    <div data-testid="mission-inspector">Inspector for {session.session_id}</div>
  ),
}));

import { MissionControlPage } from "./MissionControlPage";

function makeSession(overrides: Partial<LogSession> = {}): LogSession {
  return {
    session_id: "sess-x",
    conversation_id: "conv-x",
    agent_id: "agent:root",
    agent_name: "root",
    started_at: new Date().toISOString(),
    status: "running",
    token_count: 1440,
    tool_call_count: 0,
    error_count: 0,
    child_session_ids: [],
    ...overrides,
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  mockUseMissionControlSessions.mockReturnValue({
    sessions: [],
    tokenIndex: { byRootExecId: new Map(), executionsByRootExecId: new Map() },
    refreshGeneration: 1,
    loading: false,
    error: null,
    refetch: vi.fn(),
  });
});

describe("MissionControlPage", () => {
  it("renders the Attention Radar header and bounded snapshot metric", () => {
    render(<MissionControlPage />);
    expect(screen.getByRole("main", { name: /mission control attention radar/i })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Attention Radar" })).toBeInTheDocument();
    expect(screen.getByRole("region", { name: "Mission overview" })).toHaveTextContent("Attention now");
    expect(screen.getByText("Bounded session summary")).toBeInTheDocument();
  });

  it("shows a calm empty Radar state", () => {
    render(<MissionControlPage />);
    expect(screen.getByText(/no recent missions/i)).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Radar standing by" })).toBeInTheDocument();
  });

  it("ranks a failed mission before an active mission", () => {
    mockUseMissionControlSessions.mockReturnValue({
      sessions: [
        makeSession({ session_id: "active-1", title: "active mission", status: "running" }),
        makeSession({ session_id: "failed-1", title: "failed mission", status: "error" }),
      ],
      tokenIndex: { byRootExecId: new Map(), executionsByRootExecId: new Map() },
      refreshGeneration: 1,
      loading: false,
      error: null,
      refetch: vi.fn(),
    });
    render(<MissionControlPage />);
    const missions = screen.getAllByRole("button", { name: /focus (active|failed) mission/i });
    expect(missions[0]).toHaveAccessibleName("Focus failed mission");
  });

  it("keeps the inspector expanded when the focused mission changes", () => {
    mockUseMissionControlSessions.mockReturnValue({
      sessions: [
        makeSession({ session_id: "first-1", title: "first mission" }),
        makeSession({ session_id: "second-2", title: "second mission" }),
      ],
      tokenIndex: { byRootExecId: new Map(), executionsByRootExecId: new Map() },
      refreshGeneration: 1,
      loading: false,
      error: null,
      refetch: vi.fn(),
    });
    render(<MissionControlPage />);
    fireEvent.click(screen.getByRole("button", { name: "Focus second mission" }));
    expect(screen.getByRole("heading", { name: "second mission" })).toBeInTheDocument();
    expect(screen.getByTestId("mission-inspector")).toHaveTextContent("second-2");
  });

  it("shows the detailed trace by default and lets the user collapse it", () => {
    mockUseMissionControlSessions.mockReturnValue({
      sessions: [makeSession({ session_id: "live-1", title: "live mission" })],
      tokenIndex: { byRootExecId: new Map(), executionsByRootExecId: new Map() },
      refreshGeneration: 1,
      loading: false,
      error: null,
      refetch: vi.fn(),
    });
    render(<MissionControlPage />);
    expect(screen.getByTestId("mission-inspector")).toHaveTextContent("live-1");
    fireEvent.click(screen.getByRole("button", { name: /hide inspector/i }));
    expect(screen.queryByTestId("mission-inspector")).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /inspect mission/i }));
    expect(screen.getByTestId("mission-inspector")).toHaveTextContent("live-1");
  });

  it("places live operations and system posture in the right column", () => {
    render(<MissionControlPage />);
    const sidebar = screen.getByRole("complementary", { name: "Live operations and system posture" });
    expect(sidebar).toHaveTextContent("Recent mission activity");
    expect(sidebar).toHaveTextContent("Ready for focused work");
  });
});
