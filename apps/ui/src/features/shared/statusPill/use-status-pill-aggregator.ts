import { useMemo, useReducer } from "react";
import { type PillState, EMPTY_PILL, NARRATION_MAX } from "./types";
import { describeTool } from "./tool-phrase";

// Normalized events consumed by the pill.
//
// Thinking events are intentionally *not* in this union: provider-emitted
// thinking is either absent (simple prompts) or streams per-token (complex
// prompts), so driving pill narration from it flashes unreadably or leaves
// it empty. The pill is deterministic: AgentStarted → "Thinking…",
// ToolCall → tool phrase, Respond → "Responding", AgentCompleted → fade.
//
// Error events (LLM-level or tool-level) are sticky: once rendered, they
// persist through further tool_call/respond events until the next
// agent_started or a reset. Goal: users see why something failed without
// having to dig into logs.
export type PillEvent =
  | { kind: "idle" }
  | { kind: "reset" }
  | { kind: "agent_started"; agent_id: string }
  | { kind: "agent_completed"; agent_id: string; is_final: boolean }
  | { kind: "tool_call"; tool: string; args: Record<string, unknown> }
  | { kind: "tool_ok"; tool: string }
  | { kind: "respond" }
  | { kind: "error"; message: string; source: "llm" | "tool"; tool?: string };

const STARTING_NARRATION = "Thinking…";
const RESPONDING_NARRATION = "Responding";
const ERROR_NARRATION_LLM = "LLM error";
const ERROR_NARRATION_TOOL_PREFIX = "Tool error";

function truncateMessage(text: string, max: number): string {
  if (text.length <= max) return text;
  return text.slice(0, max - 1) + "…";
}

function errorNarration(source: "llm" | "tool", tool: string | undefined): string {
  if (source === "llm") return ERROR_NARRATION_LLM;
  return tool ? `${ERROR_NARRATION_TOOL_PREFIX}: ${tool}` : ERROR_NARRATION_TOOL_PREFIX;
}

// `agent_started` always overwrites narration/suffix/category, which is what
// implicitly clears a sticky error state when the next agent begins running.
function handleAgentStarted(state: PillState): PillState {
  return {
    ...state,
    visible: true,
    starting: true,
    narration: STARTING_NARRATION,
    suffix: "",
    category: "neutral",
    swapCounter: state.swapCounter + 1,
  };
}

function handleError(
  state: PillState,
  ev: { kind: "error"; message: string; source: "llm" | "tool"; tool?: string }
): PillState {
  return {
    ...state,
    visible: true,
    starting: false,
    narration: errorNarration(ev.source, ev.tool),
    suffix: truncateMessage(ev.message, NARRATION_MAX),
    category: "error",
    swapCounter: state.swapCounter + 1,
  };
}

function handleAgentCompleted(
  state: PillState,
  ev: { kind: "agent_completed"; agent_id: string; is_final: boolean }
): PillState {
  if (ev.is_final) {
    return { ...EMPTY_PILL, swapCounter: state.swapCounter + 1 };
  }
  return state;
}

function handleToolCall(
  state: PillState,
  tool: string,
  args: Record<string, unknown>
): PillState {
  const phrase = describeTool(tool, args);
  return {
    ...state,
    visible: true,
    starting: false,
    narration: phrase.narration,
    suffix: phrase.suffix,
    category: phrase.category,
    swapCounter: state.swapCounter + 1,
  };
}

function handleRespond(state: PillState): PillState {
  return {
    ...state,
    visible: true,
    starting: false,
    narration: RESPONDING_NARRATION,
    suffix: "",
    category: "respond",
    swapCounter: state.swapCounter + 1,
  };
}

export function reducePillState(state: PillState, ev: PillEvent): PillState {
  // Sticky error: once the pill is in "error" category, keep it until the
  // model recovers, the next `agent_started`, or `reset`. tool_call /
  // respond / agent_completed must not overwrite the error — the whole
  // point is to give the user a chance to see why something failed without
  // chasing logs. A SUCCESSFUL tool result is the recovery signal: it
  // clears the error so a retried-and-fixed call doesn't leave the pill
  // stuck on "Tool error" for the rest of the session.
  if (state.category === "error") {
    if (ev.kind === "tool_ok") {
      return {
        ...state,
        narration: "Recovered — continuing",
        suffix: "",
        category: "neutral",
        swapCounter: state.swapCounter + 1,
      };
    }
    if (ev.kind === "tool_call" || ev.kind === "respond" || ev.kind === "agent_completed") {
      return state;
    }
  }
  switch (ev.kind) {
    case "idle":
      return state;
    case "reset":
      return EMPTY_PILL;
    case "agent_started":
      return handleAgentStarted(state);
    case "agent_completed":
      return handleAgentCompleted(state, ev);
    case "tool_call":
      return handleToolCall(state, ev.tool, ev.args);
    case "tool_ok":
      // Successes are invisible outside the error state — the pill's normal
      // narration is driven by tool_call, and a "done" flicker after every
      // call would be noise.
      return state;
    case "respond":
      return handleRespond(state);
    case "error":
      return handleError(state, ev);
    default:
      return state;
  }
}

/**
 * React hook wrapper — subscribe to pill events through the returned sink.
 * The parent page's event router calls `sink.push(PillEvent)` for each relevant
 * event; the hook folds them into PillState via the pure reducer.
 */
export interface PillEventSink {
  push(ev: PillEvent): void;
}

export function useStatusPill(): { state: PillState; sink: PillEventSink } {
  const [state, dispatch] = useReducer(reducePillState, EMPTY_PILL);
  // `dispatch` is already stable across renders; memoising `sink` preserves
  // its identity so consumers that pass it into effect-deps don't tear down
  // and rebuild their subscriptions on every render.
  const sink = useMemo<PillEventSink>(() => ({ push: dispatch }), []);
  return { state, sink };
}
