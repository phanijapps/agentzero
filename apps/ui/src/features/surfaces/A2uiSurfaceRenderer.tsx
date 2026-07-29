import { useState } from "react";
import type { WorkSurface, WorkSurfaceComponent } from "@/services/transport/types";
import {
  Bar,
  BarChart as RechartsBarChart,
  CartesianGrid,
  Cell,
  Line,
  LineChart as RechartsLineChart,
  Pie,
  PieChart as RechartsPieChart,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from "recharts";
import "./a2ui-surface.css";

function atPath(data: Record<string, unknown>, path: unknown): unknown {
  if (path === "") return data;
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

const CHART_COLORS = [
  "var(--chart-1)",
  "var(--chart-2)",
  "var(--chart-3)",
  "var(--chart-4)",
  "var(--chart-5)",
];

function Empty() {
  return <p>Nothing to display.</p>;
}

function displayValue(value: unknown): string | null {
  if (typeof value === "string") return value;
  if (typeof value === "number" && Number.isFinite(value)) return String(value);
  if (typeof value === "boolean") return String(value);
  return null;
}

function stringProp(value: unknown): string | null {
  return typeof value === "string" ? value : null;
}

function stringArrayProp(value: unknown): string[] {
  return Array.isArray(value) ? value.filter((item): item is string => typeof item === "string") : [];
}

function MetricCard({ value, detail, label }: { value: unknown; detail: unknown; label: unknown }) {
  const rendered = displayValue(value);
  if (rendered === null) return <Empty />;
  const renderedDetail = displayValue(detail);
  return <div className="a2ui-metric">
    {stringProp(label) && <span className="a2ui-metric__label">{String(label)}</span>}
    <strong className="a2ui-metric__value">{rendered}</strong>
    {renderedDetail !== null && <span className="a2ui-metric__detail">{renderedDetail}</span>}
  </div>;
}

function ProgressBar({ value, max, label }: { value: unknown; max: unknown; label: unknown }) {
  if (typeof value !== "number" || !Number.isFinite(value)) return <Empty />;
  const maximum = typeof max === "number" && Number.isFinite(max) && max > 0 ? max : 100;
  const boundedValue = Math.max(0, Math.min(value, maximum));
  const accessibleLabel = stringProp(label) ?? "Progress";
  return <div className="a2ui-progress">
    <div className="a2ui-progress__header"><span>{accessibleLabel}</span><span>{value} / {maximum}</span></div>
    <progress aria-label={accessibleLabel} aria-valuemin={0} aria-valuemax={maximum} aria-valuenow={boundedValue} max={maximum} value={boundedValue} />
  </div>;
}

function statusTone(value: string): "info" | "success" | "warning" | "error" {
  const normalized = value.toLowerCase();
  if (["healthy", "success", "complete", "completed", "ready", "done"].includes(normalized)) return "success";
  if (["warning", "warn", "pending", "blocked"].includes(normalized)) return "warning";
  if (["error", "failed", "failure", "critical"].includes(normalized)) return "error";
  return "info";
}

function StatusBadge({ value, label }: { value: unknown; label: unknown }) {
  const rendered = displayValue(value);
  if (rendered === null) return <Empty />;
  return <div className="a2ui-status">
    {stringProp(label) && <span>{String(label)}</span>}
    <span className={`a2ui-status__badge a2ui-status__badge--${statusTone(rendered)}`}>{rendered}</span>
  </div>;
}

function Callout({ value, tone }: { value: unknown; tone: unknown }) {
  const rendered = displayValue(value);
  if (rendered === null) return <Empty />;
  const safeTone = ["info", "success", "warning", "error"].includes(String(tone)) ? String(tone) : "info";
  return <p className={`a2ui-callout a2ui-callout--${safeTone}`} role="note">{rendered}</p>;
}

function KeyValueList({ value }: { value: unknown }) {
  if (!value || typeof value !== "object" || Array.isArray(value)) return <Empty />;
  const entries = Object.entries(value).flatMap(([key, item]) => {
    const rendered = displayValue(item);
    return rendered === null ? [] : [[key, rendered] as const];
  });
  if (entries.length === 0) return <Empty />;
  return <dl className="a2ui-key-values">{entries.map(([key, rendered]) => <div key={key}><dt>{key}</dt><dd>{rendered}</dd></div>)}</dl>;
}

function DataTable({ value, columns, label }: { value: unknown; columns: unknown; label: string }) {
  const rows = records(value);
  if (rows.length === 0) return <Empty />;
  const declared = stringArrayProp(columns);
  const visibleColumns = (declared.length > 0 ? declared : Object.keys(rows[0] ?? {})).slice(0, 20);
  if (visibleColumns.length === 0) return <Empty />;
  return <div className="a2ui-table-wrap"><table className="a2ui-table" aria-label={label}><thead><tr>{visibleColumns.map(column => <th key={column}>{column}</th>)}</tr></thead><tbody>{rows.map((row, index) => <tr key={index}>{visibleColumns.map(column => <td key={column}>{displayValue(row[column]) ?? "—"}</td>)}</tr>)}</tbody></table></div>;
}

function Timeline({ value }: { value: unknown }) {
  const items = records(value).filter(item => [item.title, item.time, item.description, item.status].some(field => displayValue(field) !== null));
  if (items.length === 0) return <Empty />;
  return <ol className="a2ui-timeline">{items.map((item, index) => <li key={index}>
    <div className="a2ui-timeline__heading"><strong>{displayValue(item.title) ?? "Timeline item"}</strong>{displayValue(item.time) && <time>{displayValue(item.time)}</time>}</div>
    {displayValue(item.description) && <p>{displayValue(item.description)}</p>}
    {displayValue(item.status) && <span className="a2ui-timeline__status">{displayValue(item.status)}</span>}
  </li>)}</ol>;
}

function chartRecords(value: unknown, xKey: string, series: string[]): Record<string, string | number>[] {
  return records(value).flatMap(row => {
    const x = displayValue(row[xKey]);
    if (x === null) return [];
    const values = series.map(key => row[key]);
    if (!values.some(value => typeof value === "number" && Number.isFinite(value))) return [];
    const normalized: Record<string, string | number> = { [xKey]: x };
    for (const key of series) {
      const value = row[key];
      if (typeof value === "number" && Number.isFinite(value)) normalized[key] = value;
    }
    return [normalized];
  });
}

function ChartLegend({ labels }: { labels: string[] }) {
  return <ul className="a2ui-chart__legend" aria-label="Chart legend">{labels.map((label, index) => <li key={label}><span className={`a2ui-chart__swatch a2ui-chart__swatch--${index % CHART_COLORS.length}`} aria-hidden="true" />{label}</li>)}</ul>;
}

function CartesianChart({ kind, value, xKey, series, label }: { kind: "line" | "bar"; value: unknown; xKey: unknown; series: unknown; label: string }) {
  const safeXKey = stringProp(xKey);
  const safeSeries = stringArrayProp(series);
  if (!safeXKey || safeSeries.length === 0) return <Empty />;
  const data = chartRecords(value, safeXKey, safeSeries);
  if (data.length === 0) return <Empty />;
  const chart = kind === "line"
    ? <RechartsLineChart data={data} accessibilityLayer><CartesianGrid strokeDasharray="3 3" /><XAxis dataKey={safeXKey} /><YAxis /><Tooltip />{safeSeries.map((key, index) => <Line key={key} type="monotone" dataKey={key} stroke={CHART_COLORS[index % CHART_COLORS.length]} />)}</RechartsLineChart>
    : <RechartsBarChart data={data} accessibilityLayer><CartesianGrid strokeDasharray="3 3" /><XAxis dataKey={safeXKey} /><YAxis /><Tooltip />{safeSeries.map((key, index) => <Bar key={key} dataKey={key} fill={CHART_COLORS[index % CHART_COLORS.length]} />)}</RechartsBarChart>;
  return <div className="a2ui-chart" role="group" aria-label={label} data-accessibility-layer="true">
    <ResponsiveContainer width="100%" height={280} initialDimension={{ width: 600, height: 280 }}>{chart}</ResponsiveContainer>
    <ChartLegend labels={safeSeries} />
  </div>;
}

function PieChart({ value, nameKey, valueKey, label }: { value: unknown; nameKey: unknown; valueKey: unknown; label: string }) {
  const safeNameKey = stringProp(nameKey);
  const safeValueKey = stringProp(valueKey);
  if (!safeNameKey || !safeValueKey) return <Empty />;
  const data = records(value).flatMap(row => {
    const name = displayValue(row[safeNameKey]);
    const amount = row[safeValueKey];
    return name !== null && typeof amount === "number" && Number.isFinite(amount) ? [{ [safeNameKey]: name, [safeValueKey]: amount }] : [];
  });
  if (data.length === 0) return <Empty />;
  const labels = data.map(row => String(row[safeNameKey]));
  return <div className="a2ui-chart" role="group" aria-label={label} data-accessibility-layer="true">
    <ResponsiveContainer width="100%" height={280} initialDimension={{ width: 600, height: 280 }}>
      <RechartsPieChart accessibilityLayer><Pie data={data} dataKey={safeValueKey} nameKey={safeNameKey} outerRadius={100}>{data.map((_, index) => <Cell key={labels[index]} fill={CHART_COLORS[index % CHART_COLORS.length]} />)}</Pie><Tooltip /></RechartsPieChart>
    </ResponsiveContainer>
    <ChartLegend labels={labels} />
  </div>;
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
    case "MetricCard": return <><h3>{title(component)}</h3><MetricCard value={atPath(data, props.value_path)} detail={atPath(data, props.detail_path)} label={props.label} /></>;
    case "ProgressBar": return <><h3>{title(component)}</h3><ProgressBar value={atPath(data, props.value_path)} max={props.max} label={props.label ?? props.title} /></>;
    case "StatusBadge": return <><h3>{title(component)}</h3><StatusBadge value={atPath(data, props.value_path)} label={props.label} /></>;
    case "Callout": return <><h3>{title(component)}</h3><Callout value={atPath(data, props.message_path)} tone={props.tone} /></>;
    case "KeyValueList": return <><h3>{title(component)}</h3><KeyValueList value={atPath(data, props.items_path)} /></>;
    case "DataTable": return <><h3>{title(component)}</h3><DataTable value={atPath(data, props.rows_path)} columns={props.columns} label={title(component)} /></>;
    case "Timeline": return <><h3>{title(component)}</h3><Timeline value={atPath(data, props.items_path)} /></>;
    case "LineChart": return <><h3>{title(component)}</h3><CartesianChart kind="line" value={atPath(data, props.data_path)} xKey={props.x_key} series={props.series} label={title(component)} /></>;
    case "BarChart": return <><h3>{title(component)}</h3><CartesianChart kind="bar" value={atPath(data, props.data_path)} xKey={props.x_key} series={props.series} label={title(component)} /></>;
    case "PieChart": return <><h3>{title(component)}</h3><PieChart value={atPath(data, props.data_path)} nameKey={props.name_key} valueKey={props.value_key} label={title(component)} /></>;
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
