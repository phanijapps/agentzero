// ============================================================================
// MISSION CONTROL — Attention Radar composition
//
// The Radar deliberately renders from the bounded Mission Control summary
// instead of continuously refreshing all sessions. Rich messages and tool
// traces remain an explicit, focused inspection action.
// ============================================================================

import { useMemo, useState } from "react";
import {
  Activity,
  ArrowUpRight,
  Bot,
  Check,
  CircleAlert,
  Clock3,
  Eye,
  Gauge,
  GitBranch,
  Radio,
  Square,
  Sparkles,
} from "lucide-react";
import type { LogSession } from "@/services/transport/types";
import { computeKpis } from "./kpi";
import { SessionDetailPane } from "./SessionDetailPane";
import { useMissionControlSessions } from "./useMissionControlSessions";

type AttentionState = "active" | "watch" | "alert" | "complete";

export function MissionControlPage() {
  const { sessions, loading, error, tokenIndex, refreshGeneration, refetch } = useMissionControlSessions(50);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [inspecting, setInspecting] = useState(true);
  const [cancellingSessionId, setCancellingSessionId] = useState<string | null>(null);

  const ranked = useMemo(() => [...sessions].sort(compareAttention), [sessions]);
  const selected = useMemo(() => {
    if (selectedId) {
      const found = sessions.find((session) => session.session_id === selectedId);
      if (found) return found;
    }
    return ranked[0] ?? null;
  }, [ranked, selectedId, sessions]);
  const kpis = useMemo(() => computeKpis(sessions), [sessions]);
  const activeCount = sessions.filter((session) => session.status === "running").length;

  const chooseMission = (sessionId: string) => {
    setSelectedId(sessionId);
    setInspecting(true);
  };
  const cancelMission = async (session: LogSession) => {
    setCancellingSessionId(session.session_id);
    try {
      const response = await fetch(`/api/gateway/cancel/${encodeURIComponent(session.conversation_id)}`, {
        method: "POST",
      });
      if (!response.ok) throw new Error(`Cancellation failed (${response.status})`);
      refetch();
    } finally {
      setCancellingSessionId(null);
    }
  };

  return (
    <main className="mission-control mission-radar" aria-label="Mission Control Attention Radar">
      <header className="mission-radar__header">
        <div className="mission-radar__identity">
          <span className="mission-radar__mark" aria-hidden="true">z</span>
          <div>
            <p>Mission Control</p>
            <h1>Attention Radar</h1>
          </div>
        </div>
        <nav className="mission-radar__tabs" aria-label="Mission Control views">
          <button className="mission-radar__tab mission-radar__tab--active" type="button">Radar</button>
          <button className="mission-radar__tab" type="button" disabled>All sessions</button>
          <button className="mission-radar__tab" type="button" disabled>Flight recorder</button>
        </nav>
        <div className="mission-radar__connection" aria-live="polite">
          <Radio size={14} aria-hidden="true" />
          <span>{loading ? "Loading bounded snapshot" : "Snapshot connected"}</span>
        </div>
      </header>

      <section className="mission-radar__metrics" aria-label="Mission overview">
        <Metric label="Attention now" value={String(ranked.filter((session) => attentionOf(session) !== "complete").length)} detail="ranked missions" variant="attention" />
        <Metric label="Active missions" value={String(activeCount)} detail={activeCount === 1 ? "mission executing" : "missions executing"} variant="active" />
        <Metric label="Completed · 24h" value={String(kpis.done24h)} detail={kpis.successRate === null ? "no terminal sessions" : `${kpis.successRate}% success`} variant="complete" />
        <Metric label="Token burn" value={formatTokens(kpis.runningTokens)} detail="active mission total" variant="tokens" />
        <div className="mission-radar__metric mission-radar__metric--stream">
          <span className="mission-radar__metric-label">Operations feed</span>
          <span className="mission-radar__metric-detail">Bounded session summary</span>
        </div>
      </section>

      {error && <p className="mission-radar__error" role="alert">Mission snapshot unavailable: {error}</p>}

      <section className="mission-radar__grid">
        <aside className="mission-radar__panel mission-radar__panel--radar" aria-label="Mission radar">
          <PanelHeading eyebrow="Mission radar" title="What needs you" count={ranked.length} />
          <div className="mission-radar__mission-list">
            {loading && sessions.length === 0 && <p className="mission-radar__empty">Acquiring mission snapshot…</p>}
            {!loading && ranked.length === 0 && <p className="mission-radar__empty">No recent missions. Start a conversation to bring the radar online.</p>}
            {ranked.map((session) => (
              <MissionRow
                key={session.session_id}
                session={session}
                selected={selected?.session_id === session.session_id}
                onSelect={() => chooseMission(session.session_id)}
              />
            ))}
          </div>
        </aside>

        <section className="mission-radar__focus" aria-label="Focused mission">
          {selected ? (
            <FocusedMission
              session={selected}
              tokenTotal={tokenIndex.byRootExecId.get(selected.session_id)?.total ?? selected.token_count}
              inspecting={inspecting}
              onInspect={() => setInspecting((current) => !current)}
              onCancel={() => void cancelMission(selected)}
              cancelling={cancellingSessionId === selected.session_id}
            />
          ) : (
            <div className="mission-radar__focus-empty">
              <Sparkles size={20} aria-hidden="true" />
              <h2>Radar standing by</h2>
              <p>Mission Control will surface active and recent work here.</p>
            </div>
          )}
          {inspecting && selected && (
            <div className="mission-radar__inspector">
              <SessionDetailPane
                session={selected}
                tokenIndex={tokenIndex}
                refreshGeneration={refreshGeneration}
                embedded
              />
            </div>
          )}
        </section>

        <aside className="mission-radar__side-stack" aria-label="Live operations and system posture">
        <section className="mission-radar__activity" aria-label="Live operations feed">
          <PanelHeading eyebrow="Live operations" title="Recent mission activity" />
          <div className="mission-radar__activity-list">
            {ranked.slice(0, 5).map((session) => <ActivityRow key={session.session_id} session={session} />)}
            {!loading && ranked.length === 0 && <p className="mission-radar__empty">No activity in the bounded snapshot.</p>}
          </div>
        </section>
        <section className="mission-radar__posture" aria-label="System posture">
          <PanelHeading eyebrow="System posture" title="Ready for focused work" />
          <p>Detailed tool calls stay inside an explicit mission inspector, keeping this radar calm as history grows.</p>
          <div className="mission-radar__posture-tags">
            <span><Check size={13} aria-hidden="true" /> bounded</span>
            <span><Eye size={13} aria-hidden="true" /> inspect on demand</span>
          </div>
        </section>
        </aside>
      </section>
    </main>
  );
}

function PanelHeading({ eyebrow, title, count }: { eyebrow: string; title: string; count?: number }) {
  return (
    <header className="mission-radar__panel-heading">
      <div>
        <p>{eyebrow}</p>
        <h2>{title}</h2>
      </div>
      {count !== undefined && <span className="mission-radar__count">{count}</span>}
    </header>
  );
}

function Metric({ label, value, detail, variant }: { label: string; value: string; detail: string; variant: string }) {
  return (
    <div className={`mission-radar__metric mission-radar__metric--${variant}`}>
      <span className="mission-radar__metric-label">{label}</span>
      <strong>{value}</strong>
      <span className="mission-radar__metric-detail">{detail}</span>
    </div>
  );
}

function MissionRow({ session, selected, onSelect }: { session: LogSession; selected: boolean; onSelect(): void }) {
  const attention = attentionOf(session);
  return (
    <button
      type="button"
      className={`mission-radar__mission mission-radar__mission--${attention}${selected ? " mission-radar__mission--selected" : ""}`}
      onClick={onSelect}
      aria-label={`Focus ${missionTitle(session)}`}
      aria-current={selected ? "true" : undefined}
    >
      <span className="mission-radar__signal" aria-hidden="true" />
      <span className="mission-radar__mission-copy">
        <strong>{missionTitle(session)}</strong>
        <small>{attentionLabel(session)} · {relativeTime(session.started_at)}</small>
      </span>
      <span className="mission-radar__mission-state">{attention}</span>
    </button>
  );
}

function FocusedMission({ session, tokenTotal, inspecting, onInspect, onCancel, cancelling }: {
  session: LogSession;
  tokenTotal: number;
  inspecting: boolean;
  onInspect(): void;
  onCancel(): void;
  cancelling: boolean;
}) {
  const attention = attentionOf(session);
  const delegationCount = session.subagent_count ?? session.child_session_ids?.length ?? 0;
  return (
    <article className="mission-radar__focus-card">
      <div className="mission-radar__focus-topline">
        <span className={`mission-radar__status mission-radar__status--${attention}`}>
          {attention === "alert" ? <CircleAlert size={14} aria-hidden="true" /> : <Activity size={14} aria-hidden="true" />}
          {attentionLabel(session)}
        </span>
        <span>#{shortId(session.session_id)}</span>
      </div>
      <h2>{missionTitle(session)}</h2>
      <p className="mission-radar__focus-summary">A focused operational view of this mission. Open the inspector when you need messages, tools, and its current structured plan.</p>
      <dl className="mission-radar__focus-stats">
        <div><dt><Bot size={14} aria-hidden="true" /> Agent</dt><dd>{session.agent_name}</dd></div>
        <div><dt><GitBranch size={14} aria-hidden="true" /> Delegation</dt><dd>{delegationCount} linked</dd></div>
        <div><dt><Gauge size={14} aria-hidden="true" /> Tokens</dt><dd>{formatTokens(tokenTotal)}</dd></div>
        <div><dt><Clock3 size={14} aria-hidden="true" /> Started</dt><dd>{relativeTime(session.started_at)}</dd></div>
      </dl>
      <section className="mission-radar__plan-card" aria-label="Current plan availability">
        <div>
          <span>Current plan</span>
          <strong>{session.status === "running" ? "Live plan available" : "Recorded mission detail"}</strong>
        </div>
        <button type="button" onClick={onInspect} aria-expanded={inspecting}>
          {inspecting ? "Hide inspector" : "Inspect mission"} <ArrowUpRight size={14} aria-hidden="true" />
        </button>
        {session.status === "running" && (
          <button type="button" onClick={onCancel} disabled={cancelling} aria-label="Stop session">
            <Square size={14} aria-hidden="true" /> {cancelling ? "Stopping…" : "Stop session"}
          </button>
        )}
      </section>
    </article>
  );
}

function ActivityRow({ session }: { session: LogSession }) {
  const attention = attentionOf(session);
  return (
    <button type="button" className="mission-radar__activity-row">
      <span className={`mission-radar__activity-icon mission-radar__activity-icon--${attention}`} aria-hidden="true"><Activity size={14} /></span>
      <span><strong>{missionTitle(session)}</strong><small>{attentionLabel(session)} · {session.agent_name}</small></span>
      <time>{relativeTime(session.started_at)}</time>
    </button>
  );
}

function attentionOf(session: LogSession): AttentionState {
  if (session.status === "error" || session.status === "stopped") return "alert";
  if (session.status === "completed") return "complete";
  const age = Date.now() - new Date(session.started_at).getTime();
  return age > 5 * 60 * 1000 ? "watch" : "active";
}

function compareAttention(a: LogSession, b: LogSession): number {
  const rank: Record<AttentionState, number> = { alert: 0, watch: 1, active: 2, complete: 3 };
  const byAttention = rank[attentionOf(a)] - rank[attentionOf(b)];
  if (byAttention !== 0) return byAttention;
  return new Date(b.started_at).getTime() - new Date(a.started_at).getTime();
}

function attentionLabel(session: LogSession): string {
  switch (attentionOf(session)) {
    case "alert": return "Needs review";
    case "watch": return "Watching progress";
    case "active": return "Executing";
    case "complete": return "Completed";
  }
}

function missionTitle(session: LogSession): string {
  return session.title || session.agent_name || session.session_id;
}

function shortId(value: string): string {
  return value.length > 8 ? value.slice(-6) : value;
}

function relativeTime(value: string): string {
  const delta = Math.max(0, Date.now() - new Date(value).getTime());
  const minutes = Math.floor(delta / 60_000);
  if (minutes < 1) return "just now";
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.floor(minutes / 60);
  return `${hours}h ago`;
}

function formatTokens(value: number): string {
  if (value >= 1_000_000) return `${(value / 1_000_000).toFixed(1)}m`;
  if (value >= 1_000) return `${(value / 1_000).toFixed(1)}k`;
  return String(value);
}
