// ============================================================================
// HttpTransport class — direct method tests
//
// The sibling `http.test.ts` covers raw-fetch contracts via MSW; this file
// drills into the `HttpTransport` class methods themselves, focusing on:
//
//   - URL building for path-id and query-string methods (encoding, escaping,
//     conditional params).
//   - Response decoding for the `success: true|false` wire envelope used by
//     settings endpoints.
//   - The four private HTTP helpers (get/post/put/delete) exercised through
//     the public surface that wraps them.
//   - Error-path branches: non-2xx responses, fetch throws, AbortError
//     timeouts, transport-uninitialized.
//
// Mocking strategy: vi.stubGlobal('fetch') per-test. Simpler than MSW for
// shape-of-call assertions; cleaner isolation than vi.spyOn since vitest
// auto-restores stubbed globals between tests.
// ============================================================================

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { HttpTransport } from './http';

const HTTP = 'http://localhost:18791';
const WS = 'ws://localhost:18790';

interface MockResponseInit {
  ok?: boolean;
  status?: number;
  statusText?: string;
  body?: unknown;
  text?: string;
  headers?: Record<string, string>;
  arrayBuffer?: ArrayBuffer;
}

// Build a Response-like object that satisfies the bits of the Response
// interface our transport touches (ok, status, statusText, json(), text()).
function mockResponse(init: MockResponseInit = {}): Response {
  const ok = init.ok ?? true;
  const status = init.status ?? (ok ? 200 : 500);
  const statusText = init.statusText ?? (ok ? 'OK' : 'Internal Server Error');
  const body = init.body ?? null;
  const textBody = init.text ?? '';
  const headers = new Map(Object.entries(init.headers ?? {}));
  return {
    ok,
    status,
    statusText,
    headers: {
      get: (name: string) => headers.get(name.toLowerCase()) ?? headers.get(name) ?? null,
    },
    json: async () => body,
    text: async () => textBody,
    arrayBuffer: async () => init.arrayBuffer ?? new ArrayBuffer(0),
  } as unknown as Response;
}

let fetchMock: ReturnType<typeof vi.fn>;

function newTransport(): HttpTransport {
  const t = new HttpTransport();
  // Fire-and-forget initialize — synchronous in practice (no awaits inside).
  void t.initialize({ httpUrl: HTTP, wsUrl: WS });
  return t;
}

beforeEach(() => {
  fetchMock = vi.fn();
  vi.stubGlobal('fetch', fetchMock);
});

afterEach(() => {
  vi.unstubAllGlobals();
  vi.useRealTimers();
});

// ===========================================================================
// Initialization & uninitialized error paths
// ===========================================================================

describe('HttpTransport — uninitialized', () => {
  it('GET methods return success:false without calling fetch when transport not initialized', async () => {
    const t = new HttpTransport();
    const res = await t.health();
    expect(res.success).toBe(false);
    expect(res.error).toMatch(/not initialized/i);
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it('POST methods return success:false without calling fetch when transport not initialized', async () => {
    const t = new HttpTransport();
    const res = await t.createAgent({
      name: 'foo',
      providerId: 'p',
      model: 'm',
    } as never);
    expect(res.success).toBe(false);
    expect(res.error).toMatch(/not initialized/i);
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it('DELETE methods return success:false without calling fetch when transport not initialized', async () => {
    const t = new HttpTransport();
    const res = await t.deleteAgent('a');
    expect(res.success).toBe(false);
    expect(res.error).toMatch(/not initialized/i);
    expect(fetchMock).not.toHaveBeenCalled();
  });
});

// ===========================================================================
// GET helper — happy + error paths
// ===========================================================================

describe('HttpTransport — GET helper', () => {
  it('returns the parsed JSON body on success', async () => {
    fetchMock.mockResolvedValue(mockResponse({ body: { status: 'ok', version: '1' } }));
    const t = newTransport();

    const res = await t.health();
    expect(res.success).toBe(true);
    expect(res.data).toEqual({ status: 'ok', version: '1' });
    expect(fetchMock).toHaveBeenCalledTimes(1);
    const [url, init] = fetchMock.mock.calls[0];
    expect(url).toBe(`${HTTP}/api/health`);
    expect(init).toMatchObject({
      method: 'GET',
      headers: { 'Content-Type': 'application/json' },
    });
  });

  it('returns success:false with HTTP status on non-2xx', async () => {
    fetchMock.mockResolvedValue(mockResponse({ ok: false, status: 503, statusText: 'Service Unavailable' }));
    const t = newTransport();

    const res = await t.health();
    expect(res.success).toBe(false);
    expect(res.error).toContain('HTTP 503');
    expect(res.error).toContain('Service Unavailable');
  });

  it('returns success:false when fetch throws (network error)', async () => {
    fetchMock.mockRejectedValue(new Error('connection refused'));
    const t = newTransport();

    const res = await t.health();
    expect(res.success).toBe(false);
    expect(res.error).toContain('connection refused');
  });

  it('returns "Request timeout" message when AbortError is raised', async () => {
    const abort = new Error('aborted');
    abort.name = 'AbortError';
    fetchMock.mockRejectedValue(abort);
    const t = newTransport();

    const res = await t.health();
    expect(res.success).toBe(false);
    expect(res.error).toMatch(/Request timeout/i);
  });
});

// ===========================================================================
// POST helper — body serialization, errors, timeout
// ===========================================================================

describe('HttpTransport — POST helper', () => {
  it('serializes the body and posts JSON', async () => {
    fetchMock.mockResolvedValue(mockResponse({ body: { id: 'a', name: 'A' } }));
    const t = newTransport();

    const res = await t.createAgent({ name: 'A' } as never);
    expect(res.success).toBe(true);
    const [url, init] = fetchMock.mock.calls[0];
    expect(url).toBe(`${HTTP}/api/agents`);
    expect(init.method).toBe('POST');
    expect(init.headers).toEqual({ 'Content-Type': 'application/json' });
    expect(JSON.parse(init.body)).toEqual({ name: 'A' });
  });

  it('returns the HTTP status on non-2xx', async () => {
    fetchMock.mockResolvedValue(mockResponse({ ok: false, status: 422, statusText: 'Unprocessable' }));
    const t = newTransport();
    const res = await t.createAgent({ name: 'bad' } as never);
    expect(res.success).toBe(false);
    expect(res.error).toContain('HTTP 422');
  });

  it('surfaces network errors via String(error)', async () => {
    fetchMock.mockRejectedValue('plain string failure');
    const t = newTransport();
    const res = await t.createAgent({ name: 'X' } as never);
    expect(res.success).toBe(false);
    expect(res.error).toContain('plain string failure');
  });
});

// ===========================================================================
// PUT helper
// ===========================================================================

describe('HttpTransport — PUT helper', () => {
  it('sends PUT with body and returns parsed JSON', async () => {
    fetchMock.mockResolvedValue(mockResponse({ body: { id: 'a', name: 'updated' } }));
    const t = newTransport();
    const res = await t.updateAgent('a', { name: 'updated' } as never);
    expect(res.success).toBe(true);
    const [url, init] = fetchMock.mock.calls[0];
    expect(url).toBe(`${HTTP}/api/agents/a`);
    expect(init.method).toBe('PUT');
    expect(JSON.parse(init.body)).toEqual({ name: 'updated' });
  });
});

// ===========================================================================
// DELETE helper
// ===========================================================================

describe('HttpTransport — DELETE helper', () => {
  it('treats 200 as success', async () => {
    fetchMock.mockResolvedValue(mockResponse({ status: 200 }));
    const t = newTransport();
    const res = await t.deleteAgent('a');
    expect(res.success).toBe(true);
  });

  it('treats 204 No Content as success even though Response.ok may be false in some envs', async () => {
    // Simulate the rare jsdom case where ok is false but status is 204:
    // production code special-cases this since DELETE often returns 204.
    fetchMock.mockResolvedValue(mockResponse({ ok: false, status: 204, statusText: 'No Content' }));
    const t = newTransport();
    const res = await t.deleteAgent('a');
    expect(res.success).toBe(true);
  });

  it('returns HTTP error on 404', async () => {
    fetchMock.mockResolvedValue(mockResponse({ ok: false, status: 404, statusText: 'Not Found' }));
    const t = newTransport();
    const res = await t.deleteAgent('does-not-exist');
    expect(res.success).toBe(false);
    expect(res.error).toContain('HTTP 404');
  });

  it('encodes path id segments', async () => {
    fetchMock.mockResolvedValue(mockResponse({ status: 200 }));
    const t = newTransport();
    await t.deleteAgent('agent with spaces');
    const [url] = fetchMock.mock.calls[0];
    expect(url).toBe(`${HTTP}/api/agents/agent%20with%20spaces`);
  });
});

// ===========================================================================
// Path-id encoding for resource methods
// ===========================================================================

describe('HttpTransport — path-id encoding', () => {
  it('encodes special characters in agent ids', async () => {
    fetchMock.mockResolvedValue(mockResponse({ body: {} }));
    const t = newTransport();
    await t.getAgent('weird/id?with=stuff');
    const [url] = fetchMock.mock.calls[0];
    expect(url).toBe(`${HTTP}/api/agents/weird%2Fid%3Fwith%3Dstuff`);
  });

  it('encodes session ids in getSessionState', async () => {
    fetchMock.mockResolvedValue(mockResponse({ body: {} }));
    const t = newTransport();
    await t.getSessionState('sess-abc/123');
    const [url] = fetchMock.mock.calls[0];
    expect(url).toBe(`${HTTP}/api/sessions/sess-abc%2F123/state`);
  });

  it('encodes session ids in deleteSession', async () => {
    fetchMock.mockResolvedValue(mockResponse({ status: 200 }));
    const t = newTransport();
    await t.deleteSession('sess  with  spaces');
    const [url, init] = fetchMock.mock.calls[0];
    expect(url).toBe(`${HTTP}/api/sessions/sess%20%20with%20%20spaces`);
    expect(init.method).toBe('DELETE');
  });
});

// ===========================================================================
// Query-string building methods
// ===========================================================================

describe('HttpTransport — query-string builders', () => {
  it('listLogSessions includes all set filter params', async () => {
    fetchMock.mockResolvedValue(mockResponse({ body: [] }));
    const t = newTransport();
    await t.listLogSessions({
      agent_id: 'planner',
      level: 'error',
      from_time: '2026-01-01',
      to_time: '2026-12-31',
      limit: 25,
      offset: 50,
      root_only: true,
    } as never);
    const [url] = fetchMock.mock.calls[0];
    const u = new URL(url);
    expect(u.pathname).toBe('/api/logs/sessions');
    expect(u.searchParams.get('agent_id')).toBe('planner');
    expect(u.searchParams.get('level')).toBe('error');
    expect(u.searchParams.get('from_time')).toBe('2026-01-01');
    expect(u.searchParams.get('to_time')).toBe('2026-12-31');
    expect(u.searchParams.get('limit')).toBe('25');
    expect(u.searchParams.get('offset')).toBe('50');
    expect(u.searchParams.get('root_only')).toBe('true');
  });

  it('listLogSessions omits the query-string entirely when no filter is provided', async () => {
    fetchMock.mockResolvedValue(mockResponse({ body: [] }));
    const t = newTransport();
    await t.listLogSessions();
    const [url] = fetchMock.mock.calls[0];
    expect(url).toBe(`${HTTP}/api/logs/sessions`);
    expect(url).not.toContain('?');
  });

  it('listLogSessions skips falsy filter fields (undefined / empty string / 0)', async () => {
    fetchMock.mockResolvedValue(mockResponse({ body: [] }));
    const t = newTransport();
    await t.listLogSessions({ limit: 10 } as never);
    const [url] = fetchMock.mock.calls[0];
    const u = new URL(url);
    expect(u.searchParams.get('limit')).toBe('10');
    expect(u.searchParams.get('agent_id')).toBeNull();
    expect(u.searchParams.get('root_only')).toBeNull();
  });

  it('getSessionMessages encodes session id and includes scope/execution_id/agent_id when set', async () => {
    fetchMock.mockResolvedValue(mockResponse({ body: [] }));
    const t = newTransport();
    await t.getSessionMessages('sess-abc', {
      scope: 'root',
      execution_id: 'exec-1',
      agent_id: 'planner',
    });
    const [url] = fetchMock.mock.calls[0];
    const u = new URL(url);
    expect(u.pathname).toBe('/api/executions/v2/sessions/sess-abc/messages');
    expect(u.searchParams.get('scope')).toBe('root');
    expect(u.searchParams.get('execution_id')).toBe('exec-1');
    expect(u.searchParams.get('agent_id')).toBe('planner');
  });

  it('getSessionMessages omits the query-string when no query object is provided', async () => {
    fetchMock.mockResolvedValue(mockResponse({ body: [] }));
    const t = newTransport();
    await t.getSessionMessages('sess-abc');
    const [url] = fetchMock.mock.calls[0];
    expect(url).toBe(`${HTTP}/api/executions/v2/sessions/sess-abc/messages`);
  });

  it('listSessionsFull builds query params and base path', async () => {
    fetchMock.mockResolvedValue(mockResponse({ body: [] }));
    const t = newTransport();
    await t.listSessionsFull({
      status: 'running',
      root_agent_id: 'root',
      limit: 20,
      offset: 0, // 0 is falsy → should be omitted
    } as never);
    const [url] = fetchMock.mock.calls[0];
    const u = new URL(url);
    expect(u.pathname).toBe('/api/executions/v2/sessions/full');
    expect(u.searchParams.get('status')).toBe('running');
    expect(u.searchParams.get('root_agent_id')).toBe('root');
    expect(u.searchParams.get('limit')).toBe('20');
    // Documented behavior: filter `if (filter?.offset)` skips zero.
    expect(u.searchParams.get('offset')).toBeNull();
  });

  it('listMissionControlSessions builds query params and base path', async () => {
    fetchMock.mockResolvedValue(mockResponse({ body: [] }));
    const t = newTransport();
    await t.listMissionControlSessions({ limit: 50, offset: 20 });
    const [url] = fetchMock.mock.calls[0];
    const u = new URL(url);
    expect(u.pathname).toBe('/api/executions/v2/mission-control/sessions');
    expect(u.searchParams.get('limit')).toBe('50');
    expect(u.searchParams.get('offset')).toBe('20');
  });

  it('getMissionControlSessionTokens encodes session id and uses token endpoint', async () => {
    fetchMock.mockResolvedValue(mockResponse({ body: {} }));
    const t = newTransport();
    await t.getMissionControlSessionTokens('sess-abc/123');
    const [url] = fetchMock.mock.calls[0];
    expect(url).toBe(`${HTTP}/api/executions/v2/mission-control/sessions/sess-abc%2F123/tokens`);
  });
});

// ===========================================================================
// Vault filesystem browser
// ===========================================================================

describe('HttpTransport — Vault API', () => {
  it('listVaultWards calls the filesystem-backed Vault endpoint', async () => {
    fetchMock.mockResolvedValue(mockResponse({ body: { wards: [{ id: 'w1', name: 'w1' }] } }));
    const t = newTransport();
    const res = await t.listVaultWards();

    expect(res.success).toBe(true);
    expect(res.data).toEqual({ wards: [{ id: 'w1', name: 'w1' }] });
    const [url] = fetchMock.mock.calls[0];
    expect(url).toBe(`${HTTP}/api/vault/wards`);
  });

  it('getVaultTree encodes ward id and ward-relative path query', async () => {
    fetchMock.mockResolvedValue(mockResponse({ body: { ward_id: 'ward/id', path: 'reports/q1', children: [], truncated: false } }));
    const t = newTransport();
    await t.getVaultTree('ward/id', 'reports/q1');

    const [url] = fetchMock.mock.calls[0];
    const u = new URL(url);
    expect(u.pathname).toBe('/api/vault/wards/ward%2Fid/tree');
    expect(u.searchParams.get('path')).toBe('reports/q1');
  });

  it('searchVaultFiles encodes ward id, query, and limit', async () => {
    fetchMock.mockResolvedValue(mockResponse({
      body: { ward_id: 'ward/id', query: 'val md', matches: [], truncated: false },
    }));
    const t = newTransport();
    await t.searchVaultFiles('ward/id', 'val md', 12);

    const [url] = fetchMock.mock.calls[0];
    const u = new URL(url);
    expect(u.pathname).toBe('/api/vault/wards/ward%2Fid/search');
    expect(u.searchParams.get('q')).toBe('val md');
    expect(u.searchParams.get('limit')).toBe('12');
  });

  it('getVaultFile decodes JSON text previews', async () => {
    fetchMock.mockResolvedValue(mockResponse({
      headers: { 'content-type': 'application/json' },
      body: {
        kind: 'text',
        ward_id: 'stock',
        path: 'README.md',
        name: 'README.md',
        extension: 'md',
        size: 7,
        content: '# Stock',
      },
    }));
    const t = newTransport();
    const res = await t.getVaultFile('stock', 'README.md');

    expect(res.success).toBe(true);
    expect(res.data?.kind).toBe('text');
    const [url, init] = fetchMock.mock.calls[0];
    const u = new URL(url);
    expect(u.pathname).toBe('/api/vault/wards/stock/file');
    expect(u.searchParams.get('path')).toBe('README.md');
    expect(init.method).toBe('GET');
  });

  it('getVaultFile decodes Office previews as ArrayBuffer responses', async () => {
    const data = new Uint8Array([1, 2, 3]).buffer;
    fetchMock.mockResolvedValue(mockResponse({
      headers: {
        'content-type': 'application/vnd.openxmlformats-officedocument.presentationml.presentation',
        'x-vault-ward-id': 'stock',
        'x-vault-path': 'deck.pptx',
      },
      arrayBuffer: data,
    }));
    const t = newTransport();
    const res = await t.getVaultFile('stock', 'deck.pptx');

    expect(res.success).toBe(true);
    expect(res.data).toMatchObject({
      kind: 'office',
      ward_id: 'stock',
      path: 'deck.pptx',
      extension: 'pptx',
    });
    expect(res.data?.kind === 'office' ? res.data.data : null).toBe(data);
  });

  it('openWard still posts to the existing ward-open endpoint', async () => {
    fetchMock.mockResolvedValue(mockResponse({ body: { path: '/tmp/wards/stock' } }));
    const t = newTransport();
    const res = await t.openWard('stock/analysis');

    expect(res.success).toBe(true);
    const [url, init] = fetchMock.mock.calls[0];
    expect(url).toBe(`${HTTP}/api/wards/stock%2Fanalysis/open`);
    expect(init.method).toBe('POST');
  });
});

// ===========================================================================
// Settings envelope unwrapping (success/error in body)
// ===========================================================================

describe('HttpTransport — settings envelope', () => {
  it('getToolSettings unwraps a successful envelope to its data field', async () => {
    fetchMock.mockResolvedValue(
      mockResponse({
        body: { success: true, data: { fileTools: true, offloadLargeResults: true } },
      }),
    );
    const t = newTransport();
    const res = await t.getToolSettings();
    expect(res.success).toBe(true);
    expect(res.data).toEqual({ fileTools: true, offloadLargeResults: true });
  });

  it('getToolSettings surfaces envelope-level error messages', async () => {
    fetchMock.mockResolvedValue(
      mockResponse({
        body: { success: false, error: 'config corrupt' },
      }),
    );
    const t = newTransport();
    const res = await t.getToolSettings();
    expect(res.success).toBe(false);
    expect(res.error).toBe('config corrupt');
  });

  it('updateToolSettings PUTs and unwraps the response envelope', async () => {
    fetchMock.mockResolvedValue(
      mockResponse({ body: { success: true, data: { fileTools: false } } }),
    );
    const t = newTransport();
    const res = await t.updateToolSettings({ fileTools: false } as never);
    expect(res.success).toBe(true);
    expect(res.data).toEqual({ fileTools: false });
    const [, init] = fetchMock.mock.calls[0];
    expect(init.method).toBe('PUT');
    expect(JSON.parse(init.body)).toEqual({ fileTools: false });
  });
});

// ===========================================================================
// cleanupOldLogs — bespoke fetch (not the helper)
// ===========================================================================

describe('HttpTransport — cleanupOldLogs', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date('2026-04-24T00:00:00Z'));
  });

  it('encodes older_than as ISO from Date.now() - days, returns deletedCount', async () => {
    fetchMock.mockResolvedValue(mockResponse({ body: { deletedCount: 12 } }));
    const t = newTransport();
    const res = await t.cleanupOldLogs(7);

    expect(res.success).toBe(true);
    expect(res.data).toEqual({ deletedCount: 12 });
    const [url, init] = fetchMock.mock.calls[0];
    const u = new URL(url);
    expect(u.pathname).toBe('/api/logs/cleanup');
    // 7 days before 2026-04-24 = 2026-04-17. Encoded ISO timestamp.
    expect(decodeURIComponent(u.searchParams.get('older_than') ?? '')).toBe(
      '2026-04-17T00:00:00.000Z',
    );
    expect(init.method).toBe('DELETE');
  });

  it('returns the response body text on non-2xx', async () => {
    fetchMock.mockResolvedValue(
      mockResponse({ ok: false, status: 500, text: 'something broke' }),
    );
    const t = newTransport();
    const res = await t.cleanupOldLogs(7);
    expect(res.success).toBe(false);
    expect(res.error).toBe('something broke');
  });

  it('falls back to "HTTP <status>" when response body is empty', async () => {
    fetchMock.mockResolvedValue(
      mockResponse({ ok: false, status: 502, text: '' }),
    );
    const t = newTransport();
    const res = await t.cleanupOldLogs(1);
    expect(res.success).toBe(false);
    expect(res.error).toBe('HTTP 502');
  });
});

// ===========================================================================
// initChatSession — POST with empty body
// ===========================================================================

describe('HttpTransport — initChatSession', () => {
  it('POSTs an empty object to /api/chat/init and returns the parsed body', async () => {
    fetchMock.mockResolvedValue(
      mockResponse({
        body: { sessionId: 'sess-1', conversationId: 'chat-1', created: true },
      }),
    );
    const t = newTransport();
    const res = await t.initChatSession();
    expect(res.success).toBe(true);
    expect(res.data?.sessionId).toBe('sess-1');
    expect(res.data?.conversationId).toBe('chat-1');
    expect(res.data?.created).toBe(true);
    const [url, init] = fetchMock.mock.calls[0];
    expect(url).toBe(`${HTTP}/api/chat/init`);
    expect(init.method).toBe('POST');
    expect(JSON.parse(init.body)).toEqual({});
  });
});

// ===========================================================================
// deleteChatSession — bespoke DELETE without Content-Type
// ===========================================================================

describe('HttpTransport — deleteChatSession', () => {
  it('issues DELETE /api/chat/session and returns success', async () => {
    fetchMock.mockResolvedValue(mockResponse({ status: 200 }));
    const t = newTransport();
    const res = await t.deleteChatSession();
    expect(res.success).toBe(true);
    const [url, init] = fetchMock.mock.calls[0];
    expect(url).toBe(`${HTTP}/api/chat/session`);
    expect(init.method).toBe('DELETE');
    // Custom delete helper path — no Content-Type header by design.
    expect(init.headers).toBeUndefined();
  });

  it('reports HTTP error on non-2xx', async () => {
    fetchMock.mockResolvedValue(mockResponse({ ok: false, status: 500, statusText: 'Boom' }));
    const t = newTransport();
    const res = await t.deleteChatSession();
    expect(res.success).toBe(false);
    expect(res.error).toContain('HTTP 500');
  });

  it('catches and stringifies network errors', async () => {
    fetchMock.mockRejectedValue(new Error('socket closed'));
    const t = newTransport();
    const res = await t.deleteChatSession();
    expect(res.success).toBe(false);
    expect(res.error).toContain('socket closed');
  });
});
