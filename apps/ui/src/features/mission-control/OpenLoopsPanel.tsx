import { useCallback, useEffect, useState } from "react";
import { getTransport } from "@/services/transport";
import type { AutonomyItem, AutonomyState } from "@/services/transport";

const ACTIONS: Record<AutonomyState, Array<{ state: AutonomyState; label: string }>> = {
  proposed: [{ state: "approved", label: "Approve" }, { state: "stale", label: "Discard" }],
  approved: [{ state: "blocked", label: "Block" }, { state: "complete", label: "Complete" }],
  blocked: [{ state: "approved", label: "Reopen" }, { state: "complete", label: "Complete" }],
  complete: [],
  stale: [{ state: "approved", label: "Reopen" }, { state: "complete", label: "Complete" }],
};

export function OpenLoopsPanel() {
  const [items, setItems] = useState<AutonomyItem[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [updating, setUpdating] = useState<string | null>(null);

  const load = useCallback(async () => {
    const transport = await getTransport();
    const result = await transport.listAutonomyItems();
    if (result.success && result.data) {
      setItems(result.data);
      setError(null);
    } else {
      setError(result.error ?? "Unable to load open loops");
    }
    setLoading(false);
  }, []);

  useEffect(() => { void load(); }, [load]);

  const transition = async (item: AutonomyItem, next: AutonomyState) => {
    setUpdating(item.id);
    const transport = await getTransport();
    const result = await transport.transitionAutonomyItem(item.id, next);
    if (result.success && result.data) {
      const updated = result.data;
      setItems((current) => current.map((entry) => entry.id === item.id ? updated : entry));
      setError(null);
    } else {
      setError(result.error ?? `Unable to ${next} this open loop`);
    }
    setUpdating(null);
  };

  return (
    <section className="open-loops" aria-label="Open loops">
      <div className="open-loops__header"><h2>Open Loops</h2><span>{items.length}</span></div>
      {loading && <p className="open-loops__empty">Loading...</p>}
      {!loading && error && <p className="open-loops__error">{error}</p>}
      {!loading && !error && items.length === 0 && <p className="open-loops__empty">No active decision threads.</p>}
      <div className="open-loops__items">
        {items.map((item) => (
          <article className="open-loops__item" key={item.id}>
            <div className="open-loops__title"><strong>{item.title}</strong><span data-state={item.state}>{item.state}</span></div>
            <p>{item.next_action}</p>
            <div className="open-loops__actions">
              {ACTIONS[item.state].map((action) => (
                <button key={action.state} type="button" disabled={updating === item.id} onClick={() => void transition(item, action.state)}>
                  {action.label}
                </button>
              ))}
            </div>
          </article>
        ))}
      </div>
    </section>
  );
}
