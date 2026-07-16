import { useCallback, useEffect, useState } from "react";
import { getTransport } from "@/services/transport";
import type { AutonomyItem, AutonomyItemDetail, AutonomyState } from "@/services/transport";

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
  const [details, setDetails] = useState<Record<string, AutonomyItemDetail>>({});
  const [resumedSession, setResumedSession] = useState<Record<string, string>>({});

  const load = useCallback(async () => {
    const transport = await getTransport();
    const result = await transport.listAutonomyItems();
    if (result.success && result.data) {
      setItems(result.data);
      setError(null);
    } else {
      setError(result.error ?? "Unable to load decision threads");
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
      setError(result.error ?? `Unable to ${next} this decision thread`);
    }
    setUpdating(null);
  };

  const inspect = async (item: AutonomyItem) => {
    setUpdating(item.id);
    const transport = await getTransport();
    const result = await transport.getAutonomyItem(item.id);
    if (result.success && result.data) {
      setDetails((current) => ({ ...current, [item.id]: result.data! }));
      setError(null);
    } else {
      setError(result.error ?? "Unable to inspect this decision thread");
    }
    setUpdating(null);
  };

  const resume = async (item: AutonomyItem) => {
    setUpdating(item.id);
    const transport = await getTransport();
    const result = await transport.resumeAutonomyItem(item.id);
    if (result.success && result.data) {
      setResumedSession((current) => ({ ...current, [item.id]: result.data!.session_id }));
      setError(null);
    } else {
      setError(result.error ?? "Unable to start this decision thread");
    }
    setUpdating(null);
  };

  return (
    <section className="open-loops" aria-label="Decision threads">
      <div className="open-loops__header"><h2>Decision threads</h2><span>{items.length}</span></div>
      {loading && <p className="open-loops__empty">Loading...</p>}
      {!loading && error && <p className="open-loops__error">{error}</p>}
      {!loading && !error && items.length === 0 && <p className="open-loops__empty">No active decision threads.</p>}
      <div className="open-loops__items">
        {items.map((item) => (
          <article className="open-loops__item" key={item.id}>
            <div className="open-loops__title"><strong>{item.title}</strong><span data-state={item.state}>{item.state}</span></div>
            <p>{item.next_action}</p>
            <div className="open-loops__actions">
              <button type="button" disabled={updating === item.id} onClick={() => void inspect(item)}>
                Inspect
              </button>
              {ACTIONS[item.state].map((action) => (
                <button key={action.state} type="button" disabled={updating === item.id} onClick={() => void transition(item, action.state)}>
                  {action.label}
                </button>
              ))}
              {item.state === "approved" && item.source_session_id && (
                <button type="button" disabled={updating === item.id} onClick={() => void resume(item)}>
                  Resume
                </button>
              )}
            </div>
            {details[item.id] && (
              <div className="open-loops__detail">
                <strong>Source session</strong>
                <span>{details[item.id].source_session_id ?? "No source session."}</span>
                <strong>Evidence references ({details[item.id].evidence.length})</strong>
                {details[item.id].evidence.length === 0 ? (
                  <span>No linked evidence.</span>
                ) : (
                  <ul>
                    {details[item.id].evidence.map((entry) => (
                      <li key={entry.id}>{entry.kind}: {entry.reference_id}</li>
                    ))}
                  </ul>
                )}
              </div>
            )}
            {resumedSession[item.id] && (
              <p className="open-loops__resume-status">Started new session {resumedSession[item.id]}.</p>
            )}
          </article>
        ))}
      </div>
    </section>
  );
}
