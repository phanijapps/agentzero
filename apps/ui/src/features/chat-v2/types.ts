export type QuickChatMessageRole = "user" | "assistant";

export interface QuickChatInlineChip {
  id: string;
  kind: "recall" | "skill" | "delegate";
  label: string;   // e.g., "recalled 2", "loaded web-read", "→ writer-agent"
  detail?: string; // expanded tooltip / panel content
}

/** A persisted upload rendered without exposing its server-side path. */
export interface QuickChatAttachment {
  name: string;
  mimeType: string;
  sizeLabel: string;
}

export interface QuickChatMessage {
  id: string;
  role: QuickChatMessageRole;
  content: string;                       // markdown for assistant, plain for user
  timestamp: number;
  attachments?: QuickChatAttachment[];   // user-only
  chips?: QuickChatInlineChip[];         // assistant-only
  streaming?: boolean;                   // true while Token events still arriving
}

export type QuickChatStatus = "idle" | "running" | "error";

export interface QuickChatArtifactRef {
  id: string;
  fileName: string;
  fileType?: string;
  fileSize?: number;
  label?: string;
}

export interface QuickChatState {
  /** Set by HYDRATE after the reserved chat session is bootstrapped. */
  sessionId: string | null;
  /** Stable WS routing id for the reserved chat session. */
  conversationId: string | null;
  messages: QuickChatMessage[];
  status: QuickChatStatus;
  activeWardName: string | null;
  /** Files the agent wrote during this session, newest-last. */
  artifacts: QuickChatArtifactRef[];
}

export const EMPTY_QUICK_CHAT_STATE: QuickChatState = {
  sessionId: null,
  conversationId: null,
  messages: [],
  status: "idle",
  activeWardName: null,
  artifacts: [],
};
