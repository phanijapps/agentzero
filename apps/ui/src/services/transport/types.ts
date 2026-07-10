// ============================================================================
// TRANSPORT TYPES
// Shared types for the transport abstraction layer
// ============================================================================

// ============================================================================
// Transport Configuration
// ============================================================================

export interface TransportConfig {
  /** HTTP base URL for the gateway (e.g., http://localhost:18791) */
  httpUrl: string;
  /** WebSocket URL for the gateway (e.g., ws://localhost:18790) */
  wsUrl: string;
}

// ============================================================================
// Transport Result
// ============================================================================

export interface TransportResult<T> {
  success: boolean;
  data?: T;
  error?: string;
}

// ============================================================================
// Agent Types (for HTTP API)
// ============================================================================

export interface AgentResponse {
  id: string;
  name: string;
  displayName: string;
  description: string;
  providerId: string;
  model: string;
  temperature: number;
  maxInputTokens?: number;
  maxOutputTokens?: number;
  /** Legacy alias for maxOutputTokens. */
  maxTokens: number;
  thinkingEnabled: boolean;
  voiceRecordingEnabled: boolean;
  instructions: string;
  mcps: string[];
  skills: string[];
  middleware?: string;
  createdAt?: string;
}

export interface CreateAgentRequest {
  name: string;
  displayName?: string;
  description?: string;
  providerId: string;
  model: string;
  temperature?: number;
  maxInputTokens?: number;
  maxOutputTokens?: number;
  /** Legacy alias for maxOutputTokens. */
  maxTokens?: number;
  instructions?: string;
  mcps?: string[];
  skills?: string[];
}

export interface UpdateAgentRequest {
  name?: string;
  displayName?: string;
  description?: string;
  providerId?: string;
  model?: string;
  temperature?: number;
  maxInputTokens?: number;
  maxOutputTokens?: number;
  /** Legacy alias for maxOutputTokens. */
  maxTokens?: number;
  thinkingEnabled?: boolean;
  voiceRecordingEnabled?: boolean;
  instructions?: string;
  mcps?: string[];
  skills?: string[];
  middleware?: string;
}

// ============================================================================
// Conversation Types
// ============================================================================

export interface ConversationResponse {
  id: string;
  agentId: string;
  title?: string;
  createdAt: string;
  updatedAt: string;
  messageCount: number;
}

export interface MessageResponse {
  id: string;
  role: string;
  content: string;
  timestamp: string;
  metadata?: Record<string, unknown>;
}

/**
 * Extended message response for session-scoped queries.
 * Includes execution metadata (agent_id, delegation_type) for context.
 */
export interface SessionMessage {
  id: string;
  execution_id: string;
  agent_id: string;
  delegation_type: string;
  role: string;
  content: string;
  created_at: string;
  tool_calls?: unknown;
  tool_call_id?: string | null;
  tool_results?: unknown;
}

/**
 * Response from POST /api/chat/init.
 *
 * `created` is `true` only when the server actually created the reserved
 * chat session on this call. Subsequent calls from any client return the
 * same `sessionId` / `conversationId` with `created: false`. The flag is
 * informational — callers can log "first ever" or skip a history fetch
 * when it's a brand-new session.
 */
export interface ChatSessionInit {
  sessionId: string;
  conversationId: string;
  created: boolean;
}

/**
 * Scope for session messages query.
 */
export type MessageScope = 'all' | 'root' | 'execution' | 'delegates';

/**
 * Query parameters for session messages endpoint.
 */
export interface SessionMessagesQuery {
  scope?: MessageScope;
  execution_id?: string;
  agent_id?: string;
}

// ============================================================================
// Gateway Status Types
// ============================================================================

export interface HealthResponse {
  status: string;
  version: string;
  buildDate?: string;
  buildTimestamp?: string;
  uptime: number;
}

export interface StatusResponse {
  status: string;
  version?: string;
  buildDate?: string;
  buildTimestamp?: string;
  websocket_port: number;
  http_port: number;
  active_connections: number;
  active_executions: number;
}

// ============================================================================
// Skill Types
// ============================================================================

export interface SkillResponse {
  id: string;
  name: string;
  displayName: string;
  description: string;
  category: string;
  instructions: string;
  createdAt?: string;
}

export interface CreateSkillRequest {
  name: string;
  displayName?: string;
  description?: string;
  category?: string;
  instructions?: string;
}

export interface UpdateSkillRequest {
  name?: string;
  displayName?: string;
  description?: string;
  category?: string;
  instructions?: string;
}

// ============================================================================
// Provider Types
// ============================================================================

export interface ProviderResponse {
  id?: string;
  name: string;
  description: string;
  apiKey: string;
  baseUrl: string;
  models: string[];
  embeddingModels?: string[];
  defaultModel?: string;
  verified?: boolean;
  isDefault?: boolean;
  createdAt?: string;
  rateLimits?: RateLimits;
  modelConfigs?: Record<string, ModelConfig>;
}

export interface CreateProviderRequest {
  id?: string;
  name: string;
  description: string;
  apiKey: string;
  baseUrl: string;
  models: string[];
  embeddingModels?: string[];
  defaultModel?: string;
  rateLimits?: RateLimits;
  modelConfigs?: Record<string, ModelConfig>;
}

export interface UpdateProviderRequest {
  name?: string;
  description?: string;
  apiKey?: string;
  baseUrl?: string;
  models?: string[];
  embeddingModels?: string[];
  defaultModel?: string;
  rateLimits?: RateLimits;
  modelConfigs?: Record<string, ModelConfig>;
}

/** Get the default model for a provider response. */
export function getProviderDefaultModel(provider: ProviderResponse): string {
  return provider.defaultModel || provider.models[0] || "";
}

export interface ProviderTestResult {
  success: boolean;
  message: string;
  models?: string[];
}

// ============================================================================
// Model Registry Types
// ============================================================================

export interface ModelProfile {
  name: string;
  provider: string;
  capabilities: ModelCapabilities;
  context: ContextWindow;
  embedding?: EmbeddingSpec;
}

export interface ModelCapabilities {
  tools: boolean;
  vision: boolean;
  thinking: boolean;
  embeddings: boolean;
  voice: boolean;
  imageGeneration: boolean;
  videoGeneration: boolean;
}

export interface RateLimits {
  requestsPerMinute: number;
  concurrentRequests: number;
}

export interface ModelConfig {
  capabilities: ModelCapabilities;
  maxInput?: number;
  maxOutput?: number;
  source: "registry" | "discovered" | "user";
}

export interface ContextWindow {
  input: number;
  output: number | null;
}

export interface EmbeddingSpec {
  dimensions: number;
  maxDimensions?: number;
}

/** Full registry response: model ID → profile */
export type ModelRegistryResponse = Record<string, ModelProfile>;

// ============================================================================
// MCP Types
// ============================================================================

export interface McpServerSummary {
  id: string;
  name: string;
  description: string;
  type: string;
  enabled: boolean;
  authStatus?: "not_configured" | "not_connected" | "connected" | "reauth_required";
}

export interface McpListResponse {
  servers: McpServerSummary[];
}

export interface CreateMcpRequest {
  type: "stdio" | "http" | "sse" | "streamable-http";
  id?: string;
  name: string;
  description: string;
  // stdio fields
  command?: string;
  args?: string[];
  env?: Record<string, string>;
  // http/sse/streamable-http fields
  url?: string;
  headers?: Record<string, string>;
  auth?: McpAuthConfig;
  enabled?: boolean;
}

export interface McpServerConfig {
  type: "stdio" | "http" | "sse" | "streamable-http";
  id?: string;
  name: string;
  description: string;
  command?: string;
  args?: string[];
  env?: Record<string, string>;
  url?: string;
  headers?: Record<string, string>;
  auth?: McpAuthConfig;
  enabled: boolean;
  validated?: boolean;
}

export interface McpAuthConfig {
  type: "oauth2";
  clientId?: string;
  scopes?: string[];
}

export interface McpTestResult {
  success: boolean;
  message: string;
  tools?: string[];
}

export interface McpOAuthStatusResponse {
  status: "not_configured" | "not_connected" | "connected" | "reauth_required";
}

export interface McpOAuthStartRequest {
  redirectUri?: string;
}

export interface McpOAuthStartResponse {
  authUrl: string;
  state: string;
}

// ============================================================================
// Settings Types
// ============================================================================

export interface ToolSettings {
  /** Enable optional file-discovery tools such as glob */
  fileTools: boolean;
  /** Offload large tool results to filesystem instead of keeping in context */
  offloadLargeResults: boolean;
  /** Token threshold for offloading (default: 5000 tokens ≈ 20000 chars) */
  offloadThresholdTokens: number;
}

export interface ToolSettingsResponse {
  success: boolean;
  data?: ToolSettings;
  error?: string;
}

/** Log settings for daemon file logging */
export interface LogSettings {
  /** Enable file logging */
  enabled: boolean;
  /** Custom log directory (null = default {data_dir}/logs) */
  directory: string | null;
  /** Log level: trace, debug, info, warn, error */
  level: "trace" | "debug" | "info" | "warn" | "error";
  /** Rotation strategy: daily, hourly, minutely, never */
  rotation: "daily" | "hourly" | "minutely" | "never";
  /** Maximum log files to keep (0 = unlimited) */
  maxFiles: number;
  /** Suppress stdout output (only log to file) */
  suppressStdout: boolean;
}

/** Response from log settings API (includes restart warning) */
export interface LogSettingsResponse {
  success: boolean;
  data?: LogSettings & { restartRequired: boolean };
  error?: string;
}

/** Request to update log settings */
export interface UpdateLogSettingsRequest {
  enabled?: boolean;
  directory?: string | null;
  level?: "trace" | "debug" | "info" | "warn" | "error";
  rotation?: "daily" | "hourly" | "minutely" | "never";
  maxFiles?: number;
  suppressStdout?: boolean;
}

/** Orchestrator (root agent) configuration */
export interface OrchestratorConfig {
  /** Provider ID. null = use default provider */
  providerId?: string | null;
  /** Model. null = use provider's default model */
  model?: string | null;
  /** Temperature (0-2). Default: 0.7 */
  temperature: number;
  /** Max input tokens. Default: 200000 */
  maxInputTokens?: number;
  /** Max output tokens. Default: 32000 */
  maxOutputTokens?: number;
  /** Legacy alias for maxOutputTokens. */
  maxTokens: number;
  /** Enable extended thinking/reasoning. Default: true */
  thinkingEnabled: boolean;
}

/** Distillation model configuration (provider/model override) */
export interface DistillationConfig {
  /** Provider ID override. null = inherit from orchestrator */
  providerId?: string | null;
  /** Model override. null = inherit from orchestrator */
  model?: string | null;
  /** Max input tokens override. null/undefined = inherit from orchestrator */
  maxInputTokens?: number | null;
  /** Max output tokens override. null/undefined = inherit from orchestrator */
  maxOutputTokens?: number | null;
  /** Legacy alias for maxOutputTokens. */
  maxTokens?: number | null;
}

/** Ward-curator model configuration (Phase C consolidation LLM call) */
export interface CuratorConfig {
  /** Provider ID override. null = inherit from orchestrator */
  providerId?: string | null;
  /** Model override. null = inherit from orchestrator */
  model?: string | null;
  /** Max input tokens override. null/undefined = inherit from orchestrator */
  maxInputTokens?: number | null;
  /** Max output tokens override. null/undefined = inherit from orchestrator */
  maxOutputTokens?: number | null;
  /** Legacy alias for maxOutputTokens. */
  maxTokens?: number | null;
}

/** Intent-analysis model configuration (every root prompt) */
export interface IntentAnalysisConfig {
  /** Provider ID override. null = inherit from orchestrator */
  providerId?: string | null;
  /** Model override. null = inherit from orchestrator */
  model?: string | null;
  /** Max input tokens override. null/undefined = inherit from orchestrator */
  maxInputTokens?: number | null;
  /** Max output tokens override. null/undefined = inherit from orchestrator */
  maxOutputTokens?: number | null;
  /** Legacy alias for maxOutputTokens. */
  maxTokens?: number | null;
}

// ============================================================================
// Session State (snapshot API for reconnection)
// ============================================================================

export type SessionPhase = "intent" | "planning" | "executing" | "responding" | "completed" | "error";

export interface SessionState {
  session: {
    id: string;
    title: string | null;
    status: "running" | "completed" | "error" | "stopped";
    startedAt: string;
    durationMs: number;
    tokenCount: number;
    model: string | null;
  };
  userMessage: string | null;
  phase: SessionPhase;
  response: string | null;
  intentAnalysis: Record<string, unknown> | null;
  ward: { name: string; content: string } | null;
  recalledFacts: Array<Record<string, unknown>>;
  plan: Array<{ text: string; status: string }>;
  subagents: SubagentStateData[];
  isLive: boolean;
}

export interface SubagentStateData {
  agentId: string;
  executionId: string;
  task: string;
  status: "queued" | "running" | "completed" | "error";
  durationMs: number | null;
  tokenCount: number | null;
  toolCalls: ToolCallEntryData[];
}

export interface ToolCallEntryData {
  toolName: string;
  status: "running" | "completed" | "error";
  durationMs: number | null;
  summary: string | null;
}

/** Default multimodal (vision) model configuration */
export interface MultimodalConfig {
  /** Provider ID. null = not configured */
  providerId?: string | null;
  /** Vision-capable model. null = not configured */
  model?: string | null;
  /** Temperature for analysis calls (default: 0.3) */
  temperature: number;
  /** Max input tokens (default: 200000) */
  maxInputTokens?: number;
  /** Max output tokens (default: 32000) */
  maxOutputTokens?: number;
  /** Legacy alias for maxOutputTokens. */
  maxTokens: number;
}

/** Execution settings for controlling agent concurrency */
export interface ExecutionSettings {
  /** Maximum parallel subagents across all sessions (default: 2) */
  maxParallelAgents: number;
  /** Whether the first-time setup wizard has been completed (default: false) */
  setupComplete: boolean;
  /** The user-chosen name for the root agent */
  agentName?: string;
  /** Orchestrator (root agent) configuration */
  orchestrator?: OrchestratorConfig;
  /** Distillation model configuration (inherits from orchestrator by default) */
  distillation?: DistillationConfig;
  /** Ward-curator (Phase C consolidation) model — inherits from orchestrator by default */
  curator?: CuratorConfig;
  /** Intent-analysis model (every root prompt) — inherits from orchestrator by default */
  intentAnalysis?: IntentAnalysisConfig;
  /** Default multimodal (vision) model for the multimodal_analyze tool */
  multimodal?: MultimodalConfig;
  /** Opt-in feature flags (gate beta UI surfaces and experimental behavior) */
  featureFlags?: Record<string, boolean>;
}

export interface ExecutionSettingsResponse {
  success: boolean;
  data?: ExecutionSettings & { restartRequired: boolean };
  error?: string;
}

/** Setup wizard status check */
export interface SetupStatus {
  setupComplete: boolean;
  hasProviders: boolean;
}

// ============================================================================
// Artifact Types
// ============================================================================

export interface Artifact {
  id: string;
  sessionId: string;
  wardId?: string;
  executionId?: string;
  agentId?: string;
  filePath: string;
  fileName: string;
  fileType?: string;
  fileSize?: number;
  label?: string;
  createdAt: string;
}

// ============================================================================
// Event Types
// ============================================================================

export interface StreamEvent {
  type: string;
  timestamp: number;
  [key: string]: unknown;
}

export type EventCallback = (event: StreamEvent) => void;
export type UnsubscribeFn = () => void;

// ============================================================================
// Execution Log Types
// ============================================================================

export type LogLevel = "debug" | "info" | "warn" | "error";
export type LogCategory = "session" | "token" | "tool_call" | "tool_result" | "delegation" | "error" | "thinking" | "system" | "response" | "intent";
export type SessionStatus = "running" | "completed" | "error" | "stopped";

/** A single execution log entry (snake_case from API) */
export interface ExecutionLog {
  id: string;
  session_id: string;
  conversation_id: string;
  agent_id: string;
  parent_session_id?: string;
  timestamp: string;
  level: LogLevel;
  category: LogCategory;
  message: string;
  metadata?: Record<string, unknown>;
  duration_ms?: number;
}

/** Session summary (snake_case from API) */
export interface LogSession {
  session_id: string;
  conversation_id: string;
  agent_id: string;
  agent_name: string;
  /** Title derived from the first user message in the session */
  title?: string;
  parent_session_id?: string;
  started_at: string;
  ended_at?: string;
  status: SessionStatus;
  token_count: number;
  tool_call_count: number;
  error_count: number;
  duration_ms?: number;
  subagent_count?: number;
  child_session_ids: string[];
  /**
   * Execution mode persisted on the underlying `sessions` row. `"fast"`
   * and `"chat"` indicate chat-mode sessions; anything else (including
   * undefined from older daemons) means research-mode. Use the
   * `isChatSession` / `isResearchSession` predicates in
   * `services/session-kind.ts` instead of reading this directly.
   */
  mode?: string | null;
}

/** Session with all its logs */
export interface SessionDetail {
  session: LogSession;
  logs: ExecutionLog[];
}

/** Filter for querying logs */
export interface LogFilter {
  agent_id?: string;
  level?: LogLevel;
  from_time?: string;
  to_time?: string;
  limit?: number;
  offset?: number;
  root_only?: boolean;
}

// ============================================================================
// Execution State Types (V2 API)
// ============================================================================

/** Session state status (top-level container) - different from LogSession status */
export type SessionStateStatus = "queued" | "running" | "paused" | "completed" | "crashed";

/** Execution status (agent participation) */
export type ExecutionStatus =
  | "queued"
  | "running"
  | "paused"
  | "crashed"
  | "cancelled"
  | "completed";

/** Delegation type */
export type DelegationType = "root" | "sequential" | "parallel";

/** Session - top-level work container (V2 API) */
/** Trigger source for a session */
export type TriggerSource = "web" | "cli" | "cron" | "api" | "connector";

export interface Session {
  id: string;
  status: SessionStateStatus;
  /** Trigger source (web, cli, cron, api, connector) */
  source: TriggerSource;
  root_agent_id: string;
  title?: string;
  created_at: string;
  started_at?: string;
  completed_at?: string;
  total_tokens_in: number;
  total_tokens_out: number;
  metadata?: Record<string, unknown>;
}

/** Agent Execution - agent's participation in a session (V2 API) */
export interface AgentExecution {
  id: string;
  session_id: string;
  agent_id: string;
  parent_execution_id?: string;
  delegation_type: DelegationType;
  task?: string;
  status: ExecutionStatus;
  started_at?: string;
  completed_at?: string;
  tokens_in: number;
  tokens_out: number;
  error?: string;
  child_session_id?: string;
}

/** Session with all its executions (V2 API response) */
export interface SessionWithExecutions {
  id: string;
  status: SessionStateStatus;
  /** Trigger source (web, cli, cron, api, connector) */
  source: TriggerSource;
  root_agent_id: string;
  title?: string;
  created_at: string;
  started_at?: string;
  completed_at?: string;
  total_tokens_in: number;
  total_tokens_out: number;
  metadata?: Record<string, unknown>;
  executions: AgentExecution[];
  subagent_count: number;
}

/** Minimal execution token slice for selected Mission Control detail */
export interface MissionControlExecutionSummary {
  execution_id: string;
  agent_id: string;
  delegation_type: DelegationType;
  tokens_in: number;
  tokens_out: number;
  child_session_id?: string;
}

/** Bounded Mission Control session row */
export interface MissionControlSessionSummary {
  conversation_id: string;
  root_execution_id: string;
  status: SessionStateStatus;
  source: TriggerSource;
  root_agent_id: string;
  title?: string;
  created_at: string;
  started_at?: string;
  completed_at?: string;
  total_tokens_in: number;
  total_tokens_out: number;
  subagent_count: number;
  mode?: string | null;
}

/** Per-execution token slices for one selected Mission Control session */
export interface MissionControlSessionTokens {
  conversation_id: string;
  root_execution_id: string;
  total_tokens_in: number;
  total_tokens_out: number;
  executions: MissionControlExecutionSummary[];
}

/** Filter for querying sessions */
export interface SessionFilter {
  status?: SessionStateStatus;
  root_agent_id?: string;
  from_time?: string;
  to_time?: string;
  limit?: number;
  offset?: number;
}

/** Filter for Mission Control summary rows */
export interface MissionControlFilter {
  limit?: number;
  offset?: number;
}

/** Filter for querying executions */
export interface ExecutionFilter {
  session_id?: string;
  agent_id?: string;
  status?: ExecutionStatus;
  limit?: number;
  offset?: number;
}

/** Dashboard stats (V2 - session + execution counts) */
export interface DashboardStats {
  sessions_queued: number;
  sessions_running: number;
  sessions_paused: number;
  sessions_completed: number;
  sessions_crashed: number;
  executions_queued: number;
  executions_running: number;
  executions_completed: number;
  executions_crashed: number;
  executions_cancelled: number;
  today_sessions: number;
  today_tokens: number;
  /** Sessions count by trigger source (e.g., { web: 5, connector: 2 }) */
  sessions_by_source: Record<TriggerSource, number>;
}

// ============================================================================
// Legacy Types (for backwards compatibility during migration)
// ============================================================================

/** @deprecated Use SessionWithExecutions instead */
export interface ExecutionSession {
  id: string;
  conversation_id: string;
  agent_id: string;
  parent_session_id?: string;
  status: ExecutionStatus;
  created_at: string;
  started_at?: string;
  completed_at?: string;
  tokens_in: number;
  tokens_out: number;
  checkpoint?: string;
  error?: string;
}

/** @deprecated Use SessionFilter instead */
export interface ExecutionSessionFilter {
  agent_id?: string;
  status?: ExecutionStatus;
  limit?: number;
  offset?: number;
}

/** @deprecated Use DashboardStats instead */
export type ExecutionStats = Record<string, number>;

// ============================================================================
// Connection State Types (for subscription-based event routing)
// ============================================================================

/**
 * Connection state for WebSocket transport.
 * Used to track connection lifecycle and show appropriate UI.
 */
export type ConnectionState =
  | { status: "disconnected"; reason?: "user" | "server" | "network" }
  | { status: "connecting" }
  | { status: "connected" }
  | { status: "reconnecting"; attempt: number; maxAttempts?: number }
  | { status: "failed"; error: string };

/**
 * Subscription error codes from the server.
 */
export type SubscriptionErrorCode =
  | "NOT_FOUND"
  | "LIMIT_EXCEEDED"
  | "SERVER_ERROR";

/**
 * Subscription error message from server.
 */
export interface SubscriptionErrorMessage {
  type: "subscription_error";
  conversation_id: string;
  code: SubscriptionErrorCode;
  message: string;
}

/**
 * Conversation event with session/execution identifiers for routing and filtering.
 *
 * - `session_id`: Top-level session ID (primary routing key)
 * - `execution_id`: Specific execution ID (for filtering root vs subagent)
 * - `conversation_id`: Legacy field for backward compatibility
 */
export interface ConversationEvent extends StreamEvent {
  /** Session ID for subscription routing */
  session_id: string;
  /** Execution ID for filtering (root vs subagent) */
  execution_id: string;
  /** Legacy conversation ID for backward compatibility */
  conversation_id?: string;
  /** Sequence number for ordering events */
  seq?: number;
}

/**
 * Global event (stats updates, notifications, customization file changes).
 *
 * `customization_file_changed` is broadcast by the gateway file watcher
 * whenever a markdown file under <vault>/config/ is created, modified, or
 * deleted. The Settings → Customization tab subscribes to refresh its
 * file list. `modified_at` is RFC3339 (or empty string for deletions).
 */
export interface GlobalEvent extends StreamEvent {
  type:
    | "stats_update"
    | "session_notification"
    | "customization_file_changed"
    | "recall_trace";
  /** Present when type is "customization_file_changed". */
  path?: string;
  /** Present when type is "customization_file_changed". RFC3339, or "" for deletions. */
  modified_at?: string;
  // ---- recall_trace fields (Observatory v2 Phase 3) ----
  /** Agent that ran the recall. */
  agent_id?: string;
  /** L0 entity ids the graph-ANN step seeded on. */
  seed_entity_ids?: string[];
  /** L1+ aggregate ids of the LCA walk (seed parents up to and including the LCA). */
  seed_aggregate_ids?: string[];
  /** LCA aggregate id when the seeds share a common ancestor. */
  lca_aggregate_id?: string;
  /** Number of ScoredItems that survived ranking. */
  surfaced_item_count?: number;
}

/**
 * Callback for conversation-specific events.
 */
export type ConversationCallback = (event: ConversationEvent) => void;

/**
 * Callback for global events.
 */
export type GlobalCallback = (event: GlobalEvent) => void;

/**
 * Callback for connection state changes.
 */
export type ConnectionStateCallback = (state: ConnectionState) => void;

/**
 * Subscription scope for event filtering.
 *
 * - `all`: All events (backward compatible, includes subagent events)
 * - `session`: Root execution events + delegation lifecycle markers only
 * - `execution:{id}`: All events for a specific execution
 */
export type SubscriptionScope = "all" | "session" | `execution:${string}`;

/**
 * Options for subscribing to conversation events.
 */
export interface SubscriptionOptions {
  /** Called when an event is received for this conversation */
  onEvent: ConversationCallback;
  /** Called when a subscription error occurs */
  onError?: (error: SubscriptionErrorMessage) => void;
  /** Called when subscription is confirmed with current sequence */
  onConfirmed?: (seq: number, rootExecutionIds?: string[]) => void;
  /** Subscription scope - defaults to "all" for backward compatibility */
  scope?: SubscriptionScope;
}

// ============================================================================
// Plugin Types
// ============================================================================

/** Plugin status from GET /api/plugins */
export interface PluginInfo {
  id: string;
  name: string;
  version: string;
  description: string;
  state: "running" | "stopped" | "failed" | "starting";
  auto_restart: boolean;
  enabled: boolean;
  error?: string;
}

export interface PluginsResponse {
  plugins: PluginInfo[];
  total: number;
}

// ============================================================================
// Bridge Worker Types
// ============================================================================

/** Capability declared by a bridge worker */
export interface BridgeWorkerCapability {
  name: string;
  description?: string;
  schema?: Record<string, unknown>;
}

/** Resource declared by a bridge worker */
export interface BridgeWorkerResource {
  name: string;
  description?: string;
}

/** Connected bridge worker (read-only, from GET /api/bridge/workers) */
export interface BridgeWorker {
  adapter_id: string;
  capabilities: BridgeWorkerCapability[];
  resources: BridgeWorkerResource[];
  connected_at: string;
}

// ============================================================================
// Cron Job Types
// ============================================================================

/** Cron job configuration */
export interface CronJobResponse {
  id: string;
  name: string;
  schedule: string;
  agent_id: string;
  message: string;
  respond_to: string[];
  enabled: boolean;
  timezone?: string;
  metadata?: Record<string, unknown>;
  last_run?: string;
  next_run?: string;
  created_at?: string;
  updated_at?: string;
}

/** Request to create a new cron job */
export interface CreateCronJobRequest {
  id: string;
  name: string;
  schedule: string;
  agent_id: string;
  message: string;
  respond_to?: string[];
  enabled?: boolean;
  timezone?: string;
  metadata?: Record<string, unknown>;
}

/** Request to update a cron job */
export interface UpdateCronJobRequest {
  name?: string;
  schedule?: string;
  agent_id?: string;
  message?: string;
  respond_to?: string[];
  enabled?: boolean;
  timezone?: string;
  metadata?: Record<string, unknown>;
}

/** Result of triggering a cron job manually */
export interface CronTriggerResult {
  success: boolean;
  session_id?: string;
  execution_id?: string;
  message: string;
}

// ============================================================================
// Memory Types
// ============================================================================

/** Memory fact scope - determines visibility of the fact */
export type MemoryScope = "agent" | "shared" | "ward";

/** Memory fact category - type of information stored */
export type MemoryCategory =
  | "preference"
  | "decision"
  | "pattern"
  | "entity"
  | "instruction"
  | "correction"
  | "user"
  | "domain"
  | "strategy"
  | "skill"
  | "agent"
  | "ward";

/** Public memory categories users can create from the UI/API. */
export type CreatableMemoryCategory = Exclude<
  MemoryCategory,
  "instruction" | "correction"
>;

/** A memory fact stored in the agent's memory system */
export interface MemoryFact {
  id: string;
  agent_id: string;
  scope: MemoryScope;
  category: MemoryCategory;
  key: string;
  content: string;
  confidence: number;
  mention_count: number;
  source_summary?: string;
  contradicted_by?: string;
  created_at: string;
  updated_at: string;
  /** Pinned facts can't be overwritten by distillation. User-authored facts are pinned. */
  pinned?: boolean;
  /** Ward scope when set (e.g. "literature-library"); facts default to "__global__" when null on the server. */
  ward_id?: string;
}

/** Filter options for listing memory facts */
export interface MemoryFilter {
  /** Optional agent filter - when provided, only that agent's memories are returned */
  agent_id?: string;
  category?: MemoryCategory;
  scope?: MemoryScope;
  limit?: number;
  offset?: number;
}

/** Response for memory list operations */
export interface MemoryListResponse {
  facts: MemoryFact[];
  total: number;
}

// ============================================================================
// Knowledge Graph Types
// ============================================================================

/** Graph statistics response */
export interface GraphStatsResponse {
  entity_count: number;
  relationship_count: number;
  entity_types: Record<string, number>;
  relationship_types: Record<string, number>;
  most_connected_entities: Array<[string, number]>;
}

/** Graph entity */
export interface GraphEntity {
  id: string;
  agent_id: string;
  entity_type: string;
  name: string;
  properties: Record<string, unknown>;
  mention_count: number;
  first_seen_at: string;
  last_seen_at: string;
}

/** Graph relationship */
export interface GraphRelationship {
  id: string;
  agent_id: string;
  source_entity_id: string;
  target_entity_id: string;
  relationship_type: string;
  mention_count: number;
}

/** Entity list response */
export interface GraphEntityListResponse {
  entities: GraphEntity[];
  total: number;
}

/** Relationship list response */
export interface GraphRelationshipListResponse {
  relationships: GraphRelationship[];
  total: number;
}

/** Filter for entity queries */
export interface GraphEntityFilter {
  entity_type?: string;
  limit?: number;
  offset?: number;
}

/** Filter for relationship queries */
export interface GraphRelationshipFilter {
  relationship_type?: string;
  limit?: number;
  offset?: number;
}

/** Neighbor entry in neighbor response */
export interface GraphNeighborEntry {
  entity: GraphEntity;
  relationship: GraphRelationship;
  direction: "incoming" | "outgoing";
}

/** Neighbor response */
export interface GraphNeighborResponse {
  entity_id: string;
  neighbors: GraphNeighborEntry[];
}

/** Subgraph response */
export interface GraphSubgraphResponse {
  entities: GraphEntity[];
  relationships: GraphRelationship[];
  center: string;
  max_hops: number;
}

/** Options for neighbor queries */
export interface GraphNeighborOptions {
  direction?: "incoming" | "outgoing" | "both";
  limit?: number;
}

/** Options for subgraph queries */
export interface GraphSubgraphOptions {
  max_hops?: number;
}

// ============================================================================
// Embedding Backend Types
// ============================================================================

export type EmbeddingsBackend = "internal" | "ollama";

export type EmbeddingsStatus =
  | "ready"
  | "ollama_unreachable"
  | "model_missing"
  | "misconfigured";

export interface EmbeddingsHealth {
  backend: EmbeddingsBackend;
  model?: string;
  dim: number;
  status: EmbeddingsStatus;
  indexed_count: number;
}

export interface CuratedModel {
  tag: string;
  dim: number;
  size_mb: number;
  mteb: number;
}

export interface EmbeddingConfig {
  /**
   * `true` selects the built-in BGE-small-en-v1.5 (384-d). `false` selects
   * the Ollama configuration below. Wire shape matches `config/settings.json`.
   */
  internal: boolean;
  /**
   * Ollama connection + model. Preserved across `internal` toggles so
   * flipping back doesn't force the user to retype the URL and model.
   * Required when `internal: false`.
   */
  ollama?: { url: string; model: string; dimensions: number };
}

/** Response from GET /api/embeddings/ollama-models. */
export interface OllamaModelsResponse {
  /** Every model the user's Ollama instance currently has. */
  all: string[];
  /** Subset of `all` that looks like an embedding model (substring heuristic). */
  likely_embedding: string[];
  /** `false` when the URL was unreachable — UI should fall back to curated. */
  reachable: boolean;
}

export type ConfigureProgressEvent =
  | { kind: "pulling"; mb_done: number; mb_total: number }
  | { kind: "reindexing"; table: string; current: number; total: number }
  | { kind: "ready"; backend: string; model?: string; dim: number }
  | { kind: "error"; reason: string; rollback?: string };

// ============================================================================
// Memory v2 — Ward Content + Unified Hybrid Search
// ============================================================================

/** Coarse age bucket assigned by backend ward content handler. */
export type AgeBucket = "today" | "last_7_days" | "historical";

/** How a hybrid search hit was matched. */
export type MatchSource = "hybrid" | "fts" | "vec" | "title";

/** Minimal wiki article fields for ward content / search results. */
export interface WikiArticle {
  id: string;
  ward_id: string;
  title: string;
  content: string;
  updated_at: string;
}

/** Minimal procedure fields for ward content / search results. */
export interface Procedure {
  id: string;
  ward_id: string;
  name: string;
  description?: string;
  last_used?: string;
  created_at: string;
  updated_at: string;
}

/** Session episode fields from the backend (matches `zbot_stores_domain::SessionEpisode`). */
export interface SessionEpisode {
  id: string;
  session_id: string;
  agent_id: string;
  ward_id: string;
  task_summary: string;
  /** One of: 'success', 'partial', 'failed', 'crashed'. */
  outcome: string;
  strategy_used?: string | null;
  key_learnings?: string | null;
  token_cost?: number | null;
  created_at: string;
}

/** Summary block for a ward (lightweight header). */
export interface WardContentSummary {
  title: string;
  description?: string;
  updated_at?: string;
}

/** Counts per memory type in a ward. */
export interface WardContentCounts {
  facts: number;
  wiki: number;
  procedures: number;
  episodes: number;
}

/** Response for GET /api/wards/:ward_id/content */
export interface WardContent {
  ward_id: string;
  summary: WardContentSummary;
  facts: Array<MemoryFact & { age_bucket: AgeBucket }>;
  wiki: Array<WikiArticle & { age_bucket: AgeBucket }>;
  procedures: Array<Procedure & { age_bucket: AgeBucket }>;
  episodes: Array<SessionEpisode & { age_bucket: AgeBucket }>;
  counts: WardContentCounts;
}

// ============================================================================
// Vault — read-only ward filesystem browser
// ============================================================================

export interface VaultWard {
  id: string;
  name: string;
}

export interface VaultWardsResponse {
  wards: VaultWard[];
}

export type VaultNodeKind = "directory" | "file";

export interface VaultNode {
  ward_id: string;
  path: string;
  name: string;
  kind: VaultNodeKind;
  extension?: string | null;
  size?: number | null;
  modified_at?: string | null;
  previewable: boolean;
}

export interface VaultTreeResponse {
  ward_id: string;
  path: string;
  children: VaultNode[];
  truncated: boolean;
}

export interface VaultSearchResponse {
  ward_id: string;
  query: string;
  matches: VaultNode[];
  truncated: boolean;
}

export interface VaultTextFileResponse {
  kind: "text";
  ward_id: string;
  path: string;
  name: string;
  extension: string;
  size: number;
  modified_at?: string | null;
  content: string;
}

export interface VaultOfficeFileResponse {
  kind: "office";
  ward_id: string;
  path: string;
  extension: "docx" | "pptx";
  contentType: string;
  data: ArrayBuffer;
}

export type VaultFileResponse = VaultTextFileResponse | VaultOfficeFileResponse;

/** Request body for POST /api/memory/search (unified hybrid search). */
export interface HybridSearchRequest {
  query: string;
  mode?: "hybrid" | "fts" | "semantic";
  types?: Array<"facts" | "wiki" | "procedures" | "episodes">;
  ward_ids?: string[];
  filters?: {
    category?: MemoryCategory;
    confidence_gte?: number;
  };
  limit?: number;
}

/** Hybrid search result block for a single memory type. */
export interface HybridSearchTypeBlock<T> {
  hits: T[];
  latency_ms: number;
}

/** Wiki search hit (snippet-based, includes score). */
export interface WikiSearchHit {
  id: string;
  ward_id: string;
  title: string;
  snippet: string;
  updated_at: string;
  score: number;
  match_source: MatchSource;
}

/** Response for POST /api/memory/search (unified hybrid search). */
export interface HybridSearchResponse {
  facts: HybridSearchTypeBlock<MemoryFact & { match_source?: MatchSource; score?: number }>;
  wiki: HybridSearchTypeBlock<WikiSearchHit>;
  procedures: HybridSearchTypeBlock<Procedure & { match_source?: MatchSource }>;
  episodes: HybridSearchTypeBlock<SessionEpisode & { match_source?: MatchSource }>;
}
