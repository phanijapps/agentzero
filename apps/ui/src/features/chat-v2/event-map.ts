import type { ConversationEvent } from "@/services/transport/types";
import { randomId } from "@/shared/utils/randomId";
import type { PillEvent } from "../shared/statusPill";
import type { QuickChatAction } from "./reducer";

// ---------------------------------------------------------------------------
// mapGatewayEventToQuickChatAction
// ---------------------------------------------------------------------------

function mapTokenEvent(ev: Record<string, unknown>): QuickChatAction | null {
  // Gateway emits streaming tokens with `delta: string`; older/alternative
  // shapes may use `content` (kept for forward-compat with replay streams).
  const text = ev["delta"] ?? ev["content"];
  if (typeof text !== "string" || text.length === 0) return null;
  return { type: "TOKEN", text };
}

function mapRespondEvent(ev: Record<string, unknown>): QuickChatAction | null {
  // Gateway emits final responses with `message: string`; accept `content`
  // as a fallback in case the wire format ever changes.
  const text = ev["message"] ?? ev["content"];
  if (typeof text !== "string") return null;
  return { type: "RESPOND", text };
}

function mapRootAgentCompletedEvent(ev: Record<string, unknown>): QuickChatAction | null {
  // Quick Chat always invokes the root agent. Child completion events must not
  // replace the root bubble, but root AgentCompleted is a durable fallback if
  // the preceding turn_complete websocket frame was missed.
  if (ev["agent_id"] !== "root") return null;
  const result = ev["result"];
  return {
    type: "AGENT_COMPLETED",
    ...(typeof result === "string" && result.trim().length > 0
      ? { result: result.trim() }
      : {}),
  };
}

/**
 * Extract the assistant's final answer from a `turn_complete` event.
 *
 * When the agent uses the **`respond` tool** instead of streaming
 * tokens, the gateway maps that to `ServerMessage::TurnComplete` with
 * `final_message: Some(text)` (see `gateway/src/websocket/handler.rs`).
 * Without this helper, the assistant's final text is dropped on the
 * floor — `mapTurnCompleteEvent` only sets status to idle, the
 * streaming-token branch never fires (there were no tokens), and the
 * chat bubble stays empty. Returns null when no final_message is
 * present (the streaming-token path; the bubble is already populated).
 */
export function mapTurnCompleteFinalMessage(
  ev: ConversationEvent
): QuickChatAction | null {
  const raw = ev as unknown as Record<string, unknown>;
  if (raw["type"] !== "turn_complete") return null;
  const text = raw["final_message"];
  if (typeof text !== "string" || text.length === 0) return null;
  return { type: "RESPOND", text };
}

function mapWardChangedEvent(ev: Record<string, unknown>): QuickChatAction | null {
  // Gateway emits `ward_id: string` on the wire. Accept either the flat
  // `ward_id` (current wire format) or a nested `ward.name` (reserved for a
  // future enrichment that resolves display names server-side).
  const flat = ev["ward_id"];
  if (typeof flat === "string" && flat.length > 0) {
    return { type: "WARD_CHANGED", wardName: flat };
  }
  const ward = ev["ward"] as Record<string, unknown> | undefined;
  const name = ward?.["name"];
  if (typeof name === "string" && name.length > 0) {
    return { type: "WARD_CHANGED", wardName: name };
  }
  return null;
}

function mapSessionInitializedEvent(ev: Record<string, unknown>): QuickChatAction | null {
  const sid = ev["session_id"];
  if (!sid) return null;
  return { type: "SESSION_BOUND", sessionId: sid as string };
}

function mapDelegateToolCall(args: Record<string, unknown>): QuickChatAction {
  const agentId = (args["agent_id"] ?? args["agentId"] ?? "subagent") as string;
  return {
    type: "ADD_CHIP",
    chip: { id: randomId(), kind: "delegate", label: `→ ${agentId}` },
  };
}

function mapLoadSkillToolCall(args: Record<string, unknown>): QuickChatAction {
  const skill = (args["skill"] ?? "skill") as string;
  return {
    type: "ADD_CHIP",
    chip: { id: randomId(), kind: "skill", label: `loaded ${skill}` },
  };
}

function mapMemoryToolCall(args: Record<string, unknown>): QuickChatAction | null {
  if (args["action"] !== "recall" && args["action"] !== "get_fact") return null;
  return {
    type: "ADD_CHIP",
    chip: { id: randomId(), kind: "recall", label: "recalled" },
  };
}

function mapToolCallEvent(ev: Record<string, unknown>): QuickChatAction | null {
  // Gateway emits `tool_name: string`; some older paths use `tool`.
  const tool = (ev["tool_name"] ?? ev["tool"]) as string | undefined;
  if (typeof tool !== "string") return null;
  const args = (ev["args"] ?? {}) as Record<string, unknown>;
  switch (tool) {
    case "delegate_to_agent": return mapDelegateToolCall(args);
    case "load_skill":        return mapLoadSkillToolCall(args);
    case "memory":            return mapMemoryToolCall(args);
    default:                  return null;
  }
}

export function mapGatewayEventToQuickChatAction(ev: ConversationEvent): QuickChatAction | null {
  const raw = ev as unknown as Record<string, unknown>;
  const type = raw["type"] as string;
  switch (type) {
    case "token":               return mapTokenEvent(raw);
    case "respond":             return mapRespondEvent(raw);
    case "ward_changed":        return mapWardChangedEvent(raw);
    // Gateway emits `invoke_accepted` with session_id; `session_initialized`
    // is reserved for a future event-stream revision.
    case "invoke_accepted":
    case "session_initialized": return mapSessionInitializedEvent(raw);
    case "agent_started":       return { type: "AGENT_STARTED", agentId: (raw["agent_id"] ?? "") as string };
    case "agent_completed":     return mapRootAgentCompletedEvent(raw);
    case "turn_complete":       return { type: "TURN_COMPLETE" };
    case "tool_call":           return mapToolCallEvent(raw);
    case "error":               return { type: "ERROR", message: (raw["message"] ?? "error") as string };
    default:                    return null;
  }
}

// ---------------------------------------------------------------------------
// mapGatewayEventToPillEvent
// ---------------------------------------------------------------------------

function mapPillToolCall(ev: Record<string, unknown>): PillEvent | null {
  const tool = (ev["tool_name"] ?? ev["tool"]) as string | undefined;
  if (typeof tool !== "string") return null;
  return { kind: "tool_call", tool, args: (ev["args"] ?? {}) as Record<string, unknown> };
}

function mapPillAgentCompleted(ev: Record<string, unknown>): PillEvent {
  // Quick-chat is single-agent (optionally one subagent) so any
  // AgentCompleted from the root execution hides the pill. A more general
  // Research page would track pending delegations to decide is_final.
  return {
    kind: "agent_completed",
    agent_id: (ev["agent_id"] ?? "") as string,
    is_final: true,
  };
}

export function mapGatewayEventToPillEvent(ev: ConversationEvent): PillEvent | null {
  const raw = ev as unknown as Record<string, unknown>;
  const type = raw["type"] as string;
  switch (type) {
    case "agent_started":   return { kind: "agent_started", agent_id: (raw["agent_id"] ?? "") as string };
    case "agent_completed": return mapPillAgentCompleted(raw);
    case "tool_call":       return mapPillToolCall(raw);
    case "respond":         return { kind: "respond" };
    default:                return null;
  }
}
