import { useCallback, useEffect, useRef, useState } from "react";
import { getTransport } from "@/services/transport";
import type {
  LogSession,
  MissionControlSessionSummary,
  SessionStatus,
  SessionStateStatus,
} from "@/services/transport/types";
import type { ExecutionTokenEntry, SessionTokenIndex } from "./useSessionTokens";

interface UseMissionControlSessionsResult {
  sessions: LogSession[];
  tokenIndex: SessionTokenIndex;
  refreshGeneration: number;
  loading: boolean;
  error: string | null;
  refetch: () => void;
}

const EMPTY_INDEX: SessionTokenIndex = {
  byRootExecId: new Map(),
  executionsByRootExecId: new Map(),
};

export function useMissionControlSessions(limit = 50): UseMissionControlSessionsResult {
  const [sessions, setSessions] = useState<LogSession[]>([]);
  const [tokenIndex, setTokenIndex] = useState<SessionTokenIndex>(EMPTY_INDEX);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [tick, setTick] = useState(0);
  const [refreshGeneration, setRefreshGeneration] = useState(0);
  const loadInFlightRef = useRef<Promise<MissionControlSessionSummary[]> | null>(null);
  const loadingKeyRef = useRef<string | null>(null);

  const refetch = useCallback(() => setTick((t) => t + 1), []);

  useEffect(() => {
    let cancelled = false;
    const loadKey = `${limit}:${tick}`;

    const load = async () => {
      if (loadInFlightRef.current && loadingKeyRef.current === loadKey) {
        return loadInFlightRef.current;
      }

      loadingKeyRef.current = loadKey;
      const loadPromise = (async () => {
        const transport = await getTransport();
        const result = await transport.listMissionControlSessions({ limit });
        if (result.success && result.data) {
          return result.data;
        }
        throw new Error(result.error || "Failed to load Mission Control sessions");
      })();

      loadInFlightRef.current = loadPromise;
      const clearInFlight = () => {
        if (loadingKeyRef.current === loadKey) {
          loadInFlightRef.current = null;
          loadingKeyRef.current = null;
        }
      };
      loadPromise.then(clearInFlight, clearInFlight);
      return loadPromise;
    };

    setLoading(true);
    setError(null);
    load()
      .then((summaries) => {
        if (cancelled) return;
        setSessions(summariesToLogSessions(summaries));
        setTokenIndex(buildTokenIndexFromSummaries(summaries));
        setRefreshGeneration((generation) => generation + 1);
      })
      .catch((err) => {
        if (!cancelled) {
          setError(err instanceof Error ? err.message : "Unknown error");
        }
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });

    return () => {
      cancelled = true;
    };
  }, [limit, tick]);

  return { sessions, tokenIndex, refreshGeneration, loading, error, refetch };
}

export function summariesToLogSessions(summaries: MissionControlSessionSummary[]): LogSession[] {
  return summaries.map((s) => ({
    session_id: s.root_execution_id,
    conversation_id: s.conversation_id,
    agent_id: s.root_agent_id,
    agent_name: s.root_agent_id,
    title: s.title,
    started_at: s.started_at ?? s.created_at,
    ended_at: s.completed_at,
    status: normalizeSummaryStatus(s.status),
    token_count: (s.total_tokens_in ?? 0) + (s.total_tokens_out ?? 0),
    tool_call_count: 0,
    error_count: s.status === "crashed" ? 1 : 0,
    child_session_ids: [],
    subagent_count: s.subagent_count,
    mode: s.mode,
  }));
}

export function buildTokenIndexFromSummaries(
  summaries: MissionControlSessionSummary[],
): SessionTokenIndex {
  const byRootExecId = new Map();
  const executionsByRootExecId = new Map<string, ExecutionTokenEntry[]>();

  for (const s of summaries) {
    const tokensIn = s.total_tokens_in ?? 0;
    const tokensOut = s.total_tokens_out ?? 0;
    byRootExecId.set(s.root_execution_id, {
      in: tokensIn,
      out: tokensOut,
      total: tokensIn + tokensOut,
      status: s.status,
    });
    executionsByRootExecId.set(
      s.root_execution_id,
      [],
    );
  }

  return { byRootExecId, executionsByRootExecId };
}

function normalizeSummaryStatus(status: SessionStateStatus): SessionStatus {
  switch (status) {
    case "completed":
      return "completed";
    case "crashed":
      return "error";
    case "queued":
    case "paused":
    case "running":
    default:
      return "running";
  }
}
