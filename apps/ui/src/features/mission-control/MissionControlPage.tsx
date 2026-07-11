// ============================================================================
// MISSION CONTROL — page composition
// Three zones:
//   1. KpiStrip — live aggregate counts (running, queued, done, failed, paused)
//   2. SessionListPanel — left rail with search + filters
//   3. SessionDetailPane — right pane with messages + tools dual view
//
// Data: useMissionControlSessions pulls a bounded summary page with root
// execution rows plus minimal per-execution token slices.
// Live: ToolsPane wires the WS subscription per selected session; the list
// auto-refreshes every 5s while any session is running.
// ============================================================================

import { useMemo, useState } from "react";
import type { LogSession } from "@/services/transport/types";
import { useAutoRefresh } from "../logs/log-hooks";
import { computeKpis } from "./kpi";
import { KpiStrip } from "./KpiStrip";
import { SessionListPanel, applyFilters, DEFAULT_FILTERS } from "./SessionListPanel";
import { SessionDetailPane } from "./SessionDetailPane";
import { OpenLoopsPanel } from "./OpenLoopsPanel";
import { useMissionControlSessions } from "./useMissionControlSessions";
import type { SessionFilters } from "./types";

export function MissionControlPage() {
  const { sessions, loading, refetch, tokenIndex } = useMissionControlSessions(50);
  useAutoRefresh(sessions, refetch);

  const [filters, setFilters] = useState<SessionFilters>(DEFAULT_FILTERS);
  const [selectedId, setSelectedId] = useState<string | null>(null);

  // Auto-pick the first matching session when nothing's selected (or the
  // current selection has dropped out of the list).
  const visible = useMemo(() => applyFilters(sessions, filters), [sessions, filters]);
  const selected: LogSession | null = useMemo(() => {
    if (!selectedId) return visible[0] ?? null;
    const found = sessions.find((s) => s.session_id === selectedId);
    if (found) return found;
    return visible[0] ?? null;
  }, [sessions, visible, selectedId]);

  const kpis = useMemo(() => computeKpis(sessions), [sessions]);

  const handleStatusToggle = (key: keyof SessionFilters["status"]) => {
    setFilters((prev) => ({
      ...prev,
      status: { ...prev.status, [key]: !prev.status[key] },
    }));
  };

  return (
    <div className="mission-control">
      <KpiStrip kpis={kpis} />
      <div className="mission-control__body">
        <SessionListPanel
          sessions={sessions}
          selectedId={selected?.session_id ?? null}
          filters={filters}
          loading={loading}
          tokenIndex={tokenIndex}
          onSearchChange={(value) =>
            setFilters((prev) => ({ ...prev, search: value }))
          }
          onStatusToggle={handleStatusToggle}
          onSelect={setSelectedId}
        />
        <SessionDetailPane session={selected} tokenIndex={tokenIndex} />
        <OpenLoopsPanel />
      </div>
    </div>
  );
}
