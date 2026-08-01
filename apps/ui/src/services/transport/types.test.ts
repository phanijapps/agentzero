// ============================================================================
// TRANSPORT TYPES TESTS
// Unit tests for transport type definitions
// ============================================================================

import { describe, it, expect } from 'vitest';
import type {
  Session,
  AgentExecution,
  SessionWithExecutions,
  DashboardStats,
  TriggerSource,
  SessionStateStatus,
  ExecutionStatus,
  DelegationType,
  TransportConfig,
  TransportResult,
  SessionFilter,
  ExecutionFilter,
  BridgeWorker,
  BridgeWorkerCapability,
  BridgeWorkerResource,
} from './types';

// ============================================================================
// Session Type Tests
// ============================================================================

describe('Session Type', () => {
  it('has required fields', () => {
    const session: Session = {
      id: 'sess-123',
      status: 'running',
      source: 'web',
      root_agent_id: 'root',
      created_at: '2026-01-01T00:00:00Z',
      total_tokens_in: 1000,
      total_tokens_out: 500,
    };

    expect(session.id).toBe('sess-123');
    expect(session.status).toBe('running');
    expect(session.source).toBe('web');
    expect(session.root_agent_id).toBe('root');
  });

  it('supports optional fields', () => {
    const session: Session = {
      id: 'sess-123',
      status: 'crashed',
      source: 'cli',
      root_agent_id: 'root',
      created_at: '2026-01-01T00:00:00Z',
      started_at: '2026-01-01T00:00:01Z',
      completed_at: '2026-01-01T01:00:00Z',
      total_tokens_in: 5000,
      total_tokens_out: 3000,
      title: 'Test Session',
      metadata: { key: 'value' },
    };

    expect(session.started_at).toBeDefined();
    expect(session.completed_at).toBeDefined();
    expect(session.title).toBe('Test Session');
    expect(session.metadata).toEqual({ key: 'value' });
  });
});

// ============================================================================
// AgentExecution Type Tests
// ============================================================================

describe('AgentExecution Type', () => {
  it('has required fields', () => {
    const execution: AgentExecution = {
      id: 'exec-123',
      session_id: 'sess-123',
      agent_id: 'root',
      delegation_type: 'root',
      status: 'running',
      tokens_in: 500,
      tokens_out: 250,
    };

    expect(execution.id).toBe('exec-123');
    expect(execution.agent_id).toBe('root');
    expect(execution.delegation_type).toBe('root');
    expect(execution.status).toBe('running');
  });

  it('supports parent_execution_id for subagents', () => {
    const subagent: AgentExecution = {
      id: 'exec-456',
      session_id: 'sess-123',
      agent_id: 'researcher',
      parent_execution_id: 'exec-123',
      delegation_type: 'sequential',
      task: 'Research the topic',
      status: 'running',
      tokens_in: 200,
      tokens_out: 100,
      started_at: '2026-01-01T00:00:00Z',
    };

    expect(subagent.parent_execution_id).toBe('exec-123');
    expect(subagent.delegation_type).toBe('sequential');
    expect(subagent.task).toBe('Research the topic');
  });

  it('supports error field for crashed executions', () => {
    const crashed: AgentExecution = {
      id: 'exec-crash',
      session_id: 'sess-123',
      agent_id: 'root',
      delegation_type: 'root',
      status: 'crashed',
      tokens_in: 100,
      tokens_out: 0,
      error: 'Connection timeout',
    };

    expect(crashed.status).toBe('crashed');
    expect(crashed.error).toBe('Connection timeout');
  });
});

// ============================================================================
// SessionWithExecutions Type Tests
// ============================================================================

describe('SessionWithExecutions Type', () => {
  it('combines session with executions array', () => {
    const swe: SessionWithExecutions = {
      id: 'sess-123',
      status: 'running',
      source: 'web',
      root_agent_id: 'root',
      created_at: '2026-01-01T00:00:00Z',
      total_tokens_in: 1000,
      total_tokens_out: 500,
      executions: [
        {
          id: 'exec-root',
          session_id: 'sess-123',
          agent_id: 'root',
          delegation_type: 'root',
          status: 'completed',
          tokens_in: 600,
          tokens_out: 300,
        },
        {
          id: 'exec-sub',
          session_id: 'sess-123',
          agent_id: 'researcher',
          parent_execution_id: 'exec-root',
          delegation_type: 'sequential',
          status: 'running',
          tokens_in: 400,
          tokens_out: 200,
        },
      ],
      subagent_count: 1,
    };

    expect(swe.executions).toHaveLength(2);
    expect(swe.subagent_count).toBe(1);

    const rootExec = swe.executions.find(e => e.delegation_type === 'root');
    expect(rootExec?.agent_id).toBe('root');

    const subagent = swe.executions.find(e => e.parent_execution_id !== undefined);
    expect(subagent?.agent_id).toBe('researcher');
  });
});

// ============================================================================
// TriggerSource Type Tests
// ============================================================================

describe('TriggerSource Type', () => {
  it('supports all valid values', () => {
    const sources: TriggerSource[] = ['web', 'cli', 'cron', 'api', 'connector'];
    expect(sources).toHaveLength(5);

    // All sources should be strings
    sources.forEach(source => {
      expect(typeof source).toBe('string');
    });
  });

  it('can be used in typed objects', () => {
    const sourceMap: Record<TriggerSource, string> = {
      web: 'Web UI',
      cli: 'Command Line',
      cron: 'Scheduled',
      api: 'API Call',
      connector: 'Plugin',
    };

    expect(sourceMap.web).toBe('Web UI');
    expect(sourceMap.cron).toBe('Scheduled');
  });
});

// ============================================================================
// SessionStateStatus Type Tests
// ============================================================================

describe('SessionStateStatus Type', () => {
  it('supports all valid values', () => {
    const statuses: SessionStateStatus[] = [
      'queued',
      'running',
      'paused',
      'completed',
      'crashed',
    ];
    expect(statuses).toHaveLength(5);
  });

  it('distinguishes active from terminal states', () => {
    const activeStatuses: SessionStateStatus[] = ['queued', 'running', 'paused'];
    const terminalStatuses: SessionStateStatus[] = ['completed', 'crashed'];

    // This is how the dashboard categorizes them
    const isActive = (status: SessionStateStatus) =>
      activeStatuses.includes(status);
    const isTerminal = (status: SessionStateStatus) =>
      terminalStatuses.includes(status);

    expect(isActive('running')).toBe(true);
    expect(isActive('completed')).toBe(false);
    expect(isTerminal('crashed')).toBe(true);
    expect(isTerminal('paused')).toBe(false);
  });
});

// ============================================================================
// ExecutionStatus Type Tests
// ============================================================================

describe('ExecutionStatus Type', () => {
  it('supports all valid values', () => {
    const statuses: ExecutionStatus[] = [
      'queued',
      'running',
      'paused',
      'crashed',
      'cancelled',
      'completed',
    ];
    expect(statuses).toHaveLength(6);
  });

  it('has more states than SessionStateStatus', () => {
    // ExecutionStatus includes 'cancelled' which SessionStateStatus doesn't
    const execStatus: ExecutionStatus = 'cancelled';
    expect(execStatus).toBe('cancelled');
  });
});

// ============================================================================
// DelegationType Type Tests
// ============================================================================

describe('DelegationType Type', () => {
  it('supports all valid values', () => {
    const types: DelegationType[] = ['root', 'sequential', 'parallel'];
    expect(types).toHaveLength(3);
  });

  it('identifies root vs subagent executions', () => {
    const isRoot = (type: DelegationType) => type === 'root';
    const isSubagent = (type: DelegationType) => type !== 'root';

    expect(isRoot('root')).toBe(true);
    expect(isRoot('sequential')).toBe(false);
    expect(isSubagent('parallel')).toBe(true);
  });
});

// ============================================================================
// DashboardStats Type Tests
// ============================================================================

describe('DashboardStats Type', () => {
  it('has session and execution counts', () => {
    const stats: DashboardStats = {
      sessions_queued: 1,
      sessions_running: 2,
      sessions_paused: 0,
      sessions_completed: 10,
      sessions_crashed: 1,
      executions_queued: 0,
      executions_running: 3,
      executions_completed: 15,
      executions_crashed: 2,
      executions_cancelled: 1,
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

    expect(stats.sessions_running).toBe(2);
    expect(stats.executions_running).toBe(3);
    expect(stats.today_tokens).toBe(50000);
  });

  it('has sessions_by_source breakdown', () => {
    const stats: DashboardStats = {
      sessions_queued: 0,
      sessions_running: 0,
      sessions_paused: 0,
      sessions_completed: 14,
      sessions_crashed: 0,
      executions_queued: 0,
      executions_running: 0,
      executions_completed: 20,
      executions_crashed: 0,
      executions_cancelled: 0,
      today_sessions: 14,
      today_tokens: 100000,
      sessions_by_source: {
        web: 10,
        cli: 2,
        cron: 1,
        api: 1,
        connector: 0,
      },
    };

    expect(stats.sessions_by_source.web).toBe(10);
    expect(stats.sessions_by_source.cli).toBe(2);
    expect(stats.sessions_by_source.connector).toBe(0);

    // Total should match
    const total = Object.values(stats.sessions_by_source).reduce((a, b) => a + b, 0);
    expect(total).toBe(stats.sessions_completed);
  });
});

// ============================================================================
// TransportConfig Type Tests
// ============================================================================

describe('TransportConfig Type', () => {
  it('has required URL fields', () => {
    // Unified-port default: WebSocket shares the HTTP port via the /ws
    // upgrade path. Explicit custom URLs remain a valid TransportConfig
    // shape for development and reverse-proxy deployments.
    const config: TransportConfig = {
      httpUrl: 'http://localhost:18791',
      wsUrl: 'ws://localhost:18791/ws',
    };

    expect(config.httpUrl).toContain('18791');
    expect(config.wsUrl).toContain('/ws');
  });
});

// ============================================================================
// TransportResult Type Tests
// ============================================================================

describe('TransportResult Type', () => {
  it('represents successful result', () => {
    const success: TransportResult<string> = {
      success: true,
      data: 'test data',
    };

    expect(success.success).toBe(true);
    expect(success.data).toBe('test data');
    expect(success.error).toBeUndefined();
  });

  it('represents failed result', () => {
    const failure: TransportResult<string> = {
      success: false,
      error: 'Something went wrong',
    };

    expect(failure.success).toBe(false);
    expect(failure.error).toBe('Something went wrong');
    expect(failure.data).toBeUndefined();
  });

  it('works with complex types', () => {
    const result: TransportResult<SessionWithExecutions[]> = {
      success: true,
      data: [],
    };

    expect(result.success).toBe(true);
    expect(Array.isArray(result.data)).toBe(true);
  });
});

// ============================================================================
// Filter Type Tests
// ============================================================================

describe('Filter Types', () => {
  describe('SessionFilter', () => {
    it('supports all filter fields', () => {
      const filter: SessionFilter = {
        status: 'running',
        root_agent_id: 'root',
        from_time: '2026-01-01T00:00:00Z',
        to_time: '2026-01-31T23:59:59Z',
        limit: 50,
        offset: 0,
      };

      expect(filter.status).toBe('running');
      expect(filter.limit).toBe(50);
    });

    it('allows partial filters', () => {
      const filter: SessionFilter = {
        status: 'completed',
      };

      expect(filter.status).toBe('completed');
      expect(filter.limit).toBeUndefined();
    });
  });

  describe('ExecutionFilter', () => {
    it('supports all filter fields', () => {
      const filter: ExecutionFilter = {
        session_id: 'sess-123',
        agent_id: 'root',
        status: 'running',
        limit: 100,
        offset: 10,
      };

      expect(filter.session_id).toBe('sess-123');
      expect(filter.status).toBe('running');
    });
  });
});

// ============================================================================
// Bridge Worker Type Tests
// ============================================================================

describe('BridgeWorker Type', () => {
  it('has required fields', () => {
    const worker: BridgeWorker = {
      adapter_id: 'slack-worker',
      capabilities: [],
      resources: [],
      connected_at: '2026-01-01T00:00:00Z',
    };

    expect(worker.adapter_id).toBe('slack-worker');
    expect(worker.capabilities).toHaveLength(0);
    expect(worker.resources).toHaveLength(0);
    expect(worker.connected_at).toBeDefined();
  });

  it('supports capabilities and resources', () => {
    const worker: BridgeWorker = {
      adapter_id: 'crm-worker',
      capabilities: [
        { name: 'send_email', description: 'Send email via CRM' },
        { name: 'create_ticket', schema: { type: 'object' } },
      ],
      resources: [
        { name: 'contacts', description: 'CRM contacts' },
      ],
      connected_at: '2026-01-01T00:00:00Z',
    };

    expect(worker.capabilities).toHaveLength(2);
    expect(worker.capabilities[0].name).toBe('send_email');
    expect(worker.capabilities[0].description).toBe('Send email via CRM');
    expect(worker.resources).toHaveLength(1);
    expect(worker.resources[0].name).toBe('contacts');
  });
});

describe('BridgeWorkerCapability Type', () => {
  it('has required name field', () => {
    const cap: BridgeWorkerCapability = { name: 'send_message' };
    expect(cap.name).toBe('send_message');
    expect(cap.description).toBeUndefined();
    expect(cap.schema).toBeUndefined();
  });

  it('supports optional fields', () => {
    const cap: BridgeWorkerCapability = {
      name: 'create_ticket',
      description: 'Create a support ticket',
      schema: { type: 'object', properties: { title: { type: 'string' } } },
    };

    expect(cap.description).toBe('Create a support ticket');
    expect(cap.schema).toBeDefined();
  });
});

describe('BridgeWorkerResource Type', () => {
  it('has required name field', () => {
    const res: BridgeWorkerResource = { name: 'channels' };
    expect(res.name).toBe('channels');
    expect(res.description).toBeUndefined();
  });

  it('supports optional description', () => {
    const res: BridgeWorkerResource = {
      name: 'contacts',
      description: 'CRM contact list',
    };

    expect(res.description).toBe('CRM contact list');
  });
});
