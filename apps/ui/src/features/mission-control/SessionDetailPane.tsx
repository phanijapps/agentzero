// ============================================================================
// MISSION CONTROL — SessionDetailPane
// Right side of the page: header (title, status, meta, actions) + dual-pane
// grid hosting MessagesPane and ToolsPane side-by-side.
// ============================================================================

import { useMemo } from "react";
import { useNavigate } from "react-router-dom";
import { Pause, Square, RotateCcw, ExternalLink } from "lucide-react";
import type { LogSession } from "@/services/transport/types";
import { formatDuration } from "../logs/trace-types";
import { MessagesPane } from "./MessagesPane";
import { ToolsPane } from "./ToolsPane";
import { TokenPair } from "./SessionListPanel";
import type { SessionTokenIndex } from "./useSessionTokens";
import { useSessionDetailBundle } from "./useSessionDetailBundle";
import { useSelectedSessionTokens } from "./useSelectedSessionTokens";
import type { CurrentSessionPlan } from "@/services/transport/types";

interface SessionDetailPaneProps {
  session: LogSession | null;
  /** Optional — when supplied, header shows in/out tokens, ToolsPane shows
   *  per-execution tokens in each agent group header. */
  tokenIndex?: SessionTokenIndex;
  /** Successful Mission Control list-load generation for the running-session
   *  selected-token refresh. */
  refreshGeneration?: number;
  /** The Radar already supplies the mission title and controls. Keep the
   * trace/plan body without duplicating that command surface. */
  embedded?: boolean;
}

export function SessionDetailPane({ session, tokenIndex, refreshGeneration, embedded = false }: SessionDetailPaneProps) {
  const navigate = useNavigate();
  const sessionId = session?.session_id ?? null;
  const conversationId = session?.conversation_id ?? null;
  const isRunning = session?.status === "running";
  const detail = useSessionDetailBundle(sessionId, isRunning);
  const selectedTokenState = useSelectedSessionTokens(
    conversationId,
    isRunning ? refreshGeneration : undefined,
  );
  const detailTokenIndex = useMemo(
    () => mergeTokenIndexes(tokenIndex, selectedTokenState),
    [tokenIndex, selectedTokenState],
  );

  if (!session) {
    return (
      <section className="session-detail-pane session-detail-pane--empty">
        <div className="session-detail-pane__placeholder">
          <p>Select a session from the list to inspect its messages and tool calls.</p>
        </div>
      </section>
    );
  }

  const status = session.status;
  const title = session.title || session.agent_name || session.session_id;
  const duration = formatDuration(session.duration_ms);
  const sessionTokens = detailTokenIndex?.byRootExecId.get(session.session_id);

  return (
    <section className={`session-detail-pane${embedded ? " session-detail-pane--embedded" : ""}`}>
      {!embedded && <header className="session-detail-pane__head">
        <div className="session-detail-pane__title">
          #{shortId(session.session_id)} · {title}
        </div>
        <div className="session-detail-pane__meta">
          <span className={`session-detail-pane__status session-detail-pane__status--${status}`}>
            ● {status}
          </span>
          {duration && (
            <>
              <span>·</span>
              <span><strong>{duration}</strong> elapsed</span>
            </>
          )}
          <span>·</span>
          <span>{session.agent_name}</span>
          {sessionTokens && sessionTokens.total > 0 && (
            <>
              <span>·</span>
              <TokenPair inTok={sessionTokens.in} outTok={sessionTokens.out} />
            </>
          )}
        </div>
        <div className="session-detail-pane__actions">
          <button
            type="button"
            className="btn btn--secondary btn--sm"
            disabled={!isRunning}
            title="Pause session"
          >
            <Pause size={14} /> Pause
          </button>
          <button
            type="button"
            className="btn btn--destructive btn--sm"
            disabled={!isRunning}
            title="Stop session"
          >
            <Square size={14} /> Stop
          </button>
          <button
            type="button"
            className="btn btn--secondary btn--sm"
            title="Retry session"
          >
            <RotateCcw size={14} /> Retry
          </button>
          <button
            type="button"
            className="btn btn--ghost btn--sm"
            title="Open in Research"
            onClick={() => navigate(`/research/${session.session_id}`)}
          >
            <ExternalLink size={14} /> Open in Research
          </button>
        </div>
      </header>}
      {(!embedded || selectedTokenState.currentPlan) && (
        <CurrentPlanCard currentPlan={selectedTokenState.currentPlan} />
      )}
      <div className="session-detail-pane__panes">
        {!embedded && (
          <MessagesPane
            session={session}
            detailBundle={detail.bundle}
            detailLoading={detail.loading}
            detailError={detail.error}
          />
        )}
        <ToolsPane
          session={session}
          tokenIndex={detailTokenIndex}
          detailBundle={detail.bundle}
          detailLoading={detail.loading}
          onDetailEvent={detail.refetch}
        />
      </div>
    </section>
  );
}

function CurrentPlanCard({ currentPlan }: { currentPlan?: CurrentSessionPlan }) {
  return (
    <section className="session-current-plan" aria-label="Current plan">
      <h2>Current plan</h2>
      {!currentPlan && <p>No plan recorded for this session.</p>}
      {currentPlan && (
        <>
          {currentPlan.explanation && <p className="session-current-plan__explanation">{currentPlan.explanation}</p>}
          <ol>
            {currentPlan.plan.map((item, index) => (
              <li key={`${index}-${item.step}`}>
                <span>{item.step}</span>
                <small data-status={item.status}>{item.status.replace("_", " ")}</small>
              </li>
            ))}
          </ol>
        </>
      )}
    </section>
  );
}

function shortId(id: string): string {
  return id.length > 8 ? id.slice(-6) : id;
}

function mergeTokenIndexes(
  base?: SessionTokenIndex,
  selected?: SessionTokenIndex,
): SessionTokenIndex | undefined {
  if (!base) return selected;
  if (!selected) return base;
  return {
    byRootExecId: new Map([...base.byRootExecId, ...selected.byRootExecId]),
    executionsByRootExecId: new Map([
      ...base.executionsByRootExecId,
      ...selected.executionsByRootExecId,
    ]),
  };
}
