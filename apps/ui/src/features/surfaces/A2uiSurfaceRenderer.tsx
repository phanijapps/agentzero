import { useState } from "react";
import type { WorkSurface, WorkSurfaceComponent } from "@/services/transport/types";
import "./a2ui-surface.css";

function atPath(data: Record<string, unknown>, path: unknown): unknown {
  if (typeof path !== "string" || !path.startsWith("/")) return undefined;
  return path.slice(1).split("/").reduce<unknown>((value, segment) => (
    value && typeof value === "object" ? (value as Record<string, unknown>)[segment.replace(/~1/g, "/").replace(/~0/g, "~")] : undefined
  ), data);
}

function title(component: WorkSurfaceComponent): string {
  return typeof component.props?.title === "string" ? component.props.title : component.type;
}

function List({ value }: { value: unknown }) {
  if (!Array.isArray(value)) return <p>Nothing to display.</p>;
  return <ul>{value.map((item, index) => <li key={index}>{typeof item === "string" ? item : JSON.stringify(item)}</li>)}</ul>;
}

type PlanItem = { step: string; status?: string };

function isPlanItem(value: unknown): value is PlanItem {
  return Boolean(value && typeof value === "object" && typeof (value as Record<string, unknown>).step === "string");
}

function PlanChecklist({ value }: { value: unknown }) {
  if (!Array.isArray(value)) return <p>Nothing to display.</p>;
  return <ol className="a2ui-plan" aria-label="Plan checklist">
    {value.map((item, index) => {
      if (!isPlanItem(item)) return <li key={index}>{typeof item === "string" ? item : JSON.stringify(item)}</li>;
      const status = item.status?.toLowerCase() ?? "pending";
      const completed = status === "completed" || status === "complete" || status === "done";
      return <li className="a2ui-plan__item" key={index}>
        <span aria-hidden="true" className={`a2ui-plan__mark a2ui-plan__mark--${status}`}>{completed ? "✓" : "○"}</span>
        <span className="a2ui-plan__step">{item.step}</span>
        <span className="a2ui-plan__status">{status}</span>
      </li>;
    })}
  </ol>;
}

function records(value: unknown): Record<string, unknown>[] {
  return Array.isArray(value)
    ? value.filter((item): item is Record<string, unknown> => Boolean(item && typeof item === "object" && !Array.isArray(item)))
    : [];
}

function EvidenceTable({ value }: { value: unknown }) {
  const rows = records(value);
  if (rows.length === 0) return <p>Nothing to display.</p>;
  return <div className="a2ui-table-wrap"><table className="a2ui-table"><thead><tr><th>Claim</th><th>Source</th><th>Confidence</th></tr></thead><tbody>{rows.map((row, index) => <tr key={index}><td>{String(row.claim ?? row.summary ?? "—")}</td><td>{String(row.source ?? row.reference ?? "—")}</td><td>{String(row.confidence ?? "—")}</td></tr>)}</tbody></table></div>;
}

function DecisionMatrix({ criteria, options }: { criteria: unknown; options: unknown }) {
  const columns = Array.isArray(criteria) ? criteria.filter((item): item is string => typeof item === "string") : [];
  const rows = records(options);
  if (rows.length === 0) return <p>Nothing to display.</p>;
  return <div className="a2ui-table-wrap"><table className="a2ui-table"><thead><tr><th>Option</th>{columns.map(column => <th key={column}>{column}</th>)}</tr></thead><tbody>{rows.map((row, index) => {
    const scores = row.scores && typeof row.scores === "object" ? row.scores as Record<string, unknown> : {};
    return <tr key={index}><td>{String(row.name ?? row.option ?? "Option")}</td>{columns.map(column => <td key={column}>{String(scores[column] ?? "—")}</td>)}</tr>;
  })}</tbody></table></div>;
}

function AssumptionRegister({ value }: { value: unknown }) {
  const rows = records(value);
  if (rows.length === 0) return <p>Nothing to display.</p>;
  return <ul className="a2ui-record-list">{rows.map((row, index) => <li key={index}><strong>{String(row.assumption ?? row.title ?? "Assumption")}</strong><span>{String(row.status ?? row.validation ?? "unvalidated")}</span></li>)}</ul>;
}

function OpenLoops({ value }: { value: unknown }) {
  const rows = records(value);
  if (rows.length === 0) return <List value={value} />;
  return <ul className="a2ui-record-list">{rows.map((row, index) => <li key={index}><strong>{String(row.title ?? row.item ?? row.loop ?? row.step ?? "Open loop")}</strong><span>{String(row.next_action ?? row.status ?? "needs attention")}</span></li>)}</ul>;
}

function SurfaceComponentView({ component, data }: { component: WorkSurfaceComponent; data: Record<string, unknown> }) {
  const props = component.props ?? {};
  switch (component.type) {
    case "DecisionMatrix": return <><h3>{title(component)}</h3><DecisionMatrix criteria={atPath(data, props.criteria_path)} options={atPath(data, props.options_path)} /></>;
    case "EvidenceTable": return <><h3>{title(component)}</h3><EvidenceTable value={atPath(data, props.evidence_path)} /></>;
    case "AssumptionRegister": return <><h3>{title(component)}</h3><AssumptionRegister value={atPath(data, props.assumptions_path)} /></>;
    case "PlanChecklist": return <><h3>{title(component)}</h3><PlanChecklist value={atPath(data, props.plan_path)} /></>;
    case "OpenLoops": return <><h3>{title(component)}</h3><OpenLoops value={atPath(data, props.items_path)} /></>;
    case "ApprovalGate": return <ApprovalGate component={component} />;
  }
}

function ApprovalGate({ component }: { component: WorkSurfaceComponent }) {
  const [status, setStatus] = useState<string | null>(null);
  const props = component.props ?? {};
  const actionId = typeof props.action_id === "string" ? props.action_id : null;
  const target = typeof props.target === "string" ? props.target : null;
  const expectedState = typeof props.expected_state === "string" ? props.expected_state : null;
  const invoke = async () => {
    if (!actionId || !target || !expectedState) return;
    setStatus("Submitting…");
    const response = await fetch("/api/surfaces/actions", { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ action_id: actionId, surface_id: component.id, target, expected_state: expectedState }) });
    setStatus(response.ok ? "Updated." : "Gateway denied this action.");
  };
  return <><h3>{title(component)}</h3><button type="button" onClick={invoke} disabled={!actionId || !target || !expectedState}>{title(component)}</button>{status && <p role="status">{status}</p>}</>;
}

/** Native, static catalog renderer. It never evaluates agent-supplied code or HTML. */
export function A2uiSurfaceRenderer({ surface }: { surface: WorkSurface }) {
  if (surface.catalog_id !== "zbot/work-surface/v1") return null;
  return <section className="a2ui-surface" aria-label="Agent work surface">
    {surface.components.map(component => <article className="a2ui-surface__card" key={component.id}><SurfaceComponentView component={component} data={surface.data} /></article>)}
  </section>;
}
