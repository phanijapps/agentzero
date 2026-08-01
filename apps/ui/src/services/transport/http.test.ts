// ============================================================================
// HTTP TRANSPORT TESTS
// Unit tests for HTTP transport service
// ============================================================================

import { describe, it, expect } from 'vitest';
import { server } from '@/test/mocks/server';
import { http, HttpResponse } from 'msw';

const API_BASE = 'http://localhost:18791';

// ============================================================================
// Dashboard Stats API Tests
// ============================================================================

describe('Dashboard Stats API', () => {
  it('returns stats from /api/executions/stats/counts', async () => {
    const response = await fetch(`${API_BASE}/api/executions/stats/counts`);
    expect(response.ok).toBe(true);

    const stats = await response.json();

    expect(stats).toHaveProperty('sessions_running');
    expect(stats).toHaveProperty('sessions_queued');
    expect(stats).toHaveProperty('sessions_completed');
    expect(stats).toHaveProperty('executions_running');
    expect(stats).toHaveProperty('sessions_by_source');
    expect(typeof stats.sessions_running).toBe('number');
  });

  it('has sessions_by_source breakdown', async () => {
    const response = await fetch(`${API_BASE}/api/executions/stats/counts`);
    const stats = await response.json();

    expect(stats.sessions_by_source).toHaveProperty('web');
    expect(stats.sessions_by_source).toHaveProperty('cli');
    expect(stats.sessions_by_source).toHaveProperty('cron');
  });

  it('handles API errors', async () => {
    server.use(
      http.get(`${API_BASE}/api/executions/stats/counts`, () => {
        return new HttpResponse(null, { status: 500 });
      })
    );

    const response = await fetch(`${API_BASE}/api/executions/stats/counts`);
    expect(response.ok).toBe(false);
    expect(response.status).toBe(500);
  });

  it('handles network errors gracefully', async () => {
    server.use(
      http.get(`${API_BASE}/api/executions/stats/counts`, () => {
        return HttpResponse.error();
      })
    );

    await expect(
      fetch(`${API_BASE}/api/executions/stats/counts`)
    ).rejects.toThrow();
  });
});

// ============================================================================
// Sessions V2 API Tests
// ============================================================================

describe('Sessions V2 API', () => {
  describe('listSessionsFull', () => {
    it('returns sessions array from /api/executions/v2/sessions/full', async () => {
      const response = await fetch(`${API_BASE}/api/executions/v2/sessions/full`);
      expect(response.ok).toBe(true);

      const sessions = await response.json();

      expect(Array.isArray(sessions)).toBe(true);
      expect(sessions.length).toBeGreaterThan(0);
    });

    it('each session has required fields', async () => {
      const response = await fetch(`${API_BASE}/api/executions/v2/sessions/full`);
      const sessions = await response.json();

      sessions.forEach((session: Record<string, unknown>) => {
        expect(session).toHaveProperty('id');
        expect(session).toHaveProperty('status');
        expect(session).toHaveProperty('source');
        expect(session).toHaveProperty('executions');
        expect(session).toHaveProperty('subagent_count');
        expect(Array.isArray(session.executions)).toBe(true);
      });
    });

    it('executions have required fields', async () => {
      const response = await fetch(`${API_BASE}/api/executions/v2/sessions/full`);
      const sessions = await response.json();

      sessions.forEach((session: Record<string, unknown>) => {
        const executions = session.executions as Record<string, unknown>[];
        executions.forEach((exec) => {
          expect(exec).toHaveProperty('id');
          expect(exec).toHaveProperty('session_id');
          expect(exec).toHaveProperty('agent_id');
          expect(exec).toHaveProperty('delegation_type');
          expect(exec).toHaveProperty('status');
        });
      });
    });

    it('handles empty sessions', async () => {
      server.use(
        http.get(`${API_BASE}/api/executions/v2/sessions/full`, () => {
          return HttpResponse.json([]);
        })
      );

      const response = await fetch(`${API_BASE}/api/executions/v2/sessions/full`);
      const sessions = await response.json();

      expect(sessions).toEqual([]);
    });

    it('handles filter parameters', async () => {
      const params = new URLSearchParams({
        status: 'running',
        limit: '10',
      });

      const response = await fetch(
        `${API_BASE}/api/executions/v2/sessions/full?${params}`
      );
      expect(response.ok).toBe(true);
    });
  });

  describe('getSessionFull', () => {
    it('returns single session with executions', async () => {
      const sessionId = 'sess-test-123';
      const response = await fetch(
        `${API_BASE}/api/executions/v2/sessions/${sessionId}/full`
      );
      expect(response.ok).toBe(true);

      const session = await response.json();
      expect(session).toHaveProperty('id');
      expect(session).toHaveProperty('executions');
    });
  });
});

// ============================================================================
// Gateway Submit API Tests
// ============================================================================

describe('Gateway Submit API', () => {
  it('creates new session', async () => {
    const response = await fetch(`${API_BASE}/api/gateway/submit`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        agent_id: 'root',
        message: 'Hello!',
      }),
    });

    expect(response.ok).toBe(true);

    const handle = await response.json();

    expect(handle).toHaveProperty('session_id');
    expect(handle).toHaveProperty('execution_id');
    expect(handle).toHaveProperty('conversation_id');
    expect(handle.session_id).toMatch(/^sess-/);
    expect(handle.execution_id).toMatch(/^exec-/);
  });

  it('accepts source parameter', async () => {
    const response = await fetch(`${API_BASE}/api/gateway/submit`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        agent_id: 'root',
        message: 'Hello from connector!',
        source: 'connector',
      }),
    });

    expect(response.ok).toBe(true);
  });

  it('accepts conversation_id parameter', async () => {
    const customConvId = 'custom-conv-123';
    const response = await fetch(`${API_BASE}/api/gateway/submit`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        agent_id: 'root',
        message: 'Test',
        conversation_id: customConvId,
      }),
    });

    const handle = await response.json();
    expect(handle.conversation_id).toBe(customConvId);
  });

  it('handles missing agent_id', async () => {
    server.use(
      http.post(`${API_BASE}/api/gateway/submit`, () => {
        return HttpResponse.json(
          { error: 'agent_id is required', code: 'VALIDATION_ERROR' },
          { status: 400 }
        );
      })
    );

    const response = await fetch(`${API_BASE}/api/gateway/submit`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ message: 'No agent' }),
    });

    expect(response.ok).toBe(false);
    expect(response.status).toBe(400);
  });
});

// ============================================================================
// Gateway Status API Tests
// ============================================================================

describe('Gateway Status API', () => {
  it('returns session status', async () => {
    const sessionId = 'sess-test';
    const response = await fetch(`${API_BASE}/api/gateway/status/${sessionId}`);

    expect(response.ok).toBe(true);

    const status = await response.json();
    expect(status).toHaveProperty('session_id');
    expect(status).toHaveProperty('status');
    expect(status.session_id).toBe(sessionId);
  });

  it('returns 404 for non-existent session', async () => {
    server.use(
      http.get(`${API_BASE}/api/gateway/status/:session_id`, () => {
        return HttpResponse.json(
          { error: 'Session not found', code: 'SESSION_NOT_FOUND' },
          { status: 404 }
        );
      })
    );

    const response = await fetch(`${API_BASE}/api/gateway/status/nonexistent`);
    expect(response.status).toBe(404);
  });
});

// ============================================================================
// Health and Status API Tests
// ============================================================================

describe('Health and Status API', () => {
  it('returns health check', async () => {
    const response = await fetch(`${API_BASE}/api/health`);
    expect(response.ok).toBe(true);

    const health = await response.json();
    expect(health).toHaveProperty('status');
    expect(health).toHaveProperty('version');
    expect(health.status).toBe('ok');
  });

  it('returns gateway status', async () => {
    const response = await fetch(`${API_BASE}/api/status`);
    expect(response.ok).toBe(true);

    const status = await response.json();
    expect(status).toHaveProperty('status', 'ok');
    expect(status).toHaveProperty('version', '0.1.0');
    expect(status).toHaveProperty('agentCount', 2);
  });
});

// ============================================================================
// Agents API Tests
// ============================================================================

describe('Agents API', () => {
  it('lists agents', async () => {
    const response = await fetch(`${API_BASE}/api/agents`);
    expect(response.ok).toBe(true);

    const agents = await response.json();
    expect(Array.isArray(agents)).toBe(true);
    expect(agents.length).toBeGreaterThan(0);

    const agent = agents[0];
    expect(agent).toHaveProperty('id');
    expect(agent).toHaveProperty('name');
    expect(agent).toHaveProperty('displayName');
    expect(agent).toHaveProperty('providerId');
    expect(agent).toHaveProperty('model');
  });
});

// ============================================================================
// Conversations API Tests
// ============================================================================

describe('Conversations API', () => {
  it('lists conversations', async () => {
    const response = await fetch(`${API_BASE}/api/conversations`);
    expect(response.ok).toBe(true);

    const conversations = await response.json();
    expect(Array.isArray(conversations)).toBe(true);

    if (conversations.length > 0) {
      const conv = conversations[0];
      expect(conv).toHaveProperty('id');
      expect(conv).toHaveProperty('agentId');
      expect(conv).toHaveProperty('messageCount');
    }
  });
});

// ============================================================================
// Error Handling Tests
// ============================================================================

describe('Error Handling', () => {
  it('handles 401 Unauthorized', async () => {
    server.use(
      http.get(`${API_BASE}/api/agents`, () => {
        return HttpResponse.json(
          { error: 'Unauthorized' },
          { status: 401 }
        );
      })
    );

    const response = await fetch(`${API_BASE}/api/agents`);
    expect(response.status).toBe(401);
  });

  it('handles 404 Not Found', async () => {
    server.use(
      http.get(`${API_BASE}/api/agents/:id`, () => {
        return HttpResponse.json(
          { error: 'Agent not found' },
          { status: 404 }
        );
      })
    );

    const response = await fetch(`${API_BASE}/api/agents/nonexistent`);
    expect(response.status).toBe(404);
  });

  it('handles 500 Internal Server Error', async () => {
    server.use(
      http.get(`${API_BASE}/api/health`, () => {
        return HttpResponse.json(
          { error: 'Internal server error' },
          { status: 500 }
        );
      })
    );

    const response = await fetch(`${API_BASE}/api/health`);
    expect(response.status).toBe(500);
  });

  it('handles malformed JSON response', async () => {
    server.use(
      http.get(`${API_BASE}/api/health`, () => {
        return new HttpResponse('not json', {
          headers: { 'Content-Type': 'text/plain' },
        });
      })
    );

    const response = await fetch(`${API_BASE}/api/health`);
    const text = await response.text();
    expect(text).toBe('not json');
  });
});

// ============================================================================
// Connector Inbound Log API Tests
// ============================================================================

describe('Connector Inbound Log API', () => {
  it('returns inbound log entries', async () => {
    const response = await fetch(`${API_BASE}/api/connectors/test-connector/inbound-log`);
    expect(response.ok).toBe(true);

    const entries = await response.json();
    expect(Array.isArray(entries)).toBe(true);
    expect(entries.length).toBeGreaterThan(0);
  });

  it('each entry has required fields', async () => {
    const response = await fetch(`${API_BASE}/api/connectors/test-connector/inbound-log`);
    const entries = await response.json();

    entries.forEach((entry: Record<string, unknown>) => {
      expect(entry).toHaveProperty('connector_id');
      expect(entry).toHaveProperty('message');
      expect(entry).toHaveProperty('session_id');
      expect(entry).toHaveProperty('received_at');
    });
  });

  it('entries match requested connector id', async () => {
    const connectorId = 'my-connector';
    const response = await fetch(`${API_BASE}/api/connectors/${connectorId}/inbound-log`);
    const entries = await response.json();

    entries.forEach((entry: Record<string, unknown>) => {
      expect(entry.connector_id).toBe(connectorId);
    });
  });

  it('supports limit parameter', async () => {
    const response = await fetch(`${API_BASE}/api/connectors/test-connector/inbound-log?limit=10`);
    expect(response.ok).toBe(true);
  });

  it('handles empty log', async () => {
    server.use(
      http.get(`${API_BASE}/api/connectors/:id/inbound-log`, () => {
        return HttpResponse.json([]);
      })
    );

    const response = await fetch(`${API_BASE}/api/connectors/test-connector/inbound-log`);
    const entries = await response.json();
    expect(entries).toEqual([]);
  });
});

// ============================================================================
// Ward Actions API Tests — POST /api/wards/:ward_id/open (R14c)
// ============================================================================

describe('Ward Open API', () => {
  it('POSTs to /api/wards/:ward_id/open and returns the resolved path', async () => {
    const wardId = 'stock-analysis';
    let receivedMethod: string | null = null;
    let receivedPath: string | null = null;

    server.use(
      http.post(`${API_BASE}/api/wards/:ward_id/open`, ({ request, params }) => {
        receivedMethod = request.method;
        receivedPath = String(params.ward_id);
        return HttpResponse.json({ path: `/vault/wards/${params.ward_id}` });
      })
    );

    const response = await fetch(`${API_BASE}/api/wards/${wardId}/open`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({}),
    });

    expect(response.ok).toBe(true);
    expect(receivedMethod).toBe('POST');
    expect(receivedPath).toBe(wardId);

    const body = await response.json();
    expect(body).toHaveProperty('path');
    expect(body.path).toBe(`/vault/wards/${wardId}`);
  });

  it('URL-encodes the ward id segment', async () => {
    const wardId = 'project/alpha beta';
    let capturedParam: string | null = null;

    server.use(
      http.post(`${API_BASE}/api/wards/:ward_id/open`, ({ params }) => {
        capturedParam = String(params.ward_id);
        return HttpResponse.json({ path: '/vault/wards/x' });
      })
    );

    await fetch(`${API_BASE}/api/wards/${encodeURIComponent(wardId)}/open`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({}),
    });

    expect(capturedParam).toBe(wardId);
  });

  it('surfaces 404 when the ward folder does not exist', async () => {
    server.use(
      http.post(`${API_BASE}/api/wards/:ward_id/open`, () => {
        return HttpResponse.json(
          { error: "Ward 'ghost' has no folder on disk" },
          { status: 404 }
        );
      })
    );

    const response = await fetch(`${API_BASE}/api/wards/ghost/open`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({}),
    });

    expect(response.ok).toBe(false);
    expect(response.status).toBe(404);
    const body = await response.json();
    expect(body.error).toMatch(/ghost/);
  });

  it('surfaces 500 when the launcher fails to spawn', async () => {
    server.use(
      http.post(`${API_BASE}/api/wards/:ward_id/open`, () => {
        return HttpResponse.json(
          { error: 'Failed to open folder with xdg-open: No such file or directory' },
          { status: 500 }
        );
      })
    );

    const response = await fetch(`${API_BASE}/api/wards/stock-analysis/open`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({}),
    });

    expect(response.status).toBe(500);
  });
});

// ============================================================================
// Request/Response Format Tests
// ============================================================================

describe('Request/Response Formats', () => {
  it('sends JSON content type for POST requests', async () => {
    let receivedContentType: string | null = null;

    server.use(
      http.post(`${API_BASE}/api/gateway/submit`, async ({ request }) => {
        receivedContentType = request.headers.get('content-type');
        return HttpResponse.json({
          session_id: 'sess-1',
          execution_id: 'exec-1',
          conversation_id: 'conv-1',
        });
      })
    );

    await fetch(`${API_BASE}/api/gateway/submit`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ agent_id: 'root', message: 'test' }),
    });

    expect(receivedContentType).toBe('application/json');
  });

  it('receives JSON responses with proper content type', async () => {
    const response = await fetch(`${API_BASE}/api/health`);
    const contentType = response.headers.get('content-type');
    expect(contentType).toContain('application/json');
  });
});
