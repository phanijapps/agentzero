import type React from "react";
import { useEffect, useState } from "react";
import { Markdown } from "../shared/markdown";
import {
  CheckCircle2,
  ChevronDown,
  ChevronRight,
  Square,
  AlertCircle,
  Loader2,
} from "lucide-react";
import type { AgentTurn, AgentTurnStatus } from "./types";
import { childrenOf } from "./turn-tree";
import { AgentAvatar, CopyButton } from "./ResearchMessages";
import { describeTool } from "../shared/statusPill/tool-phrase";
import { ToolActivity } from "./ToolActivity";

/**
 * Per-turn inline live ticker — shows the latest timeline entry while the
 * turn is running. Picks from the same data the reducer collects for thinking
 * / tool_call / tool_result events, keyed by execution_id. Hidden once the
 * turn leaves the running state (status flips to completed/error/stopped).
 * Used in both SubagentCard headers and the root block.
 *
 * Complements the top global StatusPill: the pill is "last event across all
 * agents", this ticker is per-agent and survives even after the top pill
 * switches narration to a different agent.
 */
const TICKER_MAX_LEN = 60;

function truncate(text: string, max: number): string {
  return text.length <= max ? text : text.slice(0, max - 1) + "…";
}

function describeTimelineEntry(turn: AgentTurn): string | null {
  const last = turn.timeline[turn.timeline.length - 1];
  if (!last) {
    return turn.status === "running" ? "waiting…" : null;
  }
  if (last.kind === "thinking" || last.kind === "note") {
    return truncate(last.text, TICKER_MAX_LEN);
  }
  if (last.kind === "tool_call" && last.toolName) {
    const phrase = describeTool(last.toolName, {});
    const head = phrase.narration || last.toolName;
    const args = last.toolArgsPreview ? ` ${truncate(last.toolArgsPreview, 30)}` : "";
    return truncate(`${head}${args}`, TICKER_MAX_LEN);
  }
  if (last.kind === "tool_result") {
    return "↳ done";
  }
  if (last.kind === "error") {
    return truncate(`⚠ ${last.text}`, TICKER_MAX_LEN);
  }
  return truncate(last.text, TICKER_MAX_LEN);
}

interface LiveTickerProps {
  turn: AgentTurn;
}

function LiveTicker({ turn }: LiveTickerProps) {
  if (turn.status !== "running") return null;
  const text = describeTimelineEntry(turn);
  if (!text) return null;
  return (
    <span className="live-ticker" title={text} aria-live="polite">
      {text}
    </span>
  );
}

export interface AgentTurnBlockProps {
  turn: AgentTurn;
  /** Kept for API stability; the redesigned layout no longer exposes a
   *  toggle surface — all tool/thinking events land in the top pill ticker. */
  onToggleThinking?(turnId: string): void;
  /** Direct children of `turn`. */
  childTurns?: AgentTurn[];
  /** Full flat turn list so subagent cards can recurse into grand-children. */
  allTurns?: AgentTurn[];
}

// Agent identity → accent colour. Theme tokens where possible.
const AGENT_COLOR: Record<string, string> = {
  planner: "var(--success)",
  "planner-agent": "var(--success)",
  solution: "var(--purple)",
  "solution-agent": "var(--purple)",
  builder: "var(--warning)",
  "builder-agent": "var(--warning)",
  writer: "var(--blue)",
  "writer-agent": "var(--blue)",
  root: "var(--foreground)",
};

function agentColour(agentId: string): string {
  return AGENT_COLOR[agentId] ?? "var(--muted-foreground)";
}

function formatDuration(startedAt: number, completedAt: number | null): string {
  const end = completedAt ?? Date.now();
  const ms = end - startedAt;
  if (ms < 1000) return `${ms}ms`;
  if (ms < 60_000) return `${(ms / 1000).toFixed(1)}s`;
  return `${Math.round(ms / 1000)}s`;
}

function StatusIcon({ status }: { status: AgentTurnStatus }) {
  switch (status) {
    case "running":   return <Loader2 size={14} className="spin" />;
    case "completed": return <CheckCircle2 size={14} />;
    case "stopped":   return <Square size={14} />;
    case "error":     return <AlertCircle size={14} />;
  }
}

function ErrorBanner({ message }: { message: string }) {
  return (
    <div
      className="agent-turn-block__error"
      data-testid="turn-error-banner"
      role="alert"
    >
      <AlertCircle size={14} aria-hidden="true" />
      <span>{message}</span>
    </div>
  );
}

function RespondMarkdown({ content }: { content: string }) {
  return <Markdown>{content}</Markdown>;
}

function StreamingBuffer({ text }: { text: string }) {
  // Parity with chat-v2: stream tokens render as markdown too (same component
  // as the final respond, just styled with a "streaming" class for cursor/
  // opacity). Without this, code fences and lists flash as raw text until the
  // turn completes.
  return <Markdown className="agent-turn-block__streaming">{text}</Markdown>;
}

function WaitingPlaceholder() {
  return <span className="agent-turn-block__placeholder">waiting…</span>;
}

/** Decides what to render in the Respond body slot. Each branch is a named helper. */
function RespondBody({ turn }: { turn: AgentTurn }): React.ReactElement {
  if (turn.status === "error" && turn.errorMessage) {
    return <ErrorBanner message={turn.errorMessage} />;
  }
  if (turn.respond) {
    return <RespondMarkdown content={turn.respond} />;
  }
  if (turn.respondStreaming) {
    return <StreamingBuffer text={turn.respondStreaming} />;
  }
  return <WaitingPlaceholder />;
}

function respondIsStreaming(turn: AgentTurn): boolean {
  return turn.respond === null && turn.respondStreaming.length > 0;
}

/** Collapse a subagent card when it transitions out of the "running" state.
 *  Keeps the effect local to the card so there's no reducer bookkeeping.
 *  User-toggled expand wins until the status changes again. */
function useSubagentStatusTransition(
  status: AgentTurnStatus,
  setExpanded: (v: boolean) => void,
): AgentTurnStatus {
  const [prev, setPrev] = useState<AgentTurnStatus>(status);
  useEffect(() => {
    if (prev !== status) {
      setExpanded(status === "running");
      setPrev(status);
    }
  }, [status, prev, setExpanded]);
  return prev;
}

/** What the copy button should copy. Prefer finalized respond, then the
 *  streaming buffer; return null when there's nothing useful. */
function copyableRespondText(turn: AgentTurn): string | null {
  if (turn.respond && turn.respond.length > 0) return turn.respond;
  if (turn.respondStreaming && turn.respondStreaming.length > 0) return turn.respondStreaming;
  return null;
}

/**
 * Subagent card: request, response, and the safe tool-name audit recorded for
 * this execution. Tool arguments and results stay out of the UI. Running →
 * Request + "waiting…" and its live ticker. Completed → Request + Response
 * + the durable tools-used list.
 */
interface SubagentCardProps {
  turn: AgentTurn;
}

function SubagentResponseBody({ turn }: SubagentCardProps): React.ReactElement {
  if (turn.status === "error" && turn.errorMessage) {
    return <ErrorBanner message={turn.errorMessage} />;
  }
  if (turn.respond) return <RespondMarkdown content={turn.respond} />;
  if (turn.respondStreaming) return <StreamingBuffer text={turn.respondStreaming} />;
  if (turn.status === "running") return <WaitingPlaceholder />;
  return <span className="agent-turn-block__placeholder">(no response)</span>;
}

function SubagentCard({ turn }: SubagentCardProps) {
  const color = agentColour(turn.agentId);
  const respondText = copyableRespondText(turn);
  // Default: expanded while running, collapsed once done. User can override
  // either way by clicking the header. Reset on status transition.
  const [expanded, setExpanded] = useState(turn.status === "running");
  useSubagentStatusTransition(turn.status, setExpanded);
  return (
    <div
      className="subagent-card"
      style={{ borderLeft: `3px solid ${color}` }}
      data-parent={turn.parentExecutionId ?? ""}
      data-expanded={expanded}
      data-copy-host="true"
    >
      <button
        type="button"
        className="subagent-card__header subagent-card__toggle"
        onClick={() => setExpanded((e) => !e)}
        aria-expanded={expanded}
        aria-label={expanded ? "Collapse subagent" : "Expand subagent"}
      >
        <span className="subagent-card__chevron" aria-hidden="true">
          {expanded ? <ChevronDown size={12} /> : <ChevronRight size={12} />}
        </span>
        <span className="subagent-card__agent" style={{ color }}>
          {turn.agentId}
        </span>
        <LiveTicker turn={turn} />
        <span className="subagent-card__meta">
          <StatusIcon status={turn.status} />
          <span>{formatDuration(turn.startedAt, turn.completedAt)}</span>
        </span>
      </button>
      {expanded && turn.request && (
        <div className="subagent-card__section">
          <div className="subagent-card__label">Request</div>
          <div className="subagent-card__text">{turn.request}</div>
        </div>
      )}
      {expanded && (
        <div className="subagent-card__section">
          <div className="subagent-card__label">Response</div>
          <div className="subagent-card__text">
            <SubagentResponseBody turn={turn} />
          </div>
        </div>
      )}
      {expanded && turn.status !== "running" && <ToolActivity entries={turn.timeline} />}
      {expanded && respondText !== null && (
        <CopyButton text={respondText} label="Copy response" />
      )}
    </div>
  );
}

/**
 * Root block: avatar + nested subagent cards + final respond + copy.
 * No thinking chevron. While running, root tool activity appears only in the
 * live ticker; once complete, the safe durable tools-used list remains.
 */
interface RootTurnProps {
  turn: AgentTurn;
  childTurns: AgentTurn[];
  allTurns: AgentTurn[];
}

function RootTurn({ turn, childTurns, allTurns }: RootTurnProps) {
  const respondText = copyableRespondText(turn);
  return (
    <div
      className={`research-msg research-msg--assistant${respondIsStreaming(turn) ? " research-msg--streaming" : ""}`}
      data-parent=""
      data-copy-host="true"
    >
      <div className="research-msg__card">
        <div className="root-turn__avatar-row">
          <AgentAvatar />
          <LiveTicker turn={turn} />
        </div>
        <div className="research-msg__body">
          {childTurns.length > 0 && (
            <div className="root-turn__subagents">
              {childTurns.map((child) => (
                <SubagentCardTree key={child.id} turn={child} allTurns={allTurns} />
              ))}
            </div>
          )}
          <div className="research-page__assistant">
            <RespondBody turn={turn} />
          </div>
          {turn.status !== "running" && <ToolActivity entries={turn.timeline} />}
        </div>
      </div>
      {respondText !== null && (
        <CopyButton text={respondText} label="Copy response" />
      )}
    </div>
  );
}

/**
 * Subagent cards can themselves delegate — render grand-children inside the
 * card recursively. Kept as a separate component so the root block stays
 * readable and the recursion is isolated.
 */
interface SubagentCardTreeProps {
  turn: AgentTurn;
  allTurns: AgentTurn[];
}

export function SubagentCardTree({ turn, allTurns }: SubagentCardTreeProps) {
  const grandChildren = childrenOf(turn, allTurns);
  return (
    <div className="subagent-card-tree">
      <SubagentCard turn={turn} />
      {grandChildren.length > 0 && (
        <div className="subagent-card-tree__nested">
          {grandChildren.map((gc) => (
            <SubagentCardTree key={gc.id} turn={gc} allTurns={allTurns} />
          ))}
        </div>
      )}
    </div>
  );
}

export function AgentTurnBlock({
  turn,
  childTurns,
  allTurns,
}: AgentTurnBlockProps) {
  const childList = childTurns ?? [];
  const fullList = allTurns ?? childList;

  // Root turn → clean assistant layout with nested subagent cards.
  if (turn.parentExecutionId === null) {
    return <RootTurn turn={turn} childTurns={childList} allTurns={fullList} />;
  }

  // Subagent turn rendered standalone (shouldn't normally happen — the root
  // wraps them — but covers orphan-subagent edge cases).
  return <SubagentCardTree turn={turn} allTurns={fullList} />;
}
