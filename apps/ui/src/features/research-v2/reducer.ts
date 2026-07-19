// =============================================================================
// reducer — multi-turn state machine
//
// State shape:
//   `state.turns: SessionTurn[]` — chronological user→assistant exchanges.
//   `state.rootExecutionId` — disambiguates root vs subagent for events
//      that arrive keyed by execution id (TOKEN, RESPOND, AGENT_*).
//
// Routing rules (used by every event that carries a `turnId`):
//   if turnId === state.rootExecutionId  → applies to the latest SessionTurn
//   else                                  → applies to the matching subagent
//                                           (located by execution id within
//                                           any turn's subagents[])
//
// "Latest open turn" is always `state.turns[turns.length - 1]`. We never
// reorder, never close out-of-order, and never re-open a closed turn.
// See `memory-bank/future-state/2026-05-05-research-multi-turn-design.md`.
// =============================================================================

import type {
  AgentTurn,
  ResearchArtifactRef,
  ResearchSessionState,
  SessionTurn,
  TimelineEntry,
} from "./types";
import type { MessageAttachment } from "../chat/attachments";
import { EMPTY_RESEARCH_STATE } from "./types";

const SILENT_CRASH_MESSAGE =
  "Turn ended with no output (provider error or context limit)";

export interface UserMessagePayload {
  id: string;
  content: string;
  attachments?: MessageAttachment[];
  /** ISO timestamp from the gateway, or `now` when the UI mints it. */
  createdAt: string;
}

export type ResearchAction =
  | {
      type: "HYDRATE";
      sessionId: string;
      conversationId: string | null;
      title: string;
      status: ResearchSessionState["status"];
      wardId: string | null;
      wardName: string | null;
      rootExecutionId: string | null;
      turns: SessionTurn[];
      artifacts: ResearchArtifactRef[];
      intentAnalyzing?: boolean;
      intentClassification?: string | null;
    }
  | { type: "APPEND_USER"; message: UserMessagePayload }
  | { type: "SESSION_BOUND"; sessionId: string | null; conversationId: string }
  | { type: "TITLE_CHANGED"; title: string }
  | { type: "WARD_CHANGED"; wardId: string; wardName: string }
  | {
      type: "AGENT_STARTED";
      turnId: string;
      agentId: string;
      parentExecutionId: string | null;
      wardId: string | null;
      startedAt: number;
      /** Authoritative server identity carried by root `agent_started`. */
      sessionId?: string;
      conversationId?: string;
      /** Optional — populated when this event came from delegation_started. */
      request?: string | null;
    }
  | { type: "AGENT_COMPLETED"; turnId: string; completedAt: number; result?: string }
  | { type: "AGENT_STOPPED"; turnId: string; completedAt: number }
  | { type: "THINKING_DELTA"; turnId: string; entry: TimelineEntry }
  | { type: "TOOL_CALL"; turnId: string; entry: TimelineEntry }
  | { type: "TOOL_RESULT"; turnId: string; entry: TimelineEntry }
  | { type: "TOKEN"; turnId: string; text: string }
  | { type: "RESPOND"; turnId: string; text: string }
  | { type: "TOGGLE_THINKING"; turnId: string }
  | { type: "TURN_COMPLETE"; turnId: string }
  | { type: "INTENT_ANALYSIS_STARTED" }
  | { type: "INTENT_ANALYSIS_COMPLETE"; classification: string }
  | { type: "INTENT_ANALYSIS_SKIPPED" }
  | { type: "PLAN_UPDATE"; planPath: string }
  | { type: "SESSION_COMPLETE" }
  | { type: "ERROR"; message: string }
  | { type: "RESET" }
  | { type: "SET_ARTIFACTS"; artifacts: ResearchArtifactRef[] };

// ---------------------------------------------------------------------------
// SessionTurn-level helpers
// ---------------------------------------------------------------------------

function newOpenTurn(payload: UserMessagePayload, prior: number): SessionTurn {
  return {
    id: `turn-${payload.id}`,
    index: prior,
    userMessage: {
      id: payload.id,
      content: payload.content,
      createdAt: payload.createdAt,
      ...(payload.attachments?.length ? { attachments: payload.attachments } : {}),
    },
    subagents: [],
    assistantText: null,
    assistantStreaming: "",
    timeline: [],
    status: "running",
    startedAt: payload.createdAt,
    endedAt: null,
    durationMs: null,
  };
}

function setLastTurn(
  state: ResearchSessionState,
  fn: (t: SessionTurn) => SessionTurn,
): ResearchSessionState {
  if (state.turns.length === 0) return state;
  const lastIndex = state.turns.length - 1;
  const current = state.turns[lastIndex];
  const updated = fn(current);
  if (updated === current) return state;
  const next = state.turns.slice();
  next[lastIndex] = updated;
  return { ...state, turns: next };
}

// ---------------------------------------------------------------------------
// Subagent helpers — locate / update by execution id within any turn
// ---------------------------------------------------------------------------

interface SubagentLocation {
  turnIndex: number;
  subagentIndex: number;
}

function locateSubagent(
  state: ResearchSessionState,
  executionId: string,
): SubagentLocation | null {
  for (let ti = 0; ti < state.turns.length; ti++) {
    const ix = state.turns[ti].subagents.findIndex((s) => s.id === executionId);
    if (ix !== -1) return { turnIndex: ti, subagentIndex: ix };
  }
  return null;
}

function updateSubagent(
  state: ResearchSessionState,
  executionId: string,
  fn: (s: AgentTurn) => AgentTurn,
): ResearchSessionState {
  const loc = locateSubagent(state, executionId);
  if (!loc) return state;
  const turns = state.turns.slice();
  const turn = turns[loc.turnIndex];
  const subs = turn.subagents.slice();
  subs[loc.subagentIndex] = fn(subs[loc.subagentIndex]);
  turns[loc.turnIndex] = { ...turn, subagents: subs };
  return { ...state, turns };
}

function appendSubagent(
  state: ResearchSessionState,
  sub: AgentTurn,
): ResearchSessionState {
  return setLastTurn(state, (t) => ({
    ...t,
    subagents: [...t.subagents, sub],
  }));
}

function newSubagent(args: {
  turnId: string;
  agentId: string;
  parentExecutionId: string | null;
  wardId: string | null;
  startedAt: number;
  request: string | null;
}): AgentTurn {
  return {
    id: args.turnId,
    agentId: args.agentId,
    parentExecutionId: args.parentExecutionId,
    startedAt: args.startedAt,
    completedAt: null,
    status: "running",
    wardId: args.wardId,
    request: args.request,
    timeline: [],
    tokenCount: 0,
    respond: null,
    respondStreaming: "",
    thinkingExpanded: false,
    errorMessage: null,
  };
}

function subagentHasMeaningfulContent(s: AgentTurn): boolean {
  if (s.respond && s.respond.length > 0) return true;
  if (s.respondStreaming && s.respondStreaming.length > 0) return true;
  return s.timeline.some(
    (e) => e.kind === "tool_call" || e.kind === "tool_result",
  );
}

function turnHasMeaningfulContent(t: SessionTurn): boolean {
  if (t.assistantText && t.assistantText.length > 0) return true;
  if (t.assistantStreaming && t.assistantStreaming.length > 0) return true;
  return t.subagents.some(subagentHasMeaningfulContent);
}

// ---------------------------------------------------------------------------
// Per-action handlers
// ---------------------------------------------------------------------------

function handleHydrate(
  state: ResearchSessionState,
  action: Extract<ResearchAction, { type: "HYDRATE" }>,
): ResearchSessionState {
  const pendingTurn = state.pendingUserTurn
    ? state.turns.find(
        (turn) => turn.userMessage.id === state.pendingUserTurn?.messageId,
      )
    : undefined;
  // The Research caller supplies this opaque id through invoke metadata. It
  // becomes the durable Message.id, so content (including identical prompts)
  // never participates in reconciliation.
  const snapshotHasPendingTurn =
    pendingTurn !== undefined &&
    (state.pendingUserTurn?.sessionId === null ||
      state.pendingUserTurn?.sessionId === action.sessionId) &&
    (state.pendingUserTurn?.rootExecutionId === null ||
      state.pendingUserTurn?.rootExecutionId === action.rootExecutionId) &&
    action.turns.some(
      (turn) => turn.userMessage.id === state.pendingUserTurn?.messageId,
    );
  const retainedOptimisticTurn =
    pendingTurn && !snapshotHasPendingTurn ? pendingTurn : null;

  return {
    ...state,
    sessionId: action.sessionId,
    conversationId: action.conversationId ?? state.conversationId,
    title: action.title,
    status: retainedOptimisticTurn ? state.status : action.status,
    wardId: action.wardId,
    wardName: action.wardName,
    rootExecutionId: retainedOptimisticTurn
      ? state.rootExecutionId ?? action.rootExecutionId
      : action.rootExecutionId,
    turns: retainedOptimisticTurn
      ? [...action.turns, retainedOptimisticTurn]
      : action.turns,
    // Keep the latest submitted id after confirmation as well: a request made
    // before the durable row existed can resolve after this newer snapshot and
    // must not erase the confirmed turn. The next APPEND_USER or RESET replaces
    // this session-scoped guard.
    pendingUserTurn: state.pendingUserTurn,
    artifacts: action.artifacts,
    // Snapshot data is authoritative on a route change. Without resetting
    // these fields, a late hydrate can leave the prior session's context
    // inspector attached to the newly selected session.
    intentAnalyzing: action.intentAnalyzing ?? false,
    intentClassification: action.intentClassification ?? null,
  };
}

function handleAppendUser(
  state: ResearchSessionState,
  action: Extract<ResearchAction, { type: "APPEND_USER" }>,
): ResearchSessionState {
  // Promote any in-flight buffer on the previous turn before opening the new
  // one — prevents a streaming cursor from blinking forever after a
  // follow-up message arrives mid-stream.
  const promoted = state.turns.length > 0 ? promotePriorTurn(state) : state;
  const fresh = newOpenTurn(action.message, promoted.turns.length);
  return {
    ...promoted,
    turns: [...promoted.turns, fresh],
    pendingUserTurn: {
      messageId: action.message.id,
      rootExecutionId: promoted.rootExecutionId,
      sessionId: promoted.sessionId,
    },
    status: "running",
  };
}

function promotePriorTurn(state: ResearchSessionState): ResearchSessionState {
  return setLastTurn(state, (t) => {
    if (t.status !== "running") return t;
    const promotedReply =
      t.assistantText ??
      (t.assistantStreaming.length > 0 ? t.assistantStreaming : null);
    return {
      ...t,
      status: "completed",
      assistantText: promotedReply,
      assistantStreaming: promotedReply ? "" : t.assistantStreaming,
    };
  });
}

function handleAgentStarted(
  state: ResearchSessionState,
  action: Extract<ResearchAction, { type: "AGENT_STARTED" }>,
): ResearchSessionState {
  const boundState = action.sessionId
    ? {
        ...state,
        sessionId: action.sessionId,
        conversationId: action.conversationId ?? state.conversationId,
      }
    : state;

  // Sticky ward: null wardId on the event inherits from state (never clear).
  const wardForTurn = action.wardId ?? boundState.wardId;

  // Root agent: stamp rootExecutionId once. If no SessionTurn exists yet
  // (e.g. live session that hasn't seen APPEND_USER), open a placeholder.
  if (action.parentExecutionId === null) {
    const withRoot =
      boundState.rootExecutionId == null
        ? { ...boundState, rootExecutionId: action.turnId }
        : boundState;
    if (!withRoot.pendingUserTurn) return withRoot;
    return {
      ...withRoot,
      pendingUserTurn: {
        ...withRoot.pendingUserTurn,
        rootExecutionId:
          withRoot.pendingUserTurn.rootExecutionId ?? withRoot.rootExecutionId,
        sessionId: withRoot.pendingUserTurn.sessionId ?? withRoot.sessionId,
      },
    };
  }

  // Subagent. Append to the latest open turn.
  const sub = newSubagent({
    turnId: action.turnId,
    agentId: action.agentId,
    parentExecutionId: action.parentExecutionId,
    wardId: wardForTurn,
    startedAt: action.startedAt,
    request: action.request ?? null,
  });
  // Idempotent: if we already have this subagent (from snapshot), skip.
  if (locateSubagent(boundState, action.turnId)) return boundState;
  return appendSubagent(boundState, sub);
}

function handleAgentCompleted(
  state: ResearchSessionState,
  action: Extract<ResearchAction, { type: "AGENT_COMPLETED" }>,
): ResearchSessionState {
  if (action.turnId === state.rootExecutionId) {
    return setLastTurn(state, (t) => {
      const withResult =
        action.result &&
        (t.assistantText !== action.result || t.assistantStreaming !== "")
          ? { ...t, assistantText: action.result, assistantStreaming: "" }
          : t;
      return closeTurn(withResult, action.completedAt);
    });
  }
  return updateSubagent(state, action.turnId, (s) => closeSubagent(
    action.result
      ? { ...s, respond: action.result, respondStreaming: "" }
      : s,
    action.completedAt,
  ));
}

function closeTurn(t: SessionTurn, completedAt: number): SessionTurn {
  if (t.status === "completed") return t;
  if (turnHasMeaningfulContent(t)) {
    const promotedReply =
      t.assistantText ??
      (t.assistantStreaming.length > 0 ? t.assistantStreaming : null);
    return {
      ...t,
      status: "completed",
      assistantText: promotedReply,
      assistantStreaming: promotedReply ? "" : t.assistantStreaming,
      endedAt: t.endedAt ?? new Date(completedAt).toISOString(),
      durationMs:
        t.durationMs ?? Math.max(0, completedAt - Date.parse(t.startedAt)),
    };
  }
  // Silent crash: turn ended with nothing useful. Mirror today's per-turn
  // error inference so the user sees a clear failure message.
  return {
    ...t,
    status: "error",
    endedAt: t.endedAt ?? new Date(completedAt).toISOString(),
    assistantText: t.assistantText ?? SILENT_CRASH_MESSAGE,
  };
}

function closeSubagent(s: AgentTurn, completedAt: number): AgentTurn {
  if (subagentHasMeaningfulContent(s)) {
    const promotedRespond =
      s.respond ?? (s.respondStreaming.length > 0 ? s.respondStreaming : null);
    return {
      ...s,
      status: "completed",
      completedAt,
      respond: promotedRespond,
      respondStreaming: promotedRespond ? "" : s.respondStreaming,
    };
  }
  return {
    ...s,
    status: "error",
    completedAt,
    errorMessage: SILENT_CRASH_MESSAGE,
  };
}

function handleAgentStopped(
  state: ResearchSessionState,
  action: Extract<ResearchAction, { type: "AGENT_STOPPED" }>,
): ResearchSessionState {
  if (action.turnId === state.rootExecutionId) {
    return setLastTurn(state, (t) => ({
      ...t,
      status: "stopped",
      endedAt: t.endedAt ?? new Date(action.completedAt).toISOString(),
    }));
  }
  return updateSubagent(state, action.turnId, (s) => ({
    ...s,
    status: "stopped",
    completedAt: action.completedAt,
  }));
}

function handleTimelineAppend(
  state: ResearchSessionState,
  turnId: string,
  entry: TimelineEntry,
): ResearchSessionState {
  if (turnId === state.rootExecutionId) {
    return setLastTurn(state, (t) => ({
      ...t,
      timeline: [...t.timeline, entry],
    }));
  }
  return updateSubagent(state, turnId, (s) => ({
    ...s,
    timeline: [...s.timeline, entry],
  }));
}

function handleToken(
  state: ResearchSessionState,
  action: Extract<ResearchAction, { type: "TOKEN" }>,
): ResearchSessionState {
  if (action.turnId === state.rootExecutionId) {
    return setLastTurn(state, (t) => ({
      ...t,
      assistantStreaming: t.assistantStreaming + action.text,
    }));
  }
  return updateSubagent(state, action.turnId, (s) => ({
    ...s,
    respondStreaming: s.respondStreaming + action.text,
  }));
}

function handleRespond(
  state: ResearchSessionState,
  action: Extract<ResearchAction, { type: "RESPOND" }>,
): ResearchSessionState {
  if (action.turnId === state.rootExecutionId) {
    return setLastTurn(state, (t) => {
      if (
        t.status !== "running" ||
        (t.assistantText === action.text && t.assistantStreaming === "")
      ) {
        return t;
      }
      return {
        ...t,
        assistantText: action.text,
        assistantStreaming: "",
      };
    });
  }
  return updateSubagent(state, action.turnId, (s) => ({
    ...s,
    respond: action.text,
    respondStreaming: "",
  }));
}

function handleToggleThinking(
  state: ResearchSessionState,
  action: Extract<ResearchAction, { type: "TOGGLE_THINKING" }>,
): ResearchSessionState {
  // Thinking expand toggle is per-subagent only; root has no chevron.
  return updateSubagent(state, action.turnId, (s) => ({
    ...s,
    thinkingExpanded: !s.thinkingExpanded,
  }));
}

// ---------------------------------------------------------------------------
// Reducer
// ---------------------------------------------------------------------------

export function reduceResearch(
  state: ResearchSessionState,
  action: ResearchAction,
): ResearchSessionState {
  switch (action.type) {
    case "HYDRATE":
      return handleHydrate(state, action);
    case "APPEND_USER":
      return handleAppendUser(state, action);
    case "SESSION_BOUND":
      // Idempotent on matching conv_id: the hook dispatches a pre-invoke
      // SESSION_BOUND with a client-owned conversationId and sessionId:null;
      // the server's invoke_accepted re-dispatches with the server-assigned
      // sessionId. Preserving an existing sessionId when action.sessionId is
      // null ensures the client-owned pre-invoke dispatch never clobbers the
      // server-assigned id.
      return {
        ...state,
        conversationId: action.conversationId,
        sessionId: action.sessionId ?? state.sessionId,
      };
    case "TITLE_CHANGED":
      return { ...state, title: action.title };
    case "WARD_CHANGED":
      return { ...state, wardId: action.wardId, wardName: action.wardName };
    case "AGENT_STARTED":
      return handleAgentStarted(state, action);
    case "AGENT_COMPLETED":
      return handleAgentCompleted(state, action);
    case "AGENT_STOPPED":
      return handleAgentStopped(state, action);
    case "THINKING_DELTA":
    case "TOOL_CALL":
    case "TOOL_RESULT":
      return handleTimelineAppend(state, action.turnId, action.entry);
    case "TOKEN":
      return handleToken(state, action);
    case "RESPOND":
      return handleRespond(state, action);
    case "TOGGLE_THINKING":
      return handleToggleThinking(state, action);
    case "TURN_COMPLETE":
      // Informational — real completion signal is AGENT_COMPLETED.
      return state;
    case "INTENT_ANALYSIS_STARTED":
      return { ...state, intentAnalyzing: true };
    case "INTENT_ANALYSIS_COMPLETE":
      return { ...state, intentAnalyzing: false, intentClassification: action.classification };
    case "INTENT_ANALYSIS_SKIPPED":
      return { ...state, intentAnalyzing: false };
    case "PLAN_UPDATE":
      return { ...state, planPath: action.planPath };
    case "SESSION_COMPLETE":
      return { ...state, status: "complete" };
    case "ERROR": {
      const pendingMessageId = state.pendingUserTurn?.messageId;
      return {
        ...state,
        status: "error",
        intentAnalyzing: false,
        turns: pendingMessageId
          ? state.turns.map((turn) =>
              turn.userMessage.id === pendingMessageId
                ? {
                    ...turn,
                    status: "error",
                    assistantText: "Request failed. Please try again.",
                    assistantStreaming: "",
                  }
                : turn,
            )
          : state.turns,
      };
    }
    case "RESET":
      return EMPTY_RESEARCH_STATE;
    case "SET_ARTIFACTS":
      return { ...state, artifacts: action.artifacts };
    default:
      return state;
  }
}
