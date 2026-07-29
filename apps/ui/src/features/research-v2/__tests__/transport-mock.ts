// =============================================================================
// transport-mock — minimal Transport stub + `ev` DSL for building synthetic
// ConversationEvent streams. Used by flows.integration.test.tsx to drive
// useResearchSession without a real WebSocket.
//
// Design notes:
// - Two subscriptions fire in the R14g hook (one keyed by client-minted
//   conv_id, one keyed by session_id with scope="session"). This mock keeps
//   a single handler slot and updates it on every subscribeConversation —
//   both subscriptions share the same underlying `ctx.dispatch` in the hook
//   via `makeEventHandler`, so one dispatch is enough to route reducer
//   actions.
// - All methods the hooks might call — including R14g/R14h/R19 additions
//   (listLogSessions, deleteSession, onConnectionStateChange) — MUST be
//   stubs, otherwise the hook throws "transport.X is not a function".
// =============================================================================

import { vi } from "vitest";
import type {
  Artifact,
  ConversationEvent,
  LogSession,
  SessionMessage,
} from "@/services/transport/types";

// -----------------------------------------------------------------------------
// Constants — extracted per python-code-quality.md / DRY
// -----------------------------------------------------------------------------

export const MOCK_SESSION_ID = "sess-mock";
export const MOCK_CONV_ID = "conv-mock";
const DEFAULT_AGENT_ID = "root";

// -----------------------------------------------------------------------------
// Public types
// -----------------------------------------------------------------------------

export interface MockTransportCalls {
  initChatSession: ReturnType<typeof vi.fn>;
  getSessionMessages: ReturnType<typeof vi.fn>;
  listSessionArtifacts: ReturnType<typeof vi.fn>;
  executeAgent: ReturnType<typeof vi.fn>;
  stopAgent: ReturnType<typeof vi.fn>;
  deleteSession: ReturnType<typeof vi.fn>;
  subscribeConversation: ReturnType<typeof vi.fn>;
  listLogSessions: ReturnType<typeof vi.fn>;
  getSessionState: ReturnType<typeof vi.fn>;
  listSavedSessionSurfaces: ReturnType<typeof vi.fn>;
  onConnectionStateChange: ReturnType<typeof vi.fn>;
  unsubscribe: ReturnType<typeof vi.fn>;
}

export interface MockTransport {
  __pushEvent(event: ConversationEvent): void;
  __close(): void;
  calls: MockTransportCalls;

  // Methods hooks under test may call.
  mode: "web";
  initChatSession: MockTransportCalls["initChatSession"];
  getSessionMessages: MockTransportCalls["getSessionMessages"];
  listSessionArtifacts: MockTransportCalls["listSessionArtifacts"];
  executeAgent: MockTransportCalls["executeAgent"];
  stopAgent: MockTransportCalls["stopAgent"];
  deleteSession: MockTransportCalls["deleteSession"];
  subscribeConversation: MockTransportCalls["subscribeConversation"];
  listLogSessions: MockTransportCalls["listLogSessions"];
  getSessionState: MockTransportCalls["getSessionState"];
  listSavedSessionSurfaces: MockTransportCalls["listSavedSessionSurfaces"];
  onConnectionStateChange: MockTransportCalls["onConnectionStateChange"];
  getConnectionState: () => { status: "connected" };
  getArtifactContentUrl: (id: string) => string;
  isConnected: () => boolean;
}

export interface MockTransportOptions {
  messages?: SessionMessage[];
  artifacts?: Artifact[];
  logSessions?: LogSession[];
}

// -----------------------------------------------------------------------------
// Factory
// -----------------------------------------------------------------------------

/**
 * Build a mock transport just rich enough to run `useResearchSession` and
 * `useSessionsList` under jsdom. We expose the call spies on `.calls` so
 * assertions stay explicit.
 */
export function makeMockTransport(
  opts: MockTransportOptions = {},
): MockTransport {
  let handler: ((e: ConversationEvent) => void) | null = null;

  const unsubscribe = vi.fn<() => void>(() => {
    handler = null;
  });

  const calls: MockTransportCalls = {
    initChatSession: vi.fn().mockResolvedValue({
      success: true,
      data: {
        sessionId: MOCK_SESSION_ID,
        conversationId: MOCK_CONV_ID,
        created: false,
      },
    }),
    getSessionMessages: vi
      .fn()
      .mockResolvedValue({ success: true, data: opts.messages ?? [] }),
    listSessionArtifacts: vi
      .fn()
      .mockResolvedValue({ success: true, data: opts.artifacts ?? [] }),
    executeAgent: vi.fn().mockResolvedValue({
      success: true,
      data: { conversationId: MOCK_CONV_ID },
    }),
    stopAgent: vi.fn().mockResolvedValue({ success: true, data: undefined }),
    deleteSession: vi
      .fn()
      .mockResolvedValue({ success: true, data: undefined }),
    subscribeConversation: vi.fn(
      (
        _convId: string,
        options: { onEvent: (e: ConversationEvent) => void },
      ) => {
        handler = options.onEvent;
        return unsubscribe;
      },
    ),
    listLogSessions: vi
      .fn()
      .mockResolvedValue({ success: true, data: opts.logSessions ?? [] }),
    getSessionState: vi
      .fn()
      .mockResolvedValue({ success: false, error: "not available" }),
    listSavedSessionSurfaces: vi.fn().mockResolvedValue({ success: true, data: [] }),
    onConnectionStateChange: vi.fn(() => () => undefined),
    unsubscribe,
  };

  return {
    calls,
    mode: "web",
    initChatSession: calls.initChatSession,
    getSessionMessages: calls.getSessionMessages,
    listSessionArtifacts: calls.listSessionArtifacts,
    executeAgent: calls.executeAgent,
    stopAgent: calls.stopAgent,
    deleteSession: calls.deleteSession,
    subscribeConversation: calls.subscribeConversation,
    listLogSessions: calls.listLogSessions,
    getSessionState: calls.getSessionState,
    listSavedSessionSurfaces: calls.listSavedSessionSurfaces,
    onConnectionStateChange: calls.onConnectionStateChange,
    getConnectionState: () => ({ status: "connected" }),
    getArtifactContentUrl: (id: string) => `/api/artifacts/${id}`,
    isConnected: () => true,
    __pushEvent: (e: ConversationEvent) => {
      if (handler) handler(e);
    },
    __close: () => {
      handler = null;
    },
  };
}

// -----------------------------------------------------------------------------
// Event-stream DSL. Each builder returns a ConversationEvent-compatible
// object; tests cast via `as ConversationEvent` at the push site to avoid
// leaking the cast to every call site.
// -----------------------------------------------------------------------------

type Ev = ConversationEvent;

function baseEvent(
  type: string,
  execution_id: string,
  extra: Record<string, unknown> = {},
): Ev {
  return {
    type,
    session_id: MOCK_SESSION_ID,
    execution_id,
    conversation_id: MOCK_CONV_ID,
    timestamp: Date.now(),
    ...extra,
  } as Ev;
}

export const ev = {
  invokeAccepted: (): Ev => baseEvent("invoke_accepted", ""),
  agentStarted: (exec = "exec-1", agent: string = DEFAULT_AGENT_ID): Ev =>
    baseEvent("agent_started", exec, { agent_id: agent }),
  agentCompleted: (exec: string, agent: string = DEFAULT_AGENT_ID): Ev =>
    baseEvent("agent_completed", exec, { agent_id: agent }),
  thinking: (exec: string, content: string): Ev =>
    baseEvent("thinking", exec, { content }),
  toolCall: (exec: string, tool_name: string, args: unknown = {}): Ev =>
    baseEvent("tool_call", exec, { tool_name, args }),
  toolResult: (exec: string, tool_name: string, result: unknown = ""): Ev =>
    baseEvent("tool_result", exec, { tool_name, result }),
  token: (exec: string, delta: string): Ev =>
    baseEvent("token", exec, { delta }),
  respond: (exec: string, message: string): Ev =>
    baseEvent("respond", exec, { message }),
  turnComplete: (exec: string, final_message = ""): Ev =>
    baseEvent("turn_complete", exec, { final_message }),
  wardChanged: (ward_id: string): Ev =>
    baseEvent("ward_changed", "", { ward_id }),
  delegationStarted: (
    parent: string,
    child: string,
    childAgent: string,
    task: string | null = null,
  ): Ev =>
    baseEvent("delegation_started", parent, {
      parent_execution_id: parent,
      child_execution_id: child,
      child_agent_id: childAgent,
      task,
    }),
  childAgentStarted: (child: string, parent: string, agent: string): Ev =>
    baseEvent("agent_started", child, {
      agent_id: agent,
      parent_execution_id: parent,
    }),
  childAgentCompleted: (child: string, parent: string, agent: string): Ev =>
    baseEvent("agent_completed", child, {
      agent_id: agent,
      parent_execution_id: parent,
    }),
  error: (message: string): Ev => baseEvent("error", "", { message }),
};
