import { describe, it, expect } from "vitest";
import { reduceQuickChat } from "./reducer";
import { EMPTY_QUICK_CHAT_STATE } from "./types";

describe("reduceQuickChat", () => {
  it("appends user message and flips status to running", () => {
    const s = reduceQuickChat(EMPTY_QUICK_CHAT_STATE, {
      type: "APPEND_USER",
      message: { id: "u1", role: "user", content: "hello", timestamp: 1 },
    });
    expect(s.messages).toHaveLength(1);
    expect(s.status).toBe("running");
  });

  it("SESSION_BOUND sets sessionId", () => {
    const s = reduceQuickChat(EMPTY_QUICK_CHAT_STATE, {
      type: "SESSION_BOUND", sessionId: "sess-x",
    });
    expect(s.sessionId).toBe("sess-x");
  });

  it("TOKEN appends to the latest assistant message or creates one", () => {
    const s1 = reduceQuickChat(EMPTY_QUICK_CHAT_STATE, { type: "TOKEN", text: "Hi " });
    expect(s1.messages).toHaveLength(1);
    expect(s1.messages[0].role).toBe("assistant");
    expect(s1.messages[0].content).toBe("Hi ");
    const s2 = reduceQuickChat(s1, { type: "TOKEN", text: "there" });
    expect(s2.messages[0].content).toBe("Hi there");
  });

  it("RESPOND overrides streaming content with final text", () => {
    let s = reduceQuickChat(EMPTY_QUICK_CHAT_STATE, { type: "TOKEN", text: "partial" });
    s = reduceQuickChat(s, { type: "RESPOND", text: "final answer" });
    expect(s.messages[0].content).toBe("final answer");
    expect(s.messages[0].streaming).toBe(false);
  });

  it("TURN_COMPLETE sets status back to idle", () => {
    const s = reduceQuickChat(
      { ...EMPTY_QUICK_CHAT_STATE, status: "running" },
      { type: "TURN_COMPLETE" }
    );
    expect(s.status).toBe("idle");
  });

  it("AGENT_COMPLETED recovers a missed root turn_complete result and closes the turn", () => {
    const s = reduceQuickChat(
      { ...EMPTY_QUICK_CHAT_STATE, status: "running" },
      { type: "AGENT_COMPLETED", result: "durable final answer" } as any,
    );
    expect(s).toMatchObject({ status: "idle" });
    expect(s.messages[0]).toMatchObject({
      role: "assistant",
      content: "durable final answer",
      streaming: false,
    });
  });

  it("keeps one final bubble when turn_complete and agent_completed carry the same response", () => {
    let s = reduceQuickChat(
      { ...EMPTY_QUICK_CHAT_STATE, status: "running" },
      { type: "RESPOND", text: "The air quality is moderate." },
    );
    s = reduceQuickChat(s, { type: "TURN_COMPLETE" });
    s = reduceQuickChat(s, {
      type: "AGENT_COMPLETED",
      result: "The air quality is moderate.",
    });

    expect(s.status).toBe("idle");
    expect(s.messages).toHaveLength(1);
    expect(s.messages[0]).toMatchObject({
      role: "assistant",
      content: "The air quality is moderate.",
      streaming: false,
    });
  });

  it("ADD_CHIP attaches chip to latest assistant message", () => {
    let s = reduceQuickChat(EMPTY_QUICK_CHAT_STATE, { type: "TOKEN", text: "foo" });
    s = reduceQuickChat(s, {
      type: "ADD_CHIP",
      chip: { id: "c1", kind: "recall", label: "recalled 2" },
    });
    expect(s.messages[0].chips).toHaveLength(1);
  });

  it("WARD_CHANGED updates active ward", () => {
    const s = reduceQuickChat(EMPTY_QUICK_CHAT_STATE, {
      type: "WARD_CHANGED", wardName: "stock-analysis",
    });
    expect(s.activeWardName).toBe("stock-analysis");
  });

  it("HYDRATE replaces state from snapshot", () => {
    const s = reduceQuickChat(EMPTY_QUICK_CHAT_STATE, {
      type: "HYDRATE",
      sessionId: "sess-1",
      conversationId: "quick-chat-1",
      messages: [{ id: "m1", role: "user", content: "hi", timestamp: 1 }],
      wardName: "default",
      artifacts: [],
    });
    expect(s.sessionId).toBe("sess-1");
    expect(s.messages).toHaveLength(1);
    expect(s.activeWardName).toBe("default");
  });

  it("SET_ARTIFACTS replaces artifact list", () => {
    const s = reduceQuickChat(EMPTY_QUICK_CHAT_STATE, {
      type: "SET_ARTIFACTS",
      artifacts: [{ id: "art-1", fileName: "scratch-today.md", fileType: "md" }],
    });
    expect(s.artifacts).toHaveLength(1);
    expect(s.artifacts[0].fileName).toBe("scratch-today.md");
  });

  it("AGENT_STARTED flips status to running", () => {
    const s = reduceQuickChat(EMPTY_QUICK_CHAT_STATE, {
      type: "AGENT_STARTED", agentId: "quick-chat",
    });
    expect(s.status).toBe("running");
  });

  it("ERROR flips status to error", () => {
    const s = reduceQuickChat(
      { ...EMPTY_QUICK_CHAT_STATE, status: "running" },
      { type: "ERROR", message: "network down" }
    );
    expect(s.status).toBe("error");
  });

});
