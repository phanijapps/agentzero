import { describe, it, expect } from "vitest";
import {
  mapGatewayEventToQuickChatAction,
  mapGatewayEventToPillEvent,
  mapTurnCompleteFinalMessage,
} from "./event-map";

describe("mapGatewayEventToQuickChatAction", () => {
  it("maps Token (wire field: delta) to TOKEN action", () => {
    expect(mapGatewayEventToQuickChatAction({ type: "token", delta: "hi" } as any))
      .toEqual({ type: "TOKEN", text: "hi" });
  });
  it("maps Token with content (forward-compat) to TOKEN action", () => {
    expect(mapGatewayEventToQuickChatAction({ type: "token", content: "hi" } as any))
      .toEqual({ type: "TOKEN", text: "hi" });
  });
  it("maps Respond (wire field: message) to RESPOND action", () => {
    expect(mapGatewayEventToQuickChatAction({ type: "respond", message: "done" } as any))
      .toEqual({ type: "RESPOND", text: "done" });
  });
  it("maps Respond with content (forward-compat) to RESPOND action", () => {
    expect(mapGatewayEventToQuickChatAction({ type: "respond", content: "done" } as any))
      .toEqual({ type: "RESPOND", text: "done" });
  });
  it("maps WardChanged with flat ward_id (current wire format)", () => {
    expect(mapGatewayEventToQuickChatAction({ type: "ward_changed", ward_id: "stock-analysis" } as any))
      .toEqual({ type: "WARD_CHANGED", wardName: "stock-analysis" });
  });
  it("maps WardChanged with nested ward.name (forward-compat)", () => {
    expect(mapGatewayEventToQuickChatAction({ type: "ward_changed", ward: { name: "x" } } as any))
      .toEqual({ type: "WARD_CHANGED", wardName: "x" });
  });
  it("returns null for WardChanged without id or name", () => {
    expect(mapGatewayEventToQuickChatAction({ type: "ward_changed" } as any)).toBeNull();
    expect(mapGatewayEventToQuickChatAction({ type: "ward_changed", ward_id: "" } as any)).toBeNull();
  });
  it("maps invoke_accepted → SESSION_BOUND (current wire format)", () => {
    expect(mapGatewayEventToQuickChatAction({ type: "invoke_accepted", session_id: "sess-1" } as any))
      .toEqual({ type: "SESSION_BOUND", sessionId: "sess-1" });
  });
  it("maps session_initialized → SESSION_BOUND (forward-compat)", () => {
    expect(mapGatewayEventToQuickChatAction({ type: "session_initialized", session_id: "sess-1" } as any))
      .toEqual({ type: "SESSION_BOUND", sessionId: "sess-1" });
  });
  it("maps tool_call delegate_to_agent to ADD_CHIP (wire field: tool_name)", () => {
    const a = mapGatewayEventToQuickChatAction({
      type: "tool_call", tool_name: "delegate_to_agent", args: { agent_id: "writer-agent" },
    } as any);
    expect(a?.type).toBe("ADD_CHIP");
    expect((a as any).chip.kind).toBe("delegate");
  });
  it("maps tool_call with legacy `tool` field (forward-compat)", () => {
    const a = mapGatewayEventToQuickChatAction({
      type: "tool_call", tool: "delegate_to_agent", args: { agent_id: "x" },
    } as any);
    expect(a?.type).toBe("ADD_CHIP");
  });
  it("returns null for unmapped events", () => {
    expect(mapGatewayEventToQuickChatAction({ type: "iterations_extended" } as any)).toBeNull();
  });

  it("maps agent_started to AGENT_STARTED", () => {
    expect(mapGatewayEventToQuickChatAction({ type: "agent_started", agent_id: "quick-chat" } as any))
      .toEqual({ type: "AGENT_STARTED", agentId: "quick-chat" });
  });

  it("maps turn_complete to TURN_COMPLETE", () => {
    expect(mapGatewayEventToQuickChatAction({ type: "turn_complete" } as any))
      .toEqual({ type: "TURN_COMPLETE" });
  });

  it("maps the root agent_completed result as the terminal response fallback", () => {
    expect(mapGatewayEventToQuickChatAction({
      type: "agent_completed",
      agent_id: "root",
      result: "durable final answer",
    } as any)).toEqual({
      type: "AGENT_COMPLETED",
      result: "durable final answer",
    });
  });

  it("does not apply a delegated child completion to the root chat bubble", () => {
    expect(mapGatewayEventToQuickChatAction({
      type: "agent_completed",
      agent_id: "ward:financial-analysis",
      result: "child result",
    } as any)).toBeNull();
  });

  it("maps error with message to ERROR", () => {
    expect(mapGatewayEventToQuickChatAction({ type: "error", message: "network down" } as any))
      .toEqual({ type: "ERROR", message: "network down" });
  });

  it("maps error without message to ERROR with default", () => {
    expect(mapGatewayEventToQuickChatAction({ type: "error" } as any))
      .toEqual({ type: "ERROR", message: "error" });
  });

  it("maps tool_call load_skill to skill chip", () => {
    const a = mapGatewayEventToQuickChatAction({
      type: "tool_call", tool_name: "load_skill", args: { skill: "web-read" },
    } as any);
    expect(a?.type).toBe("ADD_CHIP");
    expect((a as any).chip.kind).toBe("skill");
    expect((a as any).chip.label).toBe("loaded web-read");
  });

  it("maps tool_call memory.recall to recall chip", () => {
    const a = mapGatewayEventToQuickChatAction({
      type: "tool_call", tool_name: "memory", args: { action: "recall" },
    } as any);
    expect(a?.type).toBe("ADD_CHIP");
    expect((a as any).chip.kind).toBe("recall");
  });

  it("returns null for tool_call memory with non-read action", () => {
    expect(mapGatewayEventToQuickChatAction({
      type: "tool_call", tool_name: "memory", args: { action: "save_fact" },
    } as any)).toBeNull();
  });
});

describe("mapGatewayEventToPillEvent", () => {
  it("maps agent_started", () => {
    expect(mapGatewayEventToPillEvent({ type: "agent_started", agent_id: "quick-chat" } as any))
      .toEqual({ kind: "agent_started", agent_id: "quick-chat" });
  });
  it("ignores thinking events (too noisy for pill narration)", () => {
    expect(mapGatewayEventToPillEvent({ type: "thinking", content: "…" } as any))
      .toBeNull();
  });
  it("maps tool_call with wire field tool_name", () => {
    expect(mapGatewayEventToPillEvent({ type: "tool_call", tool_name: "write_file", args: { path: "a.py" } } as any))
      .toEqual({ kind: "tool_call", tool: "write_file", args: { path: "a.py" } });
  });
  it("maps tool_call with legacy `tool` field (forward-compat)", () => {
    expect(mapGatewayEventToPillEvent({ type: "tool_call", tool: "write_file", args: {} } as any))
      .toEqual({ kind: "tool_call", tool: "write_file", args: {} });
  });
  it("maps agent_completed with is_final always true (single-agent chat)", () => {
    expect(mapGatewayEventToPillEvent({ type: "agent_completed", agent_id: "quick-chat" } as any))
      .toEqual({ kind: "agent_completed", agent_id: "quick-chat", is_final: true });
  });
  it("maps respond to respond pill event", () => {
    expect(mapGatewayEventToPillEvent({ type: "respond", message: "done" } as any))
      .toEqual({ kind: "respond" });
  });
  it("returns null for unmapped pill events", () => {
    expect(mapGatewayEventToPillEvent({ type: "heartbeat" } as any)).toBeNull();
  });
});

describe("mapTurnCompleteFinalMessage", () => {
  it("returns a RESPOND action when turn_complete carries final_message (respond-tool path)", () => {
    expect(
      mapTurnCompleteFinalMessage({
        type: "turn_complete",
        final_message: "Here's your life snapshot…",
      } as any),
    ).toEqual({ type: "RESPOND", text: "Here's your life snapshot…" });
  });

  it("returns null when turn_complete has no final_message (token-streaming path)", () => {
    expect(mapTurnCompleteFinalMessage({ type: "turn_complete" } as any)).toBeNull();
  });

  it("returns null when final_message is an empty string (avoid clobbering streamed content)", () => {
    expect(
      mapTurnCompleteFinalMessage({ type: "turn_complete", final_message: "" } as any),
    ).toBeNull();
  });

  it("returns null for non-turn_complete events", () => {
    expect(
      mapTurnCompleteFinalMessage({ type: "respond", final_message: "x" } as any),
    ).toBeNull();
    expect(
      mapTurnCompleteFinalMessage({ type: "token", final_message: "x" } as any),
    ).toBeNull();
  });

  it("returns null when final_message is not a string", () => {
    expect(
      mapTurnCompleteFinalMessage({ type: "turn_complete", final_message: 42 } as any),
    ).toBeNull();
    expect(
      mapTurnCompleteFinalMessage({ type: "turn_complete", final_message: null } as any),
    ).toBeNull();
  });
});
