# Services

Standalone data services with minimal framework dependencies. Each service is independently testable.

## Crates

| Crate | Purpose | Storage |
|-------|---------|---------|
| `execution-state` | Session lifecycle, execution tracking, token consumption, dashboard stats | SQLite (via `StateDbProvider`) |
| `api-logs` | Execution logging and tracing with categories and filtering | SQLite (via `DbProvider`) |
| `knowledge-graph` | Entity/relationship type definitions, extraction, name resolution | Types only; SQLite storage in `zbot-stores-sqlite` |
| `daily-sessions` | Daily session continuity with message archiving | SQLite + moka cache |

## Build & Test

```bash
cargo test -p execution-state    # 82 tests
cargo test -p api-logs
cargo test -p knowledge-graph    # 19 tests
cargo test -p daily-sessions     # 16 tests
```

## Design Pattern

Services expose traits that the gateway implements:

```rust
// In execution-state
pub trait StateDbProvider {
    fn get_connection(&self) -> Result<PooledConnection>;
}

// In api-logs
pub trait DbProvider {
    fn get_connection(&self) -> &Connection;
}

// Gateway's DatabaseManager (in zbot-stores-sqlite) implements both traits
```

This inverts dependencies — services don't depend on the gateway; the gateway depends on services.

## execution-state

The most critical service. Tracks sessions, agent executions, delegations, ward assignments, and token usage.

**Key types**: `Session`, `AgentExecution`, `SessionStatus`, `ExecutionStatus`, `DelegationType`, `TriggerSource`, `DashboardStats`, `Checkpoint`

**HTTP routes** (mounted by gateway at `/api`):
- `GET /v2/sessions` — List sessions
- `GET /v2/sessions/:id/full` — Session with all executions
- `POST /sessions/:id/{pause,resume,cancel}` — Control operations
- `GET /executions/:id` — Execution details
- `GET /stats` — Dashboard statistics

## api-logs

Execution tracing with structured log entries. Each entry has level, category, timestamp, and metadata.

**Key types**: `ExecutionLog`, `LogLevel`, `LogCategory`, `LogFilter`, `LogSession`

**Categories**: Session, Token, ToolCall, ToolResult, Thinking, Delegation, System, Error

## knowledge-graph

Entity type definitions, extraction, and name resolution. **Storage was relocated to `zbot-stores-sqlite::kg`** (Slice D6b).

**Key types**: `Entity`, `Relationship`, `EntityType`, `RelationshipType`, `ExtractedKnowledge`, `ResolveOutcome`

**Public API**: `EntityExtractor::extract_from_message()`, `resolve()`, `normalize_name()`

## daily-sessions

Manages daily conversation sessions with context continuity and system prompt version tracking.

**Key types**: `DailySession`, `SessionMessage`, `DaySummary`, `Agent`, `SystemPromptCheck`
