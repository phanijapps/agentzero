import { randomId } from "@/shared/utils/randomId";
import type { QuickChatArtifactRef, QuickChatMessage, QuickChatState, QuickChatInlineChip } from "./types";

export type QuickChatAction =
  | { type: "HYDRATE"; sessionId: string; conversationId: string; messages: QuickChatMessage[]; wardName: string | null; artifacts: QuickChatArtifactRef[] }
  | { type: "APPEND_USER"; message: QuickChatMessage }
  | { type: "SESSION_BOUND"; sessionId: string }
  | { type: "AGENT_STARTED"; agentId: string }
  | { type: "AGENT_COMPLETED"; result?: string }
  | { type: "TOKEN"; text: string }
  | { type: "RESPOND"; text: string }
  | { type: "ADD_CHIP"; chip: QuickChatInlineChip }
  | { type: "TURN_COMPLETE" }
  | { type: "ERROR"; message: string }
  | { type: "WARD_CHANGED"; wardName: string }
  | { type: "SET_ARTIFACTS"; artifacts: QuickChatArtifactRef[] };

function upsertStreamingAssistant(
  messages: QuickChatMessage[],
  text: string,
  replace: boolean,
): QuickChatMessage[] {
  const last = messages[messages.length - 1];
  if (last?.role === "assistant" && last.streaming) {
    const updated: QuickChatMessage = {
      ...last,
      content: replace ? text : last.content + text,
      streaming: !replace,
    };
    return [...messages.slice(0, -1), updated];
  }
  return [
    ...messages,
    {
      id: randomId(),
      role: "assistant",
      content: text,
      timestamp: Date.now(),
      streaming: !replace,
    },
  ];
}

function attachChipToLatestAssistant(
  messages: QuickChatMessage[],
  chip: QuickChatInlineChip,
): QuickChatMessage[] {
  for (let i = messages.length - 1; i >= 0; i--) {
    if (messages[i].role === "assistant") {
      const chips = [...(messages[i].chips ?? []), chip];
      return [
        ...messages.slice(0, i),
        { ...messages[i], chips },
        ...messages.slice(i + 1),
      ];
    }
  }
  return messages;
}

export function reduceQuickChat(state: QuickChatState, action: QuickChatAction): QuickChatState {
  switch (action.type) {
    case "HYDRATE":
      return {
        ...state,
        sessionId: action.sessionId,
        conversationId: action.conversationId,
        messages: action.messages,
        activeWardName: action.wardName,
        status: "idle",
        artifacts: action.artifacts,
      };
    case "APPEND_USER":
      return { ...state, messages: [...state.messages, action.message], status: "running" };
    case "SESSION_BOUND":
      return { ...state, sessionId: action.sessionId };
    case "AGENT_STARTED":
      return { ...state, status: "running" };
    case "AGENT_COMPLETED": {
      const lastMessage = state.messages[state.messages.length - 1];
      // `agent_completed.result` is recovery-only. A preceding
      // `turn_complete.final_message` has already rendered the final bubble,
      // so applying the same terminal response again would duplicate it.
      const alreadyHasFinalResponse =
        lastMessage?.role === "assistant" && !lastMessage.streaming;
      return {
        ...state,
        messages: action.result && !alreadyHasFinalResponse
          ? upsertStreamingAssistant(state.messages, action.result, true)
          : state.messages,
        status: "idle",
      };
    }
    case "TOKEN":
      return { ...state, messages: upsertStreamingAssistant(state.messages, action.text, false) };
    case "RESPOND":
      return { ...state, messages: upsertStreamingAssistant(state.messages, action.text, true) };
    case "ADD_CHIP":
      return { ...state, messages: attachChipToLatestAssistant(state.messages, action.chip) };
    case "TURN_COMPLETE":
      return { ...state, status: "idle" };
    case "ERROR":
      return { ...state, status: "error" };
    case "WARD_CHANGED":
      return { ...state, activeWardName: action.wardName };
    case "SET_ARTIFACTS":
      return { ...state, artifacts: action.artifacts };
    default:
      return state;
  }
}
