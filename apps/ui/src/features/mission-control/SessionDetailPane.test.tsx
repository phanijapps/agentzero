import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@/test/utils";
import { SessionDetailPane } from "./SessionDetailPane";
import type {
  ExecutionLog,
  LogSession,
  SessionDetail,
} from "@/services/transport/types";

const mockGetLogSession = vi.fn();
const mockGetSessionMessages = vi.fn();
const mockGetMissionControlSessionTokens = vi.fn();
const mockUseTraceSubscription = vi.fn();

vi.mock("@/services/transport", async () => {
  const actual = await vi.importActual<Record<string, unknown>>(
    "@/services/transport",
  );
  return {
    ...actual,
    getTransport: async () => ({
      getLogSession: mockGetLogSession,
      getSessionMessages: mockGetSessionMessages,
      getMissionControlSessionTokens: mockGetMissionControlSessionTokens,
    }),
  };
});

vi.mock("../logs/useTraceSubscription", () => ({
  useTraceSubscription: (...args: unknown[]) =>
    mockUseTraceSubscription(...args),
}));

function makeSession(overrides: Partial<LogSession> = {}): LogSession {
  return {
    session_id: "exec-root-1",
    conversation_id: "sess-1",
    agent_id: "root-agent",
    agent_name: "root-agent",
    title: "Investigate performance",
    started_at: "2026-06-09T10:00:00Z",
    status: "completed",
    token_count: 0,
    tool_call_count: 0,
    error_count: 0,
    child_session_ids: [],
    ...overrides,
  };
}

function makeLog(
  category: ExecutionLog["category"],
  overrides: Partial<ExecutionLog> = {},
): ExecutionLog {
  return {
    id: "log-1",
    session_id: "exec-root-1",
    conversation_id: "sess-1",
    agent_id: "root-agent",
    timestamp: "2026-06-09T10:00:01Z",
    level: "info",
    category,
    message: "done",
    ...overrides,
  };
}

function makeDetail(): SessionDetail {
  return {
    session: makeSession(),
    logs: [
      makeLog("response", {
        id: "response-1",
        message: "Loaded from shared detail.",
      }),
      makeLog("tool_call", {
        id: "tool-1",
        message: "shell",
        metadata: { tool_id: "tc-1", tool_name: "shell" },
      }),
      makeLog("tool_result", {
        id: "tool-result-1",
        message: "ok",
        metadata: { tool_id: "tc-1" },
      }),
    ],
  };
}

beforeEach(() => {
  mockGetLogSession.mockReset();
  mockGetSessionMessages.mockReset();
  mockGetSessionMessages.mockResolvedValue({ success: true, data: [] });
  mockGetMissionControlSessionTokens.mockReset();
  mockUseTraceSubscription.mockReset();
});

describe("SessionDetailPane", () => {
  it("shares the selected session detail between Messages and Tools panes", async () => {
    mockGetLogSession.mockResolvedValue({ success: true, data: makeDetail() });
    mockGetMissionControlSessionTokens.mockResolvedValue({
      success: true,
      data: {
        conversation_id: "sess-1",
        root_execution_id: "exec-root-1",
        total_tokens_in: 1000,
        total_tokens_out: 200,
        executions: [],
      },
    });

    render(<SessionDetailPane session={makeSession()} />);

    await waitFor(() => {
      expect(screen.getByText(/Loaded from shared detail/)).toBeInTheDocument();
    });

    expect(screen.getByText("No plan recorded for this session.")).toBeInTheDocument();

    expect(mockGetLogSession).toHaveBeenCalledTimes(1);
    expect(mockGetLogSession).toHaveBeenCalledWith("exec-root-1");
    expect(mockGetMissionControlSessionTokens).toHaveBeenCalledTimes(1);
    expect(mockGetMissionControlSessionTokens).toHaveBeenCalledWith("sess-1");
  });

  it("renders the persisted current plan returned for the selected session", async () => {
    mockGetLogSession.mockResolvedValue({ success: true, data: makeDetail() });
    mockGetMissionControlSessionTokens.mockResolvedValue({
      success: true,
      data: {
        conversation_id: "sess-1",
        root_execution_id: "exec-root-1",
        total_tokens_in: 1000,
        total_tokens_out: 200,
        executions: [],
        current_plan: {
          execution_id: "exec-root-1",
          explanation: "Compare the operational tradeoffs.",
          plan: [{ step: "Inspect the current configuration", status: "in_progress" }],
          updated_at: "2026-07-14T12:00:00Z",
        },
      },
    });

    render(<SessionDetailPane session={makeSession()} />);

    expect(await screen.findByText("Inspect the current configuration")).toBeInTheDocument();
    expect(screen.getByText("Compare the operational tradeoffs.")).toBeInTheDocument();
    expect(screen.getByText("in progress")).toBeInTheDocument();
  });

  it("keeps only useful operational detail when embedded in the Radar inspector", async () => {
    mockGetLogSession.mockResolvedValue({ success: true, data: makeDetail() });
    mockGetMissionControlSessionTokens.mockResolvedValue({
      success: true,
      data: {
        conversation_id: "sess-1",
        root_execution_id: "exec-root-1",
        total_tokens_in: 0,
        total_tokens_out: 0,
        executions: [],
      },
    });

    render(<SessionDetailPane session={makeSession()} embedded />);

    await waitFor(() => expect(mockGetLogSession).toHaveBeenCalledTimes(1));
    expect(screen.queryByTitle("Pause session")).not.toBeInTheDocument();
    expect(screen.queryByTitle("Open in Research")).not.toBeInTheDocument();
    expect(screen.queryByText("No plan recorded for this session.")).not.toBeInTheDocument();
    expect(screen.queryByText(/Loaded from shared detail/)).not.toBeInTheDocument();
  });
});
