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

    expect(mockGetLogSession).toHaveBeenCalledTimes(1);
    expect(mockGetLogSession).toHaveBeenCalledWith("exec-root-1");
    expect(mockGetMissionControlSessionTokens).toHaveBeenCalledTimes(1);
    expect(mockGetMissionControlSessionTokens).toHaveBeenCalledWith("sess-1");
  });
});
