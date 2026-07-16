// =============================================================================
// SessionTurnBlock — renders one user→assistant exchange.
//
// Replaces the legacy "user bubbles flat at top + one root turn block at
// bottom" layout: each SessionTurn now owns its own user bubble, its
// subagent cards, and its assistant reply, in chronological order. The
// page just iterates `state.turns` and renders one of these per turn.
// See `memory-bank/future-state/2026-05-05-research-multi-turn-design.md`.
// =============================================================================

import type React from "react";
import { AlertCircle } from "lucide-react";
import { Markdown } from "../shared/markdown";
import { describeTool } from "../shared/statusPill/tool-phrase";
import { SubagentCardTree } from "./AgentTurnBlock";
import { AgentAvatar, CopyButton } from "./ResearchMessages";
import { ToolActivity } from "./ToolActivity";
import type { SessionTurn, TimelineEntry } from "./types";

const TICKER_MAX_LEN = 60;

function truncate(text: string, max: number): string {
  return text.length <= max ? text : text.slice(0, max - 1) + "…";
}

function describeTurnTimelineEntry(turn: SessionTurn): string | null {
  const last: TimelineEntry | undefined = turn.timeline[turn.timeline.length - 1];
  if (!last) return turn.status === "running" ? "waiting…" : null;
  if (last.kind === "thinking" || last.kind === "note") {
    return truncate(last.text, TICKER_MAX_LEN);
  }
  if (last.kind === "tool_call" && last.toolName) {
    const phrase = describeTool(last.toolName, {});
    const head = phrase.narration || last.toolName;
    const args = last.toolArgsPreview ? ` ${truncate(last.toolArgsPreview, 30)}` : "";
    return truncate(`${head}${args}`, TICKER_MAX_LEN);
  }
  if (last.kind === "tool_result") return "↳ done";
  if (last.kind === "error") return truncate(`⚠ ${last.text}`, TICKER_MAX_LEN);
  return truncate(last.text, TICKER_MAX_LEN);
}

function LiveTicker({ turn }: { turn: SessionTurn }) {
  if (turn.status !== "running") return null;
  const text = describeTurnTimelineEntry(turn);
  if (!text) return null;
  return (
    <span className="live-ticker" title={text} aria-live="polite">
      {text}
    </span>
  );
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

function RespondBody({ turn }: { turn: SessionTurn }): React.ReactElement {
  if (turn.status === "error" && !turn.assistantText) {
    return <ErrorBanner message="Turn ended with no output" />;
  }
  if (turn.assistantText) {
    return <Markdown>{turn.assistantText}</Markdown>;
  }
  if (turn.assistantStreaming) {
    return (
      <Markdown className="agent-turn-block__streaming">
        {turn.assistantStreaming}
      </Markdown>
    );
  }
  return <span className="agent-turn-block__placeholder">waiting…</span>;
}

function copyableReply(turn: SessionTurn): string | null {
  if (turn.assistantText && turn.assistantText.length > 0) return turn.assistantText;
  if (turn.assistantStreaming.length > 0) return turn.assistantStreaming;
  return null;
}

function isStreaming(turn: SessionTurn): boolean {
  return turn.assistantText === null && turn.assistantStreaming.length > 0;
}

interface Props {
  turn: SessionTurn;
  /** The Research context inspector owns delegated-agent cards when present. */
  showSubagents?: boolean;
}

/**
 * Renders one chronological turn: user bubble at top, subagent cards in
 * the middle (one per delegation made during this turn's window), and
 * the assistant text reply at the bottom. Visual divider above is
 * applied via `.session-turn + .session-turn` CSS in `research.css`.
 */
export function SessionTurnBlock({ turn, showSubagents = true }: Props) {
  const reply = copyableReply(turn);
  return (
    <section
      className="session-turn"
      data-turn-index={turn.index}
      data-turn-status={turn.status}
    >
      <div className="research-msg research-msg--user">
        <div className="research-msg__card">
          <div className="research-msg__body">{turn.userMessage.content}</div>
        </div>
        <CopyButton text={turn.userMessage.content} label="Copy question" />
      </div>

      <div
        className={
          "research-msg research-msg--assistant" +
          (isStreaming(turn) ? " research-msg--streaming" : "")
        }
        data-copy-host="true"
      >
        <div className="research-msg__card">
          <div className="root-turn__avatar-row">
            <AgentAvatar />
            <LiveTicker turn={turn} />
          </div>
          <div className="research-msg__body">
            {showSubagents && turn.subagents.length > 0 && (
              <div className="root-turn__subagents">
                {turn.subagents.map((sub) => (
                  <SubagentCardTree
                    key={sub.id}
                    turn={sub}
                    allTurns={turn.subagents}
                  />
                ))}
              </div>
            )}
            <div className="research-page__assistant">
              <RespondBody turn={turn} />
            </div>
            {turn.status !== "running" && <ToolActivity entries={turn.timeline} />}
          </div>
        </div>
        {reply !== null && <CopyButton text={reply} label="Copy response" />}
      </div>
    </section>
  );
}
