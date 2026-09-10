// =============================================================================
// turns — pure-function turn builder
//
// Walks a session's root-execution messages by `created_at`, treats each
// `role==="user"` entry as a turn boundary, then buckets subagents and
// assistant replies into the `[user_msg_i, user_msg_{i+1})` window each
// belongs to. See
// `memory-bank/future-state/2026-05-05-research-multi-turn-design.md`
// for the full algorithm and edge cases.
// =============================================================================

import type { LogSession, SessionMessage } from "@/services/transport/types";
import type { AgentTurn, AgentTurnStatus, SessionTurn, TimelineEntry } from "./types";
import { splitMessageAttachments, type MessageAttachment } from "../chat/attachments";
import { turnFromLogRow } from "./session-snapshot";

const TOOL_CALLS_PLACEHOLDER = "[tool calls]";
const RESPOND_TOOL_NAME = "respond";
const DELEGATE_TOOL_NAME = "delegate_to_agent";

// -----------------------------------------------------------------------------
// Turn boundary detection
// -----------------------------------------------------------------------------

export interface TurnBoundary {
  userMessage: {
    id: string;
    content: string;
    createdAt: string;
    attachments?: MessageAttachment[];
    executionId?: string;
  };
  startedAt: string;
  endedAt: string | null;
}

/**
 * Walks the root execution's messages chronologically; each user-role
 * message opens a new boundary. The right edge of each boundary is the
 * next user message's `created_at` — so a turn closes only when the
 * NEXT turn begins. The very last turn is always open-ended (`null`),
 * even on completed sessions: the gateway sometimes persists a
 * `respond()` tool call a few ms after `root.ended_at`, and we want
 * those trailing messages to fall inside the last turn's window
 * instead of going nowhere. The session-level `rootEndedAt` is still
 * used for duration math via [`buildSessionTurns`].
 */
export function findTurnBoundaries(
  rootMessages: SessionMessage[],
  _rootEndedAt: string | null,
): TurnBoundary[] {
  const sorted = [...rootMessages].sort((a, b) =>
    a.created_at.localeCompare(b.created_at),
  );
  const userMessages = sorted.filter((m) => m.role === "user");
  return userMessages.map((m, i) => {
    const nextStart = userMessages[i + 1]?.created_at ?? null;
    const parsed = splitMessageAttachments(m.content);
    return {
      userMessage: {
        id: m.id,
        executionId: m.execution_id ?? undefined,
        content: parsed.content,
        createdAt: m.created_at,
        ...(parsed.attachments.length ? { attachments: parsed.attachments } : {}),
      },
      startedAt: m.created_at,
      endedAt: nextStart,
    };
  });
}

// -----------------------------------------------------------------------------
// Subagent bucketing
// -----------------------------------------------------------------------------

/**
 * Buckets each child execution into the turn whose
 * `[startedAt, endedAt)` interval contains its `started_at`. Children
 * whose timestamp falls before the first user message are dropped — we
 * have no turn to attach them to.
 */
export function bucketSubagents(
  boundaries: TurnBoundary[],
  childRows: LogSession[],
): Map<number, LogSession[]> {
  const out = new Map<number, LogSession[]>();
  for (const child of childRows) {
    const ts = child.started_at;
    if (!ts) continue;
    const idx = boundaries.findIndex((b) => {
      const startsOk = b.startedAt <= ts;
      const endsOk = b.endedAt === null || ts < b.endedAt;
      return startsOk && endsOk;
    });
    if (idx === -1) continue;
    if (!out.has(idx)) out.set(idx, []);
    out.get(idx)!.push(child);
  }
  return out;
}

// -----------------------------------------------------------------------------
// Assistant-reply extraction (per turn)
// -----------------------------------------------------------------------------

/**
 * Finds the latest assistant text reply within a window of messages.
 * Prefers plain `content` over the legacy `respond()` tool call, but
 * falls back to `respond()` when the model never emitted plain text.
 * Mirrors the plain-text fallback shipped in PR #108 / commit f9ca5bd7,
 * scoped per turn instead of per execution.
 */
export function extractAssistantReplyForTurn(
  windowMessages: SessionMessage[],
): string | null {
  const sorted = [...windowMessages].sort((a, b) =>
    a.created_at.localeCompare(b.created_at),
  );
  let answer: string | null = null;
  for (const m of sorted) {
    if (m.role !== "assistant") continue;
    let respondText: string | null = null;
    for (const call of parseToolCalls(m)) {
      if (call?.tool_name !== RESPOND_TOOL_NAME) continue;
      const message = call.args?.["message"] ?? call.args?.["text"];
      if (typeof message === "string" && message.length > 0) {
        respondText = message;
      }
    }
    const plainText =
      typeof m.content === "string" &&
      m.content.length > 0 &&
      m.content !== TOOL_CALLS_PLACEHOLDER
        ? m.content
        : null;
    // Persisted rows are chronological. A later `respond()` result is the
    // terminal answer and must supersede earlier progress prose; plain text
    // still wins when both are carried by the same assistant row.
    answer = plainText ?? respondText ?? answer;
  }
  return answer;
}

interface ToolCall {
  tool_name?: string;
  args?: Record<string, unknown>;
}

function parseToolCalls(m: SessionMessage): ToolCall[] {
  const camel = (m as unknown as { toolCalls?: unknown }).toolCalls;
  const candidate = camel ?? m.tool_calls;
  if (candidate == null) return [];
  try {
    const raw = typeof candidate === "string" ? candidate : JSON.stringify(candidate);
    const parsed = JSON.parse(raw);
    return Array.isArray(parsed) ? (parsed as ToolCall[]) : [];
  } catch {
    return [];
  }
}

/**
 * Rebuild the safe, durable part of a root turn's tool timeline from the
 * persisted assistant-message log. Live WS events contain richer previews,
 * but a completion re-hydration deliberately rebuilds the UI from storage.
 * Keep only tool names here: arguments and results can contain sensitive
 * context and are neither needed nor appropriate for the compact activity
 * audit row.
 */
export function extractToolActivityForTurn(
  windowMessages: SessionMessage[],
): TimelineEntry[] {
  const sorted = [...windowMessages].sort((a, b) =>
    a.created_at.localeCompare(b.created_at),
  );
  const entries: TimelineEntry[] = [];
  for (const message of sorted) {
    if (message.role !== "assistant") continue;
    for (const [index, call] of parseToolCalls(message).entries()) {
      const toolName = call?.tool_name;
      // `respond` is rendered as the assistant message, not duplicate tool
      // activity. Empty/unrecognized names have no safe user-facing label.
      if (!toolName || toolName === RESPOND_TOOL_NAME) continue;
      const at = Date.parse(message.created_at);
      entries.push({
        id: `snapshot-tool-${message.id}-${index}`,
        at: Number.isFinite(at) ? at : 0,
        kind: "tool_call",
        text: toolName,
        toolName,
      });
    }
  }
  return entries;
}

// -----------------------------------------------------------------------------
// Composition
// -----------------------------------------------------------------------------

export interface BuildSessionTurnsInput {
  rootSessionId: string;
  rootEndedAt: string | null;
  rootStatus: AgentTurnStatus;
  /** Messages whose `execution_id == rootSessionId`. */
  rootMessages: SessionMessage[];
  /** All session messages, used to rehydrate each delegated execution. */
  allMessages?: SessionMessage[];
  /** Child execution rows whose `parent_session_id == rootSessionId`. */
  childRows: LogSession[];
}

/**
 * Composes the per-turn rollup: boundaries → buckets → assistant reply
 * per turn → per-turn status.
 */
export function buildSessionTurns(input: BuildSessionTurnsInput): SessionTurn[] {
  const {
    rootSessionId,
    rootEndedAt,
    rootStatus,
    rootMessages,
    allMessages = rootMessages,
    childRows,
  } = input;
  const boundaries = findTurnBoundaries(rootMessages, rootEndedAt);
  const buckets = bucketSubagents(boundaries, childRows);
  const messagesByExecution = groupMessagesByExecution(allMessages);

  return boundaries.map((b, i) => {
    const subRows = buckets.get(i) ?? [];
    const baseSubagents: AgentTurn[] = subRows
      .map((row) => {
        const subagentMessages = messagesByExecution.get(row.session_id) ?? [];
        return {
          ...turnFromLogRow(row, row.parent_session_id || rootSessionId),
          timeline: extractToolActivityForTurn(subagentMessages),
          respond: extractAssistantReplyForTurn(subagentMessages),
        };
      })
      .sort((a, b2) => a.startedAt - b2.startedAt);

    const windowMessages = rootMessages.filter((m) => {
      const ts = m.created_at;
      return ts >= b.startedAt && (b.endedAt === null || ts < b.endedAt);
    });

    // Zip delegation task text (from root's `delegate_to_agent` tool calls)
    // onto subagents in the same chronological order. Preserves the
    // "Request:" header on each subagent card.
    const tasks = extractDelegationTasksInWindow(windowMessages);
    const subagents = baseSubagents.map((sa, idx) => ({
      ...sa,
      request: idx < tasks.length ? tasks[idx] : null,
    }));

    const assistantText = extractAssistantReplyForTurn(windowMessages);
    const timeline = extractToolActivityForTurn(windowMessages);

    const status = deriveTurnStatus({
      isLast: i === boundaries.length - 1,
      rootStatus,
      assistantText,
      subagents,
    });

    // Duration: prefer the next turn's start (intermediate turns); fall
    // back to rootEndedAt for the last turn on a closed session.
    const startedMs = Date.parse(b.startedAt);
    const isLast = i === boundaries.length - 1;
    const closingEdge = b.endedAt ?? (isLast ? rootEndedAt : null);
    const endedMs = closingEdge ? Date.parse(closingEdge) : null;
    const durationMs =
      endedMs !== null && Number.isFinite(startedMs) && Number.isFinite(endedMs)
        ? endedMs - startedMs
        : null;

    return {
      id: `turn-${b.userMessage.id}`,
      executionId: b.userMessage.executionId,
      index: i,
      userMessage: b.userMessage,
      subagents,
      assistantText,
      assistantStreaming: "",
      timeline,
      lastHeartbeatAt: null,
      status,
      startedAt: b.startedAt,
      endedAt: b.endedAt,
      durationMs,
    };
  });
}

function groupMessagesByExecution(
  messages: SessionMessage[],
): Map<string, SessionMessage[]> {
  const grouped = new Map<string, SessionMessage[]>();
  for (const message of messages) {
    const existing = grouped.get(message.execution_id);
    if (existing) existing.push(message);
    else grouped.set(message.execution_id, [message]);
  }
  return grouped;
}

/**
 * Extracts delegation task text from a window of root-execution
 * assistant messages, in chronological order. Each entry is the `task`
 * arg of a `delegate_to_agent` tool call.
 */
export function extractDelegationTasksInWindow(
  windowMessages: SessionMessage[],
): string[] {
  const sorted = [...windowMessages].sort((a, b) =>
    a.created_at.localeCompare(b.created_at),
  );
  const out: string[] = [];
  for (const m of sorted) {
    if (m.role !== "assistant") continue;
    for (const call of parseToolCalls(m)) {
      if (call?.tool_name !== DELEGATE_TOOL_NAME) continue;
      const task = call.args?.["task"];
      if (typeof task === "string") out.push(task);
    }
  }
  return out;
}

interface DeriveStatusInput {
  isLast: boolean;
  rootStatus: AgentTurnStatus;
  assistantText: string | null;
  subagents: AgentTurn[];
}

function deriveTurnStatus(args: DeriveStatusInput): AgentTurnStatus {
  const { isLast, rootStatus, assistantText, subagents } = args;
  if (rootStatus === "error") return "error";
  if (rootStatus === "stopped") return "stopped";
  if (isLast && rootStatus === "running") {
    if (assistantText === null) return "running";
    if (subagents.some((s) => s.status === "running")) return "running";
  }
  return "completed";
}
