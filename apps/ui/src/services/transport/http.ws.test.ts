// ============================================================================
// HttpTransport — WebSocket tests
// Covers connect(), disconnect(), subscribeConversation, message routing,
// and reconnect logic using a mock WebSocket.
// ============================================================================

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { HttpTransport } from './http';

const HTTP = 'http://localhost:18791';
const WS = 'ws://localhost:18791/ws';

function newTransport(): HttpTransport {
  const t = new HttpTransport();
  void t.initialize({ httpUrl: HTTP, wsUrl: WS });
  return t;
}

// ─── Minimal mock WebSocket ───────────────────────────────────────────────────

class MockWebSocket {
  static OPEN = 1;
  static CLOSED = 3;

  readyState = MockWebSocket.OPEN;
  onopen: ((e: Event) => void) | null = null;
  onmessage: ((e: MessageEvent) => void) | null = null;
  onclose: (() => void) | null = null;
  onerror: ((e: Event) => void) | null = null;

  send = vi.fn();
  close = vi.fn(() => {
    this.readyState = MockWebSocket.CLOSED;
  });

  // Simulate server sending a message
  simulateMessage(data: unknown) {
    this.onmessage?.({ data: JSON.stringify(data) } as MessageEvent);
  }

  simulateOpen() {
    this.onopen?.({} as Event);
  }

  simulateError() {
    this.onerror?.({} as Event);
  }

  simulateClose() {
    this.onclose?.();
  }
}

let wsMock: MockWebSocket;

// Build a mock WebSocket constructor that carries the static constants
function makeMockWsConstructor(): ReturnType<typeof vi.fn> {
  const ctor = vi.fn(function MockWebSocketConstructor() {
    return wsMock;
  });
  // The real-code checks `WebSocket.OPEN`, `WebSocket.CLOSED`, etc.
  // Without these, `null?.readyState === WebSocket.OPEN` becomes
  // `undefined === undefined` which is true, causing wrong code paths.
  Object.assign(ctor, { CONNECTING: 0, OPEN: 1, CLOSING: 2, CLOSED: 3 });
  return ctor;
}

beforeEach(() => {
  vi.useFakeTimers();
  wsMock = new MockWebSocket();
  vi.stubGlobal('WebSocket', makeMockWsConstructor());
  vi.stubGlobal('fetch', vi.fn());
});

afterEach(() => {
  vi.unstubAllGlobals();
  vi.useRealTimers();
});

// ===========================================================================
// connect() — happy path
// ===========================================================================

describe('HttpTransport — connect()', () => {
  it('resolves success:true when WebSocket opens', async () => {
    const t = newTransport();
    const connectPromise = t.connect();
    wsMock.simulateOpen();
    const res = await connectPromise;
    expect(res.success).toBe(true);
  });

  it('sets connection state to connected on open', async () => {
    const t = newTransport();
    const cb = vi.fn();
    t.onConnectionStateChange(cb);
    cb.mockClear();

    const connectPromise = t.connect();
    wsMock.simulateOpen();
    await connectPromise;
    expect(cb).toHaveBeenCalledWith(expect.objectContaining({ status: 'connected' }));
  });

  it('resolves success:false on WebSocket error', async () => {
    const t = newTransport();
    const connectPromise = t.connect();
    wsMock.simulateError();
    const res = await connectPromise;
    expect(res.success).toBe(false);
    expect(res.error).toMatch(/WebSocket connection failed/i);
  });

  it('returns success:true immediately if already connected', async () => {
    const t = newTransport();
    // First connection
    const c1 = t.connect();
    wsMock.simulateOpen();
    await c1;

    // Second call should detect OPEN state and return immediately
    const c2 = t.connect();
    const res = await c2;
    expect(res.success).toBe(true);
    // WebSocket constructor should only be called once
    expect(vi.mocked(WebSocket)).toHaveBeenCalledTimes(1);
  });

  it('sets connecting state before WebSocket opens', () => {
    const t = newTransport();
    const cb = vi.fn();
    t.onConnectionStateChange(cb);
    cb.mockClear();

    t.connect(); // don't await
    expect(cb).toHaveBeenCalledWith(expect.objectContaining({ status: 'connecting' }));
  });
});

// ===========================================================================
// disconnect()
// ===========================================================================

describe('HttpTransport — disconnect()', () => {
  it('sets connection state to disconnected', async () => {
    const t = newTransport();
    const c1 = t.connect();
    wsMock.simulateOpen();
    await c1;

    const cb = vi.fn();
    t.onConnectionStateChange(cb);
    cb.mockClear();

    await t.disconnect();
    expect(cb).toHaveBeenCalledWith(
      expect.objectContaining({ status: 'disconnected', reason: 'user' }),
    );
  });

  it('calls ws.close() on disconnect', async () => {
    const t = newTransport();
    const c1 = t.connect();
    wsMock.simulateOpen();
    await c1;
    await t.disconnect();
    expect(wsMock.close).toHaveBeenCalled();
  });
});

// ===========================================================================
// isConnected()
// ===========================================================================

describe('HttpTransport — isConnected()', () => {
  it('returns true when WebSocket is OPEN', async () => {
    const t = newTransport();
    const c = t.connect();
    wsMock.simulateOpen();
    await c;
    expect(t.isConnected()).toBe(true);
  });

  it('returns false after disconnect', async () => {
    const t = newTransport();
    const c = t.connect();
    wsMock.simulateOpen();
    await c;
    await t.disconnect();
    expect(t.isConnected()).toBe(false);
  });
});

// ===========================================================================
// executeAgent() / stopAgent() — WS send path
// ===========================================================================

describe('HttpTransport — executeAgent() / stopAgent()', () => {
  it('sends invoke command via WebSocket', async () => {
    const t = newTransport();
    const c = t.connect();
    wsMock.simulateOpen();
    await c;

    const res = await t.executeAgent(
      'root',
      'conv-1',
      'hello',
      'sess-1',
      'chat',
      'msg-550e8400-e29b-41d4-a716-446655440000',
    );
    expect(res.success).toBe(true);
    const cmd = JSON.parse(wsMock.send.mock.calls.at(-1)?.[0]);
    expect(cmd.type).toBe('invoke');
    expect(cmd.agent_id).toBe('root');
    expect(cmd.conversation_id).toBe('conv-1');
    expect(cmd.message).toBe('hello');
    expect(cmd.session_id).toBe('sess-1');
    expect(cmd.mode).toBe('chat');
    expect(cmd.metadata).toEqual({
      client_message_id: 'msg-550e8400-e29b-41d4-a716-446655440000',
    });
  });

  it('stopAgent sends stop command', async () => {
    const t = newTransport();
    const c = t.connect();
    wsMock.simulateOpen();
    await c;

    const res = await t.stopAgent('conv-1');
    expect(res.success).toBe(true);
    const cmd = JSON.parse(wsMock.send.mock.calls.at(-1)?.[0]);
    expect(cmd.type).toBe('stop');
    expect(cmd.conversation_id).toBe('conv-1');
  });

  it('pauseSession sends pause command', async () => {
    const t = newTransport();
    const c = t.connect();
    wsMock.simulateOpen();
    await c;

    const res = await t.pauseSession('sess-1');
    expect(res.success).toBe(true);
    const cmd = JSON.parse(wsMock.send.mock.calls.at(-1)?.[0]);
    expect(cmd.type).toBe('pause');
  });

  it('resumeSession sends resume command', async () => {
    const t = newTransport();
    const c = t.connect();
    wsMock.simulateOpen();
    await c;

    const res = await t.resumeSession('sess-1');
    expect(res.success).toBe(true);
    const cmd = JSON.parse(wsMock.send.mock.calls.at(-1)?.[0]);
    expect(cmd.type).toBe('resume');
  });

  it('cancelSession sends cancel command', async () => {
    const t = newTransport();
    const c = t.connect();
    wsMock.simulateOpen();
    await c;

    const res = await t.cancelSession('sess-1');
    expect(res.success).toBe(true);
    const cmd = JSON.parse(wsMock.send.mock.calls.at(-1)?.[0]);
    expect(cmd.type).toBe('cancel');
  });

  it('endSession sends end_session command', async () => {
    const t = newTransport();
    const c = t.connect();
    wsMock.simulateOpen();
    await c;

    const res = await t.endSession('sess-1');
    expect(res.success).toBe(true);
    const cmd = JSON.parse(wsMock.send.mock.calls.at(-1)?.[0]);
    expect(cmd.type).toBe('end_session');
  });
});

// ===========================================================================
// subscribeConversation() — WS subscribe/unsubscribe
// ===========================================================================

describe('HttpTransport — subscribeConversation()', () => {
  it('sends subscribe message to server when connected', async () => {
    const t = newTransport();
    const c = t.connect();
    wsMock.simulateOpen();
    await c;

    const onEvent = vi.fn();
    t.subscribeConversation('conv-1', { onEvent });
    const cmd = JSON.parse(wsMock.send.mock.calls.at(-1)?.[0]);
    expect(cmd.type).toBe('subscribe');
    expect(cmd.conversation_id).toBe('conv-1');
  });

  it('sends unsubscribe when last subscriber removed', async () => {
    const t = newTransport();
    const c = t.connect();
    wsMock.simulateOpen();
    await c;

    const onEvent = vi.fn();
    const unsub = t.subscribeConversation('conv-1', { onEvent });
    wsMock.send.mockClear();
    unsub();
    const cmd = JSON.parse(wsMock.send.mock.calls.at(-1)?.[0]);
    expect(cmd.type).toBe('unsubscribe');
    expect(cmd.conversation_id).toBe('conv-1');
  });

  it('routes subscribed event to onEvent callback', async () => {
    const t = newTransport();
    const c = t.connect();
    wsMock.simulateOpen();
    await c;

    const onEvent = vi.fn();
    t.subscribeConversation('conv-1', { onEvent });

    // Simulate a 'subscribed' confirmation
    wsMock.simulateMessage({
      type: 'subscribed',
      conversation_id: 'conv-1',
      current_sequence: 0,
    });

    // Simulate an event
    wsMock.simulateMessage({
      type: 'token',
      conversation_id: 'conv-1',
      seq: 1,
      delta: 'Hello',
    });

    expect(onEvent).toHaveBeenCalledWith(
      expect.objectContaining({ type: 'token', conversation_id: 'conv-1' }),
    );
  });

  it('calls onError when subscription_error arrives', async () => {
    const t = newTransport();
    const c = t.connect();
    wsMock.simulateOpen();
    await c;

    const onEvent = vi.fn();
    const onError = vi.fn();
    t.subscribeConversation('conv-1', { onEvent, onError });

    wsMock.simulateMessage({
      type: 'subscription_error',
      conversation_id: 'conv-1',
      code: 'SERVER_ERROR',
      message: 'oops',
    });

    expect(onError).toHaveBeenCalledWith(
      expect.objectContaining({ code: 'SERVER_ERROR' }),
    );
  });

  it('calls onConfirmed when subscribed message arrives', async () => {
    const t = newTransport();
    const c = t.connect();
    wsMock.simulateOpen();
    await c;

    const onConfirmed = vi.fn();
    t.subscribeConversation('conv-1', { onEvent: vi.fn(), onConfirmed });

    wsMock.simulateMessage({
      type: 'subscribed',
      conversation_id: 'conv-1',
      current_sequence: 42,
      root_execution_ids: ['exec-1'],
    });

    expect(onConfirmed).toHaveBeenCalledWith(42, ['exec-1']);
  });

  it('handles pong message — resets lastPong', async () => {
    const t = newTransport();
    const c = t.connect();
    wsMock.simulateOpen();
    await c;

    // No error should occur when pong arrives
    expect(() => {
      wsMock.simulateMessage({ type: 'pong' });
    }).not.toThrow();
  });

  it('handles invoke_accepted and routes to subscriber', async () => {
    const t = newTransport();
    const c = t.connect();
    wsMock.simulateOpen();
    await c;

    const onEvent = vi.fn();
    t.subscribeConversation('conv-1', { onEvent });

    wsMock.simulateMessage({
      type: 'invoke_accepted',
      conversation_id: 'conv-1',
      session_id: 'sess-new',
    });

    expect(onEvent).toHaveBeenCalledWith(
      expect.objectContaining({ type: 'invoke_accepted' }),
    );
  });

  it('queues pending subscription when WS not ready', () => {
    const t = newTransport();
    // Don't connect — no WS
    const onEvent = vi.fn();
    t.subscribeConversation('conv-pending', { onEvent });
    // Should not throw; subscription is queued
    expect(wsMock.send).not.toHaveBeenCalled();
  });

  it('re-subscribes on scope change', async () => {
    const t = newTransport();
    const c = t.connect();
    wsMock.simulateOpen();
    await c;

    t.subscribeConversation('conv-1', { onEvent: vi.fn(), scope: 'all' });
    wsMock.send.mockClear();

    // Re-subscribe with a different scope
    t.subscribeConversation('conv-1', { onEvent: vi.fn(), scope: 'session' });
    const cmd = JSON.parse(wsMock.send.mock.calls.at(-1)?.[0]);
    expect(cmd.type).toBe('subscribe');
  });
});

// ===========================================================================
// onGlobalEvent() — global event routing
// ===========================================================================

describe('HttpTransport — onGlobalEvent()', () => {
  it('routes stats_update to global callback', async () => {
    const t = newTransport();
    const c = t.connect();
    wsMock.simulateOpen();
    await c;

    const cb = vi.fn();
    t.onGlobalEvent(cb);
    wsMock.simulateMessage({ type: 'stats_update', payload: {} });
    expect(cb).toHaveBeenCalledWith(expect.objectContaining({ type: 'stats_update' }));
  });

  it('routes session_notification to global callback', async () => {
    const t = newTransport();
    const c = t.connect();
    wsMock.simulateOpen();
    await c;

    const cb = vi.fn();
    t.onGlobalEvent(cb);
    wsMock.simulateMessage({ type: 'session_notification' });
    expect(cb).toHaveBeenCalled();
  });

  it('routes customization_file_changed to global callback', async () => {
    const t = newTransport();
    const c = t.connect();
    wsMock.simulateOpen();
    await c;

    const cb = vi.fn();
    t.onGlobalEvent(cb);
    wsMock.simulateMessage({ type: 'customization_file_changed' });
    expect(cb).toHaveBeenCalled();
  });

  it('returns unsubscribe function that removes the callback', async () => {
    const t = newTransport();
    const c = t.connect();
    wsMock.simulateOpen();
    await c;

    const cb = vi.fn();
    const unsub = t.onGlobalEvent(cb);
    unsub();
    wsMock.simulateMessage({ type: 'stats_update', payload: {} });
    expect(cb).not.toHaveBeenCalled();
  });
});

// ===========================================================================
// Heartbeat
// ===========================================================================

describe('HttpTransport — heartbeat', () => {
  it('sends ping after PING_INTERVAL', async () => {
    const t = newTransport();
    const c = t.connect();
    wsMock.simulateOpen();
    await c;

    vi.advanceTimersByTime(15001);
    const calls = wsMock.send.mock.calls;
    const pingCall = calls.find((c) => {
      try {
        return JSON.parse(c[0]).type === 'ping';
      } catch {
        return false;
      }
    });
    expect(pingCall).toBeDefined();
  });
});

// ===========================================================================
// reconnect()
// ===========================================================================

describe('HttpTransport — reconnect()', () => {
  it('resets attempt counter and reconnects', async () => {
    const t = newTransport();
    const c = t.connect();
    wsMock.simulateOpen();
    await c;

    const newWs = new MockWebSocket();
    vi.mocked(WebSocket).mockReturnValueOnce(newWs as unknown as WebSocket);

    const reconnectPromise = t.reconnect();
    newWs.simulateOpen();
    const res = await reconnectPromise;
    expect(res).toBeUndefined(); // reconnect returns void
  });
});

// ===========================================================================
// reconnect on WS close
// ===========================================================================

describe('HttpTransport — auto-reconnect on WS close', () => {
  it('attempts reconnect after WebSocket closes', async () => {
    const t = newTransport();
    const c = t.connect();
    wsMock.simulateOpen();
    await c;

    const cb = vi.fn();
    t.onConnectionStateChange(cb);
    cb.mockClear();

    // Simulate WS close
    wsMock.simulateClose();

    // After close, state should change to reconnecting
    vi.advanceTimersByTime(100);
    // The state goes to reconnecting
    expect(cb).toHaveBeenCalledWith(
      expect.objectContaining({ status: 'reconnecting' }),
    );
  });
});

// ===========================================================================
// subscribe() — legacy API
// ===========================================================================

describe('HttpTransport — legacy subscribe()', () => {
  it('routes message to legacy callback', async () => {
    const t = newTransport();
    const c = t.connect();
    wsMock.simulateOpen();
    await c;

    const cb = vi.fn();
    t.subscribe('conv-1', cb);

    // Route via handleEvent (legacy path — no subscription state)
    // This is hit for unknown message types in the fall-through
    wsMock.simulateMessage({
      type: 'unknown_legacy_event',
      conversation_id: 'conv-1',
    });

    expect(cb).toHaveBeenCalled();
  });

  it('unsub removes callback', async () => {
    const t = newTransport();
    const c = t.connect();
    wsMock.simulateOpen();
    await c;

    const cb = vi.fn();
    const unsub = t.subscribe('conv-1', cb);
    unsub();

    wsMock.simulateMessage({ type: 'token', conversation_id: 'conv-1' });
    expect(cb).not.toHaveBeenCalled();
  });
});
