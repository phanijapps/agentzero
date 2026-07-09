// ============================================================================
// WEB INTEGRATIONS PANEL
// Unified Integrations page: Tool Servers | Plugins & Workers
// ============================================================================

import { useState, useEffect, useCallback, useRef, useMemo } from "react";
import { useSearchParams } from "react-router-dom";
import {
  Server, Plus, Trash2, Pencil, Play, Loader2, Wrench,
  Eye, EyeOff, Key, X, Cpu, Database, Cable, Check,
  Terminal, Globe, Power, RefreshCw, Puzzle,
} from "lucide-react";
import {
  getTransport,
  type McpServerSummary,
  type McpServerConfig,
  type CreateMcpRequest,
  type McpTestResult,
  type Transport,
  type PluginInfo,
} from "@/services/transport";
import type { BridgeWorker } from "@/services/transport/types";
import { TabBar, TabPanel } from "@/components/TabBar";
import { HelpBox } from "@/components/HelpBox";
import { usePaths } from "@/hooks/usePaths";
import { ActionBar, FilterChip } from "@/components/ActionBar";
import { MetaChip } from "@/components/MetaChip";
import { Slideover } from "@/components/Slideover";
import { EmptyState } from "@/shared/ui/EmptyState";
import { randomId } from "@/shared/utils/randomId";

// ============================================================================
// Constants
// ============================================================================

const POLL_INTERVAL_MS = 5000;
const OAUTH_STATUS_POLL_INTERVAL_MS = 2000;
const OAUTH_STATUS_POLL_MAX_ATTEMPTS = 150;
type McpAuthStatus = NonNullable<McpServerSummary["authStatus"]>;

const MCP_EMOJIS = [
  "\u{1F9F0}", "\u{1F310}", "\u{1F5C4}", "\u{26A1}", "\u{1F4E6}", "\u{1F50C}", "\u{1F4BB}",
  "\u{1F527}", "\u{1F4DA}", "\u{1F680}", "\u{1F9E9}", "\u{1F50D}", "\u{2699}", "\u{1F4CA}", "\u{1F3AF}",
];

const PLUGIN_EMOJIS = [
  "\u{1F50C}", "\u{1F9E9}", "\u{2699}", "\u{1F4E1}", "\u{1F517}", "\u{1F4A0}", "\u{1F3AE}",
  "\u{1F5A5}", "\u{1F4E2}", "\u{1F916}",
];

function getMcpEmoji(id: string): string {
  let hash = 0;
  for (let i = 0; i < id.length; i++) {
    hash = Math.trunc((hash << 5) - hash + (id.codePointAt(i) ?? 0));
  }
  return MCP_EMOJIS[Math.abs(hash) % MCP_EMOJIS.length];
}

function getPluginEmoji(id: string): string {
  let hash = 0;
  for (let i = 0; i < id.length; i++) {
    hash = Math.trunc((hash << 5) - hash + (id.codePointAt(i) ?? 0));
  }
  return PLUGIN_EMOJIS[Math.abs(hash) % PLUGIN_EMOJIS.length];
}

// ============================================================================
// Env Var Helpers
// ============================================================================

interface EnvVarEntry {
  id: string;
  key: string;
  value: string;
}

function recordToEnvVars(record: Record<string, string> | undefined): EnvVarEntry[] {
  if (!record) return [];
  return Object.entries(record).map(([key, value]) => ({
    id: randomId(),
    key,
    value,
  }));
}

function envVarsToRecord(envVars: EnvVarEntry[]): Record<string, string> | undefined {
  const filtered = envVars.filter((e) => e.key.trim() !== "");
  if (filtered.length === 0) return undefined;
  return Object.fromEntries(filtered.map((e) => [e.key.trim(), e.value]));
}

function formatUptime(connectedAt: Date): string {
  const diffMs = Date.now() - connectedAt.getTime();
  const diffSec = Math.floor(diffMs / 1000);
  if (diffSec < 60) return `${diffSec}s ago`;
  const diffMin = Math.floor(diffSec / 60);
  if (diffMin < 60) return `${diffMin}m ago`;
  const diffHr = Math.floor(diffMin / 60);
  if (diffHr < 24) return `${diffHr}h ${diffMin % 60}m ago`;
  const diffDay = Math.floor(diffHr / 24);
  return `${diffDay}d ${diffHr % 24}h ago`;
}

/** Map MCP type to MetaChip variant */
function typeToVariant(type: string): "stdio" | "http" | "sse" {
  if (type === "stdio") return "stdio";
  if (type === "sse") return "sse";
  return "http"; // http and streamable-http both use http variant
}

/** Map MCP type to card icon modifier */
function typeToIconClass(type: string): string {
  if (type === "stdio") return "ts-card__icon--stdio";
  if (type === "sse") return "ts-card__icon--sse";
  return "ts-card__icon--http";
}

function isOAuthMcp(config: Pick<McpServerConfig, "auth"> | Pick<CreateMcpRequest, "auth"> | null | undefined): boolean {
  return config?.auth?.type === "oauth2";
}

function mcpConfigId(config: Pick<McpServerConfig, "id" | "name">): string {
  return config.id || config.name;
}

function authStatusLabel(status?: McpAuthStatus): string {
  switch (status) {
    case "connected": return "Connected";
    case "reauth_required": return "Reconnect required";
    case "not_connected": return "Not connected";
    case "not_configured":
    default: return "Not configured";
  }
}

function authStatusVariant(status?: McpAuthStatus): "enabled" | "disabled" | "error" {
  if (status === "connected") return "enabled";
  if (status === "reauth_required") return "error";
  return "disabled";
}

// ============================================================================
// Component
// ============================================================================

export function WebIntegrationsPanel() {
  const [searchParams, setSearchParams] = useSearchParams();
  const activeTab = searchParams.get("tab") || "tools";

  const setActiveTab = useCallback(
    (tab: string) => {
      setSearchParams(tab === "tools" ? {} : { tab });
    },
    [setSearchParams],
  );

  // ── Transport ──
  const [transport, setTransport] = useState<Transport | null>(null);
  const [isLoading, setIsLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  // ── Tool Servers state ──
  const [mcpServers, setMcpServers] = useState<McpServerSummary[]>([]);
  const [mcpSearch, setMcpSearch] = useState("");
  const [mcpFilter, setMcpFilter] = useState<string>("all");

  // ── Workers/Plugins state ──
  const [workers, setWorkers] = useState<BridgeWorker[]>([]);
  const [plugins, setPlugins] = useState<PluginInfo[]>([]);
  const [workerSearch, setWorkerSearch] = useState("");
  const [workerFilter, setWorkerFilter] = useState<string>("all");
  const intervalRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const oauthPollRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const oauthPollAttemptRef = useRef(0);

  // ── Slideover state (Tool Server) ──
  const [tsSlideoverOpen, setTsSlideoverOpen] = useState(false);
  const [tsSlideoverMode, setTsSlideoverMode] = useState<"view" | "create" | "edit">("view");
  const [selectedMcpSummary, setSelectedMcpSummary] = useState<McpServerSummary | null>(null);
  const [selectedMcpDetail, setSelectedMcpDetail] = useState<McpServerConfig | null>(null);
  const [isTesting, setIsTesting] = useState(false);
  const [testResult, setTestResult] = useState<McpTestResult | null>(null);
  const [isAuthorizing, setIsAuthorizing] = useState(false);
  const [oauthMessage, setOauthMessage] = useState<string | null>(null);
  const [oauthAuthUrl, setOauthAuthUrl] = useState<string | null>(null);

  // ── Slideover state (Plugin/Worker) ──
  const [pwSlideoverOpen, setPwSlideoverOpen] = useState(false);
  const [selectedWorker, setSelectedWorker] = useState<BridgeWorker | null>(null);

  // ── Form state (Tool Server create/edit) ──
  const [formData, setFormData] = useState<Partial<CreateMcpRequest>>({
    type: "stdio",
    name: "",
    description: "",
    command: "",
    args: [],
    url: "",
    enabled: true,
  });
  const [argsInput, setArgsInput] = useState("");
  const [envVars, setEnvVars] = useState<EnvVarEntry[]>([]);
  const [showEnvValues, setShowEnvValues] = useState<Set<string>>(new Set());
  const [editingId, setEditingId] = useState<string | null>(null);

  // ── Data Loading ──

  const loadMcps = useCallback(async (t: Transport) => {
    try {
      const result = await t.listMcps();
      if (result.success && result.data) {
        setMcpServers(result.data.servers);
      } else {
        setError(result.error || "Failed to load tool servers");
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : "Unknown error");
    }
  }, []);

  const loadWorkers = useCallback(async (t: Transport) => {
    try {
      const result = await t.listBridgeWorkers();
      if (result.success && result.data) {
        setWorkers(result.data);
      }
    } catch {
      // Silently fail on worker polling
    }
  }, []);

  const loadPlugins = useCallback(async (t: Transport) => {
    try {
      const result = await t.listPlugins();
      if (result.success && result.data) {
        setPlugins(result.data.plugins || []);
      }
    } catch {
      // Silently fail on plugin polling
    }
  }, []);

  useEffect(() => {
    let mounted = true;
    (async () => {
      try {
        const t = await getTransport();
        if (!mounted) return;
        setTransport(t);
        await Promise.all([loadMcps(t), loadWorkers(t), loadPlugins(t)]);
      } catch (err) {
        if (mounted) setError(err instanceof Error ? err.message : "Unknown error");
      } finally {
        if (mounted) setIsLoading(false);
      }
    })();
    return () => { mounted = false; };
  }, [loadMcps, loadWorkers, loadPlugins]);

  // Worker + Plugin polling
  useEffect(() => {
    if (!transport) return;
    intervalRef.current = setInterval(() => {
      loadWorkers(transport);
      loadPlugins(transport);
    }, POLL_INTERVAL_MS);
    return () => {
      if (intervalRef.current) clearInterval(intervalRef.current);
    };
  }, [transport, loadWorkers, loadPlugins]);

  // ── Tool Server Actions ──

  const loadMcpDetail = useCallback(async (id: string) => {
    if (!transport) return;
    try {
      const result = await transport.getMcp(id);
      if (result.success && result.data) {
        setSelectedMcpDetail(result.data);
      }
    } catch (err) {
      console.error("Failed to load MCP detail:", err);
    }
  }, [transport]);

  const clearOAuthPoll = useCallback(() => {
    if (oauthPollRef.current) {
      clearTimeout(oauthPollRef.current);
      oauthPollRef.current = null;
    }
  }, []);

  useEffect(() => clearOAuthPoll, [clearOAuthPoll]);

  const refreshMcpOAuthStatus = useCallback(async (id: string): Promise<McpAuthStatus | null> => {
    if (!transport) return null;
    const result = await transport.getMcpOAuthStatus(id);
    if (!result.success || !result.data) return null;
    const { status } = result.data;
    setSelectedMcpSummary((current) =>
      current && current.id === id
        ? { ...current, authStatus: status }
        : current,
    );
    setMcpServers((current) =>
      current.map((mcp) =>
        mcp.id === id ? { ...mcp, authStatus: status } : mcp,
      ),
    );
    return status;
  }, [transport]);

  const waitForMcpOAuthConnection = useCallback((id: string) => {
    clearOAuthPoll();
    oauthPollAttemptRef.current = 0;

    const poll = async () => {
      const status = await refreshMcpOAuthStatus(id);
      if (status === "connected") {
        clearOAuthPoll();
        setOauthAuthUrl(null);
        setOauthMessage("Authorization complete. This tool server is connected.");
        if (transport) {
          await loadMcps(transport);
        }
        return;
      }

      if (status === "reauth_required") {
        clearOAuthPoll();
        setOauthMessage("Authorization did not complete. Retry Authorize and check the callback tab for the exact error.");
        return;
      }

      oauthPollAttemptRef.current += 1;
      if (oauthPollAttemptRef.current >= OAUTH_STATUS_POLL_MAX_ATTEMPTS) {
        clearOAuthPoll();
        setOauthMessage("Still waiting for authorization. If the provider tab shows an error, retry Authorize.");
        return;
      }

      oauthPollRef.current = setTimeout(poll, OAUTH_STATUS_POLL_INTERVAL_MS);
    };

    void poll();
  }, [clearOAuthPoll, refreshMcpOAuthStatus, loadMcps, transport]);

  const handleSelectMcp = useCallback(async (mcp: McpServerSummary) => {
    setSelectedMcpSummary(mcp);
    setTestResult(null);
    setOauthMessage(null);
    setOauthAuthUrl(null);
    setTsSlideoverMode("view");
    setTsSlideoverOpen(true);
    // Load detail
    if (!transport) return;
    try {
      const result = await transport.getMcp(mcp.id);
      if (result.success && result.data) {
        setSelectedMcpDetail(result.data);
      }
    } catch (err) {
      console.error("Failed to load MCP detail:", err);
    }
  }, [transport]);

  const handleOpenCreate = useCallback(() => {
    setTsSlideoverMode("create");
    setEditingId(null);
    setFormData({
      type: "stdio",
      name: "",
      description: "",
      command: "",
      args: [],
      url: "",
      enabled: true,
    });
    setArgsInput("");
    setEnvVars([]);
    setShowEnvValues(new Set());
    setSelectedMcpSummary(null);
    setSelectedMcpDetail(null);
    setTestResult(null);
    setOauthMessage(null);
    setOauthAuthUrl(null);
    setTsSlideoverOpen(true);
  }, []);

  const handleOpenEdit = useCallback(() => {
    if (!selectedMcpDetail) return;
    setTsSlideoverMode("edit");
    setEditingId(selectedMcpDetail.id || selectedMcpSummary?.id || null);
    setFormData({
      type: selectedMcpDetail.type,
      name: selectedMcpDetail.name,
      description: selectedMcpDetail.description,
      command: selectedMcpDetail.command || "",
      url: selectedMcpDetail.url || "",
      auth: selectedMcpDetail.auth,
      enabled: selectedMcpDetail.enabled,
    });
    setArgsInput(selectedMcpDetail.args?.join(", ") || "");
    setEnvVars(recordToEnvVars(selectedMcpDetail.env));
    setShowEnvValues(new Set());
    setOauthMessage(null);
    setOauthAuthUrl(null);
  }, [selectedMcpDetail, selectedMcpSummary]);

  const showSavedMcp = useCallback(async (saved: McpServerConfig) => {
    const id = mcpConfigId(saved);
    await loadMcps(transport!);
    setSelectedMcpDetail(saved);
    setSelectedMcpSummary({
      id,
      name: saved.name,
      description: saved.description,
      type: saved.type,
      enabled: saved.enabled,
      authStatus: isOAuthMcp(saved) ? "not_connected" : undefined,
    });
    setTsSlideoverMode("view");
    setTsSlideoverOpen(true);
    if (isOAuthMcp(saved)) {
      await refreshMcpOAuthStatus(id);
    }
  }, [transport, loadMcps, refreshMcpOAuthStatus]);

  const handleSaveSuccess = useCallback(async () => {
    await loadMcps(transport!);
    if (tsSlideoverMode === "edit" && selectedMcpSummary) {
      await loadMcpDetail(selectedMcpSummary.id);
      setTsSlideoverMode("view");
    } else {
      setTsSlideoverOpen(false);
    }
  }, [transport, tsSlideoverMode, selectedMcpSummary, loadMcps, loadMcpDetail]);

  const handleSave = useCallback(async () => {
    if (!transport || !formData.name || !formData.type) return;

    try {
      const args = argsInput.split(",").map((a) => a.trim()).filter(Boolean);

      const request: CreateMcpRequest = {
        type: formData.type,
        name: formData.name,
        description: formData.description || "",
        enabled: formData.enabled ?? true,
      };

      if (tsSlideoverMode === "edit" && editingId) {
        request.id = editingId;
      }

      if (formData.type === "stdio") {
        request.command = formData.command || "";
        request.args = args;
        request.env = envVarsToRecord(envVars);
      } else {
        request.url = formData.url || "";
        request.auth = formData.auth;
      }

      let result;
      if (tsSlideoverMode === "edit" && editingId) {
        result = await transport.updateMcp(editingId, request);
      } else {
        result = await transport.createMcp(request);
      }

      if (result.success && result.data && isOAuthMcp(result.data)) {
        await showSavedMcp(result.data);
      } else if (result.success) {
        await handleSaveSuccess();
      } else {
        setError(result.error || `Failed to ${tsSlideoverMode} tool server`);
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : "Unknown error");
    }
  }, [transport, formData, argsInput, envVars, tsSlideoverMode, editingId, handleSaveSuccess, showSavedMcp]);

  const handleDelete = useCallback(async () => {
    if (!transport || !selectedMcpSummary) return;
    if (!confirm("Are you sure you want to delete this tool server?")) return;

    try {
      const result = await transport.deleteMcp(selectedMcpSummary.id);
      if (result.success) {
        setTsSlideoverOpen(false);
        setSelectedMcpSummary(null);
        setSelectedMcpDetail(null);
        await loadMcps(transport);
      } else {
        setError(result.error || "Failed to delete tool server");
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : "Unknown error");
    }
  }, [transport, selectedMcpSummary, loadMcps]);

  const handleTest = useCallback(async () => {
    if (!transport || !selectedMcpSummary) return;

    setIsTesting(true);
    setTestResult(null);

    try {
      const result = await transport.testMcp(selectedMcpSummary.id);
      if (result.success && result.data) {
        setTestResult(result.data);
      } else {
        setTestResult({ success: false, message: result.error || "Test failed" });
      }
    } catch (err) {
      setTestResult({ success: false, message: err instanceof Error ? err.message : "Unknown error" });
    } finally {
      setIsTesting(false);
    }
  }, [transport, selectedMcpSummary]);

  const handleAuthorizeMcp = useCallback(async () => {
    if (!transport || !selectedMcpSummary) return;
    const authWindow = window.open("about:blank", "_blank");
    if (authWindow) {
      authWindow.opener = null;
    }
    clearOAuthPoll();
    setIsAuthorizing(true);
    setOauthMessage(null);
    setOauthAuthUrl(null);

    try {
      const redirectUri = `${window.location.origin}/api/mcps/oauth/callback`;
      const result = await transport.startMcpOAuth(selectedMcpSummary.id, { redirectUri });
      if (result.success && result.data) {
        setOauthAuthUrl(result.data.authUrl);
        if (authWindow) {
          authWindow.location.href = result.data.authUrl;
          setOauthMessage("Authorization opened in a new tab. Waiting for the provider callback.");
        } else {
          setOauthMessage("Popup blocked. Use the authorization link below. This page will update after the callback completes.");
        }
        waitForMcpOAuthConnection(selectedMcpSummary.id);
      } else {
        authWindow?.close();
        setError(result.error || "Failed to start OAuth authorization");
      }
    } catch (err) {
      authWindow?.close();
      setError(err instanceof Error ? err.message : "Unknown OAuth error");
    } finally {
      setIsAuthorizing(false);
    }
  }, [transport, selectedMcpSummary, clearOAuthPoll, waitForMcpOAuthConnection]);

  const handleRefreshMcpAuth = useCallback(async () => {
    if (!selectedMcpSummary) return;
    await refreshMcpOAuthStatus(selectedMcpSummary.id);
    await loadMcps(transport!);
  }, [selectedMcpSummary, refreshMcpOAuthStatus, loadMcps, transport]);

  // ── Filtered lists ──

  const filteredMcps = useMemo(() => {
    let list = mcpServers;
    if (mcpFilter !== "all") {
      list = list.filter((m) => {
        if (mcpFilter === "http") return m.type === "http" || m.type === "streamable-http";
        return m.type === mcpFilter;
      });
    }
    if (mcpSearch.trim()) {
      const q = mcpSearch.toLowerCase();
      list = list.filter((m) =>
        m.name.toLowerCase().includes(q) || m.description.toLowerCase().includes(q),
      );
    }
    return list;
  }, [mcpServers, mcpFilter, mcpSearch]);

  const filteredPlugins = useMemo(() => {
    if (workerFilter === "workers") return [];
    let list = plugins;
    if (workerSearch.trim()) {
      const q = workerSearch.toLowerCase();
      list = list.filter((p) =>
        p.name.toLowerCase().includes(q) || p.id.toLowerCase().includes(q) || p.description.toLowerCase().includes(q),
      );
    }
    return list;
  }, [plugins, workerFilter, workerSearch]);

  // Workers that are NOT plugins (plugins have their own cards from /api/plugins)
  const pluginIds = useMemo(() => new Set(plugins.map((p) => p.id)), [plugins]);

  const filteredWorkers = useMemo(() => {
    if (workerFilter === "plugins") return [];
    // Exclude bridge workers that are actually plugins
    let list = workers.filter((w) => !pluginIds.has(w.adapter_id));
    if (workerSearch.trim()) {
      const q = workerSearch.toLowerCase();
      list = list.filter((w) => w.adapter_id.toLowerCase().includes(q));
    }
    return list;
  }, [workers, workerFilter, workerSearch, pluginIds]);

  // ── Render ──

  if (isLoading) {
    return (
      <div className="page" style={{ display: "flex", alignItems: "center", justifyContent: "center" }}>
        <Loader2 className="w-6 h-6 animate-spin" style={{ color: "var(--primary)" }} />
      </div>
    );
  }

  return (
    <div className="page" style={{ display: "flex", flexDirection: "column" }}>
      <div className="page-header-v2">
        <h1 className="page-title-v2">Integrations</h1>
        <p className="page-subtitle-v2">
          Connect z-Bot to external tools, services, and plugins. Give your agents new abilities.
        </p>
      </div>

      <TabBar
        tabs={[
          { id: "tools", label: "Tool Servers", count: mcpServers.length },
          { id: "plugins", label: "Plugins & Workers", count: plugins.length + workers.length },
        ]}
        activeTab={activeTab}
        onTabChange={setActiveTab}
      />

      {error && (
        <div className="alert alert--error" style={{ margin: "0 var(--spacing-9) var(--spacing-4)" }}>
          <span>{error}</span>
          <button className="btn btn--ghost btn--sm" onClick={() => setError(null)} style={{ marginLeft: "auto" }}>
            <X style={{ width: 14, height: 14 }} />
          </button>
        </div>
      )}

      <div className="page-content-v2">
        <TabPanel id="tools" activeTab={activeTab}>
          <ToolServersTab
            mcps={filteredMcps}
            search={mcpSearch}
            onSearchChange={setMcpSearch}
            filter={mcpFilter}
            onFilterChange={setMcpFilter}
            onSelect={handleSelectMcp}
            onCreate={handleOpenCreate}
          />
        </TabPanel>

        <TabPanel id="plugins" activeTab={activeTab}>
          <PluginsWorkersTab
            plugins={filteredPlugins}
            workers={filteredWorkers}
            allWorkers={workers}
            search={workerSearch}
            onSearchChange={setWorkerSearch}
            filter={workerFilter}
            onFilterChange={setWorkerFilter}
            onSelect={(w) => { setSelectedWorker(w); setPwSlideoverOpen(true); }}
          />
        </TabPanel>
      </div>

      {/* Tool Server Slideover */}
      <Slideover
        open={tsSlideoverOpen}
        onClose={() => { setTsSlideoverOpen(false); if (tsSlideoverMode !== "view") { setTsSlideoverMode("view"); } }}
        title={
          tsSlideoverMode === "create"
            ? "Add Tool Server"
            : tsSlideoverMode === "edit"
              ? "Edit Tool Server"
              : selectedMcpDetail?.name || "Tool Server"
        }
        subtitle={
          tsSlideoverMode === "view" && selectedMcpSummary
            ? selectedMcpSummary.id
            : tsSlideoverMode === "create"
              ? "Configure a new MCP tool server"
              : "Update the tool server configuration"
        }
        icon={
          <div className={`ts-card__icon ${selectedMcpSummary ? typeToIconClass(selectedMcpSummary.type) : "ts-card__icon--stdio"}`}>
            {selectedMcpSummary ? getMcpEmoji(selectedMcpSummary.id) : <Server style={{ width: 20, height: 20 }} />}
          </div>
        }
        footer={
          tsSlideoverMode === "view" ? (
            <div style={{ display: "flex", gap: "var(--spacing-2)", width: "100%" }}>
              {isOAuthMcp(selectedMcpDetail) && selectedMcpSummary?.authStatus !== "connected" && (
                <button className="btn btn--primary btn--sm" onClick={handleAuthorizeMcp} disabled={isAuthorizing}>
                  {isAuthorizing ? <Loader2 style={{ width: 14, height: 14 }} className="animate-spin" /> : <Key style={{ width: 14, height: 14 }} />}
                  Authorize
                </button>
              )}
              {isOAuthMcp(selectedMcpDetail) && (
                <button className="btn btn--outline btn--sm" onClick={handleRefreshMcpAuth}>
                  <RefreshCw style={{ width: 14, height: 14 }} /> Refresh
                </button>
              )}
              <button className="btn btn--outline btn--sm" onClick={handleTest} disabled={isTesting}>
                {isTesting ? <Loader2 style={{ width: 14, height: 14 }} className="animate-spin" /> : <Play style={{ width: 14, height: 14 }} />}
                Test
              </button>
              <button className="btn btn--outline btn--sm" onClick={handleOpenEdit}>
                <Pencil style={{ width: 14, height: 14 }} /> Edit
              </button>
              <button className="btn btn--ghost btn--sm" onClick={handleDelete} style={{ marginLeft: "auto", color: "var(--destructive)" }}>
                <Trash2 style={{ width: 14, height: 14 }} /> Delete
              </button>
            </div>
          ) : (
            <div style={{ display: "flex", gap: "var(--spacing-2)", width: "100%", justifyContent: "flex-end" }}>
              <button
                className="btn btn--ghost btn--sm"
                onClick={() => {
                  if (tsSlideoverMode === "edit") { setTsSlideoverMode("view"); }
                  else { setTsSlideoverOpen(false); }
                }}
              >
                Cancel
              </button>
              <button
                className="btn btn--primary btn--sm"
                onClick={handleSave}
                disabled={!formData.name || (formData.type === "stdio" ? !formData.command : !formData.url)}
              >
                {tsSlideoverMode === "create" ? "Add Server" : "Save Changes"}
              </button>
            </div>
          )
        }
      >
        {tsSlideoverMode === "view" ? (
          <ToolServerDetail
            summary={selectedMcpSummary}
            detail={selectedMcpDetail}
            testResult={testResult}
            oauthMessage={oauthMessage}
            oauthAuthUrl={oauthAuthUrl}
          />
        ) : (
          <ToolServerForm
            formData={formData}
            setFormData={setFormData}
            argsInput={argsInput}
            setArgsInput={setArgsInput}
            envVars={envVars}
            setEnvVars={setEnvVars}
            showEnvValues={showEnvValues}
            setShowEnvValues={setShowEnvValues}
          />
        )}
      </Slideover>

      {/* Plugin/Worker Slideover */}
      <Slideover
        open={pwSlideoverOpen}
        onClose={() => setPwSlideoverOpen(false)}
        title={selectedWorker?.adapter_id || "Worker"}
        subtitle={selectedWorker ? `Connected ${formatUptime(new Date(selectedWorker.connected_at))}` : undefined}
        icon={
          <div className={`pw-card__icon ${selectedWorker?.adapter_id.startsWith("plugin:") ? "pw-card__icon--plugin" : "pw-card__icon--worker"}`}>
            {selectedWorker ? getPluginEmoji(selectedWorker.adapter_id) : <Cpu style={{ width: 20, height: 20 }} />}
          </div>
        }
      >
        {selectedWorker && <WorkerDetail worker={selectedWorker} />}
      </Slideover>
    </div>
  );
}

// ============================================================================
// Tool Servers Tab
// ============================================================================

interface ToolServersTabProps {
  mcps: McpServerSummary[];
  search: string;
  onSearchChange: (v: string) => void;
  filter: string;
  onFilterChange: (v: string) => void;
  onSelect: (mcp: McpServerSummary) => void;
  onCreate: () => void;
}

function ToolServersTab({ mcps, search, onSearchChange, filter, onFilterChange, onSelect, onCreate }: ToolServersTabProps) {
  return (
    <>
      <HelpBox>
        Tool Servers give your agents superpowers &mdash; access to files, databases, APIs, and more.
        They follow the Model Context Protocol (MCP) standard.
      </HelpBox>

      <ActionBar
        searchPlaceholder="Search tool servers..."
        searchValue={search}
        onSearchChange={onSearchChange}
        filters={
          <>
            <FilterChip label="All" active={filter === "all"} onClick={() => onFilterChange("all")} />
            <FilterChip label="stdio" active={filter === "stdio"} onClick={() => onFilterChange("stdio")} />
            <FilterChip label="http" active={filter === "http"} onClick={() => onFilterChange("http")} />
            <FilterChip label="sse" active={filter === "sse"} onClick={() => onFilterChange("sse")} />
          </>
        }
        actions={
          <button className="btn btn--primary btn--sm" onClick={onCreate}>
            <Plus style={{ width: 14, height: 14 }} /> Add Tool Server
          </button>
        }
      />

      {mcps.length === 0 ? (
        <EmptyState
          icon={Server}
          title="No tool servers"
          description="Tool servers give your agents abilities like reading files, querying databases, and calling APIs."
          action={{ label: "Add Tool Server", onClick: onCreate }}
        />
      ) : (
        <div className="card-grid">
          {mcps.map((mcp) => (
            <ToolServerCard key={mcp.id} mcp={mcp} onClick={() => onSelect(mcp)} />
          ))}
        </div>
      )}
    </>
  );
}

// ============================================================================
// Tool Server Card
// ============================================================================

function ToolServerCard({ mcp, onClick }: { mcp: McpServerSummary; onClick: () => void }) {
  const variant = typeToVariant(mcp.type);
  const iconClass = typeToIconClass(mcp.type);

  return (
    <div className="ts-card" role="button" tabIndex={0} onClick={onClick} onKeyDown={(e) => { if (e.key === "Enter" || e.key === " ") onClick(); }}>
      <div className="ts-card__top">
        <div className={`ts-card__icon ${iconClass}`}>
          {getMcpEmoji(mcp.id)}
        </div>
        <div className="ts-card__info">
          <div className="ts-card__name">{mcp.name}</div>
          <div className="ts-card__type-row">
            <MetaChip variant={variant}>
              {mcp.type === "stdio" && <Terminal style={{ width: 12, height: 12 }} />}
              {(mcp.type === "http" || mcp.type === "streamable-http") && <Globe style={{ width: 12, height: 12 }} />}
              {mcp.type === "sse" && <RefreshCw style={{ width: 12, height: 12 }} />}
              {mcp.type}
            </MetaChip>
          </div>
        </div>
      </div>

      {mcp.description && (
        <div className="ts-card__desc">{mcp.description}</div>
      )}

      <div className="ts-card__meta">
        {mcp.authStatus && mcp.authStatus !== "not_configured" && (
          <MetaChip variant={authStatusVariant(mcp.authStatus)}>
            <Key style={{ width: 12, height: 12 }} />
            {authStatusLabel(mcp.authStatus)}
          </MetaChip>
        )}
        <MetaChip variant={mcp.enabled ? "enabled" : "disabled"}>
          <Power style={{ width: 12, height: 12 }} />
          {mcp.enabled ? "Enabled" : "Disabled"}
        </MetaChip>
      </div>

      <div className="ts-card__footer">
        <div className="ts-card__footer-left">
          <Server style={{ width: 12, height: 12 }} />
          <span>{mcp.id}</span>
        </div>
        <div className="ts-card__footer-actions">
          <button
            className="btn btn--icon-ghost btn--sm"
            onClick={(e) => { e.stopPropagation(); onClick(); }}
            title="View details"
          >
            <Eye style={{ width: 14, height: 14 }} />
          </button>
        </div>
      </div>
    </div>
  );
}

// ============================================================================
// Tool Server Detail (View Mode in Slideover)
// ============================================================================

interface ToolServerDetailProps {
  summary: McpServerSummary | null;
  detail: McpServerConfig | null;
  testResult: McpTestResult | null;
  oauthMessage: string | null;
  oauthAuthUrl: string | null;
}

function ToolServerDetail({ summary, detail, testResult, oauthMessage, oauthAuthUrl }: ToolServerDetailProps) {
  if (!summary || !detail) {
    return (
      <div style={{ padding: "var(--spacing-6)", textAlign: "center", color: "var(--muted-foreground)" }}>
        <Loader2 className="animate-spin" style={{ width: 24, height: 24, margin: "0 auto var(--spacing-3)" }} />
        Loading details...
      </div>
    );
  }

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: "var(--spacing-5)" }}>
      {/* Test result */}
      {oauthMessage && (
        <div className="ts-test-result ts-test-result--success">
          <Key style={{ width: 18, height: 18, flexShrink: 0 }} />
          <span className="ts-test-result__text">
            {oauthMessage}
            {oauthAuthUrl && (
              <>
                {" "}
                <a href={oauthAuthUrl} target="_blank" rel="noreferrer">Open authorization</a>
              </>
            )}
          </span>
        </div>
      )}

      {testResult && (
        <div className={`ts-test-result ${testResult.success ? "ts-test-result--success" : "ts-test-result--error"}`}>
          {testResult.success ? (
            <Check style={{ width: 18, height: 18, flexShrink: 0 }} />
          ) : (
            <X style={{ width: 18, height: 18, flexShrink: 0 }} />
          )}
          <span className="ts-test-result__text">{testResult.message}</span>
        </div>
      )}

      {/* Details */}
      <div>
        <h4 style={{ fontSize: "var(--text-sm)", fontWeight: 600, color: "var(--foreground)", marginBottom: "var(--spacing-3)" }}>
          Details
        </h4>

        <div className="ts-detail-row">
          <span className="ts-detail-label">Type</span>
          <span className="ts-detail-value">
            <MetaChip variant={typeToVariant(detail.type)}>
              {detail.type === "stdio" ? <Terminal style={{ width: 12, height: 12 }} /> : <Globe style={{ width: 12, height: 12 }} />}
              {detail.type}
            </MetaChip>
          </span>
        </div>

        {detail.type === "stdio" && detail.command && (
          <div className="ts-detail-row">
            <span className="ts-detail-label">Command</span>
            <span className="ts-detail-value ts-detail-value--mono">{detail.command}</span>
          </div>
        )}

        {detail.type === "stdio" && detail.args && detail.args.length > 0 && (
          <div className="ts-detail-row">
            <span className="ts-detail-label">Arguments</span>
            <span className="ts-detail-value ts-detail-value--mono">{detail.args.join(" ")}</span>
          </div>
        )}

        {detail.type === "stdio" && detail.env && Object.keys(detail.env).length > 0 && (
          <div className="ts-detail-row">
            <span className="ts-detail-label">Env Vars</span>
            <div className="ts-detail-value" style={{ display: "flex", flexDirection: "column", gap: "var(--spacing-1)" }}>
              {Object.entries(detail.env).map(([key, value]) => (
                <span key={key} className="ts-detail-value--mono">
                  {key}={"*".repeat(Math.min(value.length, 12))}
                </span>
              ))}
            </div>
          </div>
        )}

        {detail.type !== "stdio" && detail.url && (
          <div className="ts-detail-row">
            <span className="ts-detail-label">URL</span>
            <span className="ts-detail-value ts-detail-value--mono">{detail.url}</span>
          </div>
        )}

        {isOAuthMcp(detail) && (
          <div className="ts-detail-row">
            <span className="ts-detail-label">Auth</span>
            <span className="ts-detail-value">
              <MetaChip variant={authStatusVariant(summary.authStatus)}>
                <Key style={{ width: 12, height: 12 }} />
                OAuth 2.0 · {authStatusLabel(summary.authStatus)}
              </MetaChip>
            </span>
          </div>
        )}

        {detail.description && (
          <div className="ts-detail-row">
            <span className="ts-detail-label">Description</span>
            <span className="ts-detail-value">{detail.description}</span>
          </div>
        )}

        <div className="ts-detail-row">
          <span className="ts-detail-label">Status</span>
          <span className="ts-detail-value">
            <MetaChip variant={detail.enabled ? "enabled" : "disabled"}>
              <Power style={{ width: 12, height: 12 }} />
              {detail.enabled ? "Enabled" : "Disabled"}
            </MetaChip>
          </span>
        </div>
      </div>

      {/* Discovered tools from test */}
      {testResult?.success && testResult.tools && testResult.tools.length > 0 && (
        <div>
          <h4 style={{ fontSize: "var(--text-sm)", fontWeight: 600, color: "var(--foreground)", marginBottom: "var(--spacing-3)", display: "flex", alignItems: "center", gap: "var(--spacing-2)" }}>
            <Wrench style={{ width: 14, height: 14 }} />
            Discovered Tools ({testResult.tools.length})
          </h4>
          <div style={{ display: "flex", flexDirection: "column", gap: "var(--spacing-2)" }}>
            {testResult.tools.map((tool) => (
              <div key={tool} className="ts-tool-item">
                <div className="ts-tool-item__name">{tool}</div>
              </div>
            ))}
          </div>
        </div>
      )}

      {/* Usage hint */}
      <HelpBox>
        <span style={{ fontSize: "var(--text-sm)" }}>
          To use this tool server, add its ID to an agent's <code style={{ fontFamily: "var(--font-mono)", fontSize: "var(--text-xs)", padding: "2px 6px", background: "var(--muted)", borderRadius: "var(--radius-sm)" }}>mcps</code> field:
        </span>
        <code style={{ display: "block", fontFamily: "var(--font-mono)", fontSize: "var(--text-xs)", marginTop: "var(--spacing-2)", padding: "var(--spacing-2) var(--spacing-3)", background: "var(--background-surface)", border: "1px solid var(--border)", borderRadius: "var(--radius-md)" }}>
          "mcps": ["{summary.id}"]
        </code>
      </HelpBox>
    </div>
  );
}

// ============================================================================
// Tool Server Form (Create/Edit Mode in Slideover)
// ============================================================================

interface ToolServerFormProps {
  formData: Partial<CreateMcpRequest>;
  setFormData: (data: Partial<CreateMcpRequest>) => void;
  argsInput: string;
  setArgsInput: (v: string) => void;
  envVars: EnvVarEntry[];
  setEnvVars: React.Dispatch<React.SetStateAction<EnvVarEntry[]>>;
  showEnvValues: Set<string>;
  setShowEnvValues: React.Dispatch<React.SetStateAction<Set<string>>>;
}

function ToolServerForm({
  formData,
  setFormData,
  argsInput,
  setArgsInput,
  envVars,
  setEnvVars,
  showEnvValues,
  setShowEnvValues,
}: ToolServerFormProps) {
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: "var(--spacing-4)" }}>
      {/* Type selector */}
      <div className="form-group">
        <label className="form-label" htmlFor="ts-type">Type</label>
        <select
          id="ts-type"
          className="form-select"
          value={formData.type}
          onChange={(e) => {
            const type = e.target.value as CreateMcpRequest["type"];
            setFormData({
              ...formData,
              type,
              auth: type === "stdio" ? undefined : formData.auth,
            });
          }}
        >
          <option value="stdio">Stdio (Local Process)</option>
          <option value="http">HTTP</option>
          <option value="sse">SSE (Server-Sent Events)</option>
          <option value="streamable-http">Streamable HTTP</option>
        </select>
      </div>

      {/* Name */}
      <div className="form-group">
        <label className="form-label" htmlFor="ts-name">Name</label>
        <input
          id="ts-name"
          className="form-input"
          type="text"
          value={formData.name || ""}
          onChange={(e) => setFormData({ ...formData, name: e.target.value })}
          placeholder="filesystem, weather, etc."
        />
      </div>

      {/* Description */}
      <div className="form-group">
        <label className="form-label" htmlFor="ts-description">Description</label>
        <input
          id="ts-description"
          className="form-input"
          type="text"
          value={formData.description || ""}
          onChange={(e) => setFormData({ ...formData, description: e.target.value })}
          placeholder="What this server provides"
        />
      </div>

      {/* Stdio-specific fields */}
      {formData.type === "stdio" ? (
        <>
          <div className="form-group">
            <label className="form-label" htmlFor="ts-command">Command</label>
            <input
              id="ts-command"
              className="form-input"
              type="text"
              value={formData.command || ""}
              onChange={(e) => setFormData({ ...formData, command: e.target.value })}
              placeholder="npx, node, python, etc."
            />
          </div>

          <div className="form-group">
            <label className="form-label" htmlFor="ts-args">Arguments (comma-separated)</label>
            <input
              id="ts-args"
              className="form-input"
              type="text"
              value={argsInput}
              onChange={(e) => setArgsInput(e.target.value)}
              placeholder="-y, @anthropic/mcp-server-filesystem, /home/user"
            />
          </div>

          {/* Environment Variables */}
          <div className="form-group">
            <label className="form-label" style={{ display: "flex", alignItems: "center", gap: "var(--spacing-1)" }}>
              <Key style={{ width: 14, height: 14 }} />
              Environment Variables
            </label>
            <div style={{ display: "flex", flexDirection: "column", gap: "var(--spacing-2)" }}>
              {envVars.map((envVar) => (
                <div key={envVar.id} className="env-var-row">
                  <input
                    className="form-input"
                    type="text"
                    value={envVar.key}
                    onChange={(e) => {
                      setEnvVars((prev) =>
                        prev.map((ev) =>
                          ev.id === envVar.id ? { ...ev, key: e.target.value } : ev,
                        ),
                      );
                    }}
                    placeholder="VARIABLE_NAME"
                    style={{ fontFamily: "var(--font-mono)" }}
                  />
                  <div style={{ flex: 1, position: "relative" }}>
                    <input
                      className="form-input"
                      type={showEnvValues.has(envVar.id) ? "text" : "password"}
                      value={envVar.value}
                      onChange={(e) => {
                        setEnvVars((prev) =>
                          prev.map((ev) =>
                            ev.id === envVar.id ? { ...ev, value: e.target.value } : ev,
                          ),
                        );
                      }}
                      placeholder="value"
                      style={{ fontFamily: "var(--font-mono)", paddingRight: "var(--spacing-8)" }}
                    />
                    <button
                      type="button"
                      onClick={() => {
                        setShowEnvValues((prev) => {
                          const next = new Set(prev);
                          if (next.has(envVar.id)) next.delete(envVar.id);
                          else next.add(envVar.id);
                          return next;
                        });
                      }}
                      style={{ position: "absolute", right: 8, top: "50%", transform: "translateY(-50%)", background: "none", border: "none", cursor: "pointer", color: "var(--muted-foreground)" }}
                    >
                      {showEnvValues.has(envVar.id) ? (
                        <EyeOff style={{ width: 14, height: 14 }} />
                      ) : (
                        <Eye style={{ width: 14, height: 14 }} />
                      )}
                    </button>
                  </div>
                  <button
                    type="button"
                    className="btn btn--icon-ghost btn--sm"
                    onClick={() => setEnvVars((prev) => prev.filter((ev) => ev.id !== envVar.id))}
                    style={{ color: "var(--destructive)" }}
                  >
                    <Trash2 style={{ width: 14, height: 14 }} />
                  </button>
                </div>
              ))}
              <button
                type="button"
                className="btn btn--ghost btn--sm"
                onClick={() => setEnvVars((prev) => [...prev, { id: randomId(), key: "", value: "" }])}
                style={{ alignSelf: "flex-start" }}
              >
                <Plus style={{ width: 14, height: 14 }} /> Add Variable
              </button>
            </div>
          </div>
        </>
      ) : (
        <>
          <div className="form-group">
            <label className="form-label" htmlFor="ts-url">URL</label>
            <input
              id="ts-url"
              className="form-input"
              type="text"
              value={formData.url || ""}
              onChange={(e) => setFormData({ ...formData, url: e.target.value })}
              placeholder="http://localhost:8080/mcp"
            />
          </div>

          <div className="form-group">
            <label htmlFor="ts-oauth" style={{ display: "flex", alignItems: "center", gap: "var(--spacing-2)", fontSize: "var(--text-sm)", color: "var(--foreground)" }}>
              <input
                type="checkbox"
                id="ts-oauth"
                checked={isOAuthMcp(formData)}
                onChange={(e) => setFormData({
                  ...formData,
                  auth: e.target.checked ? { type: "oauth2" } : undefined,
                })}
                style={{ width: 16, height: 16 }}
              />
              OAuth 2.0
            </label>
          </div>
        </>
      )}

      {/* Enabled toggle */}
      <div style={{ display: "flex", alignItems: "center", gap: "var(--spacing-2)" }}>
        <input
          type="checkbox"
          id="ts-enabled"
          checked={formData.enabled ?? true}
          onChange={(e) => setFormData({ ...formData, enabled: e.target.checked })}
          style={{ width: 16, height: 16 }}
        />
        <label htmlFor="ts-enabled" style={{ fontSize: "var(--text-sm)", color: "var(--foreground)" }}>
          Enabled
        </label>
      </div>
    </div>
  );
}

// ============================================================================
// Plugins & Workers Tab
// ============================================================================

interface PluginsWorkersTabProps {
  plugins: PluginInfo[];
  workers: BridgeWorker[];
  allWorkers: BridgeWorker[];
  search: string;
  onSearchChange: (v: string) => void;
  filter: string;
  onFilterChange: (v: string) => void;
  onSelect: (w: BridgeWorker) => void;
}

function PluginsWorkersTab({ plugins, workers, allWorkers, search, onSearchChange, filter, onFilterChange, onSelect }: PluginsWorkersTabProps) {
  const isEmpty = plugins.length === 0 && workers.length === 0;
  const paths = usePaths();
  const pluginsHint = paths?.pluginsDirDisplay ?? "~/Documents/zbot/plugins";

  return (
    <>
      <HelpBox icon={<Puzzle style={{ width: 16, height: 16 }} />}>
        Plugins auto-start when z-Bot launches. Drop a plugin folder into {pluginsHint}/ and restart.
      </HelpBox>

      <ActionBar
        searchPlaceholder="Search plugins & workers..."
        searchValue={search}
        onSearchChange={onSearchChange}
        filters={
          <>
            <FilterChip label="All" active={filter === "all"} onClick={() => onFilterChange("all")} />
            <FilterChip label="Plugins" active={filter === "plugins"} onClick={() => onFilterChange("plugins")} />
            <FilterChip label="Workers" active={filter === "workers"} onClick={() => onFilterChange("workers")} />
          </>
        }
      />

      {isEmpty ? (
        <EmptyState
          icon={Cable}
          title="No plugins or workers connected"
          description={`To install a plugin, drop its folder into ${pluginsHint}/ and restart zbotd`}
        />
      ) : (
        <div className="card-grid">
          {plugins.map((p) => {
            const bridgeData = allWorkers.find((w) => w.adapter_id === p.id);
            return <PluginCard key={p.id} plugin={p} bridgeWorker={bridgeData} />;
          })}
          {workers.map((w) => (
            <PluginWorkerCard key={w.adapter_id} worker={w} onClick={() => onSelect(w)} />
          ))}
        </div>
      )}
    </>
  );
}

// ============================================================================
// Plugin Card (from /api/plugins)
// ============================================================================

function pluginStateVariant(state: PluginInfo["state"]): "running" | "error" | "stopped" | "starting" {
  switch (state) {
    case "running": return "running";
    case "failed": return "error";
    case "stopped": return "stopped";
    case "starting": return "starting";
  }
}

function pluginStateLabel(state: PluginInfo["state"]): string {
  switch (state) {
    case "running": return "Running";
    case "failed": return "Failed";
    case "stopped": return "Stopped";
    case "starting": return "Starting";
  }
}

function PluginCard({ plugin, bridgeWorker }: { plugin: PluginInfo; bridgeWorker?: BridgeWorker }) {
  const capCount = bridgeWorker?.capabilities.length || 0;
  const resCount = bridgeWorker?.resources.length || 0;

  return (
    <div className="pw-card">
      <div className="pw-card__top">
        <div className="pw-card__icon pw-card__icon--plugin">
          {getPluginEmoji(plugin.id)}
        </div>
        <div className="pw-card__info">
          <div className="pw-card__name">{plugin.name}</div>
          <div className="pw-card__type-row">
            <MetaChip variant="plugin">
              <Puzzle style={{ width: 12, height: 12 }} />
              Plugin
            </MetaChip>
            <span className="pw-card__origin">v{plugin.version}</span>
          </div>
        </div>
      </div>

      {plugin.description && (
        <div className="pw-card__desc">
          {plugin.description}
        </div>
      )}

      <div className="pw-card__meta">
        <MetaChip variant={pluginStateVariant(plugin.state)}>
          {plugin.state === "running" && <Check style={{ width: 12, height: 12 }} />}
          {plugin.state === "failed" && <X style={{ width: 12, height: 12 }} />}
          {plugin.state === "stopped" && <Power style={{ width: 12, height: 12 }} />}
          {plugin.state === "starting" && <Loader2 style={{ width: 12, height: 12 }} />}
          {pluginStateLabel(plugin.state)}
        </MetaChip>
        {capCount > 0 && (
          <MetaChip variant="tools">
            <Wrench style={{ width: 12, height: 12 }} />
            {capCount} capabilities
          </MetaChip>
        )}
        {resCount > 0 && (
          <MetaChip variant="mcps">
            <Database style={{ width: 12, height: 12 }} />
            {resCount} resources
          </MetaChip>
        )}
        {plugin.auto_restart && (
          <MetaChip variant="enabled">
            <RefreshCw style={{ width: 12, height: 12 }} />
            Auto-restart
          </MetaChip>
        )}
      </div>

      {plugin.state === "failed" && plugin.error && (
        <div style={{
          fontSize: "var(--text-xs)",
          color: "var(--destructive)",
          background: "var(--destructive-muted)",
          padding: "var(--spacing-2) var(--spacing-3)",
          borderRadius: "var(--radius-sm)",
          marginTop: "var(--spacing-2)",
          lineHeight: 1.4,
          wordBreak: "break-word",
        }}>
          {plugin.error}
        </div>
      )}

      <div className="pw-card__footer">
        <div className="pw-card__footer-left">
          <Puzzle style={{ width: 12, height: 12 }} />
          <span>{plugin.id}</span>
        </div>
      </div>
    </div>
  );
}

// ============================================================================
// Plugin / Worker Card
// ============================================================================

function PluginWorkerCard({ worker, onClick }: { worker: BridgeWorker; onClick: () => void }) {
  const connectedAt = new Date(worker.connected_at);

  return (
    <div className="pw-card" role="button" tabIndex={0} onClick={onClick} onKeyDown={(e) => { if (e.key === "Enter" || e.key === " ") onClick(); }}>
      <div className="pw-card__top">
        <div className="pw-card__icon pw-card__icon--worker">
          <Cpu style={{ width: 20, height: 20 }} />
        </div>
        <div className="pw-card__info">
          <div className="pw-card__name">{worker.adapter_id}</div>
          <div className="pw-card__type-row">
            <MetaChip variant="worker">
              <Cable style={{ width: 12, height: 12 }} />
              Worker
            </MetaChip>
            <span className="pw-card__origin">WebSocket</span>
          </div>
        </div>
      </div>

      <div className="pw-card__meta">
        {worker.capabilities.length > 0 && (
          <MetaChip variant="tools">
            <Wrench style={{ width: 12, height: 12 }} />
            {worker.capabilities.length} capabilities
          </MetaChip>
        )}
        {worker.resources.length > 0 && (
          <MetaChip variant="mcps">
            <Database style={{ width: 12, height: 12 }} />
            {worker.resources.length} resources
          </MetaChip>
        )}
        <MetaChip variant="running">
          <Check style={{ width: 12, height: 12 }} />
          Running
        </MetaChip>
      </div>

      <div className="pw-card__footer">
        <div className="pw-card__footer-left">
          <Cable style={{ width: 12, height: 12 }} />
          <span>Connected {formatUptime(connectedAt)}</span>
        </div>
        <div className="pw-card__footer-actions">
          <button
            className="btn btn--icon-ghost btn--sm"
            onClick={(e) => { e.stopPropagation(); onClick(); }}
            title="View details"
          >
            <Eye style={{ width: 14, height: 14 }} />
          </button>
        </div>
      </div>
    </div>
  );
}

// ============================================================================
// Worker Detail (Slideover content)
// ============================================================================

function WorkerDetail({ worker }: { worker: BridgeWorker }) {
  const connectedAt = new Date(worker.connected_at);
  const isPlugin = worker.adapter_id.startsWith("plugin:");

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: "var(--spacing-5)" }}>
      {/* Status */}
      <div style={{ display: "flex", gap: "var(--spacing-2)", flexWrap: "wrap" }}>
        <MetaChip variant={isPlugin ? "plugin" : "worker"}>
          {isPlugin ? <Puzzle style={{ width: 12, height: 12 }} /> : <Cpu style={{ width: 12, height: 12 }} />}
          {isPlugin ? "Plugin" : "Worker"}
        </MetaChip>
        <MetaChip variant="running">
          <Check style={{ width: 12, height: 12 }} />
          Connected {formatUptime(connectedAt)}
        </MetaChip>
      </div>

      {/* Capabilities */}
      <div>
        <h4 style={{ fontSize: "var(--text-sm)", fontWeight: 600, color: "var(--foreground)", marginBottom: "var(--spacing-3)", display: "flex", alignItems: "center", gap: "var(--spacing-2)" }}>
          <Wrench style={{ width: 14, height: 14 }} />
          Capabilities ({worker.capabilities.length})
        </h4>
        {worker.capabilities.length === 0 ? (
          <p style={{ fontSize: "var(--text-sm)", color: "var(--muted-foreground)" }}>No capabilities declared</p>
        ) : (
          <div style={{ display: "flex", flexDirection: "column", gap: "var(--spacing-2)" }}>
            {worker.capabilities.map((cap) => (
              <div key={cap.name} className="ts-tool-item">
                <div className="ts-tool-item__name">{cap.name}</div>
                {cap.description && <div className="ts-tool-item__desc">{cap.description}</div>}
                {cap.schema && (
                  <pre style={{
                    fontSize: "var(--text-xs)",
                    color: "var(--muted-foreground)",
                    marginTop: "var(--spacing-2)",
                    padding: "var(--spacing-2)",
                    backgroundColor: "var(--muted)",
                    borderRadius: "var(--radius-sm)",
                    overflow: "auto",
                    maxHeight: 120,
                  }}>
                    {JSON.stringify(cap.schema, null, 2)}
                  </pre>
                )}
              </div>
            ))}
          </div>
        )}
      </div>

      {/* Resources */}
      <div>
        <h4 style={{ fontSize: "var(--text-sm)", fontWeight: 600, color: "var(--foreground)", marginBottom: "var(--spacing-3)", display: "flex", alignItems: "center", gap: "var(--spacing-2)" }}>
          <Database style={{ width: 14, height: 14 }} />
          Resources ({worker.resources.length})
        </h4>
        {worker.resources.length === 0 ? (
          <p style={{ fontSize: "var(--text-sm)", color: "var(--muted-foreground)" }}>No resources declared</p>
        ) : (
          <div style={{ display: "flex", flexDirection: "column", gap: "var(--spacing-2)" }}>
            {worker.resources.map((res) => (
              <div key={res.name} className="ts-tool-item">
                <div className="ts-tool-item__name">{res.name}</div>
                {res.description && <div className="ts-tool-item__desc">{res.description}</div>}
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}
