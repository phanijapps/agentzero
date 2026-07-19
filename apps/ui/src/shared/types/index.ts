// ============================================================================
// SHARED TYPES
// Central type definitions used across the application
// ============================================================================

// ============================================================================
// DOMAIN: Core
// ============================================================================

/** Application-wide configuration */
export interface AppConfig {
  appName: string;
  version: string;
  theme: "light" | "dark" | "system";
}

/** Navigation route definition */
export interface Route {
  path: string;
  label: string;
  icon?: string;
  description?: string;
}

// ============================================================================
// DOMAIN: Vaults (Profiles)
// ============================================================================

/** Vault (profile) for isolating agent configurations */
export interface Vault {
  id: string;
  name: string;
  path: string;
  isDefault: boolean;
  createdAt: string; // ISO datetime string
  lastAccessed: string; // ISO datetime string
}

/** Request to create a new vault */
export interface CreateVaultRequest {
  name: string;
  path?: string;
}

/** Detailed information about a vault */
export interface VaultInfo {
  vault: Vault;
  agentCount: number;
  skillCount: number;
  storageInfo: VaultStorageInfo;
}

/** Storage information for a vault */
export interface VaultStorageInfo {
  totalUsed: number;
  databaseSize: number;
  agentsSize: number;
  skillsSize: number;
}

/** Status of the vault system for initialization */
export interface VaultStatus {
  registryExists: boolean;
  hasVaults: boolean;
  hasActiveVault: boolean;
  activeVault: Vault | null;
  vaults: Vault[];
}

// ============================================================================
// DOMAIN: Conversations (Legacy)
// ============================================================================

/** Chat message */
export interface Message {
  id: string;
  role: "user" | "assistant" | "system";
  content: string;
  timestamp: number;
  metadata?: Record<string, unknown>;
}

/** Conversation thread */
export interface Conversation {
  id: string;
  title: string;
  agentId: string;
  messages: Message[];
  createdAt: number;
  updatedAt: number;
}

// ============================================================================
// DOMAIN: Agent Channels (New)
// ============================================================================

/** Daily session for an agent */
export interface DailySession {
  id: string;
  agentId: string;
  sessionDate: string; // YYYY-MM-DD format
  summary?: string;
  previousSessionIds?: string[];
  messageCount: number;
  tokenCount: number;
  createdAt: string; // ISO datetime string
  updatedAt: string; // ISO datetime string
}

/** Day summary for displaying in the UI */
export interface DaySummary {
  sessionId: string;
  sessionDate: string;
  summary?: string;
  messageCount: number;
  isArchived: boolean;
}

/** Session message (Agent Channel model) */
export interface SessionMessage {
  id: string;
  sessionId: string;
  role: string;
  content: string;
  createdAt: string; // ISO datetime string
  tokenCount: number;
  toolCalls?: Record<string, unknown>;
  toolResults?: Record<string, unknown>;
}

/** Agent channel info for UI display */
export interface AgentChannel {
  agentId: string;
  displayName: string;
  todayMessageCount: number;
  hasHistory: boolean;
  lastActivity: string; // ISO datetime string
  lastActivityText: string; // Human-readable like "2 hours ago"
}

// ============================================================================
// DOMAIN: Agents
// ============================================================================

/** Agent type from the gateway API */
export type AgentType =
  | "llm"
  | "sequential"
  | "parallel"
  | "loop"
  | "conditional"
  | "llm_conditional"
  | "custom";

/** Agent configuration */
export interface Agent {
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
  thinkingEnabled?: boolean;
  voiceRecordingEnabled?: boolean;
  instructions: string;
  mcps: string[];
  skills: string[];
  /** Middleware configuration (YAML string) */
  middleware?: string;
  /** Zero-app framework agent type */
  agentType?: AgentType;
  /** System instruction for LLM agents (from config.yaml) */
  systemInstruction?: string;
  /** Sub-agents for workflow agents */
  subAgents?: Agent[];
  createdAt: string;
}

/** Sequential agent configuration */
export interface SequentialAgentConfig {
  description?: string;
}

/** Parallel agent configuration */
export interface ParallelAgentConfig {
  description?: string;
}

/** Loop agent configuration */
export interface LoopAgentConfig {
  maxIterations?: number;
  untilEscalation?: boolean;
  description?: string;
}

/** Conditional agent configuration (rule-based) */
export interface ConditionalAgentConfig {
  condition: string; // e.g., "state.is_premium" or "state.user_count > 10"
  ifAgent: string; // agent name
  elseAgent?: string; // agent name
  description?: string;
}

/** LLM conditional agent configuration (LLM-based routing) */
export interface LlmConditionalAgentConfig {
  instruction: string; // Classification instruction
  routes: Record<string, string>; // label -> agent name mapping
  defaultRoute?: string; // Fallback agent name
  description?: string;
}

/** Custom agent configuration */
export interface CustomAgentConfig {
  description?: string;
}

/** Middleware configuration */
export interface MiddlewareConfig {
  summarization?: SummarizationConfig;
  contextEditing?: ContextEditingConfig;
}

export interface SummarizationConfig {
  enabled: boolean;
  maxTokens: number;
  triggerAt: number; // Token threshold
  provider?: string; // Provider for summarization
}

export interface ContextEditingConfig {
  enabled: boolean;
  keepLastN: number;
  keepPolicy: "all" | "user_only" | "assistant_only";
}

// ============================================================================
// DOMAIN: Providers
// ============================================================================

/** API Provider credentials */
export interface Provider {
  id: string;
  name: string;
  description: string;
  apiKey: string;
  baseUrl: string;
  models: string[];
  /** Embedding models for vector search/memory */
  embeddingModels?: string[];
  /** Default model for auto-created agents. Falls back to models[0]. */
  defaultModel?: string;
  verified?: boolean;
  createdAt: string;
}

/** Get the default model for a provider. */
export function getDefaultModel(provider: Provider): string {
  return provider.defaultModel || provider.models[0] || "gpt-4o";
}

/** Provider test result */
export interface ProviderTestResult {
  success: boolean;
  message: string;
  models?: string[];
}

// ============================================================================
// DOMAIN: MCP Servers
// ============================================================================

/** MCP Server connection */
export interface MCPServer {
  id: string;
  name: string;
  command: string;
  args: string[];
  env?: Record<string, string>;
  enabled: boolean;
}

// ============================================================================
// DOMAIN: Skills
// ============================================================================

/** Skill/Plugin configuration */
export interface Skill {
  id: string;
  name: string;
  displayName: string;
  description: string;
  category: string;
  instructions: string;
  createdAt: string;
}

// ============================================================================
// DOMAIN: Deletion & Cache
// ============================================================================

/** Deletion result showing what was deleted */
export interface DeletionResult {
  sessionsDeleted: number;
  messagesDeleted: number;
  cacheEntriesInvalidated: number;
}

/** Check if deletion result is empty */
export function isDeletionResultEmpty(result: DeletionResult): boolean {
  return result.sessionsDeleted === 0 && result.messagesDeleted === 0;
}

/** Deletion scope for Chrome-style history clearing */
export type DeletionScope =
  | { type: "last_7_days" }
  | { type: "last_30_days" }
  | { type: "all_time" }
  | { type: "custom_range"; startDate: string; endDate: string };

/** Cache statistics */
export interface CacheStats {
  entryCount: number;
  hitCount: number;
  missCount: number;
}

// ============================================================================
// DOMAIN: Settings
// ============================================================================

/** Application settings */
export interface AppSettings {
  theme: "light" | "dark" | "system";
  fontSize: "small" | "medium" | "large";
  autoSave: boolean;
  defaultProvider?: string;
  defaultAgent?: string;
}

// ============================================================================
// DOMAIN: Search
// ============================================================================

/** Message source location */
export type MessageSource =
  | { type: "sqlite"; sessionId: string }
  | { type: "parquet"; sessionId: string; filePath: string };

/** Search result with location info */
export interface SearchResult {
  messageId: string;
  sessionId: string;
  agentId: string;
  agentName: string;
  role: string;
  content: string;
  createdAt: string; // ISO datetime string
  score: number;
  source: MessageSource;
}

/** Search query parameters */
export interface SearchQuery {
  query: string;
  agentId?: string;
  startDate?: string; // ISO datetime string
  endDate?: string; // ISO datetime string
  limit?: number;
}

/** Document to be indexed */
export interface IndexedDocument {
  messageId: string;
  sessionId: string;
  agentId: string;
  agentName: string;
  role: string;
  content: string;
  timestamp: number; // Unix timestamp
  sourceType: string; // "sqlite" or "parquet"
  sourcePath?: string; // Parquet file path if archived
}

/** Index build progress for rebuilding index */
export interface IndexBuildProgress {
  totalMessages: number;
  indexedMessages: number;
  stage: string;
  isComplete: boolean;
}

// ============================================================================
// DOMAIN: Execution Control
// ============================================================================

/** Agent execution status */
export interface AgentExecutionStatus {
  agentId: string;
  isExecuting: boolean;
  iteration: number;
  stopRequested: boolean;
}

/** Stop execution result */
export interface StopExecutionResult {
  success: boolean;
  agentId: string;
  message: string;
  sessionId?: string;
}

// ============================================================================
// DOMAIN: TODO List
// ============================================================================

/** A single TODO item */
export interface Todo {
  id: string;
  agentId: string;
  agentName: string;
  isOrchestrator: boolean;
  title: string;
  description?: string;
  completed: boolean;
  priority: "low" | "medium" | "high";
  createdAt: string;
  completedAt?: string;
}

/** The complete TODO list */
export interface TodoList {
  items: Todo[];
  lastUpdated: string;
}

// ============================================================================
// DOMAIN: Activity Tracking
// ============================================================================

/** Type of activity item */
export type ActivityType = "todo" | "tool_call" | "subagent_start" | "subagent_end";

/** Status of a tool call */
export type ToolStatus = "running" | "success" | "error";

/** Tool call record for activity tracking */
export interface ToolCallActivity {
  id: string;
  name: string;
  status: ToolStatus;
  durationMs?: number;
  argumentsPreview?: string;
  resultPreview?: string;
  error?: string;
}

/** A single activity item (tool call, TODO, or subagent event) */
export interface ActivityItem {
  id: string;
  agentId: string;
  agentName: string;
  isOrchestrator: boolean;
  itemType: ActivityType;
  timestamp: string;
  toolCall?: ToolCallActivity;
  todo?: Todo;
}

/** Activity update event payload */
export interface ActivityUpdateEvent {
  type: "activity_update";
  timestamp: number;
  activity: ActivityItem[];
}

// ============================================================================
// DOMAIN: Generative UI Events
// ============================================================================

/** Show content event - display content in generative UI canvas */
export interface ShowContentEvent {
  type: "show_content";
  contentType: "pdf" | "ppt" | "html" | "image" | "text" | "markdown";
  title: string;
  content: string; // Base64 or raw content
  metadata?: Record<string, unknown>;
  filePath?: string; // Path to attachment file
  isAttachment?: boolean; // true if content is saved to attachments directory
  base64?: boolean; // true if content is base64 encoded
}

/** Request input event - request user input via JSON Schema form */
export interface RequestInputEvent {
  type: "request_input";
  formId: string;
  formType: "json_schema" | "dynamic_form";
  title: string;
  description?: string;
  schema: Record<string, unknown>;
  submitButton?: string;
}
