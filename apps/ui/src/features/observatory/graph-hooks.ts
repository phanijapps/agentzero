// ============================================================================
// OBSERVATORY DATA HOOKS
// Data fetching hooks for the Observatory knowledge graph visualization.
// ============================================================================

import { useState, useEffect, useCallback } from "react";
import { getTransport } from "@/services/transport";
import type {
  GraphEntity,
  GraphRelationship,
  GraphEntityListResponse,
  GraphNeighborResponse,
} from "@/services/transport/types";

// ============================================================================
// TYPES
// ============================================================================

/** Aggregate statistics for the Observatory health bar. */
export interface GraphStats {
  entities: number;
  relationships: number;
  facts: number;
  episodes: number;
  governance?: {
    supported: boolean;
    ontologyIds: string[];
    taxonomySchemeIds: string[];
    validationMode: string;
    allowUnclassified: string;
    skosExpansion: {
      maxDepth: number;
      maxFanOut: number;
      maxCandidates: number;
    };
    bootstrap?: {
      ontologyId: string;
      taxonomySchemeId: string;
      classCount: number;
      propertyCount: number;
      conceptCount: number;
      relationCount: number;
    };
    findingCount: number;
    findingCodes: string[];
  } | null;
  distillation: {
    success_count: number;
    failed_count: number;
    skipped_count: number;
    permanently_failed_count: number;
    total_facts: number;
    total_entities: number;
    total_relationships: number;
    total_episodes: number;
  } | null;
}

/** Distillation status response from /api/distillation/status. */
export interface DistillationStatus {
  success_count: number;
  failed_count: number;
  skipped_count: number;
  permanently_failed_count: number;
  total_facts: number;
  total_entities: number;
  total_relationships: number;
  total_episodes: number;
}

/** Combined graph data (entities + relationships) for visualization. */
export interface GraphData {
  entities: GraphEntity[];
  relationships: GraphRelationship[];
  loading: boolean;
  error: string | null;
  refetch: () => void;
}

// ============================================================================
// INTERNAL HELPERS
// ============================================================================

/**
 * Resolve the gateway base URL from the transport layer.
 *
 * Returns "" (empty string) for same-origin browser requests — the default
 * transport config in a browser sets `httpUrl: ""` so `fetch("${base}/api/x")`
 * becomes `fetch("/api/x")`, resolved against `window.location.origin`. That
 * keeps the gateway port out of the wire URL so mobile clients hitting the
 * daemon on its LAN address don't run into a port mismatch.
 *
 * The localhost fallback is reserved for non-browser callers (SSR, unit
 * tests where window is undefined).
 */
async function getBaseUrl(): Promise<string> {
  try {
    const transport = await getTransport();
    // The HttpTransport exposes the base URL via its config.
    // We cast because the internal `config` property isn't part of the
    // public Transport interface.
    const cfg = (transport as unknown as { config?: { httpUrl: string } }).config;
    // Empty string is a valid same-origin signal — keep it as-is.
    if (cfg && typeof cfg.httpUrl === "string") return cfg.httpUrl;
  } catch {
    // swallow
  }
  return typeof window === "undefined" ? "http://localhost:18791" : "";
}

async function fetchJson<T>(path: string): Promise<T> {
  const base = await getBaseUrl();
  const controller = new AbortController();
  const timeoutId = setTimeout(() => controller.abort(), 10_000);
  try {
    const res = await fetch(`${base}${path}`, {
      method: "GET",
      headers: { "Content-Type": "application/json" },
      signal: controller.signal,
    });
    clearTimeout(timeoutId);
    if (!res.ok) {
      const text = await res.text().catch(() => res.statusText);
      throw new Error(text || `HTTP ${res.status}`);
    }
    return (await res.json()) as T;
  } catch (err) {
    clearTimeout(timeoutId);
    throw err;
  }
}

async function postJson<T>(path: string): Promise<T> {
  const base = await getBaseUrl();
  // Distillation can be slow — use a generous 120 s timeout per session.
  const controller = new AbortController();
  const timeoutId = setTimeout(() => controller.abort(), 120_000);
  try {
    const res = await fetch(`${base}${path}`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      signal: controller.signal,
    });
    clearTimeout(timeoutId);
    if (!res.ok) {
      const text = await res.text().catch(() => res.statusText);
      throw new Error(text || `HTTP ${res.status}`);
    }
    return (await res.json()) as T;
  } catch (err) {
    clearTimeout(timeoutId);
    throw err;
  }
}

// ============================================================================
// HOOKS
// ============================================================================

/**
 * Fetch entities + relationships for a single agent (or cross-agent when
 * agentId is omitted). Returns combined graph data for the D3 canvas.
 */
export function useGraphData(agentId?: string): GraphData {
  const [entities, setEntities] = useState<GraphEntity[]>([]);
  const [relationships, setRelationships] = useState<GraphRelationship[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [tick, setTick] = useState(0);

  const refetch = useCallback(() => setTick((t) => t + 1), []);

  const fetchPerAgentData = async (id: string): Promise<{ entities: GraphEntity[]; relationships: GraphRelationship[] }> => {
    const transport = await getTransport();
    const [entRes, relRes] = await Promise.all([
      transport.getGraphEntities(id, { limit: 200 }),
      transport.getGraphRelationships(id, { limit: 500 }),
    ]);
    if (!entRes.success || !entRes.data) throw new Error(entRes.error || "Failed to fetch entities");
    if (!relRes.success || !relRes.data) throw new Error(relRes.error || "Failed to fetch relationships");
    return { entities: entRes.data.entities, relationships: relRes.data.relationships };
  };

  useEffect(() => {
    let cancelled = false;

    const load = async () => {
      setLoading(true);
      setError(null);
      try {
        if (agentId) {
          const data = await fetchPerAgentData(agentId);
          if (cancelled) return;
          setEntities(data.entities);
          setRelationships(data.relationships);
        } else {
          // Cross-agent: hit the /api/graph/all/* endpoints
          const [entData, relData] = await Promise.all([
            fetchJson<GraphEntityListResponse>("/api/graph/all/entities?limit=200"),
            fetchJson<{ relationships: GraphRelationship[]; total: number }>(
              "/api/graph/all/relationships?limit=500"
            ),
          ]);
          if (cancelled) return;
          setEntities(entData.entities);
          setRelationships(relData.relationships);
        }
      } catch (err) {
        if (!cancelled) {
          setError(err instanceof Error ? err.message : String(err));
        }
      } finally {
        if (!cancelled) setLoading(false);
      }
    };

    load();
    return () => { cancelled = true; };
  }, [agentId, tick]);

  return { entities, relationships, loading, error, refetch };
}

/**
 * Fetch aggregate graph statistics for the Observatory health bar.
 * Hits GET /api/graph/stats which returns cross-agent totals.
 */
export function useGraphStats() {
  const [stats, setStats] = useState<GraphStats | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;

    const load = async () => {
      setLoading(true);
      setError(null);
      try {
        const data = await fetchJson<GraphStats>("/api/graph/stats");
        if (!cancelled) setStats(data);
      } catch (err) {
        if (!cancelled) setError(err instanceof Error ? err.message : String(err));
      } finally {
        if (!cancelled) setLoading(false);
      }
    };

    load();
    return () => { cancelled = true; };
  }, []);

  return { stats, loading, error };
}

/**
 * Fetch distillation pipeline status from GET /api/distillation/status.
 */
export function useDistillationStatus() {
  const [status, setStatus] = useState<DistillationStatus | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [tick, setTick] = useState(0);

  const refetch = useCallback(() => setTick((t) => t + 1), []);

  useEffect(() => {
    let cancelled = false;

    const load = async () => {
      setLoading(true);
      setError(null);
      try {
        const data = await fetchJson<DistillationStatus>(
          "/api/distillation/status"
        );
        if (!cancelled) setStatus(data);
      } catch (err) {
        if (!cancelled) setError(err instanceof Error ? err.message : String(err));
      } finally {
        if (!cancelled) setLoading(false);
      }
    };

    load();
    return () => { cancelled = true; };
  }, [tick]);

  return { status, loading, error, refetch };
}

/**
 * Fetch entity detail with its neighbors (connections) for the
 * entity detail panel.
 */
export function useEntityConnections(agentId: string, entityId: string) {
  const [data, setData] = useState<GraphNeighborResponse | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;

    const load = async () => {
      if (!agentId || !entityId) {
        setData(null);
        setLoading(false);
        return;
      }

      setLoading(true);
      setError(null);
      try {
        const transport = await getTransport();
        const res = await transport.getEntityNeighbors(agentId, entityId, {
          limit: 50,
        });
        if (cancelled) return;

        if (!res.success || !res.data) {
          throw new Error(res.error || "Failed to fetch entity connections");
        }
        setData(res.data);
      } catch (err) {
        if (!cancelled) setError(err instanceof Error ? err.message : String(err));
      } finally {
        if (!cancelled) setLoading(false);
      }
    };

    load();
    return () => { cancelled = true; };
  }, [agentId, entityId]);

  return { data, loading, error };
}

// ============================================================================
// BACKFILL HOOK
// ============================================================================

/** Shape returned by GET /api/distillation/undistilled */
interface UndistilledSession {
  session_id: string;
  agent_id: string;
}

/** Progress state for the backfill operation. */
export interface BackfillProgress {
  current: number;
  total: number;
}

/**
 * Hook to drive bulk-distillation ("backfill") from the UI.
 *
 * Fetches undistilled sessions, then triggers distillation for each one
 * sequentially, updating progress as it goes.
 */
export function useBackfill(onComplete?: () => void) {
  const [isRunning, setIsRunning] = useState(false);
  const [isDone, setIsDone] = useState(false);
  const [progress, setProgress] = useState<BackfillProgress>({ current: 0, total: 0 });
  const [error, setError] = useState<string | null>(null);

  const run = useCallback(async () => {
    setIsRunning(true);
    setIsDone(false);
    setError(null);
    setProgress({ current: 0, total: 0 });

    try {
      const sessions = await fetchJson<UndistilledSession[]>(
        "/api/distillation/undistilled"
      );

      if (sessions.length === 0) {
        setIsDone(true);
        setIsRunning(false);
        onComplete?.();
        return;
      }

      setProgress({ current: 0, total: sessions.length });

      for (let i = 0; i < sessions.length; i++) {
        try {
          await postJson<unknown>(
            `/api/distillation/trigger/${sessions[i].session_id}`
          );
        } catch {
          // Individual failures are non-fatal — continue with next session.
        }
        setProgress({ current: i + 1, total: sessions.length });
      }

      setIsDone(true);
      onComplete?.();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setIsRunning(false);
    }
  }, [onComplete]);

  return { run, isRunning, isDone, progress, error };
}
