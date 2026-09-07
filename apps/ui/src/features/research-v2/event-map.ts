import type { ConversationEvent } from "@/services/transport/types";
import { randomId } from "@/shared/utils/randomId";
import type { PillEvent } from "../shared/statusPill";
import type { ResearchAction } from "./reducer";
import type { TimelineEntry } from "./types";

const ORPHAN_TURN_ID = "orphan";
const ARGS_PREVIEW_LIMIT = 60;
const RESULT_PREVIEW_LIMIT = 60;

// -------------------------------------------------------------------------
// Preview helpers
// -------------------------------------------------------------------------

function previewArgs(args: Record<string, unknown>): string {
  try {
    const s = JSON.stringify(args);
    return s.length <= ARGS_PREVIEW_LIMIT ? s : s.slice(0, ARGS_PREVIEW_LIMIT - 1) + "…";
  } catch {
    return "";
  }
}

function previewResult(result: unknown): string {
  // `JSON.stringify` can return `undefined` for circular refs — coerce to "" in that case.
  const raw = typeof result === "string" ? result : JSON.stringify(result ?? "");
  const s = typeof raw === "string" ? raw : "";
  return s.length <= RESULT_PREVIEW_LIMIT ? s : s.slice(0, RESULT_PREVIEW_LIMIT - 1) + "…";
}

function turnIdOf(e: Record<string, unknown>): string {
  const id = e["execution_id"];
  return typeof id === "string" && id.length > 0 ? id : ORPHAN_TURN_ID;
}

function toolNameOf(e: Record<string, unknown>, fallback: string): string {
  const candidate = e["tool_name"] ?? e["tool"];
  return typeof candidate === "string" ? candidate : fallback;
}

// -------------------------------------------------------------------------
// Per-branch mappers for mapGatewayEventToResearchAction
// -------------------------------------------------------------------------

function mapAgentStarted(e: Record<string, unknown>, now: number): ResearchAction {
  const action: Extract<ResearchAction, { type: "AGENT_STARTED" }> = {
    type: "AGENT_STARTED",
    turnId: turnIdOf(e),
    agentId: (e["agent_id"] as string) ?? "root",
    parentExecutionId: (e["parent_execution_id"] as string) ?? null,
    wardId: (e["ward_id"] as string | null) ?? null,
    startedAt: now,
  };
  const sessionId = e["session_id"];
  if (typeof sessionId === "string" && sessionId.length > 0) {
    action.sessionId = sessionId;
  }
  const conversationId = e["conversation_id"];
  if (typeof conversationId === "string" && conversationId.length > 0) {
    action.conversationId = conversationId;
  }
  return action;
}

/**
 * `delegation_started` is the canonical signal for a subagent spawn — it's
 * the only event that carries both parent + child execution ids. The child's
 * own `agent_started` may follow on a different conv_id that isn't in our
 * subscription, so we must seed the nested turn from this event.
 * AGENT_STARTED is idempotent (ensureTurn no-ops on duplicate id), so if the
 * child's own agent_started does arrive later, no harm done.
 */
function mapDelegationStarted(e: Record<string, unknown>, now: number): ResearchAction | null {
  const childExec = e["child_execution_id"];
  const parentExec = e["parent_execution_id"];
  const childAgent = e["child_agent_id"];
  const task = e["task"];
  if (typeof childExec !== "string" || childExec.length === 0) return null;
  return {
    type: "AGENT_STARTED",
    turnId: childExec,
    agentId: typeof childAgent === "string" ? childAgent : "subagent",
    parentExecutionId: typeof parentExec === "string" ? parentExec : null,
    wardId: null,
    startedAt: now,
    request: typeof task === "string" ? task : null,
  };
}

function mapDelegationCompleted(e: Record<string, unknown>, now: number): ResearchAction | null {
  const childExec = e["child_execution_id"];
  if (typeof childExec !== "string" || childExec.length === 0) return null;
  return { type: "AGENT_COMPLETED", turnId: childExec, completedAt: now };
}

function mapWardChanged(e: Record<string, unknown>): ResearchAction | null {
  const flat = e["ward_id"];
  if (typeof flat === "string" && flat.length > 0) {
    return { type: "WARD_CHANGED", wardId: flat, wardName: flat };
  }
  const ward = e["ward"] as Record<string, unknown> | undefined;
  const id = ward?.["id"];
  const name = ward?.["name"];
  if (typeof id === "string" && id.length > 0) {
    return {
      type: "WARD_CHANGED",
      wardId: id,
      wardName: typeof name === "string" ? name : id,
    };
  }
  return null;
}

function mapThinkingDelta(e: Record<string, unknown>, now: number): ResearchAction | null {
  const content = e["content"];
  if (typeof content !== "string" || content.length === 0) return null;
  const entry: TimelineEntry = {
    id: randomId(),
    at: now,
    kind: "thinking",
    text: content,
  };
  return { type: "THINKING_DELTA", turnId: turnIdOf(e), entry };
}

function mapToolCall(e: Record<string, unknown>, now: number): ResearchAction {
  const tool = toolNameOf(e, "tool");
  const entry: TimelineEntry = {
    id: randomId(),
    at: now,
    kind: "tool_call",
    text: tool,
    toolName: tool,
    toolArgsPreview: previewArgs((e["args"] ?? {}) as Record<string, unknown>),
  };
  return { type: "TOOL_CALL", turnId: turnIdOf(e), entry };
}

function mapToolResult(e: Record<string, unknown>, now: number): ResearchAction {
  const tool = toolNameOf(e, "result");
  const entry: TimelineEntry = {
    id: randomId(),
    at: now,
    kind: "tool_result",
    text: tool,
    toolResultPreview: previewResult(e["result"]),
  };
  return { type: "TOOL_RESULT", turnId: turnIdOf(e), entry };
}

function mapToken(e: Record<string, unknown>): ResearchAction | null {
  const text = e["delta"] ?? e["content"];
  if (typeof text !== "string" || text.length === 0) return null;
  return { type: "TOKEN", turnId: turnIdOf(e), text };
}

function mapRespond(e: Record<string, unknown>): ResearchAction | null {
  const text = e["message"] ?? e["content"];
  if (typeof text !== "string") return null;
  return { type: "RESPOND", turnId: turnIdOf(e), text };
}

/**
 * Gateway's WS handler translates `GatewayEvent::Respond` into
 * `ServerMessage::TurnComplete { final_message }` (see
 * gateway/src/websocket/handler.rs `gateway_event_to_server_message`). So the frontend never receives
 * a standalone `respond` event — the final answer rides on
 * `turn_complete.final_message`. Emit RESPOND when present so the turn
 * renders its answer without a page reload.
 */
function mapTurnComplete(e: Record<string, unknown>): ResearchAction {
  const finalMessage = e["final_message"];
  if (typeof finalMessage === "string" && finalMessage.length > 0) {
    return { type: "RESPOND", turnId: turnIdOf(e), text: finalMessage };
  }
  return { type: "TURN_COMPLETE", turnId: turnIdOf(e) };
}

function mapSessionBound(e: Record<string, unknown>): ResearchAction | null {
  const sid = e["session_id"];
  const cid = e["conversation_id"];
  if (typeof sid !== "string" || sid.length === 0) return null;
  return {
    type: "SESSION_BOUND",
    sessionId: sid,
    conversationId: typeof cid === "string" ? cid : "",
  };
}

// -------------------------------------------------------------------------
// mapGatewayEventToResearchAction — flat dispatcher
// -------------------------------------------------------------------------

export function mapGatewayEventToResearchAction(ev: ConversationEvent): ResearchAction | null {
  const e = ev as unknown as Record<string, unknown>;
  const type = e["type"] as string;
  const now = Date.now();
  switch (type) {
    case "agent_started":            return mapAgentStarted(e, now);
    case "agent_completed": {
      // The lifecycle event is the recovery path when the preceding
      // turn_complete websocket frame was missed. Keep its final response so
      // the reducer can render it before closing the turn.
      const result = e["result"];
      return {
        type: "AGENT_COMPLETED",
        turnId: turnIdOf(e),
        completedAt: now,
        ...(typeof result === "string" && result.trim().length > 0
          ? { result: result.trim() }
          : {}),
      };
    }
    case "agent_stopped":            return { type: "AGENT_STOPPED",   turnId: turnIdOf(e), completedAt: now };
    case "delegation_started":       return mapDelegationStarted(e, now);
    case "delegation_completed":     return mapDelegationCompleted(e, now);
    case "ward_changed":             return mapWardChanged(e);
    case "thinking":                 return mapThinkingDelta(e, now);
    case "tool_call":                return mapToolCall(e, now);
    case "tool_result":              return mapToolResult(e, now);
    case "token":                    return mapToken(e);
    case "heartbeat":                return { type: "HEARTBEAT", turnId: turnIdOf(e), at: now };
    case "respond":                  return mapRespond(e);
    case "turn_complete":            return mapTurnComplete(e);
    case "session_title_changed":    return { type: "TITLE_CHANGED", title: (e["title"] as string) ?? "" };
    case "intent_analysis_started":  return { type: "INTENT_ANALYSIS_STARTED" };
    case "intent_analysis_complete": return { type: "INTENT_ANALYSIS_COMPLETE", classification: (e["classification"] as string) ?? "" };
    case "intent_analysis_skipped":  return { type: "INTENT_ANALYSIS_SKIPPED" };
    case "plan_update":              return { type: "PLAN_UPDATE", planPath: (e["plan_path"] as string) ?? "" };
    case "invoke_accepted":
    case "session_initialized":      return mapSessionBound(e);
    case "error":                    return { type: "ERROR", message: (e["message"] as string) ?? "error" };
    default:                         return null;
  }
}

// -------------------------------------------------------------------------
// Pill mapper — deterministic; Thinking intentionally NOT mapped.
// -------------------------------------------------------------------------

function mapPillToolCall(e: Record<string, unknown>): PillEvent | null {
  const tool = e["tool_name"] ?? e["tool"];
  if (typeof tool !== "string") return null;
  return { kind: "tool_call", tool, args: (e["args"] ?? {}) as Record<string, unknown> };
}

function mapPillToolResult(e: Record<string, unknown>): PillEvent | null {
  const toolRaw = e["tool_name"] ?? e["tool"];
  const tool = typeof toolRaw === "string" ? toolRaw : undefined;
  // Failed results surface the error so the user sees it without opening the
  // logs; successful results emit the recovery signal that un-sticks the
  // pill's sticky error state (a retried-and-fixed call must not leave
  // "Tool error" on screen for the rest of the session).
  const err = e["error"];
  if (typeof err === "string" && err.length > 0) {
    return { kind: "error", message: err, source: "tool", tool };
  }
  return tool ? { kind: "tool_ok", tool } : null;
}

function mapPillError(e: Record<string, unknown>): PillEvent {
  const message = (e["message"] as string) ?? "unknown error";
  return { kind: "error", message, source: "llm" };
}

export function mapGatewayEventToPillEvent(ev: ConversationEvent): PillEvent | null {
  const e = ev as unknown as Record<string, unknown>;
  const type = e["type"] as string;
  switch (type) {
    case "agent_started":   return { kind: "agent_started", agent_id: (e["agent_id"] as string) ?? "" };
    case "agent_completed": return { kind: "agent_completed", agent_id: (e["agent_id"] as string) ?? "", is_final: true };
    case "tool_call":       return mapPillToolCall(e);
    case "tool_result":     return mapPillToolResult(e);
    case "respond":         return { kind: "respond" };
    case "error":           return mapPillError(e);
    default:                return null;
  }
}
