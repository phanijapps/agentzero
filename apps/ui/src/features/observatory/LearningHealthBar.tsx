// ============================================================================
// LEARNING HEALTH BAR — Bottom status bar for Observatory
// ============================================================================

import { useState } from "react";
import { ArrowUpRight } from "lucide-react";
import { useGraphStats, useDistillationStatus, useBackfill } from "./graph-hooks";
import { useBeliefNetworkStats } from "./belief-network/hooks";
import { useHierarchyStats } from "./hierarchy/hooks";
import { Slideover } from "@/components/Slideover";
import { BeliefNetworkPanel } from "./belief-network/BeliefNetworkPanel";
import { HierarchyPanel } from "./hierarchy/HierarchyPanel";

export function LearningHealthBar() {
  const { stats, loading: statsLoading } = useGraphStats();
  const { status, loading: distLoading, refetch: refetchStatus } = useDistillationStatus();
  // Belief Network totals fold into this strip so the page doesn't carry
  // a separate 3-card panel. The detail surface (3 cards + activity feed
  // + propagation chain) lives in a slideover opened from the strip.
  const { stats: beliefStats } = useBeliefNetworkStats();
  // Hierarchy totals — same folding pattern as Belief Network.
  const { stats: hierStats } = useHierarchyStats(10);
  const [detailsOpen, setDetailsOpen] = useState(false);
  const [hierDetailsOpen, setHierDetailsOpen] = useState(false);

  const { run, isRunning, isDone, progress, error: backfillError } = useBackfill(
    refetchStatus
  );

  if (statsLoading && distLoading) return null;

  const distilled = status?.success_count ?? 0;
  const failed = status?.failed_count ?? 0;
  const skipped = status?.skipped_count ?? 0;
  const total = distilled + failed + skipped + (status?.permanently_failed_count ?? 0);

  return (
    <div className="observatory__health">
      {/* Distillation status */}
      <div className="observatory__health-item">
        Sessions distilled:{" "}
        <span className="observatory__health-value">
          {distilled} / {total}
        </span>
      </div>

      {/* Counts from graph stats */}
      {stats && (
        <>
          <div className="observatory__health-item">
            Facts:{" "}
            <span className="observatory__health-value">{stats.facts}</span>
          </div>
          <div className="observatory__health-item">
            Entities:{" "}
            <span className="observatory__health-value">{stats.entities}</span>
          </div>
          <div className="observatory__health-item">
            Relationships:{" "}
            <span className="observatory__health-value">{stats.relationships}</span>
          </div>
          <div className="observatory__health-item">
            Episodes:{" "}
            <span className="observatory__health-value">{stats.episodes}</span>
          </div>
          {stats.governance ? (
            <div className="observatory__health-item">
              Governance:{" "}
              <span
                className={
                  stats.governance.supported
                    ? "observatory__health-value"
                    : "observatory__health-value observatory__health-value--warning"
                }
              >
                {stats.governance.supported ? "active" : "inactive"}
                {stats.governance.findingCount > 0
                  ? ` · ${stats.governance.findingCount} finding${
                      stats.governance.findingCount === 1 ? "" : "s"
                    }`
                  : ""}
              </span>
            </div>
          ) : null}
        </>
      )}

      {/* Belief Network totals — folded into this strip so the page
          stays compact. Detail surface lives in the Memory tab
          (Beliefs / Contradictions sub-tabs). */}
      {beliefStats?.enabled ? (
        <>
          <div className="observatory__health-item">
            Beliefs:{" "}
            <span className="observatory__health-value">
              {beliefStats.totals.total_beliefs}
            </span>
          </div>
          {beliefStats.totals.total_unresolved_contradictions > 0 ? (
            <div className="observatory__health-item">
              Contradictions:{" "}
              <span className="observatory__health-value observatory__health-value--warning">
                {beliefStats.totals.total_unresolved_contradictions} unresolved
              </span>
            </div>
          ) : beliefStats.totals.total_contradictions > 0 ? (
            <div className="observatory__health-item">
              Contradictions:{" "}
              <span className="observatory__health-value">
                {beliefStats.totals.total_contradictions}
              </span>
            </div>
          ) : null}
        </>
      ) : null}

      {/* Hierarchy totals — folded into the strip the same way Belief
          Network is. Renders only when `hierarchy.enabled = true`. The
          aggregate count is the sum of layer-1+ entities (i.e. excludes
          layer 0 base entities). */}
      {hierStats?.enabled ? (() => {
        const counts = hierStats.summary.layer_counts;
        const aggregateCount = counts
          .filter(([layer]) => layer > 0)
          .reduce((sum, [, n]) => sum + n, 0);
        const interCluster = hierStats.summary.inter_cluster_relations;
        return (
          <>
            <div className="observatory__health-item">
              Hierarchy:{" "}
              <span className="observatory__health-value">
                {counts.length} layer{counts.length === 1 ? "" : "s"} ·{" "}
                {aggregateCount} agg
              </span>
            </div>
            {interCluster > 0 ? (
              <div className="observatory__health-item">
                Inter-cluster:{" "}
                <span className="observatory__health-value">
                  {interCluster}
                </span>
              </div>
            ) : null}
          </>
        );
      })() : null}

      {/* Distillation counts from /api/distillation/status */}
      {status && (
        <>
          {failed > 0 && (
            <div className="observatory__health-item">
              Failed:{" "}
              <span className="observatory__health-value observatory__health-value--error">
                {failed}
              </span>
            </div>
          )}
          {skipped > 0 && (
            <div className="observatory__health-item">
              Skipped:{" "}
              <span className="observatory__health-value observatory__health-value--warning">
                {skipped}
              </span>
            </div>
          )}
        </>
      )}

      {/* Backfill controls */}
      <div className="observatory__health-item" style={{ marginLeft: "auto" }}>
        {isRunning ? (
          <span className="observatory__health-value">
            Distilling {progress.current}/{progress.total}...
          </span>
        ) : isDone ? (
          <span className="observatory__health-value">{"\u2713"} Done</span>
        ) : backfillError ? (
          <span className="observatory__health-value observatory__health-value--error">
            Backfill failed
          </span>
        ) : null}

        {!isDone && (
          <button
            className="btn btn--sm btn--secondary"
            onClick={run}
            disabled={isRunning}
          >
            Backfill
          </button>
        )}

        {/* Belief Network details drawer trigger \u2014 only when the
            network is enabled. Opens a right-side slideover with
            the 3 worker stats cards + activity feed + propagation
            chain. Default state is closed so the strip stays clean.
            Using lucide's ArrowUpRight (same icon family as the rest
            of the app) \u2014 the unicode arrow U+2197 rendered as a
            fallback glyph in IBM Plex Sans. */}
        {beliefStats?.enabled ? (
          <button
            type="button"
            className="btn btn--sm btn--secondary"
            onClick={() => setDetailsOpen(true)}
            aria-label="Open belief network details"
            title="Belief network worker stats, activity feed, propagation chain"
            style={{ display: "inline-flex", alignItems: "center", gap: 4 }}
          >
            <ArrowUpRight style={{ width: 14, height: 14 }} aria-hidden />
            Belief Network
          </button>
        ) : null}

        {hierStats?.enabled ? (
          <button
            type="button"
            className="btn btn--sm btn--secondary"
            onClick={() => setHierDetailsOpen(true)}
            aria-label="Open hierarchy details"
            title="Hierarchy layer breakdown, inter-cluster edge count, top aggregates"
            style={{ display: "inline-flex", alignItems: "center", gap: 4 }}
          >
            <ArrowUpRight style={{ width: 14, height: 14 }} aria-hidden />
            Hierarchy
          </button>
        ) : null}
      </div>

      <Slideover
        open={detailsOpen}
        onClose={() => setDetailsOpen(false)}
        title="Belief Network details"
        subtitle="Worker stats · activity feed · propagation chain"
      >
        <BeliefNetworkPanel />
      </Slideover>

      <Slideover
        open={hierDetailsOpen}
        onClose={() => setHierDetailsOpen(false)}
        title="Hierarchy details"
        subtitle="Layer breakdown · inter-cluster edges · top aggregates"
      >
        <HierarchyPanel />
      </Slideover>
    </div>
  );
}
