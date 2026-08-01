import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { SessionsList, groupSessions } from "./SessionsList";
import type { SessionSummary } from "./types";

const ONE_DAY = 24 * 60 * 60 * 1000;
const now = Date.now();
// Compute local midnight the same way groupSessions does so fixture dates
// land in the right bucket regardless of timezone or time-of-day.
const _today = new Date(now);
_today.setHours(0, 0, 0, 0);
const startOfTodayMs = _today.getTime();

const fixture: SessionSummary[] = [
  { id: "s1", title: "Running one",   status: "running",  wardName: "stock-analysis", updatedAt: now - 1000 },
  { id: "s2", title: "Today done",    status: "complete", wardName: "stock-analysis", updatedAt: startOfTodayMs + 30 * 60 * 1000 },
  { id: "s3", title: "Yesterday",     status: "complete", wardName: "maritime",       updatedAt: startOfTodayMs - 12 * 60 * 60 * 1000 },
  { id: "s4", title: "Last week",     status: "complete", wardName: null,             updatedAt: now - 5 * ONE_DAY },
  { id: "s5", title: "Older",         status: "crashed",  wardName: null,             updatedAt: now - 30 * ONE_DAY },
];

describe("groupSessions", () => {
  it("groups into Running / Today / Yesterday / Last week / Older", () => {
    const groups = groupSessions(fixture, now);
    expect(groups.Running.map((s) => s.id)).toEqual(["s1"]);
    expect(groups.Today.map((s) => s.id)).toEqual(["s2"]);
    expect(groups.Yesterday.map((s) => s.id)).toEqual(["s3"]);
    expect(groups["Last week"].map((s) => s.id)).toEqual(["s4"]);
    expect(groups.Older.map((s) => s.id)).toEqual(["s5"]);
  });

  it("sorts each bucket newest-first", () => {
    const batch: SessionSummary[] = [
      { id: "a", title: "A", status: "complete", wardName: null, updatedAt: startOfTodayMs + 30 * 60 * 1000 },
      { id: "b", title: "B", status: "complete", wardName: null, updatedAt: startOfTodayMs + 20 * 60 * 1000 },
      { id: "c", title: "C", status: "complete", wardName: null, updatedAt: startOfTodayMs + 40 * 60 * 1000 },
    ];
    expect(groupSessions(batch, now).Today.map((s) => s.id)).toEqual(["c", "a", "b"]);
  });

  it("puts running sessions in Running regardless of recency", () => {
    const oldRunning: SessionSummary[] = [
      { id: "r1", title: "Old run", status: "running", wardName: null, updatedAt: now - 30 * ONE_DAY },
    ];
    const groups = groupSessions(oldRunning, now);
    expect(groups.Running.map((s) => s.id)).toEqual(["r1"]);
    expect(groups.Older).toHaveLength(0);
  });
});

interface RenderOpts {
  sessions?: SessionSummary[];
  currentId?: string | null;
  onSelect?: (id: string) => void;
  onNew?: () => void;
  onDelete?: (id: string) => void;
}

function renderList(opts: RenderOpts = {}) {
  return render(
    <SessionsList
      sessions={opts.sessions ?? fixture}
      currentId={opts.currentId ?? null}
      onSelect={opts.onSelect ?? (() => {})}
      onNew={opts.onNew ?? (() => {})}
      onDelete={opts.onDelete ?? (() => {})}
      renderDensity="expanded"
    />
  );
}

describe("<SessionsList>", () => {
  it("renders group headers and rows", () => {
    renderList();
    expect(screen.getByText("Running")).toBeTruthy();
    expect(screen.getByText("Today done")).toBeTruthy();
  });

  it("fires onSelect with the session id on row click", () => {
    const fn = vi.fn();
    renderList({ onSelect: fn });
    fireEvent.click(screen.getByText("Running one"));
    expect(fn).toHaveBeenCalledWith("s1");
  });

  it("fires onNew from the New button", () => {
    const fn = vi.fn();
    renderList({ onNew: fn });
    fireEvent.click(screen.getByTestId("sessions-list-new"));
    expect(fn).toHaveBeenCalled();
  });

  it("shows an empty-state message when no sessions", () => {
    renderList({ sessions: [] });
    expect(screen.getByText(/no research sessions yet/i)).toBeTruthy();
  });

  it("marks the current row with --active class", () => {
    const { container } = renderList({ currentId: "s2" });
    const active = container.querySelector(".sessions-list__row--active");
    expect(active).not.toBeNull();
    expect(active?.textContent).toContain("Today done");
  });

  // ---------------------------------------------------------------------------
  // R19 — per-row Delete button
  // ---------------------------------------------------------------------------

  it("renders a Delete button for each session row", () => {
    renderList();
    for (const s of fixture) {
      expect(screen.getByTestId(`sessions-list-delete-${s.id}`)).toBeTruthy();
    }
  });

  it("Delete click fires onDelete with the session id", () => {
    const onDelete = vi.fn();
    renderList({ onDelete });
    fireEvent.click(screen.getByTestId("sessions-list-delete-s1"));
    expect(onDelete).toHaveBeenCalledWith("s1");
  });

  it("Delete click does NOT trigger onSelect (stopPropagation)", () => {
    const onDelete = vi.fn();
    const onSelect = vi.fn();
    renderList({ onDelete, onSelect });
    fireEvent.click(screen.getByTestId("sessions-list-delete-s1"));
    expect(onDelete).toHaveBeenCalledWith("s1");
    expect(onSelect).not.toHaveBeenCalled();
  });
});
