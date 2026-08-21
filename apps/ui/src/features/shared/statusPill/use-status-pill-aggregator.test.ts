import { describe, it, expect } from "vitest";
import { reducePillState } from "./use-status-pill-aggregator";
import { EMPTY_PILL, NARRATION_MAX } from "./types";

describe("reducePillState", () => {
  it("starts hidden", () => {
    expect(reducePillState(EMPTY_PILL, { kind: "idle" })).toEqual(EMPTY_PILL);
  });

  it("shows 'Thinking…' narration on AgentStarted", () => {
    const s = reducePillState(EMPTY_PILL, { kind: "agent_started", agent_id: "root" });
    expect(s.visible).toBe(true);
    expect(s.starting).toBe(true);
    expect(s.narration).toBe("Thinking…");
    expect(s.suffix).toBe("");
    expect(s.category).toBe("neutral");
  });

  it("flips narration to the tool phrase on ToolCall", () => {
    const s1 = reducePillState(EMPTY_PILL, { kind: "agent_started", agent_id: "root" });
    const s2 = reducePillState(s1, { kind: "tool_call", tool: "write_file", args: { path: "a.py" } });
    expect(s2.narration).toBe("Creating a.py");
    expect(s2.suffix).toBe("a.py");
    expect(s2.category).toBe("write");
    expect(s2.starting).toBe(false);
  });

  it("shows 'Responding' on respond event", () => {
    const s1 = reducePillState(EMPTY_PILL, { kind: "agent_started", agent_id: "root" });
    const s2 = reducePillState(s1, { kind: "respond" });
    expect(s2.narration).toBe("Responding");
    expect(s2.suffix).toBe("");
    expect(s2.category).toBe("respond");
    expect(s2.starting).toBe(false);
    expect(s2.swapCounter).toBeGreaterThan(s1.swapCounter);
  });

  it("hides on AgentCompleted when it is the last active agent", () => {
    const s1 = reducePillState(EMPTY_PILL, { kind: "agent_started", agent_id: "root" });
    const s2 = reducePillState(s1, { kind: "agent_completed", agent_id: "root", is_final: true });
    expect(s2.visible).toBe(false);
  });

  it("stays visible on AgentCompleted with continuation pending", () => {
    const s1 = reducePillState(EMPTY_PILL, { kind: "agent_started", agent_id: "root" });
    const s2 = reducePillState(s1, { kind: "agent_completed", agent_id: "root", is_final: false });
    expect(s2.visible).toBe(true);
  });

  it("resets via reset event", () => {
    const s1 = reducePillState(EMPTY_PILL, { kind: "agent_started", agent_id: "root" });
    const s2 = reducePillState(s1, { kind: "reset" });
    expect(s2).toEqual(EMPTY_PILL);
  });

  it("LLM error event → narration 'LLM error', suffix=message, category 'error'", () => {
    const s = reducePillState(EMPTY_PILL, {
      kind: "error",
      message: "rate limited",
      source: "llm",
    });
    expect(s.visible).toBe(true);
    expect(s.starting).toBe(false);
    expect(s.narration).toBe("LLM error");
    expect(s.suffix).toBe("rate limited");
    expect(s.category).toBe("error");
  });

  it("Tool error event with tool name → narration 'Tool error: {tool}'", () => {
    const s = reducePillState(EMPTY_PILL, {
      kind: "error",
      message: "file not found",
      source: "tool",
      tool: "read_file",
    });
    expect(s.narration).toBe("Tool error: read_file");
    expect(s.suffix).toBe("file not found");
    expect(s.category).toBe("error");
  });

  it("Tool error event without tool name → narration 'Tool error'", () => {
    const s = reducePillState(EMPTY_PILL, {
      kind: "error",
      message: "timeout",
      source: "tool",
    });
    expect(s.narration).toBe("Tool error");
    expect(s.suffix).toBe("timeout");
  });

  it("Error suffix is truncated to NARRATION_MAX", () => {
    const long = "x".repeat(200);
    const s = reducePillState(EMPTY_PILL, {
      kind: "error",
      message: long,
      source: "llm",
    });
    expect(s.suffix.length).toBeLessThanOrEqual(NARRATION_MAX);
    expect(s.suffix.endsWith("…")).toBe(true);
  });

  it("a successful tool result recovers the sticky error state (retried-and-fixed)", () => {
    // Live bug (sess-a006af36): present_surface errored once, the model
    // corrected and succeeded — but successes emitted no pill event, so
    // "Tool error" stuck at the top for the rest of the session.
    const s1 = reducePillState(EMPTY_PILL, {
      kind: "error",
      message: "invalid property tone",
      source: "tool",
      tool: "present_surface",
    });
    expect(s1.category).toBe("error");
    const s2 = reducePillState(s1, { kind: "tool_ok", tool: "present_surface" });
    expect(s2.category).toBe("neutral");
    expect(s2.narration).toBe("Recovered — continuing");
  });

  it("tool_ok is invisible outside the error state (no done-flicker)", () => {
    const s1 = reducePillState(EMPTY_PILL, { kind: "agent_started", agent_id: "root" });
    const s2 = reducePillState(s1, { kind: "tool_ok", tool: "shell" });
    expect(s2).toBe(s1);
  });

  it("agent_started clears a sticky error state back to Thinking…/neutral", () => {
    const s1 = reducePillState(EMPTY_PILL, { kind: "error", message: "boom", source: "llm" });
    expect(s1.category).toBe("error");
    const s2 = reducePillState(s1, { kind: "agent_started", agent_id: "root" });
    expect(s2.category).toBe("neutral");
    expect(s2.narration).toBe("Thinking…");
    expect(s2.suffix).toBe("");
    expect(s2.starting).toBe(true);
  });

  it("error state SURVIVES a subsequent tool_call (sticky)", () => {
    const s1 = reducePillState(EMPTY_PILL, {
      kind: "error",
      message: "network down",
      source: "llm",
    });
    const s2 = reducePillState(s1, { kind: "tool_call", tool: "write_file", args: { path: "a.py" } });
    expect(s2.category).toBe("error");
    expect(s2.narration).toBe("LLM error");
    expect(s2.suffix).toBe("network down");
  });

  it("error state SURVIVES a subsequent respond (sticky)", () => {
    const s1 = reducePillState(EMPTY_PILL, { kind: "error", message: "boom", source: "llm" });
    const s2 = reducePillState(s1, { kind: "respond" });
    expect(s2.category).toBe("error");
    expect(s2.narration).toBe("LLM error");
  });

  it("error state SURVIVES a subsequent agent_completed (sticky)", () => {
    const s1 = reducePillState(EMPTY_PILL, { kind: "error", message: "boom", source: "llm" });
    const s2 = reducePillState(s1, {
      kind: "agent_completed",
      agent_id: "root",
      is_final: true,
    });
    expect(s2.category).toBe("error");
  });

  it("reset clears a sticky error state", () => {
    const s1 = reducePillState(EMPTY_PILL, { kind: "error", message: "boom", source: "llm" });
    const s2 = reducePillState(s1, { kind: "reset" });
    expect(s2).toEqual(EMPTY_PILL);
  });
});
