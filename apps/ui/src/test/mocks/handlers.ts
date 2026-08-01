import { http, HttpResponse } from 'msw';
import type {
  DashboardStats,
  SessionWithExecutions,
  TriggerSource,
} from '@/services/transport/types';

const API_BASE = 'http://localhost:18791';

// Mock data factories
export function createMockSession(
  id: string,
  status: 'queued' | 'running' | 'paused' | 'completed' | 'crashed' = 'running',
  source: TriggerSource = 'web'
): SessionWithExecutions {
  const now = new Date().toISOString();
  return {
    id,
    status,
    source,
    root_agent_id: 'root',
    created_at: now,
    started_at: status !== 'queued' ? now : undefined,
    completed_at: status === 'completed' ? now : undefined,
    total_tokens_in: 1000,
    total_tokens_out: 500,
    executions: [
      {
        id: `exec-${id}`,
        session_id: id,
        agent_id: 'root',
        delegation_type: 'root',
        status: status === 'completed' ? 'completed' : 'running',
        started_at: now,
        completed_at: status === 'completed' ? now : undefined,
        tokens_in: 1000,
        tokens_out: 500,
      },
    ],
    subagent_count: 0,
  };
}

export function createMockStats(): DashboardStats {
  return {
    sessions_queued: 1,
    sessions_running: 2,
    sessions_paused: 0,
    sessions_completed: 10,
    sessions_crashed: 1,
    executions_queued: 0,
    executions_running: 3,
    executions_completed: 15,
    executions_crashed: 2,
    executions_cancelled: 0,
    today_sessions: 5,
    today_tokens: 50000,
    sessions_by_source: {
      web: 8,
      cli: 3,
      cron: 1,
      api: 1,
      connector: 1,
    },
  };
}

// Default handlers
export const handlers = [
  // Dashboard stats (V2 API)
  http.get(`${API_BASE}/api/executions/stats`, () => {
    return HttpResponse.json(createMockStats());
  }),

  // Legacy stats endpoint (for backwards compatibility)
  http.get(`${API_BASE}/api/executions/stats/counts`, () => {
    return HttpResponse.json(createMockStats());
  }),

  // Sessions list (V2 full)
  http.get(`${API_BASE}/api/executions/v2/sessions/full`, () => {
    return HttpResponse.json([
      createMockSession('sess-001', 'running', 'web'),
      createMockSession('sess-002', 'running', 'cli'),
      createMockSession('sess-003', 'completed', 'web'),
    ]);
  }),

  // Session detail
  http.get(`${API_BASE}/api/executions/v2/sessions/:id/full`, ({ params }) => {
    const { id } = params;
    return HttpResponse.json(createMockSession(id as string, 'running', 'web'));
  }),

  // Gateway submit
  http.post(`${API_BASE}/api/gateway/submit`, async ({ request }) => {
    const body = (await request.json()) as Record<string, unknown>;
    return HttpResponse.json({
      session_id: `sess-${Date.now()}`,
      execution_id: `exec-${Date.now()}`,
      conversation_id: body.conversation_id || `conv-${Date.now()}`,
    });
  }),

  // Gateway status
  http.get(`${API_BASE}/api/gateway/status/:session_id`, ({ params }) => {
    return HttpResponse.json({
      session_id: params.session_id,
      status: 'running',
    });
  }),

  // Health check
  http.get(`${API_BASE}/api/health`, () => {
    return HttpResponse.json({
      status: 'ok',
      version: '0.1.0',
      uptime: 3600,
    });
  }),

  // Status
  http.get(`${API_BASE}/api/status`, () => {
    return HttpResponse.json({
      status: 'ok',
      version: '0.1.0',
      agentCount: 2,
    });
  }),

  // Agents list
  http.get(`${API_BASE}/api/agents`, () => {
    return HttpResponse.json([
      {
        id: 'root',
        name: 'root',
        displayName: 'Root Agent',
        description: 'Main agent',
        providerId: 'anthropic',
        model: 'claude-sonnet-4-20250514',
        temperature: 0.7,
        maxTokens: 8192,
        thinkingEnabled: true,
        voiceRecordingEnabled: false,
        instructions: 'You are a helpful assistant.',
        mcps: [],
        skills: [],
      },
    ]);
  }),

  // Connector inbound log
  http.get(`${API_BASE}/api/connectors/:id/inbound-log`, ({ params }) => {
    const connectorId = params.id as string;
    return HttpResponse.json([
      { connector_id: connectorId, message: 'test message 1', session_id: 'sess-001', received_at: new Date().toISOString() },
      { connector_id: connectorId, message: 'test message 2', session_id: 'sess-002', received_at: new Date().toISOString() },
    ]);
  }),

  // Conversations list
  http.get(`${API_BASE}/api/conversations`, () => {
    return HttpResponse.json([
      {
        id: 'conv-001',
        agentId: 'root',
        title: 'Test conversation',
        createdAt: new Date().toISOString(),
        updatedAt: new Date().toISOString(),
        messageCount: 5,
      },
    ]);
  }),

  // Belief Network stats (Phase B-6) — default to disabled so existing
  // ObservatoryPage tests render the placeholder, not the full panel.
  http.get(`${API_BASE}/api/belief-network/stats`, () => {
    return HttpResponse.json({
      enabled: false,
      synthesizer: { latest: {}, history: [] },
      contradiction_detector: { latest: {}, history: [] },
      propagator: { latest: {}, history: [] },
      totals: {
        total_beliefs: 0,
        total_contradictions: 0,
        total_unresolved_contradictions: 0,
      },
    });
  }),

  http.get(`${API_BASE}/api/belief-network/activity`, () => {
    return HttpResponse.json([]);
  }),

];
