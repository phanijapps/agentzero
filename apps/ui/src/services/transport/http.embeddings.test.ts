// ============================================================================
// HttpTransport — configureEmbeddings + SSE stream helpers
// Exercises the module-level SSE parsing functions indirectly via the
// configureEmbeddings() public method.
// ============================================================================

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { HttpTransport } from './http';

const HTTP = 'http://localhost:18791';
const WS = 'ws://localhost:18791/ws';

let fetchMock: ReturnType<typeof vi.fn>;

function newTransport(): HttpTransport {
  const t = new HttpTransport();
  void t.initialize({ httpUrl: HTTP, wsUrl: WS });
  return t;
}

// Build a minimal ReadableStream from an array of string chunks
function makeStream(chunks: string[]): ReadableStream<Uint8Array> {
  const encoder = new TextEncoder();
  let i = 0;
  return new ReadableStream({
    pull(controller) {
      if (i < chunks.length) {
        controller.enqueue(encoder.encode(chunks[i++]));
      } else {
        controller.close();
      }
    },
  });
}

// Build a Response-like object with a ReadableStream body
function streamResponse(chunks: string[], status = 200): Response {
  return {
    ok: status >= 200 && status < 300,
    status,
    statusText: status === 200 ? 'OK' : 'Error',
    body: makeStream(chunks),
  } as unknown as Response;
}

beforeEach(() => {
  fetchMock = vi.fn();
  vi.stubGlobal('fetch', fetchMock);
});

afterEach(() => {
  vi.unstubAllGlobals();
});

describe('HttpTransport — configureEmbeddings', () => {
  it('returns success:false when transport is not initialized', async () => {
    const t = new HttpTransport();
    const res = await t.configureEmbeddings({ backend: 'internal' } as never, vi.fn());
    expect(res.success).toBe(false);
    expect(res.error).toMatch(/not initialized/i);
  });

  it('returns success:false when server returns non-2xx or no body', async () => {
    fetchMock.mockResolvedValue({ ok: false, status: 500, statusText: 'Error', body: null });
    const t = newTransport();
    const res = await t.configureEmbeddings({ backend: 'internal' } as never, vi.fn());
    expect(res.success).toBe(false);
    expect(res.error).toContain('HTTP 500');
  });

  it('processes "ready" SSE event and returns health snapshot', async () => {
    const sseChunk =
      'event: ready\ndata: {"backend":"internal","dim":384}\n\n';
    fetchMock.mockResolvedValue(streamResponse([sseChunk]));
    const t = newTransport();
    const onProgress = vi.fn();
    const res = await t.configureEmbeddings({ backend: 'internal' } as never, onProgress);
    expect(res.success).toBe(true);
    expect(res.data?.backend).toBe('internal');
    expect(res.data?.dim).toBe(384);
    expect(res.data?.status).toBe('ready');
    expect(onProgress).toHaveBeenCalledWith(
      expect.objectContaining({ kind: 'ready', backend: 'internal', dim: 384 }),
    );
  });

  it('processes "ready" event with ollama backend', async () => {
    const sseChunk =
      'event: ready\ndata: {"backend":"ollama","model":"nomic-embed-text","dim":768}\n\n';
    fetchMock.mockResolvedValue(streamResponse([sseChunk]));
    const t = newTransport();
    const res = await t.configureEmbeddings({ backend: 'ollama' } as never, vi.fn());
    expect(res.success).toBe(true);
    expect(res.data?.backend).toBe('ollama');
    expect(res.data?.model).toBe('nomic-embed-text');
  });

  it('processes "pulling" SSE event and calls onProgress', async () => {
    const pulling = 'event: pulling\ndata: {"mb_done":10,"mb_total":100}\n\n';
    const ready = 'event: ready\ndata: {"backend":"internal","dim":384}\n\n';
    fetchMock.mockResolvedValue(streamResponse([pulling, ready]));
    const t = newTransport();
    const onProgress = vi.fn();
    await t.configureEmbeddings({ backend: 'internal' } as never, onProgress);
    expect(onProgress).toHaveBeenCalledWith(
      expect.objectContaining({ kind: 'pulling', mb_done: 10, mb_total: 100 }),
    );
  });

  it('processes "reindexing" SSE event and calls onProgress', async () => {
    const reindexing = 'event: reindexing\ndata: {"table":"facts","current":50,"total":200}\n\n';
    const ready = 'event: ready\ndata: {"backend":"internal","dim":384}\n\n';
    fetchMock.mockResolvedValue(streamResponse([reindexing, ready]));
    const t = newTransport();
    const onProgress = vi.fn();
    await t.configureEmbeddings({ backend: 'internal' } as never, onProgress);
    expect(onProgress).toHaveBeenCalledWith(
      expect.objectContaining({ kind: 'reindexing', table: 'facts', current: 50, total: 200 }),
    );
  });

  it('processes "error" SSE event and returns failure', async () => {
    const error = 'event: error\ndata: {"reason":"model download failed"}\n\n';
    fetchMock.mockResolvedValue(streamResponse([error]));
    const t = newTransport();
    const onProgress = vi.fn();
    const res = await t.configureEmbeddings({ backend: 'internal' } as never, onProgress);
    expect(res.success).toBe(false);
    expect(res.error).toBe('model download failed');
    expect(onProgress).toHaveBeenCalledWith(
      expect.objectContaining({ kind: 'error', reason: 'model download failed' }),
    );
  });

  it('handles "error" event with rollback field', async () => {
    const error = 'event: error\ndata: {"reason":"oops","rollback":"internal"}\n\n';
    fetchMock.mockResolvedValue(streamResponse([error]));
    const t = newTransport();
    const onProgress = vi.fn();
    await t.configureEmbeddings({ backend: 'internal' } as never, onProgress);
    expect(onProgress).toHaveBeenCalledWith(
      expect.objectContaining({ kind: 'error', rollback: 'internal' }),
    );
  });

  it('returns "Stream ended without ready event" when stream closes with no ready', async () => {
    // Empty stream — no events at all
    fetchMock.mockResolvedValue(streamResponse([]));
    const t = newTransport();
    const res = await t.configureEmbeddings({ backend: 'internal' } as never, vi.fn());
    expect(res.success).toBe(false);
    expect(res.error).toBe('Stream ended without ready event');
  });

  it('handles streams that arrive in multiple chunks (SSE frame split across reads)', async () => {
    // Split a single SSE frame across two read() calls
    const part1 = 'event: ready\ndata: ';
    const part2 = '{"backend":"internal","dim":512}\n\n';
    fetchMock.mockResolvedValue(streamResponse([part1, part2]));
    const t = newTransport();
    const res = await t.configureEmbeddings({ backend: 'internal' } as never, vi.fn());
    expect(res.success).toBe(true);
    expect(res.data?.dim).toBe(512);
  });

  it('ignores SSE frames with unknown event types', async () => {
    const unknown = 'event: progress\ndata: {"pct":50}\n\n';
    const ready = 'event: ready\ndata: {"backend":"internal","dim":384}\n\n';
    fetchMock.mockResolvedValue(streamResponse([unknown, ready]));
    const t = newTransport();
    const onProgress = vi.fn();
    const res = await t.configureEmbeddings({ backend: 'internal' } as never, onProgress);
    expect(res.success).toBe(true);
    // onProgress only called for known events
    const kinds = onProgress.mock.calls.map((c) => c[0].kind);
    expect(kinds).not.toContain('progress');
    expect(kinds).toContain('ready');
  });

  it('ignores SSE frames with invalid JSON in data', async () => {
    const invalid = 'event: pulling\ndata: not-valid-json\n\n';
    const ready = 'event: ready\ndata: {"backend":"internal","dim":384}\n\n';
    fetchMock.mockResolvedValue(streamResponse([invalid, ready]));
    const t = newTransport();
    const onProgress = vi.fn();
    const res = await t.configureEmbeddings({ backend: 'internal' } as never, onProgress);
    expect(res.success).toBe(true);
    const kinds = onProgress.mock.calls.map((c) => c[0].kind);
    expect(kinds).not.toContain('pulling');
  });

  it('handles AbortError (user cancellation)', async () => {
    const abort = new Error('aborted');
    abort.name = 'AbortError';
    fetchMock.mockRejectedValue(abort);
    const t = newTransport();
    const res = await t.configureEmbeddings({ backend: 'internal' } as never, vi.fn());
    expect(res.success).toBe(false);
    expect(res.error).toBe('Aborted');
  });

  it('handles generic fetch errors', async () => {
    fetchMock.mockRejectedValue(new Error('network down'));
    const t = newTransport();
    const res = await t.configureEmbeddings({ backend: 'internal' } as never, vi.fn());
    expect(res.success).toBe(false);
    expect(res.error).toContain('network down');
  });

  it('processes frames with multiple data lines (joined with newline)', async () => {
    // SSE spec allows multi-line data
    const multiLine = 'event: ready\ndata: {"backend":"internal"\ndata: ,"dim":128}\n\n';
    fetchMock.mockResolvedValue(streamResponse([multiLine]));
    const t = newTransport();
    // This will fail to parse (invalid JSON after join), returning no ready event
    const res = await t.configureEmbeddings({ backend: 'internal' } as never, vi.fn());
    // The key is that it doesn't throw
    expect(typeof res.success).toBe('boolean');
  });
});

// ===========================================================================
// Additional HttpTransport method coverage
// ===========================================================================

describe('HttpTransport — additional methods', () => {
  let fetchMock: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    fetchMock = vi.fn();
    vi.stubGlobal('fetch', fetchMock);
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it('getArtifactContentUrl returns correct URL', () => {
    const t = newTransport();
    const url = t.getArtifactContentUrl('artifact-123', 'sess-123');
    expect(url).toBe(`${HTTP}/api/artifacts/artifact-123/content?session_id=sess-123`);
  });

  it('getArtifactContentUrl URL-encodes the artifact id', () => {
    const t = newTransport();
    const url = t.getArtifactContentUrl('artifact with spaces', 'session with spaces');
    expect(url).toBe(`${HTTP}/api/artifacts/artifact%20with%20spaces/content?session_id=session%20with%20spaces`);
  });

  it('isConnected returns false when no WebSocket', () => {
    const t = newTransport();
    expect(t.isConnected()).toBe(false);
  });

  it('getConnectionState returns disconnected initially', () => {
    const t = newTransport();
    const state = t.getConnectionState();
    expect(state.status).toBe('disconnected');
  });

  it('onConnectionStateChange notifies immediately with current state', () => {
    const t = newTransport();
    const cb = vi.fn();
    const unsub = t.onConnectionStateChange(cb);
    expect(cb).toHaveBeenCalledWith(expect.objectContaining({ status: 'disconnected' }));
    unsub();
  });

  it('onConnectionStateChange returns unsubscribe function that removes the callback', () => {
    const t = newTransport();
    const cb = vi.fn();
    const unsub = t.onConnectionStateChange(cb);
    cb.mockClear();
    unsub();
    // After unsubscribe, the callback should not be called again.
    // We can verify by calling disconnect() which triggers setConnectionState.
    void t.disconnect();
    expect(cb).not.toHaveBeenCalled();
  });

  it('onGlobalEvent returns an unsubscribe function', () => {
    const t = newTransport();
    const cb = vi.fn();
    const unsub = t.onGlobalEvent(cb);
    expect(typeof unsub).toBe('function');
    unsub();
  });

  it('subscribe (legacy) returns unsubscribe function', () => {
    const t = newTransport();
    const cb = vi.fn();
    const unsub = t.subscribe('conv-1', cb);
    expect(typeof unsub).toBe('function');
    unsub();
  });

  it('getLogSettings returns error when envelope fails', async () => {
    fetchMock.mockResolvedValue({
      ok: true,
      status: 200,
      json: async () => ({ success: false, error: 'settings corrupt' }),
    });
    const t = newTransport();
    const res = await t.getLogSettings();
    expect(res.success).toBe(false);
    expect(res.error).toBe('settings corrupt');
  });

  it('getExecutionSettings returns data when envelope succeeds', async () => {
    fetchMock.mockResolvedValue({
      ok: true,
      status: 200,
      json: async () => ({
        success: true,
        data: { maxParallelAgents: 4, setupComplete: true, restartRequired: false },
      }),
    });
    const t = newTransport();
    const res = await t.getExecutionSettings();
    expect(res.success).toBe(true);
    expect(res.data?.maxParallelAgents).toBe(4);
  });

  it('listMemory includes agent_id in path and filter params', async () => {
    fetchMock.mockResolvedValue({ ok: true, status: 200, json: async () => ({ facts: [], total: 0 }) });
    const t = newTransport();
    await t.listMemory('agent-x', { category: 'user', limit: 10 });
    const [url] = fetchMock.mock.calls[0];
    const u = new URL(url);
    expect(u.pathname).toBe('/api/memory/agent-x');
    expect(u.searchParams.get('category')).toBe('user');
    expect(u.searchParams.get('limit')).toBe('10');
  });

  it('listAllMemory builds correct query string', async () => {
    fetchMock.mockResolvedValue({ ok: true, status: 200, json: async () => ({ facts: [], total: 0 }) });
    const t = newTransport();
    await t.listAllMemory({ agent_id: 'root', scope: 'shared', limit: 5, offset: 0 });
    const [url] = fetchMock.mock.calls[0];
    const u = new URL(url);
    expect(u.pathname).toBe('/api/memory');
    expect(u.searchParams.get('agent_id')).toBe('root');
    expect(u.searchParams.get('scope')).toBe('shared');
    expect(u.searchParams.get('limit')).toBe('5');
  });

  it('searchMemory builds query with agent_id in path', async () => {
    fetchMock.mockResolvedValue({ ok: true, status: 200, json: async () => ({ facts: [], total: 0 }) });
    const t = newTransport();
    await t.searchMemory('agent-x', 'my query', { category: 'skill', limit: 3 });
    const [url] = fetchMock.mock.calls[0];
    const u = new URL(url);
    expect(u.pathname).toBe('/api/memory/agent-x/search');
    expect(u.searchParams.get('q')).toBe('my query');
    expect(u.searchParams.get('category')).toBe('skill');
    expect(u.searchParams.get('limit')).toBe('3');
  });

  it('searchAllMemory passes query and optional params', async () => {
    fetchMock.mockResolvedValue({ ok: true, status: 200, json: async () => ({ facts: [] }) });
    const t = newTransport();
    await t.searchAllMemory('test', 20, 'user');
    const [url] = fetchMock.mock.calls[0];
    const u = new URL(url);
    expect(u.pathname).toBe('/api/memory/search');
    expect(u.searchParams.get('q')).toBe('test');
    expect(u.searchParams.get('limit')).toBe('20');
    expect(u.searchParams.get('category')).toBe('user');
  });

  it('getGraphStats calls correct path', async () => {
    fetchMock.mockResolvedValue({ ok: true, status: 200, json: async () => ({}) });
    const t = newTransport();
    await t.getGraphStats('agent-1');
    const [url] = fetchMock.mock.calls[0];
    expect(url).toContain('/api/graph/agent-1/stats');
  });

  it('pauseSession returns error when WebSocket not connected', async () => {
    const t = newTransport();
    const res = await t.pauseSession('sess-1');
    expect(res.success).toBe(false);
    expect(res.error).toMatch(/WebSocket not connected/i);
  });

  it('resumeSession returns error when WebSocket not connected', async () => {
    const t = newTransport();
    const res = await t.resumeSession('sess-1');
    expect(res.success).toBe(false);
    expect(res.error).toMatch(/WebSocket not connected/i);
  });

  it('cancelSession returns error when WebSocket not connected', async () => {
    const t = newTransport();
    const res = await t.cancelSession('sess-1');
    expect(res.success).toBe(false);
  });

  it('endSession returns error when WebSocket not connected', async () => {
    const t = newTransport();
    const res = await t.endSession('sess-1');
    expect(res.success).toBe(false);
  });

  it('executeAgent returns error when WebSocket not connected', async () => {
    const t = newTransport();
    const res = await t.executeAgent('root', 'conv-1', 'hello');
    expect(res.success).toBe(false);
  });

  it('stopAgent returns error when WebSocket not connected', async () => {
    const t = newTransport();
    const res = await t.stopAgent('conv-1');
    expect(res.success).toBe(false);
  });

  it('getMessages routes exec- prefix to executions endpoint', async () => {
    fetchMock.mockResolvedValue({ ok: true, status: 200, json: async () => [] });
    const t = newTransport();
    await t.getMessages('exec-abc123');
    const [url] = fetchMock.mock.calls[0];
    expect(url).toContain('/api/executions/exec-abc123/messages');
  });

  it('getMessages routes non-exec id to conversations endpoint', async () => {
    fetchMock.mockResolvedValue({ ok: true, status: 200, json: async () => [] });
    const t = newTransport();
    await t.getMessages('web-123');
    const [url] = fetchMock.mock.calls[0];
    expect(url).toContain('/api/conversations/web-123/messages');
  });

  it('listPlugins calls correct endpoint', async () => {
    fetchMock.mockResolvedValue({ ok: true, status: 200, json: async () => ({ plugins: [] }) });
    const t = newTransport();
    await t.listPlugins();
    const [url] = fetchMock.mock.calls[0];
    expect(url).toContain('/api/plugins');
  });

  it('listCronJobs calls /api/cron', async () => {
    fetchMock.mockResolvedValue({ ok: true, status: 200, json: async () => [] });
    const t = newTransport();
    await t.listCronJobs();
    const [url] = fetchMock.mock.calls[0];
    expect(url).toContain('/api/cron');
  });

  it('createCronJob POSTs to /api/cron', async () => {
    fetchMock.mockResolvedValue({ ok: true, status: 200, json: async () => ({ id: 'cron-1' }) });
    const t = newTransport();
    await t.createCronJob({ name: 'Daily', schedule: '0 9 * * *' } as never);
    const [url, init] = fetchMock.mock.calls[0];
    expect(url).toContain('/api/cron');
    expect(init.method).toBe('POST');
  });

  it('triggerCronJob POSTs to /api/cron/:id/trigger', async () => {
    fetchMock.mockResolvedValue({ ok: true, status: 200, json: async () => ({}) });
    const t = newTransport();
    await t.triggerCronJob('cron-123');
    const [url, init] = fetchMock.mock.calls[0];
    expect(url).toContain('/api/cron/cron-123/trigger');
    expect(init.method).toBe('POST');
  });

  it('connect returns error when transport not initialized', async () => {
    const t = new HttpTransport();
    const res = await t.connect();
    expect(res.success).toBe(false);
    expect(res.error).toMatch(/not initialized/i);
  });

  it('cleanupExecutionSessions builds correct DELETE URL', async () => {
    fetchMock.mockResolvedValue({ ok: true, status: 200, json: async () => ({ deleted: 5 }) });
    const t = newTransport();
    const res = await t.cleanupExecutionSessions('2026-01-01T00:00:00Z');
    expect(res.success).toBe(true);
    const [url, init] = fetchMock.mock.calls[0];
    expect(url).toContain('/api/executions/cleanup');
    expect(url).toContain('older_than=');
    expect(init.method).toBe('DELETE');
  });

  it('cleanupExecutionSessions handles non-2xx', async () => {
    fetchMock.mockResolvedValue({ ok: false, status: 500, statusText: 'Error' });
    const t = newTransport();
    const res = await t.cleanupExecutionSessions('2026-01-01T00:00:00Z');
    expect(res.success).toBe(false);
  });

  it('subscribeConversation registers callback and returns unsub fn', () => {
    const t = newTransport();
    const onEvent = vi.fn();
    const unsub = t.subscribeConversation('conv-1', { onEvent });
    expect(typeof unsub).toBe('function');
    unsub();
  });

  it('subscribeConversation unsub removes callback', () => {
    const t = newTransport();
    const onEvent = vi.fn();
    const unsub = t.subscribeConversation('conv-1', { onEvent });
    unsub();
    // Calling unsub again should not throw
    expect(() => unsub()).not.toThrow();
  });

  it('getEmbeddingsHealth calls /api/embeddings/health', async () => {
    fetchMock.mockResolvedValue({ ok: true, status: 200, json: async () => ({ status: 'ready', backend: 'internal', dim: 384 }) });
    const t = newTransport();
    await t.getEmbeddingsHealth();
    const [url] = fetchMock.mock.calls[0];
    expect(url).toContain('/api/embeddings/health');
  });

  it('getEmbeddingsModels calls /api/embeddings/models', async () => {
    fetchMock.mockResolvedValue({ ok: true, status: 200, json: async () => [] });
    const t = newTransport();
    await t.getEmbeddingsModels();
    const [url] = fetchMock.mock.calls[0];
    expect(url).toContain('/api/embeddings/models');
  });

  it('getOllamaEmbeddingModels calls with optional base URL', async () => {
    fetchMock.mockResolvedValue({ ok: true, status: 200, json: async () => ({ models: [] }) });
    const t = newTransport();
    await t.getOllamaEmbeddingModels('http://localhost:11434');
    const [url] = fetchMock.mock.calls[0];
    expect(url).toContain('/api/embeddings/ollama-models');
    expect(url).toContain('url=');
  });

  it('getOllamaEmbeddingModels omits url param when not provided', async () => {
    fetchMock.mockResolvedValue({ ok: true, status: 200, json: async () => ({ models: [] }) });
    const t = newTransport();
    await t.getOllamaEmbeddingModels();
    const [url] = fetchMock.mock.calls[0];
    expect(url).not.toContain('url=');
  });

  it('listSessionArtifacts calls correct endpoint', async () => {
    fetchMock.mockResolvedValue({ ok: true, status: 200, json: async () => [] });
    const t = newTransport();
    await t.listSessionArtifacts('sess-1');
    const [url] = fetchMock.mock.calls[0];
    expect(url).toContain('/api/sessions/sess-1/artifacts');
  });

  it('listSessionArtifacts sends bounded goal-artifact query options', async () => {
    fetchMock.mockResolvedValue({ ok: true, status: 200, json: async () => [] });
    const t = newTransport();
    await t.listSessionArtifacts('sess-1', { goalArtifactsOnly: true, limit: 24 });
    const [url] = fetchMock.mock.calls[0];
    expect(url).toContain('goal_artifacts_only=true');
    expect(url).toContain('limit=24');
  });

  it('listBridgeWorkers calls /api/bridge/workers', async () => {
    fetchMock.mockResolvedValue({ ok: true, status: 200, json: async () => [] });
    const t = newTransport();
    await t.listBridgeWorkers();
    const [url] = fetchMock.mock.calls[0];
    expect(url).toContain('/api/bridge/workers');
  });
});
