import { renderHook, waitFor } from "@testing-library/react";
import { createElement, StrictMode, type PropsWithChildren } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { tokenIndexFromSessionTokens, useSelectedSessionTokens } from "./useSelectedSessionTokens";
import type { MissionControlSessionTokens } from "@/services/transport/types";
import type { Transport } from "@/services/transport/interface";

const getMissionControlSessionTokens = vi.fn<Transport["getMissionControlSessionTokens"]>();

vi.mock("@/services/transport", () => ({
  getTransport: async () => ({
    getMissionControlSessionTokens,
  }),
}));

function makeTokens(overrides: Partial<MissionControlSessionTokens> = {}): MissionControlSessionTokens {
  return {
    conversation_id: "sess-1",
    root_execution_id: "exec-root-1",
    total_tokens_in: 1200,
    total_tokens_out: 300,
    executions: [
      {
        execution_id: "exec-root-1",
        agent_id: "root-agent",
        delegation_type: "root",
        tokens_in: 1000,
        tokens_out: 200,
      },
      {
        execution_id: "exec-child-1",
        agent_id: "analyst-agent",
        delegation_type: "sequential",
        tokens_in: 200,
        tokens_out: 100,
      },
    ],
    ...overrides,
  };
}

beforeEach(() => {
  getMissionControlSessionTokens.mockReset();
});

describe("tokenIndexFromSessionTokens", () => {
  it("keys aggregate totals and execution token slices by root execution id", () => {
    const index = tokenIndexFromSessionTokens(makeTokens());

    expect(index.byRootExecId.get("exec-root-1")).toEqual({
      in: 1200,
      out: 300,
      total: 1500,
    });
    expect(index.executionsByRootExecId.get("exec-root-1")).toEqual([
      { executionId: "exec-root-1", agentId: "root-agent", in: 1000, out: 200 },
      { executionId: "exec-child-1", agentId: "analyst-agent", in: 200, out: 100 },
    ]);
  });
});

describe("useSelectedSessionTokens", () => {
  it("does not load tokens until a session is selected", async () => {
    const { result } = renderHook(() => useSelectedSessionTokens(null));

    await waitFor(() => expect(result.current.byRootExecId.size).toBe(0));
    expect(getMissionControlSessionTokens).not.toHaveBeenCalled();
  });

  it("loads selected-session execution token slices", async () => {
    getMissionControlSessionTokens.mockResolvedValue({ success: true, data: makeTokens() });

    const { result } = renderHook(() => useSelectedSessionTokens("sess-1"));

    await waitFor(() => {
      expect(result.current.executionsByRootExecId.get("exec-root-1")).toHaveLength(2);
    });
    expect(getMissionControlSessionTokens).toHaveBeenCalledWith("sess-1");
  });

  it("coalesces StrictMode mount into one selected-token request", async () => {
    getMissionControlSessionTokens.mockResolvedValue({ success: true, data: makeTokens() });
    const wrapper = ({ children }: PropsWithChildren) => createElement(StrictMode, null, children);

    const { result } = renderHook(() => useSelectedSessionTokens("sess-1"), { wrapper });

    await waitFor(() => {
      expect(result.current.byRootExecId.get("exec-root-1")?.total).toBe(1500);
    });
    expect(getMissionControlSessionTokens).toHaveBeenCalledTimes(1);
  });

  it("reloads the selected running session after a Mission Control refresh", async () => {
    getMissionControlSessionTokens.mockResolvedValue({ success: true, data: makeTokens() });
    const { result, rerender } = renderHook(
      ({ refreshGeneration }) => useSelectedSessionTokens("sess-1", refreshGeneration),
      { initialProps: { refreshGeneration: 1 } },
    );

    await waitFor(() => expect(result.current.byRootExecId.size).toBe(1));
    rerender({ refreshGeneration: 2 });

    await waitFor(() => expect(getMissionControlSessionTokens).toHaveBeenCalledTimes(2));
  });
});
