import { useCallback, useEffect, useReducer, useRef, useState, type Dispatch } from "react";
import { getTransport } from "@/services/transport";
import type { Transport } from "@/services/transport";
import type {
  Artifact,
  ConversationEvent,
  SessionMessage,
  WorkSurface,
} from "@/services/transport/types";
import { randomId } from "@/shared/utils/randomId";
import { useStatusPill, type PillEventSink } from "../shared/statusPill";
import type { UploadedFile } from "../chat/ChatInput";
import { composeMessageWithAttachments } from "../chat/attachments";
import {
  type QuickChatArtifactRef,
  type QuickChatMessage,
  EMPTY_QUICK_CHAT_STATE,
} from "./types";
import { reduceQuickChat, type QuickChatAction } from "./reducer";
import {
  mapGatewayEventToQuickChatAction,
  mapGatewayEventToPillEvent,
  mapTurnCompleteFinalMessage,
} from "./event-map";

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/** Agent id for Quick Chat. The reserved chat session is bound to root server-side. */
const CHAT_AGENT_ID = "root";

/** Pinned execution mode — skips intent analysis / planning / research pipeline. */
const CHAT_MODE = "fast";

/** How many root-scoped messages to fetch on hydrate. */
const HISTORY_TAIL_LIMIT = 50;

// ---------------------------------------------------------------------------
// Pure helpers
// ---------------------------------------------------------------------------

/**
 * Filter server-side messages to the ones the user actually cares about.
 *
 * The root-scope feed carries intermediate rows — `role: "tool"` results,
 * and assistant placeholders whose content is the literal `"[tool calls]"`
 * marker emitted when the model calls a tool. Those belong in the
 * Thinking timeline (future work), not the chat bubbles.
 */
function isVisibleChatMessage(m: SessionMessage): boolean {
  if (m.role === "tool") return false;
  if (m.role === "assistant" && m.content.trim() === "[tool calls]") return false;
  return m.role === "user" || m.role === "assistant";
}

function sessionMessageToQuickChat(m: SessionMessage): QuickChatMessage {
  return {
    id: m.id,
    role: m.role === "user" ? "user" : "assistant",
    content: m.content,
    timestamp: new Date(m.created_at).getTime(),
  };
}

function artifactToRef(a: Artifact): QuickChatArtifactRef {
  return {
    id: a.id,
    fileName: a.fileName,
    fileType: a.fileType,
    fileSize: a.fileSize,
    label: a.label,
  };
}

/** Pull the session's current artifact manifest; swallow errors. */
async function fetchArtifacts(
  transport: Transport,
  sessionId: string
): Promise<QuickChatArtifactRef[]> {
  const result = await transport.listSessionArtifacts(sessionId);
  if (!result.success || !result.data) return [];
  return result.data.map(artifactToRef);
}

/** Idempotent bootstrap: init the reserved session, pull history + artifacts. */
async function bootstrapChatSession(
  transport: Transport
): Promise<{
  sessionId: string;
  conversationId: string;
  messages: QuickChatMessage[];
  artifacts: QuickChatArtifactRef[];
} | null> {
  const init = await transport.initChatSession();
  if (!init.success || !init.data) return null;

  const { sessionId, conversationId, created } = init.data;

  // New sessions have no history or artifacts to fetch.
  if (created) {
    return { sessionId, conversationId, messages: [], artifacts: [] };
  }

  const [history, artifacts] = await Promise.all([
    transport.getSessionMessages(sessionId, { scope: "root" }),
    fetchArtifacts(transport, sessionId),
  ]);
  const messages =
    history.success && history.data
      ? history.data
          .filter(isVisibleChatMessage)
          .slice(-HISTORY_TAIL_LIMIT)
          .map(sessionMessageToQuickChat)
      : [];

  return { sessionId, conversationId, messages, artifacts };
}

/** Build the WS event handler once; closure captures the stable pill sink. */
function makeEventHandler(
  pillSink: PillEventSink,
  dispatch: Dispatch<QuickChatAction>,
  onSurface: (event: ConversationEvent) => void,
) {
  return (event: ConversationEvent) => {
    if (event.type === "surface_created" || event.type === "surface_updated" || event.type === "surface_deleted") {
      onSurface(event);
      return;
    }
    // When the agent used the `respond` tool, the gateway delivers the
    // final answer in `turn_complete.final_message` rather than as a
    // bare `respond` event or as streaming tokens. Populate the bubble
    // BEFORE the TURN_COMPLETE action so the user sees the answer in
    // the same render as the status flipping back to idle.
    const respondFromComplete = mapTurnCompleteFinalMessage(event);
    if (respondFromComplete) dispatch(respondFromComplete);

    const action = mapGatewayEventToQuickChatAction(event);
    if (action) dispatch(action);
    const pillEv = mapGatewayEventToPillEvent(event);
    if (pillEv) pillSink.push(pillEv);
  };
}

// ---------------------------------------------------------------------------
// Hook
// ---------------------------------------------------------------------------

export function useQuickChat() {
  const [state, dispatch] = useReducer(reduceQuickChat, EMPTY_QUICK_CHAT_STATE);
  const { state: pillState, sink: pillSink } = useStatusPill();
  const [surfaces, setSurfaces] = useState<WorkSurface[]>([]);

  // Bootstrap idempotency guard. Set AFTER the async work resolves, not
  // before, so StrictMode's synthetic unmount doesn't leave us in a "bootstrap
  // started but never completed" state. The server-side init is idempotent
  // (same session ids on every call), so two concurrent calls in dev are
  // harmless — this ref only guarantees we dispatch HYDRATE once.
  const bootstrappedRef = useRef(false);
  const subscribedConvIdRef = useRef<string | null>(null);

  // --- Bootstrap: init reserved session + hydrate history ---
  useEffect(() => {
    if (bootstrappedRef.current) return;
    (async () => {
      const transport = await getTransport();
      const result = await bootstrapChatSession(transport);
      if (bootstrappedRef.current) return;
      bootstrappedRef.current = true;
      if (!result) {
        dispatch({ type: "ERROR", message: "Failed to initialise chat" });
        return;
      }
      dispatch({
        type: "HYDRATE",
        sessionId: result.sessionId,
        conversationId: result.conversationId,
        messages: result.messages,
        wardName: null, // populated by later WardChanged events
        artifacts: result.artifacts,
      });
    })();
  }, []);

  // --- Subscribe to WS events for the persisted conversationId ---
  useEffect(() => {
    const convId = state.conversationId;
    if (!convId || subscribedConvIdRef.current === convId) return;
    subscribedConvIdRef.current = convId;
    const onEvent = makeEventHandler(pillSink, dispatch, (event) => {
      const raw = event as unknown as { surface?: WorkSurface; surface_id?: string };
      if (event.type === "surface_deleted" && raw.surface_id) {
        setSurfaces(current => current.filter(item => item.surface_id !== raw.surface_id));
      } else if (raw.surface) {
        setSurfaces(current => [...current.filter(item => item.surface_id !== raw.surface!.surface_id), raw.surface!]);
      }
    });
    const unsubscribe = Promise.resolve().then(async () => {
      const transport = await getTransport();
      return transport.subscribeConversation(convId, { onEvent });
    });
    return () => {
      unsubscribe.then((fn) => fn?.()).catch(() => {
        /* no-op */
      });
      if (subscribedConvIdRef.current === convId) {
        subscribedConvIdRef.current = null;
      }
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [state.conversationId]);

  // --- Refresh artifacts on turn completion ---
  // When a turn finishes the agent may have written new files; pull the
  // artifact manifest so cards appear in the assistant bubble.
  useEffect(() => {
    if (state.status !== "idle" || !state.sessionId) return;
    let cancelled = false;
    (async () => {
      const transport = await getTransport();
      const fresh = await fetchArtifacts(transport, state.sessionId!);
      if (cancelled) return;
      dispatch({ type: "SET_ARTIFACTS", artifacts: fresh });
    })();
    return () => { cancelled = true; };
  }, [state.status, state.sessionId]);

  // --- Send a user message against the reserved session ---
  const sendMessage = useCallback(
    async (text: string, attachments: UploadedFile[] = []) => {
      const trimmed = text.trim();
      if (!trimmed || state.status === "running") return;
      if (!state.sessionId || !state.conversationId) return;
      // Splice uploaded-file metadata (incl. absolute server paths) into the
      // prompt — executeAgent has no separate attachments channel, so the
      // agent only learns about the upload through the message text.
      const promptText = composeMessageWithAttachments(trimmed, attachments);
      dispatch({
        type: "APPEND_USER",
        message: {
          id: randomId(),
          role: "user",
          content: promptText,
          timestamp: Date.now(),
        },
      });
      const transport = await getTransport();
      const result = await transport.executeAgent(
        CHAT_AGENT_ID,
        state.conversationId,
        promptText,
        state.sessionId,
        CHAT_MODE
      );
      if (!result.success) {
        dispatch({ type: "ERROR", message: result.error ?? "Failed to send" });
      }
    },
    [state.status, state.conversationId, state.sessionId]
  );

  // --- Stop a running turn ---
  const stopAgent = useCallback(async () => {
    if (state.status !== "running" || !state.conversationId) return;
    const transport = await getTransport();
    await transport.stopAgent(state.conversationId);
  }, [state.status, state.conversationId]);

  // --- Clear the reserved session and bootstrap a fresh one ---
  const clearSession = useCallback(async () => {
    const transport = await getTransport();
    const deleted = await transport.deleteChatSession();
    if (!deleted.success) {
      dispatch({ type: "ERROR", message: deleted.error ?? "Failed to clear chat" });
      return;
    }
    // Bootstrap again; the init endpoint self-heals into a new session.
    const fresh = await bootstrapChatSession(transport);
    if (!fresh) {
      dispatch({ type: "ERROR", message: "Failed to initialise a new chat after clear" });
      return;
    }
    dispatch({
      type: "HYDRATE",
      sessionId: fresh.sessionId,
      conversationId: fresh.conversationId,
      messages: fresh.messages,
      wardName: null,
      artifacts: fresh.artifacts,
    });
  }, []);

  return { state, pillState, surfaces, sendMessage, stopAgent, clearSession };
}
