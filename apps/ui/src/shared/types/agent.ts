/**
 * Agent execution types for streaming
 * Based on LangChain 1.0 createAgent API
 */

// ============================================================================
// STREAMING EVENT TYPES
// ============================================================================

/**
 * Base streaming event
 */
export interface StreamEvent {
  type: StreamEventType;
  timestamp: number;
}

/**
 * Stream event types
 */
export type StreamEventType =
  | "token"           // Text token from LLM
  | "reasoning"       // Thinking/reasoning (if model supports)
  | "tool_call_start" // Tool call beginning
  | "tool_call_chunk" // Partial tool call arguments
  | "tool_call_end"   // Complete tool call
  | "tool_result"     // Tool execution result
  | "error"           // Error occurred
  | "done"            // Agent execution complete
  | "metadata"        // Agent metadata
  | "show_content"    // Show content in generative UI canvas
  | "request_input";  // Request input via form

/**
 * Text token event
 */
export interface TokenEvent extends StreamEvent {
  type: "token";
  content: string;
}

/**
 * Reasoning/thinking event (for models that support extended thinking)
 */
export interface ReasoningEvent extends StreamEvent {
  type: "reasoning";
  content: string;
}

/**
 * Tool call start event
 */
export interface ToolCallStartEvent extends StreamEvent {
  type: "tool_call_start";
  toolId: string;
  toolName: string;
}

/**
 * Tool call chunk event (partial arguments)
 */
export interface ToolCallChunkEvent extends StreamEvent {
  type: "tool_call_chunk";
  toolId: string;
  toolName: string;
  args: string; // Partial JSON
}

/**
 * Tool call end event (complete tool call)
 */
export interface ToolCallEndEvent extends StreamEvent {
  type: "tool_call_end";
  toolId: string;
  toolName: string;
  args: Record<string, unknown>; // Complete parsed arguments
}

/**
 * Tool result event
 */
export interface ToolResultEvent extends StreamEvent {
  type: "tool_result";
  toolId: string;
  toolName: string;
  result: string;
  error?: string;
}

/**
 * Error event
 */
export interface ErrorEvent extends StreamEvent {
  type: "error";
  error: string;
  recoverable: boolean;
}

/**
 * Done event
 */
export interface DoneEvent extends StreamEvent {
  type: "done";
  finalMessage: string;
  tokenCount: number;
}

/**
 * Metadata event (agent info, model, etc.)
 */
export interface MetadataEvent extends StreamEvent {
  type: "metadata";
  agentId: string;
  model: string;
  provider: string;
}

/**
 * Show content event - display content in generative UI canvas
 */
export interface ShowContentEvent extends StreamEvent {
  type: "show_content";
  contentType: "pdf" | "ppt" | "html" | "image" | "text" | "markdown";
  title: string;
  content: string; // Filename (if isAttachment=true) or Base64/raw content (backwards compatibility)
  metadata?: Record<string, unknown>;
  filePath?: string; // Path to attachment file (e.g., "conv_id/attachments/filename")
  isAttachment?: boolean; // true if content is saved to attachments directory
  base64?: boolean; // true if content is base64 encoded
}

/**
 * Request input event - request user input via JSON Schema form
 */
export interface RequestInputEvent extends StreamEvent {
  type: "request_input";
  formId: string;
  formType: "json_schema" | "dynamic_form";
  title: string;
  description?: string;
  schema: Record<string, unknown>;
  submitButton?: string;
}

/**
 * Union type of all stream events
 */
export type AgentStreamEvent =
  | TokenEvent
  | ReasoningEvent
  | ToolCallStartEvent
  | ToolCallChunkEvent
  | ToolCallEndEvent
  | ToolResultEvent
  | ErrorEvent
  | DoneEvent
  | MetadataEvent
  | ShowContentEvent
  | RequestInputEvent;

// ============================================================================
// MESSAGE TYPES (matching Rust repository)
// ============================================================================

/**
 * Message role
 */
export enum MessageRole {
  User = "user",
  Assistant = "assistant",
  System = "system",
  Tool = "tool",
}

/**
 * Tool call structure
 */
export interface ToolCall {
  id: string;
  name: string;
  arguments: Record<string, unknown>;
}

/**
 * Tool result structure
 */
export interface ToolResult {
  toolCallId: string;
  output: string;
  error?: string;
}

/**
 * Message structure
 */
export interface Message {
  id: string;
  conversation_id: string;
  role: MessageRole;
  content: string;
  created_at: string;
  token_count: number;
  tool_calls?: ToolCall[];
  tool_results?: ToolResult[];
}

// ============================================================================
// AGENT EXECUTION TYPES
// ============================================================================

/**
 * Agent configuration for execution
 */
export interface AgentConfig {
  agentId: string;          // Agent folder name
  providerId: string;        // Provider name
  model: string;            // Model name
  temperature?: number;     // Temperature (0-1)
  maxInputTokens?: number;  // Max input/context tokens
  maxOutputTokens?: number; // Max tokens for response
  maxTokens?: number;       // Legacy alias for maxOutputTokens
  systemPrompt?: string;    // System prompt (from AGENTS.md)
  instructions?: string;    // Additional instructions
}

/**
 * Tool reference for agent
 */
export interface AgentTool {
  name: string;
  description: string;
  // Tool is resolved from ToolRegistry by name
}

/**
 * Request to execute an agent
 */
export interface AgentExecuteRequest {
  conversationId: string;
  agentId: string;
  message: string;
  stream?: boolean;        // Enable streaming (default: true)
}

/**
 * Agent execution response (non-streaming)
 */
export interface AgentExecuteResponse {
  messageId: string;
  response: string;
  toolCalls?: ToolCall[];
  toolResults?: ToolResult[];
  tokenCount: number;
}

/**
 * Streaming response chunks
 */
export interface AgentStreamChunk {
  event: AgentStreamEvent;
}

/**
 * Message history for agent context
 */
export interface MessageHistory {
  messages: Array<{
    role: "user" | "assistant" | "system";
    content: string;
    tool_calls?: ToolCall[];
    tool_results?: ToolResult[];
  }>;
}

// ============================================================================
// TOOL EXECUTION TYPES
// ============================================================================

/**
 * Status of a tool execution
 */
export enum ToolExecutionStatus {
  Pending = "pending",
  Running = "running",
  Completed = "completed",
  Failed = "failed",
}

/**
 * Tool execution state
 */
export interface ToolExecution {
  toolId: string;
  toolName: string;
  args: Record<string, unknown>;
  status: ToolExecutionStatus;
  startTime: number;
  endTime?: number;
  result?: string;
  error?: string;
}
