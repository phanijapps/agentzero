import { describe, it, expect } from "vitest";
import { mapGatewayEventToResearchAction, mapGatewayEventToPillEvent } from "./event-map";

describe("mapGatewayEventToResearchAction", () => {
  it("AgentStarted maps with execution_id → turnId", () => {
    const a = mapGatewayEventToResearchAction({
      type: "agent_started", agent_id: "root", execution_id: "exec-1", ward_id: null,
    } as any);
    expect(a).toMatchObject({
      type: "AGENT_STARTED", turnId: "exec-1", agentId: "root",
      parentExecutionId: null, wardId: null,
    });
  });

  it("WardChanged with flat ward_id (verified wire format)", () => {
    expect(mapGatewayEventToResearchAction({ type: "ward_changed", ward_id: "stock-analysis" } as any))
      .toEqual({ type: "WARD_CHANGED", wardId: "stock-analysis", wardName: "stock-analysis" });
  });

  it("WardChanged with nested ward.name (forward-compat)", () => {
    expect(mapGatewayEventToResearchAction({ type: "ward_changed", ward: { id: "w1", name: "W1" } } as any))
      .toEqual({ type: "WARD_CHANGED", wardId: "w1", wardName: "W1" });
  });

  it("WardChanged without ward_id or nested returns null", () => {
    expect(mapGatewayEventToResearchAction({ type: "ward_changed" } as any)).toBeNull();
  });

  it("Thinking maps with execution_id", () => {
    const a = mapGatewayEventToResearchAction({
      type: "thinking", execution_id: "exec-1", content: "deep thought",
    } as any);
    expect(a?.type).toBe("THINKING_DELTA");
    expect((a as any).entry.text).toBe("deep thought");
  });

  it("Thinking with empty content returns null", () => {
    expect(mapGatewayEventToResearchAction({
      type: "thinking", execution_id: "exec-1", content: "",
    } as any)).toBeNull();
  });

  it("ToolCall maps with tool_name (verified wire field)", () => {
    const a = mapGatewayEventToResearchAction({
      type: "tool_call", execution_id: "exec-1", tool_name: "write_file", args: { path: "a.py" },
    } as any);
    expect((a as any).entry.toolName).toBe("write_file");
    expect((a as any).entry.toolArgsPreview).toContain("a.py");
  });

  it("ToolCall accepts legacy `tool` field (forward-compat)", () => {
    const a = mapGatewayEventToResearchAction({
      type: "tool_call", execution_id: "exec-1", tool: "write_file", args: {},
    } as any);
    expect((a as any).entry.toolName).toBe("write_file");
  });

  it("Token maps with `delta` (verified wire field)", () => {
    expect(mapGatewayEventToResearchAction({
      type: "token", execution_id: "exec-1", delta: "abc",
    } as any)).toEqual({ type: "TOKEN", turnId: "exec-1", text: "abc" });
  });

  it("Token accepts `content` (forward-compat)", () => {
    expect(mapGatewayEventToResearchAction({
      type: "token", execution_id: "exec-1", content: "abc",
    } as any)).toEqual({ type: "TOKEN", turnId: "exec-1", text: "abc" });
  });

  it("Token with no delta and no content returns null", () => {
    expect(mapGatewayEventToResearchAction({ type: "token", execution_id: "exec-1" } as any)).toBeNull();
  });

  it("Respond maps with `message` (verified wire field)", () => {
    expect(mapGatewayEventToResearchAction({
      type: "respond", execution_id: "exec-1", message: "final",
    } as any)).toEqual({ type: "RESPOND", turnId: "exec-1", text: "final" });
  });

  it("Respond accepts `content` (forward-compat)", () => {
    expect(mapGatewayEventToResearchAction({
      type: "respond", execution_id: "exec-1", content: "fallback",
    } as any)).toEqual({ type: "RESPOND", turnId: "exec-1", text: "fallback" });
  });

  it("Respond without execution_id uses 'orphan' turnId", () => {
    expect(mapGatewayEventToResearchAction({ type: "respond", message: "orphan" } as any))
      .toEqual({ type: "RESPOND", turnId: "orphan", text: "orphan" });
  });

  it("invoke_accepted maps to SESSION_BOUND with session + conversation", () => {
    expect(mapGatewayEventToResearchAction({
      type: "invoke_accepted", session_id: "sess-x", conversation_id: "conv-x",
    } as any)).toEqual({ type: "SESSION_BOUND", sessionId: "sess-x", conversationId: "conv-x" });
  });

  it("agent_started retains the server session identity when invoke_accepted is absent", () => {
    expect(mapGatewayEventToResearchAction({
      type: "agent_started",
      execution_id: "exec-root",
      agent_id: "root",
      parent_execution_id: null,
      session_id: "sess-root",
      conversation_id: "research-client",
    } as any)).toMatchObject({
      type: "AGENT_STARTED",
      turnId: "exec-root",
      sessionId: "sess-root",
      conversationId: "research-client",
    });
  });

  it("session_initialized maps to SESSION_BOUND (forward-compat)", () => {
    expect(mapGatewayEventToResearchAction({
      type: "session_initialized", session_id: "sess-y", conversation_id: "conv-y",
    } as any)).toEqual({ type: "SESSION_BOUND", sessionId: "sess-y", conversationId: "conv-y" });
  });

  it("SessionTitleChanged maps", () => {
    expect(mapGatewayEventToResearchAction({ type: "session_title_changed", title: "New T" } as any))
      .toEqual({ type: "TITLE_CHANGED", title: "New T" });
  });

  it("IntentAnalysis start/complete/skipped map", () => {
    expect(mapGatewayEventToResearchAction({ type: "intent_analysis_started" } as any))
      .toEqual({ type: "INTENT_ANALYSIS_STARTED" });
    expect(mapGatewayEventToResearchAction({ type: "intent_analysis_complete", classification: "research" } as any))
      .toEqual({ type: "INTENT_ANALYSIS_COMPLETE", classification: "research" });
    expect(mapGatewayEventToResearchAction({ type: "intent_analysis_skipped" } as any))
      .toEqual({ type: "INTENT_ANALYSIS_SKIPPED" });
  });

  it("plan_update maps", () => {
    expect(mapGatewayEventToResearchAction({ type: "plan_update", plan_path: "/p.md" } as any))
      .toEqual({ type: "PLAN_UPDATE", planPath: "/p.md" });
  });

  it("error maps", () => {
    expect(mapGatewayEventToResearchAction({ type: "error", message: "boom" } as any))
      .toEqual({ type: "ERROR", message: "boom" });
  });

  it("turn_complete WITHOUT final_message → TURN_COMPLETE (informational)", () => {
    expect(mapGatewayEventToResearchAction({ type: "turn_complete", execution_id: "exec-1" } as any))
      .toEqual({ type: "TURN_COMPLETE", turnId: "exec-1" });
  });

  it("turn_complete WITH final_message → RESPOND (real answer rides here on the wire)", () => {
    expect(
      mapGatewayEventToResearchAction({
        type: "turn_complete",
        execution_id: "exec-1",
        final_message: "4",
      } as any),
    ).toEqual({ type: "RESPOND", turnId: "exec-1", text: "4" });
  });

  it("turn_complete WITH empty final_message falls back to TURN_COMPLETE", () => {
    expect(
      mapGatewayEventToResearchAction({
        type: "turn_complete",
        execution_id: "exec-1",
        final_message: "",
      } as any),
    ).toEqual({ type: "TURN_COMPLETE", turnId: "exec-1" });
  });

  it("agent_completed retains its final result as a response fallback", () => {
    const a = mapGatewayEventToResearchAction({
      type: "agent_completed",
      execution_id: "exec-1",
      result: "final response",
    } as any);
    expect(a).toMatchObject({
      type: "AGENT_COMPLETED",
      turnId: "exec-1",
      result: "final response",
    });
  });

  it("agent_stopped maps", () => {
    const a = mapGatewayEventToResearchAction({ type: "agent_stopped", execution_id: "exec-1" } as any);
    expect(a?.type).toBe("AGENT_STOPPED");
    expect((a as any).turnId).toBe("exec-1");
  });

  it("unknown event type returns null", () => {
    expect(mapGatewayEventToResearchAction({ type: "something_weird" } as any)).toBeNull();
  });
});

describe("mapGatewayEventToPillEvent", () => {
  it("agent_started maps", () => {
    expect(mapGatewayEventToPillEvent({ type: "agent_started", agent_id: "planner" } as any))
      .toEqual({ kind: "agent_started", agent_id: "planner" });
  });

  it("agent_completed maps with is_final=true (no field required on wire)", () => {
    expect(mapGatewayEventToPillEvent({ type: "agent_completed", agent_id: "planner" } as any))
      .toEqual({ kind: "agent_completed", agent_id: "planner", is_final: true });
  });

  it("tool_call with wire field tool_name maps", () => {
    expect(mapGatewayEventToPillEvent({ type: "tool_call", tool_name: "write_file", args: { path: "a.py" } } as any))
      .toEqual({ kind: "tool_call", tool: "write_file", args: { path: "a.py" } });
  });

  it("tool_call with legacy `tool` field (forward-compat)", () => {
    expect(mapGatewayEventToPillEvent({ type: "tool_call", tool: "write_file", args: {} } as any))
      .toEqual({ kind: "tool_call", tool: "write_file", args: {} });
  });

  it("respond maps", () => {
    expect(mapGatewayEventToPillEvent({ type: "respond", message: "done" } as any))
      .toEqual({ kind: "respond" });
  });

  it("thinking is NOT mapped — pill-narration flicker is unusable", () => {
    expect(mapGatewayEventToPillEvent({ type: "thinking", content: "…" } as any))
      .toBeNull();
  });

  it("intent_analysis_started is NOT mapped — per-turn block handles it", () => {
    expect(mapGatewayEventToPillEvent({ type: "intent_analysis_started" } as any))
      .toBeNull();
  });

  it("heartbeat / unknown return null", () => {
    expect(mapGatewayEventToPillEvent({ type: "heartbeat" } as any)).toBeNull();
  });

  it("gateway `error` maps to pill error with source=llm", () => {
    expect(mapGatewayEventToPillEvent({ type: "error", message: "rate limited" } as any))
      .toEqual({ kind: "error", message: "rate limited", source: "llm" });
  });

  it("gateway `error` with missing message falls back to 'unknown error'", () => {
    expect(mapGatewayEventToPillEvent({ type: "error" } as any))
      .toEqual({ kind: "error", message: "unknown error", source: "llm" });
  });

  it("tool_result with error field maps to pill error (source=tool)", () => {
    expect(
      mapGatewayEventToPillEvent({
        type: "tool_result",
        tool_name: "read_file",
        error: "file not found",
      } as any),
    ).toEqual({ kind: "error", message: "file not found", source: "tool", tool: "read_file" });
  });

  it("tool_result without error returns null (no pill event)", () => {
    expect(mapGatewayEventToPillEvent({ type: "tool_result", tool_name: "read_file", result: "ok" } as any))
      .toBeNull();
  });

  it("tool_result with empty-string error returns null", () => {
    expect(mapGatewayEventToPillEvent({ type: "tool_result", tool_name: "read_file", error: "" } as any))
      .toBeNull();
  });
});
