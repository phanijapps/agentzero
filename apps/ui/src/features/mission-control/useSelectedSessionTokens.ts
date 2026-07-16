import { useEffect, useRef, useState } from "react";
import { getTransport } from "@/services/transport";
import type { CurrentSessionPlan, MissionControlSessionTokens } from "@/services/transport/types";
import type { ExecutionTokenEntry, SessionTokenIndex } from "./useSessionTokens";

export type SelectedSessionTokenState = SessionTokenIndex & {
  currentPlan?: CurrentSessionPlan;
};

const EMPTY_INDEX: SelectedSessionTokenState = {
  byRootExecId: new Map(),
  executionsByRootExecId: new Map(),
};

export function useSelectedSessionTokens(
  sessionId: string | null,
  refreshGeneration?: number,
): SelectedSessionTokenState {
  const [tokenIndex, setTokenIndex] = useState<SelectedSessionTokenState>(EMPTY_INDEX);
  const loadInFlightRef = useRef<Promise<SelectedSessionTokenState> | null>(null);
  const loadingSessionRef = useRef<string | null>(null);

  useEffect(() => {
    if (!sessionId) {
      setTokenIndex(EMPTY_INDEX);
      return;
    }

    let cancelled = false;
    const loadKey = `${sessionId}:${refreshGeneration ?? "initial"}`;
    const load = async () => {
      if (loadInFlightRef.current && loadingSessionRef.current === loadKey) {
        return loadInFlightRef.current;
      }

      loadingSessionRef.current = loadKey;
      const loadPromise = (async () => {
        const transport = await getTransport();
        const result = await transport.getMissionControlSessionTokens(sessionId);
        if (result.success && result.data) {
          return selectedStateFromSessionTokens(result.data);
        }
        return EMPTY_INDEX;
      })();

      loadInFlightRef.current = loadPromise;
      const clearInFlight = () => {
        if (loadingSessionRef.current === loadKey) {
          loadingSessionRef.current = null;
          loadInFlightRef.current = null;
        }
      };
      loadPromise.then(clearInFlight, clearInFlight);

      return loadPromise;
    };

    load()
      .then((index) => {
        if (!cancelled) setTokenIndex(index);
      })
      .catch(() => {
        if (!cancelled) setTokenIndex(EMPTY_INDEX);
      });

    return () => {
      cancelled = true;
    };
  }, [sessionId, refreshGeneration]);

  return tokenIndex;
}

export function tokenIndexFromSessionTokens(tokens: MissionControlSessionTokens): SessionTokenIndex {
  const byRootExecId = new Map();
  const executionsByRootExecId = new Map<string, ExecutionTokenEntry[]>();
  const total = (tokens.total_tokens_in ?? 0) + (tokens.total_tokens_out ?? 0);

  byRootExecId.set(tokens.root_execution_id, {
    in: tokens.total_tokens_in ?? 0,
    out: tokens.total_tokens_out ?? 0,
    total,
  });
  executionsByRootExecId.set(
    tokens.root_execution_id,
    (tokens.executions ?? []).map((e) => ({
      executionId: e.execution_id,
      agentId: e.agent_id,
      in: e.tokens_in ?? 0,
      out: e.tokens_out ?? 0,
    })),
  );

  return { byRootExecId, executionsByRootExecId };
}

function selectedStateFromSessionTokens(
  tokens: MissionControlSessionTokens,
): SelectedSessionTokenState {
  return {
    ...tokenIndexFromSessionTokens(tokens),
    currentPlan: tokens.current_plan,
  };
}
