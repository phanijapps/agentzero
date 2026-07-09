import { useCallback, useEffect, useReducer, useRef, useState, type Dispatch } from "react";
import { useNavigate, useParams } from "react-router-dom";
import { getTransport } from "@/services/transport";
import type {
  Artifact,
  ConversationEvent,
  UnsubscribeFn,
} from "@/services/transport/types";
import { randomId } from "@/shared/utils/randomId";
import { useStatusPill, type PillEventSink } from "../shared/statusPill";
import type { UploadedFile } from "../chat/ChatInput";
import { composeMessageWithAttachments } from "../chat/attachments";
import { EMPTY_RESEARCH_STATE, type ResearchSessionState } from "./types";
import { reduceResearch, type ResearchAction } from "./reducer";
import { mapGatewayEventToResearchAction, mapGatewayEventToPillEvent } from "./event-map";
import { snapshotSession } from "./session-snapshot";

const ROOT_AGENT_ID = "root";
const FILE_MUTATION_TOOLS = new Set(["write_file", "edit_file"]);
// Client-owned conv_id prefix. Research has no `/api/chat/init`, so the UI
// mints the id and subscribes to it BEFORE invoke — that ordering is what
// lets the first token reach the UI (R14a).
const CONV_ID_PREFIX = "research-";
// pillSink from useStatusPill() has a stable identity (memoised sink).
// Omitting from useCallback deps is intentional; closure captures the latest
// reference. Per-line eslint-disable is still required for the linter.

// --- Event synthesis (respond-tool + delegation_completed) ----------------

/**
 * When the agent calls the `respond` tool, the gateway broadcasts:
 *   - `tool_call` with `tool_name: "respond"` and `args: { message: "..." }`
 *   - `tool_result` with the acknowledgement
 *   - `turn_complete` with `final_message: ""` (empty — the respond message
 *     is NOT in final_message; Done.final_message is populated only from
 *     streamed tokens, not from the respond tool).
 *
 * So the definitive source of the final answer is `tool_call.args.message`
 * on the `respond` tool. Synthesize a RESPOND action for it.
 */
function respondActionFromToolCall(
  event: Record<string, unknown>,
): ResearchAction | null {
  if (event["type"] !== "tool_call") return null;
  const toolName = event["tool_name"] ?? event["tool"];
  if (toolName !== "respond") return null;
  const args = event["args"];
  if (!args || typeof args !== "object") return null;
  const message = (args as Record<string, unknown>)["message"];
  if (typeof message !== "string" || message.length === 0) return null;
  const execId = event["execution_id"];
  const turnId = typeof execId === "string" && execId.length > 0 ? execId : "orphan";
  return { type: "RESPOND", turnId, text: message };
}

/**
 * `delegation_completed` is the only event we reliably receive for a child
 * subagent (its own WS events run on a different conv_id). The `result`
 * field carries the child's final answer — populate the child turn's
 * respond body from it so the nested turn renders its output.
 */
function respondActionFromDelegationCompleted(
  event: Record<string, unknown>,
): ResearchAction | null {
  if (event["type"] !== "delegation_completed") return null;
  const childExec = event["child_execution_id"];
  if (typeof childExec !== "string" || childExec.length === 0) return null;
  const result = event["result"];
  if (typeof result !== "string" || result.length === 0) return null;
  return { type: "RESPOND", turnId: childExec, text: result };
}

interface EventHandlerCtx {
  pillSink: PillEventSink;
  dispatch: Dispatch<ResearchAction>;
  fileMutationToolIdsRef: { current: Set<string> };
  onWardFileMutation: () => void;
  /** Called once per root `agent_completed` — used to re-snapshot. */
  onRootAgentCompleted: (executionId: string) => void;
  /** Called (debounced) when a self-heal reconcile should run — e.g. a
   *  delegate_to_agent tool call fired, or a delegation_completed arrived,
   *  both of which indicate child-turn state needs a pull from REST. */
  onReconcileHint: () => void;
}

function makeEventHandler(ctx: EventHandlerCtx) {
  return (event: ConversationEvent) => {
    const raw = event as unknown as Record<string, unknown>;
    console.debug("[research-v2] event:", raw["type"], "sid:", raw["session_id"], "cid:", raw["conversation_id"], "eid:", raw["execution_id"]);
    const action = mapGatewayEventToResearchAction(event);
    if (action) ctx.dispatch(action);
    // Respond-tool path: synthesize RESPOND from tool_call.args.message
    // because turn_complete.final_message arrives empty for tool-emitted
    // responses (Done.final_message is populated only from streamed tokens).
    const synthesizedRespond = respondActionFromToolCall(raw);
    if (synthesizedRespond) ctx.dispatch(synthesizedRespond);
    const synthesizedChildRespond = respondActionFromDelegationCompleted(raw);
    if (synthesizedChildRespond) ctx.dispatch(synthesizedChildRespond);
    const pillEv = mapGatewayEventToPillEvent(event);
    if (pillEv) ctx.pillSink.push(pillEv);
    handleWardFileMutation(raw, ctx.fileMutationToolIdsRef, ctx.onWardFileMutation);
    // R14f: re-snapshot on root agent_completed to backfill anything WS dropped
    // (session title, artifacts, subagent completions, per-turn respond).
    handleRootAgentCompleted(raw, ctx.onRootAgentCompleted);
    // R14i: self-heal reconcile on delegation markers. delegation_started can
    // land BEFORE our session-scope subscription acks (seq race). The
    // delegate_to_agent tool_call always arrives via the conv-id subscription,
    // so use it as a reliable trigger to pull a fresh snapshot and backfill
    // any child turn we may have missed. delegation_completed triggers the
    // same hint so a second subagent in the same session also heals.
    handleReconcileHint(raw, ctx.onReconcileHint);
  };
}

function toolCallIdOf(raw: Record<string, unknown>): string | null {
  const id = raw["tool_call_id"] ?? raw["tool_id"];
  return typeof id === "string" && id.length > 0 ? id : null;
}

function handleWardFileMutation(
  raw: Record<string, unknown>,
  fileMutationToolIdsRef: { current: Set<string> },
  onWardFileMutation: () => void,
): void {
  const type = raw["type"];
  const id = toolCallIdOf(raw);
  if (!id) return;
  if (type === "tool_call") {
    const tool = raw["tool_name"] ?? raw["tool"];
    if (typeof tool === "string" && FILE_MUTATION_TOOLS.has(tool)) {
      fileMutationToolIdsRef.current.add(id);
    }
    return;
  }
  if (type !== "tool_result" || !fileMutationToolIdsRef.current.has(id)) return;
  fileMutationToolIdsRef.current.delete(id);
  const error = raw["error"];
  if (typeof error === "string" && error.length > 0) return;
  onWardFileMutation();
}

function handleReconcileHint(
  raw: Record<string, unknown>,
  onReconcileHint: () => void,
): void {
  const type = raw["type"];
  if (type === "delegation_started" || type === "delegation_completed") {
    onReconcileHint();
    return;
  }
  if (type === "tool_call") {
    const tool = raw["tool_name"] ?? raw["tool"];
    if (tool === "delegate_to_agent") onReconcileHint();
  }
}

function handleRootAgentCompleted(
  raw: Record<string, unknown>,
  onRootAgentCompleted: (executionId: string) => void,
): void {
  if (raw["type"] !== "agent_completed") return;
  const parent = raw["parent_execution_id"];
  // Only root turns have a null/empty parent — children's completions don't
  // need a reconcile because their own state was already snapshot-sourced.
  const isRoot = parent == null || parent === "";
  if (!isRoot) return;
  const execId = raw["execution_id"];
  if (typeof execId !== "string" || execId.length === 0) return;
  onRootAgentCompleted(execId);
}

// --- Subscription refs ----------------------------------------------------

interface SubscriptionRefs {
  subscribedConvIdRef: React.RefObject<string | null>;
  unsubscribeRef: React.RefObject<UnsubscribeFn | null>;
}

/** Idempotent — no-op when convId matches the currently-subscribed one. */
async function ensureSubscription(
  convId: string,
  onEvent: (event: ConversationEvent) => void,
  refs: SubscriptionRefs,
): Promise<void> {
  if (refs.subscribedConvIdRef.current === convId) return;
  const transport = await getTransport();
  const unsubscribe = transport.subscribeConversation(convId, { onEvent });
  refs.subscribedConvIdRef.current = convId;
  refs.unsubscribeRef.current = unsubscribe;
}

function teardownSubscription(refs: SubscriptionRefs): void {
  const unsub = refs.unsubscribeRef.current;
  refs.unsubscribeRef.current = null;
  refs.subscribedConvIdRef.current = null;
  if (!unsub) return;
  try {
    unsub();
  } catch (err) {
    console.warn("[research-v2] unsubscribe failed", err);
  }
}

// --- Snapshot → HYDRATE dispatch -----------------------------------------

async function hydrateFromSnapshot(
  sessionId: string,
  dispatch: Dispatch<ResearchAction>,
  latestArtifactsRef: { current: Artifact[] },
): Promise<void> {
  const transport = await getTransport();
  const snap = await snapshotSession(transport, sessionId);
  if (!snap) {
    dispatch({ type: "ERROR", message: "Failed to load session" });
    return;
  }
  dispatch({
    type: "HYDRATE",
    sessionId,
    conversationId: snap.conversationId,
    title: snap.title,
    status: snap.status,
    wardId: snap.wardId,
    wardName: snap.wardName,
    rootExecutionId: snap.rootExecutionId,
    turns: snap.turns,
    artifacts: snap.artifacts,
  });
  // Mirror the artifact records in the ref so the slide-out can resolve
  // id → Artifact without a second fetch. snapshotSession already pulled
  // /artifacts once; reuse its decision here via a parallel call to keep the
  // cache hot. On failure we just skip — slide-out will re-fetch on demand.
  try {
    const res = await transport.listSessionArtifacts(sessionId);
    if (res.success && res.data) latestArtifactsRef.current = res.data;
  } catch {
    // Intentionally silent — the snapshot's refs are enough for rendering.
  }
}

// --- R14h: reconnect recovery helper --------------------------------------
//
// Finds a running /api/logs/sessions row whose started_at is within a
// window of our sendMessage timestamp and dispatches SESSION_BOUND so
// R14g can take over. No-op if:
//   - state.sessionId is already set (normal flow)
//   - state.status is not "running" (nothing to recover)
//   - we never sent anything (lastSendMsRef is null)

const RECONNECT_RECOVERY_WINDOW_MS = 15_000;

async function recoverSessionIdIfNeeded(
  state: ResearchSessionState,
  lastSendMsRef: { current: number | null },
  dispatch: Dispatch<ResearchAction>,
): Promise<void> {
  if (state.sessionId || state.status !== "running") return;
  const sendAt = lastSendMsRef.current;
  if (sendAt == null) return;
  const transport = await getTransport();
  const res = await transport.listLogSessions();
  if (!res.success || !res.data) return;
  // Wire quirk: LogSession.conversation_id is the real sess-*; session_id
  // is the execution id. Find a root row (no parent) with status "running"
  // that started close to our send time.
  const match = res.data.find((row) => {
    if (row.parent_session_id && row.parent_session_id.length > 0) return false;
    if (row.status !== "running") return false;
    const t = Date.parse(row.started_at);
    if (Number.isNaN(t)) return false;
    const delta = Math.abs(t - sendAt);
    return delta <= RECONNECT_RECOVERY_WINDOW_MS;
  });
  if (!match) return;
  dispatch({
    type: "SESSION_BOUND",
    sessionId: match.conversation_id,
    conversationId: match.conversation_id,
  });
}

// --- R14i: debounced reconcile --------------------------------------------
//
// Delegation lifecycle events can race the session-scope subscription ack:
// delegation_started may arrive BEFORE the server knows the subscription
// exists, so it's filtered and dropped. The same goes for the first few
// events after each subagent spawn. Rather than polling, react to signals:
// delegate_to_agent tool_call (always flows via conv-id scope), and
// delegation_started / delegation_completed when they do arrive, trigger a
// snapshot. Debounced to 800 ms so a burst collapses to one /api call.

const RECONCILE_DEBOUNCE_MS = 800;

function scheduleReconcile(
  sessionId: string,
  dispatch: Dispatch<ResearchAction>,
  latestArtifactsRef: { current: Artifact[] },
  timerRef: { current: ReturnType<typeof setTimeout> | null },
): void {
  if (timerRef.current !== null) clearTimeout(timerRef.current);
  timerRef.current = setTimeout(() => {
    timerRef.current = null;
    void hydrateFromSnapshot(sessionId, dispatch, latestArtifactsRef);
  }, RECONCILE_DEBOUNCE_MS);
}

function makeDebouncedReconcile(
  sessionId: string,
  dispatch: Dispatch<ResearchAction>,
  latestArtifactsRef: { current: Artifact[] },
): () => void {
  const timer: { current: ReturnType<typeof setTimeout> | null } = { current: null };
  return () => scheduleReconcile(sessionId, dispatch, latestArtifactsRef, timer);
}

// --- Hook -----------------------------------------------------------------

export function useResearchSession() {
  const { sessionId: urlSessionId } = useParams<{ sessionId: string }>();
  const navigate = useNavigate();
  const [state, dispatch] = useReducer(reduceResearch, EMPTY_RESEARCH_STATE);
  const [wardVaultRevision, setWardVaultRevision] = useState(0);
  const { state: pillState, sink: pillSink } = useStatusPill();

  const hydratedForSessionRef = useRef<string | null>(null); // one-shot hydration guard (StrictMode)
  const subscribedConvIdRef = useRef<string | null>(null); // R14a: sendMessage owns subscription
  const unsubscribeRef = useRef<UnsubscribeFn | null>(null);
  // R14g: second subscription on state.sessionId + scope="session". Receives
  // events routed by session_id (delegation_started/_completed,
  // session_title_changed, subagent agent_started/_completed, etc.) that the
  // conv-id-keyed subscription misses because those events lack a top-level
  // conversation_id field. Transport's seq-based dedup handles any overlap.
  const subscribedSessionIdRef = useRef<string | null>(null);
  const unsubscribeSessionRef = useRef<UnsubscribeFn | null>(null);
  // R14h: sendMessage timestamp. On reconnect/recovery, we match the
  // server-assigned session_id by finding a running /api/logs/sessions row
  // whose started_at is within ±10s of this stamp. Without it we'd guess.
  const lastSendMsRef = useRef<number | null>(null);
  // R14i: debounce timer shared across the two subscription sites so hints
  // landing in rapid succession (e.g. delegation_started + tool_call +
  // delegation_completed in the same second) collapse to one reconcile.
  const reconcileTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  // state holds light refs; latestArtifactsRef mirrors the full Artifact
  // records so ArtifactSlideOut can resolve id → Artifact without refetching.
  const latestArtifactsRef = useRef<Artifact[]>([]);
  // Guard against redundant re-snapshots when agent_completed fires more than
  // once for the same root execution (WS redelivery or duplicate dispatch).
  const resnapshotForExecRef = useRef<string | null>(null);
  const fileMutationToolIdsRef = useRef<Set<string>>(new Set());
  const bumpWardVaultRevision = useCallback(() => {
    setWardVaultRevision((current) => current + 1);
  }, []);

  // --- Hydrate an EXISTING session (only when URL carries one) ---
  useEffect(() => {
    if (!urlSessionId || hydratedForSessionRef.current === urlSessionId) return;
    (async () => {
      await hydrateFromSnapshot(urlSessionId, dispatch, latestArtifactsRef);
      // Set AFTER the dispatch (chat-v2 learning #6) so StrictMode's first
      // mount re-entering doesn't skip dispatch via a pre-completion flag.
      hydratedForSessionRef.current = urlSessionId;
    })();
  }, [urlSessionId]);

  // --- Subscription cleanup on unmount (StrictMode-safe). ---
  useEffect(() => {
    const convRefs: SubscriptionRefs = { subscribedConvIdRef, unsubscribeRef };
    const sessionRefs: SubscriptionRefs = {
      subscribedConvIdRef: subscribedSessionIdRef,
      unsubscribeRef: unsubscribeSessionRef,
    };
    return () => {
      teardownSubscription(convRefs);
      teardownSubscription(sessionRefs);
    };
  }, []);

  // --- R14g: session-id subscription (scope="session"). Fires whenever a
  // session is RUNNING and its sessionId is known (from snapshot hydrate OR
  // invoke_accepted). Receives session-routed events the conv-id subscription
  // misses (delegation, title change, subagent lifecycle). Idle/complete
  // sessions don't subscribe — nothing more to receive. ---
  useEffect(() => {
    const sid = state.sessionId;
    if (!sid || state.status !== "running") return;
    if (subscribedSessionIdRef.current === sid) return;
    const onRootAgentCompleted = (execId: string) => {
      if (resnapshotForExecRef.current === execId) return;
      resnapshotForExecRef.current = execId;
      void hydrateFromSnapshot(sid, dispatch, latestArtifactsRef);
    };
    const onReconcileHint = makeDebouncedReconcile(sid, dispatch, latestArtifactsRef);
    const onEvent = makeEventHandler({
      pillSink,
      dispatch,
      fileMutationToolIdsRef,
      onWardFileMutation: bumpWardVaultRevision,
      onRootAgentCompleted,
      onReconcileHint,
    });
    // Tear down any prior session-id subscription, then register the new one.
    teardownSubscription({
      subscribedConvIdRef: subscribedSessionIdRef,
      unsubscribeRef: unsubscribeSessionRef,
    });
    let cancelled = false;
    void (async () => {
      const transport = await getTransport();
      if (cancelled) return;
      // scope="all" rather than "session" so child tool_calls / thinking
      // reach us. The server-side session scope filter drops events whose
      // execution_id isn't a root (see gateway/websocket/subscriptions.rs
      // should_send_to_scope) — that's what was making the top pill go
      // silent once a subagent took over. All events for this session
      // are still routed here because the server keys by session_id.
      const unsub = transport.subscribeConversation(sid, {
        scope: "all",
        onEvent,
      });
      subscribedSessionIdRef.current = sid;
      unsubscribeSessionRef.current = unsub;
      // Race catch-up: session-scope subscription takes a round-trip to ack
      // (server response arrives at some seq N > 0). Any session-level events
      // that fired between invoke_accepted and our ack — typically
      // delegation_started and the first subagent agent_started — are dropped
      // forever. Pull a snapshot immediately to backfill those turns from
      // /api/logs/sessions. Reducer actions are idempotent so any overlap
      // with live events is harmless.
      if (cancelled) return;
      void hydrateFromSnapshot(sid, dispatch, latestArtifactsRef);
    })();
    return () => {
      cancelled = true;
    };
    // pillSink has stable identity; dispatch is stable; intentional exhaustive-deps skip.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [state.sessionId, state.status, bumpWardVaultRevision]);

  // --- Sync URL when the backend hands us a session id ---
  useEffect(() => {
    if (state.sessionId && urlSessionId !== state.sessionId) {
      navigate(`/research/${state.sessionId}`, { replace: true });
    }
  }, [state.sessionId, urlSessionId, navigate]);

  // --- R14h: reconnect recovery. ----------------------------------------
  // Scenario: ping-timeout WS reconnect during an active send. invoke_accepted
  // was sent into the dead window and is lost forever (not replayed on
  // reconnect). state.sessionId stays null → R14g can't subscribe → UI stuck.
  // Recovery: watch for WS reconnects; if status=running with sessionId null
  // and we have a recent sendMessage, match a running /api/logs/sessions row
  // by started_at window and bind its session id into state.
  useEffect(() => {
    let cancelled = false;
    let unsubscribeConnState: UnsubscribeFn | null = null;
    void (async () => {
      const transport = await getTransport();
      if (cancelled) return;
      unsubscribeConnState = transport.onConnectionStateChange((connState) => {
        if (connState.status !== "connected") return;
        void recoverSessionIdIfNeeded(state, lastSendMsRef, dispatch);
      });
    })();
    return () => {
      cancelled = true;
      unsubscribeConnState?.();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [state.sessionId, state.status]);

  // --- Send a user message (subscribes BEFORE invoke, R14a) ---
  const sendMessage = useCallback(
    async (text: string, attachments: UploadedFile[] = []) => {
      const trimmed = text.trim();
      if (!trimmed || state.status === "running") return;
      // Splice uploaded-file metadata (incl. absolute server paths) into the
      // prompt — executeAgent has no separate attachments channel, so the
      // agent only learns about the upload through the message text.
      const promptText = composeMessageWithAttachments(trimmed, attachments);
      const sendAt = Date.now();
      lastSendMsRef.current = sendAt;
      dispatch({
        type: "APPEND_USER",
        message: {
          id: randomId(),
          content: promptText,
          createdAt: new Date(sendAt).toISOString(),
        },
      });
      // Closure read: safe because only SESSION_BOUND (dispatched below) mutates state.conversationId.
      const convId = state.conversationId ?? `${CONV_ID_PREFIX}${randomId()}`;
      const refs: SubscriptionRefs = { subscribedConvIdRef, unsubscribeRef };
      const onRootAgentCompleted = (execId: string) => {
        if (resnapshotForExecRef.current === execId) return;
        resnapshotForExecRef.current = execId;
        const sid = state.sessionId;
        if (!sid) return;
        void hydrateFromSnapshot(sid, dispatch, latestArtifactsRef);
      };
      // Reconcile hint reads the latest sessionId each fire via a getter so
      // pre-invoke-accepted delegations still trigger a snapshot once the
      // session id lands.
      const onReconcileHint = () => {
        const sid = state.sessionId;
        if (!sid) return;
        scheduleReconcile(sid, dispatch, latestArtifactsRef, reconcileTimerRef);
      };
      const onEvent = makeEventHandler({
        pillSink,
        dispatch,
        fileMutationToolIdsRef,
        onWardFileMutation: bumpWardVaultRevision,
        onRootAgentCompleted,
        onReconcileHint,
      });
      try {
        await ensureSubscription(convId, onEvent, refs);
        console.debug("[research-v2] sendMessage: subscribed to", convId);
        // Pre-invoke SESSION_BOUND seeds state.conversationId. The server's
        // invoke_accepted SESSION_BOUND re-dispatches with session_id; the
        // reducer's null-guard preserves whichever id lands first.
        dispatch({
          type: "SESSION_BOUND",
          conversationId: convId,
          sessionId: state.sessionId,
        });
        const transport = await getTransport();
        const result = await transport.executeAgent(
          ROOT_AGENT_ID,
          convId,
          promptText,
          state.sessionId ?? undefined,
          "deep",
        );
        console.debug("[research-v2] sendMessage: executeAgent result", result.success, result.data, result.error);
        if (!result.success) {
          dispatch({ type: "ERROR", message: result.error ?? "Failed to send" });
        }
      } catch (err) {
        const message = err instanceof Error ? err.message : "Failed to send";
        dispatch({ type: "ERROR", message });
      }
    },
    // eslint-disable-next-line react-hooks/exhaustive-deps -- pillSink stable, see module-level note above.
    [state.status, state.conversationId, state.sessionId],
  );

  const stopAgent = useCallback(async () => {
    if (!state.conversationId) return;
    const transport = await getTransport();
    await transport.stopAgent(state.conversationId);
  }, [state.conversationId]);

  // --- Reset for a brand-new research session ---
  const startNewResearch = useCallback(() => {
    teardownSubscription({ subscribedConvIdRef, unsubscribeRef });
    pillSink.push({ kind: "reset" });
    dispatch({ type: "RESET" });
    setWardVaultRevision(0);
    fileMutationToolIdsRef.current.clear();
    hydratedForSessionRef.current = null;
    resnapshotForExecRef.current = null;
    navigate("/research", { replace: true });
    // eslint-disable-next-line react-hooks/exhaustive-deps -- pillSink stable, see module-level note above.
  }, [navigate]);

  const toggleThinking = useCallback((turnId: string) => {
    dispatch({ type: "TOGGLE_THINKING", turnId });
  }, []);

  // ref → full Artifact lookup for ArtifactSlideOut. The ref is populated by
  // hydrateFromSnapshot (on open + on root agent_completed).
  const getFullArtifact = useCallback((id: string): Artifact | undefined => latestArtifactsRef.current.find((a) => a.id === id), []);

  return { state, pillState, wardVaultRevision, sendMessage, stopAgent, startNewResearch, toggleThinking, getFullArtifact };
}
