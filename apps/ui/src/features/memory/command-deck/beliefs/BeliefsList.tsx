// ============================================================================
// BeliefsList — top-level list view for the Beliefs sub-tab.
//
// Loads beliefs for the active partition (ward), applies UI-side filters,
// renders rows via BeliefCard, opens the detail drawer on click.
// ============================================================================

import { useCallback, useEffect, useMemo, useState } from "react";
import { listBeliefs, listContradictions } from "./api";
import { BeliefCard } from "./BeliefCard";
import { BeliefDetailDrawer } from "./BeliefDetailDrawer";
import type { Belief, BeliefContradiction } from "../types.beliefs";

interface Props {
  agentId: string;
  partitionId: string | null;
}

interface State {
  beliefs: Belief[];
  contradictionsByBelief: Map<string, BeliefContradiction[]>;
  loading: boolean;
  disabled: boolean;
  error: string | null;
}

const INITIAL_STATE: State = {
  beliefs: [],
  contradictionsByBelief: new Map(),
  loading: false,
  disabled: false,
  error: null,
};

export function BeliefsList({ agentId, partitionId }: Props) {
  const [state, setState] = useState<State>(INITIAL_STATE);
  const [openId, setOpenId] = useState<string | null>(null);
  const [onlyContradicted, setOnlyContradicted] = useState(false);
  const [minConfidence, setMinConfidence] = useState(0);

  const load = useCallback(async () => {
    if (!partitionId) return;
    setState((s) => ({ ...s, loading: true }));
    const [bRes, cRes] = await Promise.all([
      listBeliefs(agentId, partitionId, 50, 0),
      listContradictions(agentId, partitionId, 50),
    ]);

    if (bRes.disabled) {
      setState({ ...INITIAL_STATE, disabled: true });
      return;
    }
    if (!bRes.success || !bRes.data) {
      setState({
        ...INITIAL_STATE,
        error: bRes.error ?? "Failed to load beliefs",
      });
      return;
    }
    const map = indexContradictions(cRes.success ? cRes.data ?? [] : []);
    setState({
      beliefs: bRes.data,
      contradictionsByBelief: map,
      loading: false,
      disabled: false,
      error: null,
    });
  }, [agentId, partitionId]);

  useEffect(() => {
    if (!partitionId) {
      setState(INITIAL_STATE);
      return;
    }
    void load();
  }, [partitionId, load]);

  const visible = useMemo(
    () =>
      filterAndSort(
        state.beliefs,
        state.contradictionsByBelief,
        onlyContradicted,
        minConfidence,
      ),
    [state.beliefs, state.contradictionsByBelief, onlyContradicted, minConfidence],
  );

  if (!partitionId) {
    return <div className="memory-empty">Select a ward to view beliefs.</div>;
  }
  if (state.disabled) {
    return (
      <div className="memory-empty">
        Belief Network is disabled. Enable it under Settings → Advanced →
        Memory.
      </div>
    );
  }

  return (
    <div className="beliefs-list">
      <FilterBar
        onlyContradicted={onlyContradicted}
        onContradictedChange={setOnlyContradicted}
        minConfidence={minConfidence}
        onConfidenceChange={setMinConfidence}
      />
      {state.loading && state.beliefs.length === 0 ? (
        <div className="memory-empty">Loading…</div>
      ) : null}
      {state.error ? <div className="memory-empty">{state.error}</div> : null}
      {visible.length === 0 && !state.loading && !state.error ? (
        <div className="memory-empty">No beliefs match the current filters.</div>
      ) : null}
      <ul className="beliefs-list__items">
        {visible.map((b) => (
          <li key={b.id}>
            <BeliefCard
              belief={b}
              contradictions={state.contradictionsByBelief.get(b.id)}
              onClick={() => setOpenId(b.id)}
            />
          </li>
        ))}
      </ul>
      <BeliefDetailDrawer
        agentId={agentId}
        beliefId={openId}
        onClose={() => setOpenId(null)}
      />
    </div>
  );
}

interface FilterBarProps {
  onlyContradicted: boolean;
  onContradictedChange: (v: boolean) => void;
  minConfidence: number;
  onConfidenceChange: (v: number) => void;
}

function FilterBar(props: FilterBarProps) {
  return (
    <div className="beliefs-filter-bar">
      <label
        className="beliefs-filter-bar__chip"
        htmlFor="beliefs-filter-contradicted"
      >
        <input
          id="beliefs-filter-contradicted"
          type="checkbox"
          checked={props.onlyContradicted}
          onChange={(e) => props.onContradictedChange(e.target.checked)}
        />
        Only contradicted
      </label>
      <label
        className="beliefs-filter-bar__range"
        htmlFor="beliefs-filter-confidence"
      >
        Min confidence: {props.minConfidence.toFixed(2)}
        <input
          id="beliefs-filter-confidence"
          type="range"
          min={0}
          max={1}
          step={0.05}
          value={props.minConfidence}
          onChange={(e) =>
            props.onConfidenceChange(Number.parseFloat(e.target.value))
          }
        />
      </label>
    </div>
  );
}

function indexContradictions(
  rows: BeliefContradiction[],
): Map<string, BeliefContradiction[]> {
  const map = new Map<string, BeliefContradiction[]>();
  for (const c of rows) {
    pushTo(map, c.belief_a_id, c);
    pushTo(map, c.belief_b_id, c);
  }
  return map;
}

function pushTo(
  map: Map<string, BeliefContradiction[]>,
  key: string,
  value: BeliefContradiction,
) {
  const existing = map.get(key);
  if (existing) {
    existing.push(value);
  } else {
    map.set(key, [value]);
  }
}

function hasUnresolvedContradiction(
  beliefId: string,
  contradictions: Map<string, BeliefContradiction[]>,
): boolean {
  const list = contradictions.get(beliefId);
  if (!list) return false;
  return list.some((c) => !c.resolved_at);
}

function filterAndSort(
  beliefs: Belief[],
  contradictions: Map<string, BeliefContradiction[]>,
  onlyContradicted: boolean,
  minConfidence: number,
): Belief[] {
  return beliefs
    .filter((b) => b.confidence >= minConfidence)
    .filter(
      (b) =>
        !onlyContradicted || hasUnresolvedContradiction(b.id, contradictions),
    )
    .slice()
    .sort((x, y) => y.confidence - x.confidence);
}
