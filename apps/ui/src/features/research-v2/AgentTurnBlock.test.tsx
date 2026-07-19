import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { AgentTurnBlock } from "./AgentTurnBlock";
import type { AgentTurn } from "./types";

function makeRoot(overrides: Partial<AgentTurn> = {}): AgentTurn {
  return {
    id: "exec-root",
    agentId: "root",
    parentExecutionId: null,
    startedAt: 1000,
    completedAt: 2000,
    status: "completed",
    wardId: "w1",
    request: null,
    timeline: [],
    tokenCount: 0,
    respond: "Final answer.",
    respondStreaming: "",
    thinkingExpanded: false,
    errorMessage: null,
    ...overrides,
  };
}

function makeChild(overrides: Partial<AgentTurn> = {}): AgentTurn {
  return {
    ...makeRoot({ id: "exec-child-1", agentId: "planner-agent", parentExecutionId: "exec-root" }),
    ...overrides,
  };
}

describe("<AgentTurnBlock> (root)", () => {
  it("renders the final respond markdown and a copy button", () => {
    render(<AgentTurnBlock turn={makeRoot()} />);
    expect(screen.getByText("Final answer.")).toBeTruthy();
    expect(screen.getByRole("button", { name: /copy response/i })).toBeTruthy();
  });

  it("does NOT render a thinking chevron or tool timeline on root", () => {
    render(<AgentTurnBlock turn={makeRoot()} />);
    expect(screen.queryByTestId(/thinking-chevron/)).toBeNull();
    expect(screen.queryByTestId(/timeline/)).toBeNull();
  });

  it("uses only the live ticker for a running root tool call", () => {
    render(
      <AgentTurnBlock
        turn={makeRoot({
          status: "running",
          completedAt: null,
          respond: null,
          timeline: [{
            id: "root-tool",
            at: 1,
            kind: "tool_call",
            text: "read_file",
            toolName: "read_file",
          }],
        })}
      />,
    );

    expect(screen.getByText("Running read_file")).toBeTruthy();
    expect(screen.queryByTestId("turn-tool-activity")).toBeNull();
  });

  it("renders nested subagent cards when childTurns are provided", () => {
    const root = makeRoot();
    const child = makeChild({
      id: "exec-c1",
      request: "Make a plan.",
      respond: "Plan done.",
    });
    render(
      <AgentTurnBlock
        turn={root}
        childTurns={[child]}
        allTurns={[root, child]}
      />,
    );
    // Card header is always visible.
    expect(screen.getByText("planner-agent")).toBeTruthy();
    // Completed card is collapsed by default — Request/Response hidden.
    expect(screen.queryByText("Request")).toBeNull();
    // Expand via the header toggle.
    fireEvent.click(screen.getByRole("button", { name: /expand subagent/i }));
    expect(screen.getByText("Request")).toBeTruthy();
    expect(screen.getByText("Response")).toBeTruthy();
    expect(screen.getByText("Make a plan.")).toBeTruthy();
    expect(screen.getByText("Plan done.")).toBeTruthy();
  });

  it("subagent card stays expanded by default while running", () => {
    const root = makeRoot();
    const child = makeChild({
      id: "exec-running",
      request: "Work on it.",
      status: "running",
      respond: null,
      respondStreaming: "",
      completedAt: null,
    });
    render(
      <AgentTurnBlock
        turn={root}
        childTurns={[child]}
        allTurns={[root, child]}
      />,
    );
    // No click required — running cards default to expanded.
    expect(screen.getByText("Request")).toBeTruthy();
    expect(screen.getByText("Work on it.")).toBeTruthy();
  });

  it("subagent running → Response body shows 'waiting…' instead of content", () => {
    const root = makeRoot();
    const child = makeChild({
      id: "exec-c1",
      request: "Pending task.",
      status: "running",
      respond: null,
      respondStreaming: "",
      completedAt: null,
    });
    render(
      <AgentTurnBlock
        turn={root}
        childTurns={[child]}
        allTurns={[root, child]}
      />,
    );
    expect(screen.getByText("Pending task.")).toBeTruthy();
    // Both the header live-ticker and the response placeholder show
    // "waiting…" when a running subagent has no timeline entries yet.
    expect(screen.getAllByText(/waiting/i).length).toBeGreaterThanOrEqual(1);
  });

  it("does not render a copy-response button when respond/streaming are empty", () => {
    const root = makeRoot({ respond: null, respondStreaming: "" });
    render(<AgentTurnBlock turn={root} />);
    expect(screen.queryByRole("button", { name: /copy response/i })).toBeNull();
  });

  it("accepts onToggleThinking as an optional no-op for API stability", () => {
    const spy = vi.fn();
    render(<AgentTurnBlock turn={makeRoot()} onToggleThinking={spy} />);
    expect(spy).not.toHaveBeenCalled();
  });

  it("root turn with status=error + errorMessage renders the error banner (R16)", () => {
    render(
      <AgentTurnBlock
        turn={makeRoot({
          status: "error",
          errorMessage: "Turn ended with no output (provider error)",
          respond: null,
          respondStreaming: "",
        })}
      />,
    );
    const banner = screen.getByTestId("turn-error-banner");
    expect(banner).toBeTruthy();
    expect(banner.textContent).toContain("Turn ended with no output");
  });

  it("LiveTicker renders the latest timeline entry while running (R14j)", () => {
    const root = makeRoot();
    const child = makeChild({
      id: "exec-running",
      request: "Work.",
      status: "running",
      respond: null,
      respondStreaming: "",
      completedAt: null,
      timeline: [
        { id: "e1", at: 1, kind: "thinking", text: "preparing data" },
        { id: "e2", at: 2, kind: "tool_call", text: "read_file", toolName: "read_file", toolArgsPreview: "plan.md" },
      ],
    });
    render(
      <AgentTurnBlock
        turn={root}
        childTurns={[child]}
        allTurns={[root, child]}
      />,
    );
    // The ticker shows the latest entry — tool_call for read_file.
    expect(screen.getByText(/read_file/)).toBeTruthy();
  });

  it("LiveTicker hides once the turn leaves 'running' (R14j)", () => {
    const root = makeRoot();
    const child = makeChild({
      id: "exec-done",
      request: "Task.",
      status: "completed",
      respond: "done",
      completedAt: 3000,
      timeline: [{ id: "e1", at: 1, kind: "thinking", text: "thought something" }],
    });
    render(
      <AgentTurnBlock
        turn={root}
        childTurns={[child]}
        allTurns={[root, child]}
      />,
    );
    // Ticker is hidden on completion (collapsed card header shows only name + meta).
    expect(screen.queryByText(/thought something/)).toBeNull();
  });

  it("subagent card with status=error renders ErrorBanner when expanded (R16)", () => {
    const root = makeRoot();
    const child = makeChild({
      id: "exec-err",
      request: "Do the thing.",
      status: "error",
      errorMessage: "Subagent crashed mid-task",
      respond: null,
      respondStreaming: "",
      completedAt: 3000,
    });
    render(
      <AgentTurnBlock
        turn={root}
        childTurns={[child]}
        allTurns={[root, child]}
      />,
    );
    // Error cards are auto-collapsed on completion — expand to see banner.
    fireEvent.click(screen.getByRole("button", { name: /expand subagent/i }));
    expect(screen.getByTestId("turn-error-banner")).toBeTruthy();
    expect(screen.getByText(/Subagent crashed mid-task/)).toBeTruthy();
  });
});
