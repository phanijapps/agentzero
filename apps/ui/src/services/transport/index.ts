// ============================================================================
// TRANSPORT LAYER
// HTTP/WebSocket communication with the gateway
// ============================================================================

import type { Transport } from "./interface";
import type { TransportConfig } from "./types";
import { HttpTransport } from "./http";

// Re-export types
export type { Transport } from "./interface";
export type {
  TransportConfig,
  TransportResult,
  AgentResponse,
  CreateAgentRequest,
  UpdateAgentRequest,
  SkillResponse,
  CreateSkillRequest,
  UpdateSkillRequest,
  ProviderResponse,
  CreateProviderRequest,
  UpdateProviderRequest,
  ProviderTestResult,
  ModelRegistryResponse,
  ModelProfile,
  ModelCapabilities,
  HealthResponse,
  StatusResponse,
  EventCallback,
  UnsubscribeFn,
  StreamEvent,
  McpServerSummary,
  McpListResponse,
  McpServerConfig,
  CreateMcpRequest,
  McpTestResult,
  McpOAuthStatusResponse,
  McpOAuthStartRequest,
  McpOAuthStartResponse,
  MessageResponse,
  ChatSessionInit,
  SessionMessage,
  MessageScope,
  SessionMessagesQuery,
  ConversationResponse,
  ToolSettings,
  LogSettings,
  UpdateLogSettingsRequest,
  ExecutionSettings,
  LogLevel,
  LogCategory,
  SessionStatus,
  ExecutionLog,
  LogSession,
  SessionDetail,
  LogFilter,
  MissionControlSessionSummary,
  MissionControlSessionTokens,
  MissionControlExecutionSummary,
  MissionControlFilter,
  // Subscription types
  SubscriptionScope,
  SubscriptionOptions,
  // Plugin types
  PluginInfo,
  PluginsResponse,
  // Cron types
  CronJobResponse,
  CreateCronJobRequest,
  UpdateCronJobRequest,
  CronTriggerResult,
  SetupStatus,
  // Embedding backend types
  EmbeddingsBackend,
  EmbeddingsStatus,
  EmbeddingsHealth,
  CuratedModel,
  EmbeddingConfig,
  ConfigureProgressEvent,
  OllamaModelsResponse,
} from "./types";

export { getProviderDefaultModel } from "./types";

// ============================================================================
// Default Configuration
// ============================================================================

const GATEWAY_HTTP_PORT = 18791;
/**
 * Legacy standalone WebSocket port. No longer used in the default config
 * — the gateway now serves the WS upgrade on the HTTP port at `/ws`. Kept
 * in the codebase only to honor an explicit `?gateway_ws=` override for
 * installs that still run `--legacy-ws-port-enabled`.
 */
const LEGACY_GATEWAY_WS_PORT = 18790;

/**
 * Build default gateway URLs from the current page's origin.
 *
 * **Browser default is same-origin.** Both production (daemon serves UI +
 * API on the same port — typically 18791) and dev (Vite proxies `/api` and
 * `/ws` to the daemon) use the page origin verbatim:
 *   - `httpUrl: ""` so `fetch("${httpUrl}/api/foo")` resolves to a relative
 *     `/api/foo` against the page origin
 *   - `wsUrl: ws(s)://<page-host:port>/ws` reuses the page hostname AND port
 *     so phones loading `http://192.168.1.5:18791/` get
 *     `ws://192.168.1.5:18791/ws` automatically — no port mismatch, no
 *     second firewall hole, no CORS preflight
 *
 * SSR / no-window fallback keeps the historical localhost defaults so unit
 * tests (and any non-browser caller) keep their previous behavior.
 *
 * `LEGACY_GATEWAY_WS_PORT` is retained as a named constant for anyone
 * running the daemon with `--legacy-ws-port-enabled`; they can point the
 * UI at the old port via `?gateway_ws=ws://host:18790` for one release
 * cycle before it's removed.
 */
function defaultConfig(): TransportConfig {
  if (typeof window === "undefined" || !window.location) {
    return {
      httpUrl: `http://localhost:${GATEWAY_HTTP_PORT}`,
      wsUrl: `ws://localhost:${GATEWAY_HTTP_PORT}/ws`,
    };
  }
  const wsProto = window.location.protocol === "https:" ? "wss" : "ws";
  // window.location.host = "hostname:port" (port elided for default :80/:443).
  // Always reuse it so we never disagree with the page origin.
  return {
    httpUrl: "",
    wsUrl: `${wsProto}://${window.location.host}/ws`,
  };
}

/**
 * Warn if a configured wsUrl still points at the legacy standalone WS
 * port. Users with mobile clients behind a restrictive firewall may have
 * been bitten by this; nudge them toward the unified endpoint.
 */
function warnIfLegacyWsUrl(wsUrl: string | undefined): void {
  if (
    typeof window !== "undefined" &&
    wsUrl &&
    wsUrl.includes(`:${LEGACY_GATEWAY_WS_PORT}`)
  ) {
    // eslint-disable-next-line no-console
    console.warn(
      `[transport] WS URL still points at the legacy port ${LEGACY_GATEWAY_WS_PORT}. ` +
        `The gateway now serves WebSocket on the HTTP port at /ws. Remove the ` +
        `override or upgrade your reverse-proxy config to use ws://host:${GATEWAY_HTTP_PORT}/ws.`,
    );
  }
}

/**
 * Get configuration from environment or use defaults.
 */
function getConfig(): TransportConfig {
  const fallback = defaultConfig();
  // In web mode, check for environment variables or window config
  if (typeof window !== "undefined") {
    const windowConfig = (window as { __ZERO_CONFIG__?: TransportConfig }).__ZERO_CONFIG__;
    if (windowConfig) {
      warnIfLegacyWsUrl(windowConfig.wsUrl);
      return windowConfig;
    }
  }

  // Check for URL parameters (useful for development)
  if (typeof window !== "undefined" && window.location) {
    const params = new URLSearchParams(window.location.search);
    const httpUrl = params.get("gateway_http");
    const wsUrl = params.get("gateway_ws");

    if (httpUrl || wsUrl) {
      const merged = {
        httpUrl: httpUrl || fallback.httpUrl,
        wsUrl: wsUrl || fallback.wsUrl,
      };
      warnIfLegacyWsUrl(merged.wsUrl);
      return merged;
    }
  }

  return fallback;
}

// ============================================================================
// Transport Factory
// ============================================================================

/**
 * Create a transport instance.
 */
export function createTransport(): Transport {
  return new HttpTransport();
}

// ============================================================================
// Global Transport Instance
// ============================================================================

let globalTransport: Transport | null = null;
let initialized = false;

/**
 * Get the global transport instance.
 * Creates and initializes it if not already done.
 */
export async function getTransport(): Promise<Transport> {
  if (!globalTransport) {
    globalTransport = createTransport();
  }

  if (!initialized) {
    await globalTransport.initialize(getConfig());
    initialized = true;
  }

  return globalTransport;
}

/**
 * Initialize the transport with custom configuration.
 * Should be called early in app startup.
 */
export async function initializeTransport(config?: Partial<TransportConfig>): Promise<Transport> {
  globalTransport = createTransport();

  const finalConfig = {
    ...getConfig(),
    ...config,
  };

  await globalTransport.initialize(finalConfig);
  initialized = true;

  return globalTransport;
}

/**
 * Check if transport is initialized.
 */
export function isTransportInitialized(): boolean {
  return initialized;
}

/**
 * Reset the transport (for testing).
 */
export function resetTransport(): void {
  if (globalTransport) {
    globalTransport.disconnect();
  }
  globalTransport = null;
  initialized = false;
}
