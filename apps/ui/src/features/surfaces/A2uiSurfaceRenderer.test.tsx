import { fireEvent, render, screen, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import type { WorkSurface } from "@/services/transport/types";
import { A2uiSurfaceRenderer } from "./A2uiSurfaceRenderer";

function surface(
  components: Array<{ id: string; type: string; props: Record<string, unknown> }>,
  data: Record<string, unknown>,
): WorkSurface {
  return {
    surface_id: "expanded-catalog",
    catalog_id: "zbot/work-surface/v1",
    components,
    data,
  } as unknown as WorkSurface;
}

// STUB: AC1, AC6, AC7
describe("A2uiSurfaceRenderer expanded structured catalog", () => {
  it("renders_the_expanded_structured_catalog_safely", () => {
    render(<A2uiSurfaceRenderer surface={surface([
      { id: "metric", type: "MetricCard", props: { title: "Revenue", value_path: "/revenue", detail_path: "/detail" } },
      { id: "progress", type: "ProgressBar", props: { title: "Migration", value_path: "/progress", max: 100 } },
      { id: "status", type: "StatusBadge", props: { title: "Service", value_path: "/status" } },
      { id: "callout", type: "Callout", props: { title: "Notice", message_path: "/message", tone: "warning" } },
      { id: "pairs", type: "KeyValueList", props: { title: "Details", items_path: "/details" } },
      { id: "table", type: "DataTable", props: { title: "Deployments", rows_path: "/rows", columns: ["name", "state"] } },
      { id: "derived-table", type: "DataTable", props: { title: "Derived scores", rows_path: "/derivedRows" } },
      { id: "timeline", type: "Timeline", props: { title: "History", items_path: "/timeline" } },
    ], {
      revenue: 42,
      detail: "<img src=x onerror=alert(1)>",
      progress: 75,
      status: "healthy",
      message: "<script>alert(1)</script>",
      details: { Region: "east", Tier: "production" },
      rows: [{ name: "api", state: "ready" }],
      derivedRows: [{ person: "Grace", result: 12 }],
      timeline: [
        { title: "Deployed", time: "10:30", description: "Release complete", status: "done" },
        { time: "10:31", description: "Title-free update", status: "pending" },
      ],
    })} />);

    expect(screen.getByText("42")).toBeInTheDocument();
    expect(screen.getByRole("progressbar")).toHaveAttribute("aria-valuenow", "75");
    expect(screen.getByText("healthy")).toBeInTheDocument();
    expect(screen.getByText("<script>alert(1)</script>")).toBeInTheDocument();
    expect(screen.getByText("<img src=x onerror=alert(1)>")).toBeInTheDocument();
    expect(document.querySelector("script")).toBeNull();
    expect(document.querySelector("img")).toBeNull();
    expect(screen.getByText("production")).toBeInTheDocument();
    expect(screen.getByRole("table", { name: "Deployments" })).toBeInTheDocument();
    const derivedTable = screen.getByRole("table", { name: "Derived scores" });
    expect(derivedTable).toHaveTextContent("person");
    expect(derivedTable).toHaveTextContent("result");
    expect(derivedTable).toHaveTextContent("Grace");
    expect(derivedTable).toHaveTextContent("12");
    expect(screen.getByText("Release complete")).toBeInTheDocument();
    expect(screen.getByText("Title-free update")).toBeInTheDocument();
    expect(screen.getByText("Timeline item")).toBeInTheDocument();
  });

  it("resolves the empty JSON pointer to the surface data root", () => {
    render(<A2uiSurfaceRenderer surface={surface([
      { id: "root", type: "KeyValueList", props: { title: "Root data", items_path: "" } },
    ], { Region: "east", Tier: "production" })} />);

    expect(screen.getByText("east")).toBeInTheDocument();
    expect(screen.getByText("production")).toBeInTheDocument();
  });

  it("isolates missing and wrong-shaped bindings behind safe empty states", () => {
    render(<A2uiSurfaceRenderer surface={surface([
      { id: "metric", type: "MetricCard", props: { title: "Missing metric", value_path: "/missing" } },
      { id: "progress", type: "ProgressBar", props: { title: "Bad progress", value_path: "/badProgress" } },
      { id: "status", type: "StatusBadge", props: { title: "Missing status", value_path: "/missing" } },
      { id: "callout", type: "Callout", props: { title: "Bad notice", message_path: "/badMessage" } },
      { id: "pairs", type: "KeyValueList", props: { title: "Bad details", items_path: "/badPairs" } },
      { id: "table", type: "DataTable", props: { title: "Empty table", rows_path: "/emptyRows" } },
      { id: "timeline", type: "Timeline", props: { title: "Bad history", items_path: "/badTimeline" } },
      { id: "sibling", type: "Callout", props: { title: "Sibling", message_path: "/visible", tone: "success" } },
    ], {
      badProgress: "seventy-five",
      badMessage: { nested: true },
      badPairs: ["not", "a", "record"],
      emptyRows: [],
      badTimeline: "not-an-array",
      visible: "Still visible",
    })} />);

    expect(screen.getAllByText("Nothing to display.")).toHaveLength(7);
    expect(screen.getByText("Still visible")).toBeInTheDocument();
  });
});

// STUB: AC2, AC3, AC6
describe("A2uiSurfaceRenderer expanded chart catalog", () => {
  it("renders_accessible_dynamic_charts", () => {
    const initial = surface([
      { id: "metric", type: "MetricCard", props: { title: "Total", value_path: "/total" } },
      { id: "line", type: "LineChart", props: { title: "Traffic", data_path: "/traffic", x_key: "day", series: ["requests", "errors"] } },
      { id: "bar", type: "BarChart", props: { title: "Builds", data_path: "/builds", x_key: "day", series: ["passed"] } },
      { id: "pie", type: "PieChart", props: { title: "Usage", data_path: "/usage", name_key: "name", value_key: "value" } },
    ], {
      total: "old metric",
      traffic: [{ day: "Mon", requests: 10, errors: 1 }],
      builds: [{ day: "Mon", passed: 3 }],
      usage: [{ name: "API", value: 60 }, { name: "CLI", value: 40 }],
    });

    const { rerender } = render(<A2uiSurfaceRenderer surface={initial} />);
    expect(screen.getByText("old metric")).toBeInTheDocument();
    expect(screen.getByRole("group", { name: "Traffic" })).toHaveAttribute("data-accessibility-layer", "true");
    expect(screen.getByRole("group", { name: "Builds" })).toHaveAttribute("data-accessibility-layer", "true");
    expect(screen.getByRole("group", { name: "Usage" })).toHaveAttribute("data-accessibility-layer", "true");
    expect(screen.getByText("requests")).toBeInTheDocument();
    expect(screen.getByText("errors")).toBeInTheDocument();
    expect(screen.getByText("API")).toBeInTheDocument();
    expect(screen.getByText("CLI")).toBeInTheDocument();
    const trafficApplication = within(screen.getByRole("group", { name: "Traffic" })).getByRole("application");
    fireEvent.focus(trafficApplication);
    fireEvent.keyDown(trafficApplication, { key: "ArrowRight" });
    const trafficTooltip = within(screen.getByRole("group", { name: "Traffic" })).getByRole("status");
    expect(trafficTooltip).toHaveTextContent("Mon");
    expect(trafficTooltip).toHaveTextContent("10");
    expect(trafficTooltip).toHaveTextContent("1");
    const buildsApplication = within(screen.getByRole("group", { name: "Builds" })).getByRole("application");
    fireEvent.focus(buildsApplication);
    fireEvent.keyDown(buildsApplication, { key: "ArrowRight" });
    const buildsTooltip = within(screen.getByRole("group", { name: "Builds" })).getByRole("status");
    expect(buildsTooltip).toHaveTextContent("Mon");
    expect(buildsTooltip).toHaveTextContent("3");
    const usageApplication = within(screen.getByRole("group", { name: "Usage" })).getByRole("application");
    fireEvent.focus(usageApplication);
    fireEvent.keyDown(usageApplication, { key: "ArrowRight" });
    const usageTooltip = within(screen.getByRole("group", { name: "Usage" })).getByRole("status");
    expect(usageTooltip).toHaveTextContent("CLI");
    expect(usageTooltip).toHaveTextContent("40");

    rerender(<A2uiSurfaceRenderer surface={{
      ...initial,
      data: {
        total: "new metric",
        traffic: [{ day: "Tue", requests: 25, errors: 2 }],
        builds: [{ day: "Tue", passed: 7 }],
        usage: [{ name: "Desktop", value: 100 }],
      },
    }} />);

    expect(screen.queryByText("old metric")).not.toBeInTheDocument();
    expect(screen.getByText("new metric")).toBeInTheDocument();
    expect(screen.queryByText("API")).not.toBeInTheDocument();
    expect(screen.getAllByText("Desktop").length).toBeGreaterThan(0);
    const updatedTrafficApplication = within(screen.getByRole("group", { name: "Traffic" })).getByRole("application");
    fireEvent.focus(updatedTrafficApplication);
    fireEvent.keyDown(updatedTrafficApplication, { key: "ArrowRight" });
    const updatedTooltip = within(screen.getByRole("group", { name: "Traffic" })).getByRole("status");
    expect(updatedTooltip).toHaveTextContent("Tue");
    expect(updatedTooltip).toHaveTextContent("25");
    const updatedBuildsApplication = within(screen.getByRole("group", { name: "Builds" })).getByRole("application");
    fireEvent.focus(updatedBuildsApplication);
    fireEvent.keyDown(updatedBuildsApplication, { key: "ArrowRight" });
    const updatedBuildsTooltip = within(screen.getByRole("group", { name: "Builds" })).getByRole("status");
    expect(updatedBuildsTooltip).toHaveTextContent("Tue");
    expect(updatedBuildsTooltip).toHaveTextContent("7");
    const updatedUsageApplication = within(screen.getByRole("group", { name: "Usage" })).getByRole("application");
    fireEvent.focus(updatedUsageApplication);
    fireEvent.keyDown(updatedUsageApplication, { key: "ArrowRight" });
    const updatedUsageTooltip = within(screen.getByRole("group", { name: "Usage" })).getByRole("status");
    expect(updatedUsageTooltip).toHaveTextContent("Desktop");
    expect(updatedUsageTooltip).toHaveTextContent("100");
    expect(updatedUsageTooltip).not.toHaveTextContent("60");
    expect(updatedUsageTooltip).not.toHaveTextContent("40");
  });

  it("isolates empty malformed and non-finite chart data", () => {
    render(<A2uiSurfaceRenderer surface={surface([
      { id: "line", type: "LineChart", props: { title: "Bad line", data_path: "/line", x_key: "name", series: ["value"] } },
      { id: "bar", type: "BarChart", props: { title: "Empty bar", data_path: "/bar", x_key: "name", series: ["value"] } },
      { id: "pie", type: "PieChart", props: { title: "Bad pie", data_path: "/pie", name_key: "name", value_key: "value" } },
      { id: "sibling", type: "Callout", props: { title: "Sibling", message_path: "/visible", tone: "info" } },
    ], {
      line: [{ name: "Mon", value: "not-a-number" }],
      bar: [],
      pie: [{ name: "API", value: Number.POSITIVE_INFINITY }],
      visible: "Charts did not hide me",
    })} />);

    expect(screen.getAllByText("Nothing to display.")).toHaveLength(3);
    expect(screen.getByText("Charts did not hide me")).toBeInTheDocument();
  });
});
