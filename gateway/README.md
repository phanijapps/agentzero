# Gateway

HTTP and WebSocket gateway for the AgentZero daemon.

## Overview

The gateway provides network interfaces for clients to interact with the agent runtime:

- **HTTP API** (port 18791) - REST endpoints plus the primary `/ws` WebSocket upgrade route.
- **Legacy WebSocket port** (18790) - Off by default; only bound when legacy standalone WebSocket support is explicitly enabled.

## Architecture

```
┌─────────────────────────────────────────┐
│              Gateway                     │
├─────────────────────────────────────────┤
│  HTTP :18791 REST + /ws                 │
│  Legacy WS :18790 only when enabled     │
├─────────────────────────────────────────┤
│           Event Bus (broadcast)         │
└─────────────────────────────────────────┘
             │
             ▼
    ┌─────────────────┐
    │  Agent Runtime  │
    └─────────────────┘
```

## Modules

- `config` - Gateway configuration
- `error` - Error types
- `server` - Server lifecycle management
- `http/` - HTTP API endpoints
- `websocket/` - WebSocket handler and session management
- `events/` - Event bus for broadcasting

## HTTP API Endpoints

### Health
- `GET /api/health` - Basic health check
- `GET /api/status` - Detailed status

### Agents
- `GET /api/agents` - List all agents
- `POST /api/agents` - Create agent
- `GET /api/agents/:id` - Get agent
- `PUT /api/agents/:id` - Update agent
- `DELETE /api/agents/:id` - Delete agent

### Conversations
- `GET /api/conversations` - List conversations
- `POST /api/conversations` - Create conversation
- `GET /api/conversations/:id` - Get conversation
- `DELETE /api/conversations/:id` - Delete conversation
- `GET /api/conversations/:id/messages` - List messages

### Tools
- `GET /api/tools` - List available tools
- `GET /api/tools/:name` - Get tool schema

## WebSocket API

Connect to `ws://localhost:18791/ws?agent_id={agent_id}`. The standalone `18790` WebSocket listener is legacy and disabled unless explicitly enabled in gateway settings.

### Client Messages

```json
// Invoke agent
{ "type": "invoke", "conversation_id": "...", "message": "..." }

// Stop execution
{ "type": "stop", "conversation_id": "..." }

// Continue after limit
{ "type": "continue", "conversation_id": "..." }

// Keepalive
{ "type": "ping" }
```

### Server Messages

```json
// Streaming token
{ "type": "token", "conversation_id": "...", "delta": "..." }

// Tool call
{ "type": "tool_call", "conversation_id": "...", "tool": "...", "args": {...} }

// Tool result
{ "type": "tool_result", "conversation_id": "...", "result": "..." }

// Turn complete
{ "type": "turn_complete", "conversation_id": "..." }

// Error
{ "type": "error", "conversation_id": "...", "code": "...", "message": "..." }

// Iteration update
{ "type": "iteration", "conversation_id": "...", "current": 5, "max": 25 }
```

## Usage

```rust
use gateway::{GatewayConfig, GatewayServer};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = GatewayConfig::default();
    let mut server = GatewayServer::new(config);

    server.start().await?;

    // Wait for shutdown signal
    tokio::signal::ctrl_c().await?;
    server.shutdown().await;

    Ok(())
}
```
