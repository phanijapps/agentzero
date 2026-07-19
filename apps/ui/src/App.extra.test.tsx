// ============================================================================
// App — initialization and error state tests
// Tests the default App export and VersionBadge/ResearchV2Redirect internals.
// ============================================================================

import { StrictMode } from "react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";

// ─── Mock transport ───────────────────────────────────────────────────────────

const health = vi.fn();
const connect = vi.fn();
const disconnect = vi.fn();

vi.mock("@/services/transport", () => ({
  initializeTransport: vi.fn(async () => {}),
  getTransport: vi.fn(async () => ({ health, connect, disconnect })),
}));

// Mock all the heavy child pages so they don't need their own transport
vi.mock("./features/commissioning", () => ({
  CommissioningScreen: () => <div>CommissioningScreen</div>,
  CommissioningGuard: ({ children }: { children: React.ReactNode }) => <>{children}</>,
}));
vi.mock("./features/agent/WebAgentsPanel", () => ({ WebAgentsPanel: () => <div>WebAgentsPanel</div> }));
vi.mock("./features/settings/WebSettingsPanel", () => ({ WebSettingsPanel: () => <div>WebSettingsPanel</div> }));
vi.mock("./features/integrations/WebIntegrationsPanel", () => ({ WebIntegrationsPanel: () => <div>WebIntegrationsPanel</div> }));
vi.mock("./features/memory", () => ({ MemoryTab: () => <div>MemoryTab</div> }));
vi.mock("./features/observatory", () => ({ ObservatoryPage: () => <div>ObservatoryPage</div> }));
vi.mock("./features/chat-v2", () => ({ QuickChat: () => <div>QuickChat</div> }));
vi.mock("./features/research-v2", () => ({ ResearchPage: () => <div>ResearchPage</div> }));
vi.mock("./features/mission-control", () => ({ MissionControlPage: () => <div>MissionControlPage</div> }));
vi.mock("./components/AccentPicker", () => ({ AccentPicker: () => <button aria-label="theme accent">theme</button> }));

import { initializeTransport } from "@/services/transport";
import App from "./App";

// jsdom doesn't implement matchMedia — provide a minimal stub for Sonner/Toaster
Object.defineProperty(window, "matchMedia", {
  writable: true,
  value: (query: string) => ({
    matches: false,
    media: query,
    onchange: null,
    addListener: vi.fn(),
    removeListener: vi.fn(),
    addEventListener: vi.fn(),
    removeEventListener: vi.fn(),
    dispatchEvent: vi.fn(),
  }),
});

beforeEach(() => {
  vi.mocked(initializeTransport).mockClear();
  health.mockReset();
  connect.mockReset();
  disconnect.mockReset();
  health.mockResolvedValue({ success: true, data: { status: "ok", version: "1.0.0" } });
  connect.mockResolvedValue({ success: true });
  disconnect.mockResolvedValue(undefined);
});

describe("App — initialization flow", () => {
  it("shows loading spinner while initializing", () => {
    // Health never resolves — stay loading
    health.mockReturnValue(new Promise(() => {}));
    render(<App />);
    expect(screen.getByText(/connecting to gateway/i)).toBeInTheDocument();
  });

  it("renders the app after successful initialization", async () => {
    render(<App />);
    // Wait for initialization to complete (spinner disappears)
    await waitFor(() => {
      expect(screen.queryByText(/connecting to gateway/i)).toBeNull();
    });
    // Commissioning guard is mocked here; verify the app shell still starts.
    // Just check that the error state is NOT showing
    expect(screen.queryByText(/connection failed/i)).toBeNull();
  });

  it("does not duplicate initialization calls under React StrictMode", async () => {
    render(
      <StrictMode>
        <App />
      </StrictMode>,
    );

    await waitFor(() => {
      expect(screen.queryByText(/connecting to gateway/i)).toBeNull();
    });

    expect(initializeTransport).toHaveBeenCalledTimes(1);
    expect(health).toHaveBeenCalledTimes(1);
    expect(connect).toHaveBeenCalledTimes(1);
    expect(screen.getByText("v1.0.0")).toBeInTheDocument();
  });

  it("renders build date metadata from health in the version badge", async () => {
    health.mockResolvedValue({
      success: true,
      data: {
        status: "ok",
        version: "2026.5.3.develop",
        buildDate: "2026-06-14",
        buildTimestamp: "2026-06-14T15:30:00Z",
      },
    });

    render(<App />);

    const badge = await screen.findByText("v2026.5.3.develop (2026-06-14 15:30:00Z)");
    expect(badge).toHaveAttribute("title", "z-bot 2026.5.3.develop built 2026-06-14T15:30:00Z");
  });

  it("shows error state when health check fails", async () => {
    health.mockResolvedValue({ success: false, error: "daemon not running" });
    render(<App />);
    await waitFor(() => {
      expect(screen.getByText(/connection failed/i)).toBeInTheDocument();
    });
    expect(screen.getByText(/daemon not running/i)).toBeInTheDocument();
  });

  it("shows error state when initialization throws", async () => {
    health.mockRejectedValue(new Error("socket error"));
    render(<App />);
    await waitFor(() => {
      expect(screen.getByText(/connection failed/i)).toBeInTheDocument();
    });
    expect(screen.getByText(/socket error/i)).toBeInTheDocument();
  });

  it("retries connection when Retry button is clicked", async () => {
    health.mockResolvedValueOnce({ success: false, error: "first attempt failed" });
    health.mockResolvedValueOnce({ success: true, data: { status: "ok" } });

    render(<App />);
    await waitFor(() => screen.getByText(/connection failed/i));

    fireEvent.click(screen.getByRole("button", { name: /retry connection/i }));

    // Loading spinner appears again
    await waitFor(() => {
      expect(screen.queryByText(/connection failed/i)).toBeNull();
    });
    expect(health).toHaveBeenCalledTimes(2);
  }, 10000);
});
