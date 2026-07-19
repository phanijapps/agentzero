// ============================================================================
// WEB APP ENTRY POINT
// Main application for standalone web dashboard (no Tauri, no vaults)
// ============================================================================

import { useEffect, useRef, useState } from "react";
import { BrowserRouter, Routes, Route, Navigate, Link, useLocation, useParams } from "react-router-dom";
import { Toaster } from "sonner";
import {
  Bot,
  Settings,
  Loader2,
  AlertCircle,
  RefreshCw,
  LayoutDashboard,
  Plug,
  Brain,
  Network,
  Layers,
  MessageSquare,
  Menu,
  Search,
  Archive,
} from "lucide-react";
import { initializeTransport, getTransport } from "@/services/transport";
import { CommissioningGuard, CommissioningScreen } from "./features/commissioning";
import { WebAgentsPanel } from "./features/agent/WebAgentsPanel";
import { WebSettingsPanel } from "./features/settings/WebSettingsPanel";
import { WebIntegrationsPanel } from "./features/integrations/WebIntegrationsPanel";
import { MemoryTab as MemoryPanel } from "./features/memory";
import { ObservatoryPage } from "./features/observatory";
import { ObservatoryV2Page } from "./features/observatory-v2";
import { QuickChat } from "./features/chat-v2";
import { ResearchPage } from "./features/research-v2";
import { MissionControlPage } from "./features/mission-control";
import { VaultPage } from "./features/vault";
import { AccentPicker } from "./components/AccentPicker";

// ============================================================================
// Types
// ============================================================================

interface ConnectionStatus {
  connected: boolean;
  error?: string;
}

interface AppInitResult {
  connected: boolean;
  error?: string;
  version?: string;
  buildDate?: string;
  buildTimestamp?: string;
}

// ============================================================================
// App Component
// ============================================================================

/** Legacy redirect: /research-v2/:sessionId → /research/:sessionId. */
function ResearchV2Redirect() {
  const { sessionId } = useParams<{ sessionId: string }>();
  return <Navigate to={`/research/${sessionId ?? ""}`} replace />;
}

/**
 * Topbar version badge — fetches /api/health once and renders the
 * daemon's reported version. Hidden while the fetch is in flight or on
 * failure (rather than showing a placeholder) so the bar stays clean.
 *
 * Branch-suffixed versions and build metadata come from the daemon's
 * compile-time `/api/health` response.
 */
function VersionBadge({
  version,
  buildDate,
  buildTimestamp,
}: {
  version?: string | null;
  buildDate?: string | null;
  buildTimestamp?: string | null;
}) {
  if (!version) return null;
  const buildLabel = buildTimestamp ? formatBuildTimestamp(buildTimestamp) : buildDate;
  const label = buildLabel ? `v${version} (${buildLabel})` : `v${version}`;
  const title = buildTimestamp
    ? `z-bot ${version} built ${buildTimestamp}`
    : buildDate
      ? `z-bot ${version} built ${buildDate}`
      : `z-bot ${version}`;
  return (
    <span className="topbar__version" title={title}>
      {label}
    </span>
  );
}

function formatBuildTimestamp(value: string) {
  return value.replace("T", " ");
}

function App() {
  const [isInitializing, setIsInitializing] = useState(true);
  const [connectionStatus, setConnectionStatus] = useState<ConnectionStatus>({
    connected: false,
  });
  const [error, setError] = useState<string | null>(null);
  const [retryCount, setRetryCount] = useState(0);
  const [daemonVersion, setDaemonVersion] = useState<string | null>(null);
  const [daemonBuildDate, setDaemonBuildDate] = useState<string | null>(null);
  const [daemonBuildTimestamp, setDaemonBuildTimestamp] = useState<string | null>(null);
  const initPromiseRef = useRef<Promise<AppInitResult> | null>(null);

  useEffect(() => {
    let cancelled = false;

    const initializeApp = async (): Promise<AppInitResult> => {
      await initializeTransport();
      const transport = await getTransport();
      const healthResult = await transport.health();

      if (!healthResult.success) {
        return {
          connected: false,
          error: `Cannot connect to gateway: ${healthResult.error}`,
        };
      }

      await transport.connect();
      return {
        connected: true,
        version: healthResult.data?.version,
        buildDate: healthResult.data?.buildDate,
        buildTimestamp: healthResult.data?.buildTimestamp,
      };
    };

    if (!initPromiseRef.current) {
      initPromiseRef.current = initializeApp().catch((err) => {
        const message = err instanceof Error ? err.message : String(err);
        return { connected: false, error: message };
      });
    }

    void initPromiseRef.current.then((result) => {
      if (cancelled) return;
      if (!result.connected) {
        setError(result.error ?? "Cannot connect to gateway");
        setConnectionStatus({ connected: false, error: result.error });
      } else {
        setError(null);
        setConnectionStatus({ connected: true });
        setDaemonVersion(result.version ?? null);
        setDaemonBuildDate(result.buildDate ?? null);
        setDaemonBuildTimestamp(result.buildTimestamp ?? null);
      }
      setIsInitializing(false);
    });

    return () => {
      cancelled = true;
    };
  }, [retryCount]);

  const handleRetry = () => {
    setError(null);
    setDaemonVersion(null);
    setDaemonBuildDate(null);
    setDaemonBuildTimestamp(null);
    setIsInitializing(true);
    initPromiseRef.current = null;
    setRetryCount(c => c + 1);
  };

  if (isInitializing) {
    return (
      <div className="loading-spinner">
        <div className="text-center">
          <Loader2 className="loading-spinner__icon" />
          <p className="page-subtitle">Connecting to gateway...</p>
        </div>
      </div>
    );
  }

  if (error && !connectionStatus.connected) {
    return (
      <div className="loading-spinner">
        <div className="card card__padding--lg page-container--narrow text-center">
          <div className="card__icon card__icon--destructive empty-state__icon">
            <AlertCircle style={{ width: 24, height: 24 }} />
          </div>
          <h2 className="empty-state__title">Connection Failed</h2>
          <p className="empty-state__description">{error}</p>
          <p className="page-subtitle mb-section">
            Make sure the z-Bot daemon is running:
            <br />
            <code className="badge mt-inline">
              zbotd
            </code>
          </p>
          <button
            onClick={handleRetry}
            className="btn btn--primary btn--md"
          >
            <RefreshCw style={{ width: 16, height: 16 }} />
            Retry Connection
          </button>
        </div>
      </div>
    );
  }

  return (
    <BrowserRouter>
      <Toaster
        position="bottom-right"
        theme="system"
        toastOptions={{
          style: {
            fontWeight: 500,
            fontSize: '14px',
            borderRadius: 'var(--radius-lg)',
            boxShadow: 'var(--shadow-dropdown)',
          },
        }}
      />
      <Routes>
          {/* Commissioning renders without the application shell. */}
          <Route path="/commission" element={<CommissioningScreen />} />
          <Route path="/setup" element={<CommissioningScreen rerunSetup />} />

          {/* Main app with sidebar */}
          <Route path="/*" element={
            <CommissioningGuard>
              <WebAppShell
                connectionStatus={connectionStatus}
                version={daemonVersion}
                buildDate={daemonBuildDate}
                buildTimestamp={daemonBuildTimestamp}
              >
                <Routes>
                  <Route path="/" element={<Navigate to="/research" replace />} />
                  <Route path="/mission-control" element={<MissionControlPage />} />
                  {/* Legacy redirects — Dashboard + Logs are now Mission Control. */}
                  <Route path="/dashboard" element={<Navigate to="/mission-control" replace />} />
                  <Route path="/logs" element={<Navigate to="/mission-control" replace />} />
                  <Route path="/memory" element={<MemoryPanel agentId="root" />} />
                  <Route path="/observatory" element={<ObservatoryPage />} />
                  <Route path="/observatory-v2" element={<ObservatoryV2Page />} />
                  <Route path="/agents" element={<WebAgentsPanel />} />
                  <Route path="/vault" element={<VaultPage />} />
                  <Route path="/integrations" element={<WebIntegrationsPanel />} />
                  <Route path="/settings" element={<WebSettingsPanel />} />
                  <Route path="/chat" element={<QuickChat />} />
                  {/* Legacy bookmark redirect. */}
                  <Route path="/chat-v2" element={<Navigate to="/chat" replace />} />
                  <Route path="/research" element={<ResearchPage />} />
                  <Route path="/research/:sessionId" element={<ResearchPage />} />
                  {/* Legacy bookmark redirects. */}
                  <Route path="/research-v2" element={<Navigate to="/research" replace />} />
                  <Route path="/research-v2/:sessionId" element={<ResearchV2Redirect />} />
                  <Route path="/providers" element={<Navigate to="/settings" replace />} />
                  <Route path="/skills" element={<Navigate to="/agents?tab=skills" replace />} />
                  <Route path="/hooks" element={<Navigate to="/agents?tab=schedules" replace />} />
                  <Route path="/connectors" element={<Navigate to="/integrations?tab=plugins" replace />} />
                  <Route path="/mcps" element={<Navigate to="/integrations" replace />} />
                </Routes>
              </WebAppShell>
            </CommissioningGuard>
          } />
        </Routes>
    </BrowserRouter>
  );
}

// ============================================================================
// Web App Shell
// ============================================================================

interface WebAppShellProps {
  children: React.ReactNode;
  connectionStatus: ConnectionStatus;
  version?: string | null;
  buildDate?: string | null;
  buildTimestamp?: string | null;
}

// Flat top-bar nav — no groups, no Main/Manage/System split.
// Order matches the design system kit (Research first, Settings last).
export interface NavItem {
  to: string;
  label: string;
  icon: React.ComponentType<{ className?: string; style?: React.CSSProperties }>;
  matchPrefix?: boolean;
  badge?: string;
}

export const navItems: NavItem[] = [
  { to: "/research", label: "Research", icon: Search, matchPrefix: true },
  { to: "/chat", label: "Quick chat", icon: MessageSquare },
  { to: "/mission-control", label: "Mission Control", icon: LayoutDashboard },
  { to: "/agents", label: "Agents", icon: Bot },
  { to: "/memory", label: "Memory", icon: Brain },
  { to: "/vault", label: "Vault", icon: Archive },
  { to: "/observatory", label: "Observatory", icon: Network },
  { to: "/observatory-v2", label: "Graph", icon: Layers },
  { to: "/integrations", label: "Integrations", icon: Plug },
  { to: "/settings", label: "Settings", icon: Settings },
];

export function WebAppShell({
  children,
  connectionStatus,
  version,
  buildDate,
  buildTimestamp,
}: WebAppShellProps) {
  const [sheetOpen, setSheetOpen] = useState(false);
  const location = useLocation();
  // Close the mobile sheet whenever the route changes.
  useEffect(() => { setSheetOpen(false); }, [location.pathname]);

  return (
    <div className="app-shell">
      <header className="topbar">
        <Link to="/research" className="topbar__brand" aria-label="z-Bot home">
          <span className="topbar__brand-mark">z</span>
          <span className="topbar__brand-name">z-<b>Bot</b></span>
        </Link>

        <button
          type="button"
          className="topbar__menu-toggle"
          aria-label={sheetOpen ? "Close menu" : "Open menu"}
          aria-expanded={sheetOpen}
          onClick={() => setSheetOpen((v) => !v)}
        >
          <Menu size={18} />
        </button>

        <nav className="topbar__nav" aria-label="Primary">
          {navItems.map((item) => (
            <NavLink key={item.to} to={item.to} label={item.label} icon={item.icon} matchPrefix={item.matchPrefix} badge={item.badge} />
          ))}
        </nav>

        <div className="topbar__right">
          <VersionBadge version={version} buildDate={buildDate} buildTimestamp={buildTimestamp} />
          <AccentPicker />
          <div className="connection-status">
            <div className={`connection-status__dot ${
              connectionStatus.connected
                ? 'connection-status__dot--connected'
                : 'connection-status__dot--disconnected'
            }`} />
            <span className="connection-status__text">
              {connectionStatus.connected ? "Connected · zbotd" : "Disconnected"}
            </span>
          </div>
        </div>
      </header>

      {sheetOpen && (
        <button
          type="button"
          className="topbar__sheet-backdrop topbar__sheet-backdrop--open"
          aria-label="Close menu"
          onClick={() => setSheetOpen(false)}
        />
      )}
      <nav
        className={`topbar__sheet${sheetOpen ? " topbar__sheet--open" : ""}`}
        aria-label="Mobile primary"
      >
        {navItems.map((item) => (
          <NavLink key={item.to} to={item.to} label={item.label} icon={item.icon} matchPrefix={item.matchPrefix} badge={item.badge} />
        ))}
      </nav>

      <main className="app-shell__main">{children}</main>
    </div>
  );
}

interface NavLinkProps {
  to: string;
  label: string;
  icon: React.ComponentType<{ className?: string; style?: React.CSSProperties }>;
  matchPrefix?: boolean;
  badge?: string;
}

function NavLink({ to, label, icon: Icon, matchPrefix, badge }: NavLinkProps) {
  const location = useLocation();
  const isActive = matchPrefix
    ? location.pathname.startsWith(to)
    : location.pathname === to;

  return (
    <Link
      to={to}
      className={`nav-link ${isActive ? 'nav-link--active' : ''}`}
    >
      <Icon className="nav-link__icon" />
      <span className="nav-link__label">
        {label}
        {badge != null && (
          <span className="nav-link__badge">{badge}</span>
        )}
      </span>
    </Link>
  );
}

export default App;
