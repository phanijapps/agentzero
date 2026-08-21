// =============================================================================
// session-snapshot — REST fan-out that rebuilds the full research session
// state from three endpoints (logs, messages, artifacts).
//
// Why this file exists (R14f): the previous hydrate path only fetched the
// root-scoped message list and relied on the WS stream for everything else
// (subagent cards, titles, artifacts, per-turn respond). WS reconnects drop
// events silently — so opening an already-running / already-completed session
// left the UI stuck on the user prompt. Snapshot-on-open is truth; WS is
// delta-only while state.status === "running".
//
// Wire quirks this module hides from the rest of the hook:
// - `LogSession.session_id` is an execution id; the real session id lives in
//   `LogSession.conversation_id`.
// - `parent_session_id` is empty/absent on the root row; non-empty on children.
// - `[tool calls]` assistant content carries the real final answer in a
//   parallel `toolCalls` (camel) or `tool_calls` (snake) column whose JSON
//   entries use `tool_name: "respond"`.
//
// Delegated executions can themselves delegate. The snapshot keeps the flat
// log rows and their recorded `parent_session_id`, while the UI rebuilds the
// nested card tree from that durable relationship.
// =============================================================================

import type { Transport } from "@/services/transport";
import type {
  Artifact,
  LogSession,
  SessionMessage,
  SavedSurface,
} from "@/services/transport/types";
import type {
  AgentTurn,
  AgentTurnStatus,
  ResearchArtifactRef,
  ResearchStatus,
  SessionTurn,
} from "./types";
import {
  GOAL_ARTIFACT_LIST_OPTIONS,
  selectGoalArtifacts,
  toArtifactRef,
} from "./artifact-poll";
import { buildSessionTurns } from "./turns";

// -----------------------------------------------------------------------------
// Constants
// -----------------------------------------------------------------------------

const DEFAULT_AGENT_ID = "root";
const USER_ROLE = "user";

const SYSTEM_INJECTED_MARKERS = ["<ward_snapshot", "[Delegation "];

// -----------------------------------------------------------------------------
// Public surface
// -----------------------------------------------------------------------------

export interface ResearchSnapshot {
  title: string;
  status: ResearchStatus;
  /**
   * Chronological list of user→assistant exchanges. Each turn carries its
   * own user message, subagents, and assistant reply — see
   * memory-bank/future-state/2026-05-05-research-multi-turn-design.md.
   */
  turns: SessionTurn[];
  artifacts: ResearchArtifactRef[];
  /** Reserved for future log-row field; null today. */
  wardId: string | null;
  wardName: string | null;
  /** Root execution id, surfaced so the reducer can route WS events. */
  rootExecutionId: string | null;
  /**
   * Only non-null when the hook can resubscribe with it. For snapshots of
   * pre-existing sessions we don't know the original conv_id — left null so
   * the WS subscription stays idle until the user sends a new message.
   * The live sendMessage path mints its own conv_id, so this is never the
   * blocker for live sessions.
   */
  conversationId: string | null;
  /** Lightweight intent state so the right inspector can be conditional. */
  intentAnalyzing: boolean;
  /** Recorded primary intent, when the session completed intent analysis. */
  intentClassification: string | null;
  surfaces: SavedSurface[];
}

/**
 * Build a snapshot for `sessionId` by fanning out to
 * `/api/logs/sessions`, `/api/sessions/:id/messages?scope=all`, and
 * `/api/sessions/:id/artifacts` in parallel. Returns null if any required
 * call fails or the root row can't be located — caller typically dispatches
 * ERROR on null.
 */
export async function snapshotSession(
  transport: Transport,
  sessionId: string,
): Promise<ResearchSnapshot | null> {
  const [logsRes, msgsRes, artifactsRes, stateRes, surfacesRes] = await Promise.all([
    // Do not fetch the default, globally-limited execution list and filter it
    // locally. That loses child executions from older sessions once unrelated
    // agent work has filled the global page, so a reopened Research thread no
    // longer shows the agents it actually ran.
    transport.listLogSessions({ conversation_id: sessionId }),
    transport.getSessionMessages(sessionId, { scope: "all" }),
    transport
      .listSessionArtifacts(sessionId, GOAL_ARTIFACT_LIST_OPTIONS)
      .catch(() => ({ success: false } as const)),
    // /api/sessions/:id/state carries ward info so a reopened session
    // re-populates the header ward chip + clickable folder link. Soft
    // fail: older backends without the endpoint just leave ward null.
    transport.getSessionState(sessionId).catch(() => ({ success: false } as const)),
    transport.listSavedSessionSurfaces(sessionId).catch(() => ({ success: false } as const)),
  ]);

  if (!logsRes.success || !logsRes.data) return null;
  if (!msgsRes.success || !msgsRes.data) return null;

  const sessionRows = logsRes.data.filter((r) => r.conversation_id === sessionId);
  const rootRow = sessionRows.find(isRootRow);
  if (!rootRow) return null;

  const messages = msgsRes.data;
  const artifacts = buildArtifacts(artifactsRes);
  const title = pickTitle(sessionRows);
  // Session-level truth wins over per-execution status on reopen.
  //
  // `/api/logs/sessions` reports the *root execution's* status. That row
  // flips to "completed" as soon as the root's first pass ends — even
  // while subagents are still running and a continuation turn is pending.
  // Using it as the hydrate status was silencing the WS subscribe guard
  // (`state.status === "running"`) on reopened live sessions: we'd load
  // the history, then sit polling HTTP forever because the subscription
  // never fired.
  //
  // `/api/sessions/:id/state.isLive` is computed server-side each request
  // by checking for any running executions attached to the session, so
  // it's the freshest signal we have. Prefer it; fall back to the
  // session-level status on the same endpoint; fall back to the
  // per-execution row only when `/state` is unavailable (older backends).
  const status: ResearchStatus =
    stateRes.success && stateRes.data
      ? stateRes.data.isLive
        ? "running"
        : mapRootStatus(stateRes.data.session.status)
      : mapRootStatus(rootRow.status);
  const wardName = stateRes.success && stateRes.data?.ward?.name
    ? stateRes.data.ward.name
    : null;
  const primaryIntent = stateRes.success && stateRes.data
    ? stateRes.data.intentAnalysis?.["primary_intent"]
    : null;
  const intentClassification = typeof primaryIntent === "string" && primaryIntent.trim().length > 0
    ? primaryIntent.trim()
    : null;
  const intentAnalyzing = stateRes.success && stateRes.data?.phase === "intent";
  const surfaces = surfacesRes.success && surfacesRes.data ? surfacesRes.data : [];

  // Build per-turn rollup using only root-execution messages (subagent
  // executions carry their own user-role rows from delegation context).
  // Filter out system-injected user rows so they don't open spurious turn
  // boundaries — defense in depth against future schema additions.
  const rootMessages = messages.filter(
    (m) => m.execution_id === rootRow.session_id && !isSystemInjectedUserRow(m),
  );
  const childRows = sessionRows.filter(
    (r) => !isRootRow(r) && r.session_id !== rootRow.session_id,
  );
  const turns = buildSessionTurns({
    rootSessionId: rootRow.session_id,
    rootEndedAt: rootRow.ended_at ?? null,
    rootStatus: researchStatusToTurnStatus(status),
    rootMessages,
    allMessages: messages,
    childRows,
  });

  return {
    title,
    status,
    turns,
    artifacts,
    // The ward tool identifies a ward by its name; the gateway's
    // /api/wards/:id/open accepts that same name as the :id param.
    // There's no separate numeric ward id surfaced to the UI today.
    wardId: wardName,
    wardName,
    rootExecutionId: rootRow.session_id,
    conversationId: null,
    intentAnalyzing,
    intentClassification,
    surfaces,
  };
}

/** Subagent executions carry user-role rows whose content starts with
 *  `<ward_snapshot` or `[Delegation `. Defensive: applied even though
 *  we already filter to root execution_id, in case the schema ever adds
 *  system-injected rows on the root. */
function isSystemInjectedUserRow(m: SessionMessage): boolean {
  if (m.role !== USER_ROLE) return false;
  const trimmed = m.content?.trimStart() ?? "";
  return SYSTEM_INJECTED_MARKERS.some((p) => trimmed.startsWith(p));
}

/** Map the snapshot's `ResearchStatus` to the `AgentTurnStatus` shape
 *  `buildSessionTurns` expects for its "is the last turn open?" check. */
function researchStatusToTurnStatus(s: ResearchStatus): AgentTurnStatus {
  if (s === "running") return "running";
  if (s === "stopped") return "stopped";
  if (s === "error") return "error";
  return "completed";
}

// -----------------------------------------------------------------------------
// Log-row helpers
// -----------------------------------------------------------------------------

export function isRootRow(row: LogSession): boolean {
  const parent = row.parent_session_id;
  return parent == null || parent.length === 0;
}

export function turnFromLogRow(
  row: LogSession,
  parentId: string | null,
): AgentTurn {
  return {
    id: row.session_id,
    agentId: row.agent_id || DEFAULT_AGENT_ID,
    parentExecutionId: parentId,
    startedAt: parseTimestamp(row.started_at),
    completedAt: row.ended_at ? parseTimestamp(row.ended_at) : null,
    status: mapTurnStatus(row.status),
    wardId: null,
    request: null,
    timeline: [],
    tokenCount: row.token_count ?? 0,
    respond: null,
    respondStreaming: "",
    thinkingExpanded: false,
    errorMessage: null,
  };
}

function mapTurnStatus(raw: LogSession["status"] | string): AgentTurnStatus {
  switch (raw) {
    case "completed":
      return "completed";
    case "running":
      return "running";
    case "stopped":
    case "cancelled":
      return "stopped";
    case "error":
    case "crashed":
      return "error";
    default:
      return "error";
  }
}

function mapRootStatus(raw: LogSession["status"] | string): ResearchStatus {
  switch (raw) {
    case "completed":
      return "complete";
    case "running":
      return "running";
    case "stopped":
    case "cancelled":
      return "stopped";
    case "error":
    case "crashed":
      return "error";
    default:
      return "idle";
  }
}

function parseTimestamp(iso: string | undefined): number {
  if (!iso) return 0;
  const t = Date.parse(iso);
  return Number.isFinite(t) ? t : 0;
}

function pickTitle(rows: LogSession[]): string {
  for (const row of rows) {
    if (typeof row.title === "string" && row.title.length > 0) return row.title;
  }
  return "";
}

// -----------------------------------------------------------------------------
// Artifacts — only persisted manifest rows have a safe, previewable ID.
// -----------------------------------------------------------------------------

type ArtifactsResult = { success: boolean; data?: Artifact[] };

function buildArtifacts(res: ArtifactsResult): ResearchArtifactRef[] {
  if (res.success && res.data && res.data.length > 0) {
    return dedupeRefs(selectGoalArtifacts(res.data).map(toArtifactRef));
  }
  // A respond declaration carries an untrusted ward-relative path, not a
  // persisted artifact ID. Rendering it as a clickable artifact would produce
  // a guaranteed content-route 404 and bypass the manifest boundary.
  return [];
}

function dedupeRefs(refs: ResearchArtifactRef[]): ResearchArtifactRef[] {
  const seen = new Set<string>();
  const out: ResearchArtifactRef[] = [];
  for (const r of refs) {
    if (seen.has(r.id)) continue;
    seen.add(r.id);
    out.push(r);
  }
  return out;
}
