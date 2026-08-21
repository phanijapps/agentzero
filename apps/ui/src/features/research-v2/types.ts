// =============================================================================
// Research UI — State types
//
// Consumed by reducer.ts, event-map.ts, useResearchSession.ts, and the
// component tree (AgentTurnBlock, SessionsList, ResearchPage).
//
// Field names match the gateway wire format — especially camelCase here
// vs snake_case on the wire; mapping happens in event-map.ts.
// =============================================================================

import type { MessageAttachment } from "../chat/attachments";

export type AgentTurnStatus = "running" | "completed" | "stopped" | "error";

/** One entry in the chronological Thinking timeline inside an agent turn. */
export interface TimelineEntry {
  id: string;
  at: number; // ms epoch
  kind: "thinking" | "tool_call" | "tool_result" | "error" | "note";
  text: string;
  /** tool_call: canonical tool name. */
  toolName?: string;
  /** tool_call: ~60 char JSON preview of args. */
  toolArgsPreview?: string;
  /** tool_result: ~60 char preview of result value. */
  toolResultPreview?: string;
}

/** One agent turn (root or delegated). Block rendering unit in the UI. */
export interface AgentTurn {
  /** execution_id from the gateway. */
  id: string;
  agentId: string;
  /** Present for delegated turns; null for root turns. */
  parentExecutionId: string | null;
  startedAt: number;
  completedAt: number | null;
  status: AgentTurnStatus;
  /** Ward at the time this turn started (sticky — see WARD_CHANGED handling). */
  wardId: string | null;
  /**
   * Delegation task the parent gave this subagent (from
   * `delegation_started.task`). Null on root turns. Rendered as the
   * "Request:" header on subagent cards.
   */
  request: string | null;
  timeline: TimelineEntry[];
  tokenCount: number;
  /** Final respond() content (markdown). Null until Respond event arrives. */
  respond: string | null;
  /** Token-stream buffer; flushed when Respond arrives. */
  respondStreaming: string;
  /** Per-turn UI toggle for the Thinking chevron. */
  thinkingExpanded: boolean;
  /**
   * Populated when the turn crashed silently (Task 16): TURN_COMPLETE arrived
   * with no respond and no meaningful timeline entries. A workaround for
   * chat-v2 backlog B3 (gateway doesn't emit a proper `error` event). When
   * the backlog lands, replace the inferred text with the real error.message.
   */
  errorMessage: string | null;
}

export interface ResearchMessage {
  id: string;
  /**
   * - "user": user prompt, right-aligned bubble.
   * - "assistant": agent response from history hydration; markdown-rendered.
   *   Live sessions render the agent's answer through turn blocks instead,
   *   so this role only appears after a reload/session-switch.
   */
  role: "user" | "assistant";
  content: string;
  timestamp: number;
  attachments?: MessageAttachment[];
}

export type ResearchStatus = "idle" | "running" | "complete" | "stopped" | "error";

/**
 * One user→assistant exchange within a session.
 *
 * The session's root execution can carry many of these (each follow-up
 * message reuses the same root execution_id), and we group them by
 * user-message timestamp boundary. See
 * `memory-bank/future-state/2026-05-05-research-multi-turn-design.md`
 * for the full rollup algorithm.
 */
export interface SessionTurn {
  /** Stable id derived from the user message id. */
  id: string;
  /** Execution id that owns this turn (root execution); surfaces attach
   * here in the timeline. Null when unknown (legacy snapshots). */
  executionId?: string;
  /** 0..N-1 chronological. */
  index: number;
  /** The user message that opens this turn. */
  userMessage: {
    id: string;
    content: string;
    createdAt: string;
    attachments?: MessageAttachment[];
  };
  /** Subagents whose started_at falls in [startedAt, endedAt). */
  subagents: AgentTurn[];
  /** Final assistant text reply. Null while in flight. */
  assistantText: string | null;
  /** Streaming buffer; promoted to assistantText on turn end. */
  assistantStreaming: string;
  /** Per-turn timeline of root-execution thinking/tool events. Drives the
   *  live ticker pill while the turn is running. Empty after snapshot;
   *  filled by the WS reducer for live runs. */
  timeline: TimelineEntry[];
  /** Per-turn status (mirrors AgentTurnStatus). */
  status: AgentTurnStatus;
  /** ISO timestamp of the user message. */
  startedAt: string;
  /** Right edge: next user message's startedAt, root.ended_at, or null while running. */
  endedAt: string | null;
  /** End - start in ms (null while running). */
  durationMs: number | null;
}

/** Session-summary row used by the sessions drawer. */
export interface SessionSummary {
  id: string;
  title: string;
  status: "running" | "complete" | "crashed" | "paused";
  wardName: string | null;
  updatedAt: number; // ms epoch
}

/** Lightweight artifact reference used in ResearchSessionState. */
export interface ResearchArtifactRef {
  id: string;
  fileName: string;
  fileType?: string;
  fileSize?: number;
  label?: string;
}

/**
 * Client-only identity for a turn the browser has rendered but a session
 * snapshot may have observed. It prevents either an older snapshot before
 * confirmation or a delayed older snapshot after confirmation from replacing
 * the latest submitted turn during this session.
 */
export interface PendingUserTurn {
  /** Client-generated ID reused by the durable root message row. */
  messageId: string;
  /** Scope guard for snapshots from a different execution/session. */
  rootExecutionId: string | null;
  sessionId: string | null;
}

export interface ResearchSessionState {
  /** Server-assigned. Null until init / SESSION_BOUND lands. */
  sessionId: string | null;
  /** Server-assigned WS routing id. Null until SESSION_BOUND lands. */
  conversationId: string | null;
  title: string;
  status: ResearchStatus;
  /** STICKY — only WARD_CHANGED updates this; AGENT_STARTED inherits. */
  wardId: string | null;
  /** STICKY display name for the ward chip. */
  wardName: string | null;
  /** Root execution id for this session. Set when the first root agent
   *  AGENT_STARTED arrives (or pre-set by HYDRATE from snapshot). The
   *  reducer uses it to disambiguate "this event is for the root" vs
   *  "this event is for a subagent" when routing tokens / respond /
   *  timeline entries. */
  rootExecutionId: string | null;
  /**
   * Chronological list of user→assistant exchanges. Each turn owns its
   * own user message, subagents, and assistant reply. The reducer opens
   * a new turn on each user message and routes subsequent
   * delegation/token/respond events to the latest open turn.
   */
  turns: SessionTurn[];
  /**
   * Replaced by the next submission and cleared on reset. This is not sent to
   * the server; it only protects the local view from out-of-order snapshots.
   */
  pendingUserTurn: PendingUserTurn | null;
  /** True between IntentAnalysisStarted and Complete/Skipped. */
  intentAnalyzing: boolean;
  /** From IntentAnalysisComplete. */
  intentClassification: string | null;
  /** From PlanUpdate. */
  planPath: string | null;
  /** Files the agent wrote during this session, newest-last. */
  artifacts: ResearchArtifactRef[];
}

export const EMPTY_RESEARCH_STATE: ResearchSessionState = {
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
