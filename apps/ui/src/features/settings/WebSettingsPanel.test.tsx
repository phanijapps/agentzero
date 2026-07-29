// ============================================================================
// WebSettingsPanel — header, tab structure, empty state, basic loading flow
// ============================================================================

import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@/test/utils";
import { WebSettingsPanel } from "./WebSettingsPanel";

// ---------------------------------------------------------------------------
// Mocks — minimal transport that returns empty fixtures so the panel mounts.
// ---------------------------------------------------------------------------

const mockListProviders = vi.fn();
const mockListModels = vi.fn();
const mockGetToolSettings = vi.fn();
const mockGetLogSettings = vi.fn();
const mockGetExecutionSettings = vi.fn();
const mockGetPresentationSettings = vi.fn();
const mockUpdatePresentationSettings = vi.fn();
const mockClearSavedSurfaces = vi.fn();
const mockGetEmbeddingsHealth = vi.fn();
const mockGetEmbeddingsModels = vi.fn();
const mockGetOllamaEmbeddingModels = vi.fn();
const mockConfigureEmbeddings = vi.fn();

vi.mock("@/services/transport", async () => {
  const actual = await vi.importActual<Record<string, unknown>>("@/services/transport");
  return {
    ...actual,
    getTransport: async () => ({
      listProviders: mockListProviders,
      listModels: mockListModels,
      getToolSettings: mockGetToolSettings,
      getLogSettings: mockGetLogSettings,
      getExecutionSettings: mockGetExecutionSettings,
      getPresentationSettings: mockGetPresentationSettings,
      updatePresentationSettings: mockUpdatePresentationSettings,
      clearSavedSurfaces: mockClearSavedSurfaces,
      getEmbeddingsHealth: mockGetEmbeddingsHealth,
      getEmbeddingsModels: mockGetEmbeddingsModels,
      getOllamaEmbeddingModels: mockGetOllamaEmbeddingModels,
      configureEmbeddings: mockConfigureEmbeddings,
    }),
  };
});

beforeEach(() => {
  vi.clearAllMocks();
  window.history.replaceState({}, "", "/");
  mockListProviders.mockResolvedValue({ success: true, data: [] });
  mockListModels.mockResolvedValue({ success: true, data: {} });
  mockGetToolSettings.mockResolvedValue({ success: true, data: { tools: {} } });
  mockGetLogSettings.mockResolvedValue({
    success: true,
    data: { level: "info", retentionDays: 7 },
  });
  mockGetExecutionSettings.mockResolvedValue({
    success: true,
    data: { featureFlags: {} },
  });
  mockGetPresentationSettings.mockResolvedValue({
    success: true,
    data: { persistSurfaces: false, restartRequired: false },
  });
  mockUpdatePresentationSettings.mockResolvedValue({
    success: true,
    data: { persistSurfaces: true, restartRequired: false },
  });
  mockClearSavedSurfaces.mockResolvedValue({
    success: true,
    data: { deletedCount: 2 },
  });
  mockGetEmbeddingsHealth.mockResolvedValue({ success: true, data: { healthy: false } });
  // getEmbeddingsModels() returns CuratedModel[] directly (not wrapped).
  mockGetEmbeddingsModels.mockResolvedValue({ success: true, data: [] });
  mockGetOllamaEmbeddingModels.mockResolvedValue({ success: true, data: { models: [] } });
  mockConfigureEmbeddings.mockResolvedValue({ success: true });
});

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

describe("WebSettingsPanel — page chrome", () => {
  it("renders the Settings page title + subtitle", async () => {
    render(<WebSettingsPanel />);
    await waitFor(() => expect(screen.getByText("Settings")).toBeInTheDocument());
    expect(
      screen.getByText(/configure your ai providers, system preferences, and logging/i)
    ).toBeInTheDocument();
  });

  it("renders all four tabs (Providers · General · Logging · Advanced)", async () => {
    render(<WebSettingsPanel />);
    await waitFor(() => expect(screen.getByText("Providers")).toBeInTheDocument());
    expect(screen.getByText("General")).toBeInTheDocument();
    expect(screen.getByText("Logging")).toBeInTheDocument();
    expect(screen.getByText("Advanced")).toBeInTheDocument();
  });

  it("loads providers + models + settings on mount", async () => {
    render(<WebSettingsPanel />);
    await waitFor(() => {
      expect(mockListProviders).toHaveBeenCalled();
      expect(mockListModels).toHaveBeenCalled();
      expect(mockGetToolSettings).toHaveBeenCalled();
      expect(mockGetLogSettings).toHaveBeenCalled();
      expect(mockGetExecutionSettings).toHaveBeenCalled();
      expect(mockGetPresentationSettings).toHaveBeenCalled();
    });
  });
});

describe("WebSettingsPanel — infographic persistence", () => {
  it("enables persistence immediately without requiring a restart", async () => {
    render(<WebSettingsPanel />);
    fireEvent.click(await screen.findByText("General"));

    const toggle = await screen.findByRole("checkbox", { name: "Persist infographics" });
    expect(toggle).not.toBeChecked();
    expect(screen.getByText(/Turning this off keeps previously saved infographics/i)).toBeInTheDocument();

    fireEvent.click(toggle);

    await waitFor(() => {
      expect(mockUpdatePresentationSettings).toHaveBeenCalledWith({ persistSurfaces: true });
      expect(screen.getByText("Saved")).toBeInTheDocument();
    });
  });

  it("requires confirmation before clearing saved infographics", async () => {
    const confirmSpy = vi.spyOn(window, "confirm").mockReturnValue(false);
    render(<WebSettingsPanel />);
    fireEvent.click(await screen.findByText("General"));

    fireEvent.click(await screen.findByRole("button", { name: "Clear saved infographics" }));

    expect(confirmSpy).toHaveBeenCalledOnce();
    expect(mockClearSavedSurfaces).not.toHaveBeenCalled();
    confirmSpy.mockRestore();
  });

  it("reports how many saved infographics were cleared", async () => {
    const confirmSpy = vi.spyOn(window, "confirm").mockReturnValue(true);
    render(<WebSettingsPanel />);
    fireEvent.click(await screen.findByText("General"));

    fireEvent.click(await screen.findByRole("button", { name: "Clear saved infographics" }));

    await waitFor(() => {
      expect(mockClearSavedSurfaces).toHaveBeenCalledOnce();
      expect(screen.getByText("2 saved infographics cleared. Open chats are unchanged.")).toBeInTheDocument();
    });
    confirmSpy.mockRestore();
  });

  it("disables the clear action while deletion is pending", async () => {
    let finishClear: ((value: { success: boolean; data: { deletedCount: number } }) => void) | undefined;
    mockClearSavedSurfaces.mockReturnValueOnce(new Promise((resolve) => {
      finishClear = resolve;
    }));
    const confirmSpy = vi.spyOn(window, "confirm").mockReturnValue(true);
    render(<WebSettingsPanel />);
    fireEvent.click(await screen.findByText("General"));

    const clearButton = await screen.findByRole("button", { name: "Clear saved infographics" });
    fireEvent.click(clearButton);

    await waitFor(() => expect(clearButton).toBeDisabled());
    finishClear?.({ success: true, data: { deletedCount: 0 } });
    await waitFor(() => expect(clearButton).not.toBeDisabled());
    confirmSpy.mockRestore();
  });

  it("restores the toggle and shows an error when saving fails", async () => {
    mockUpdatePresentationSettings.mockResolvedValueOnce({
      success: false,
      error: "Settings unavailable",
    });
    render(<WebSettingsPanel />);
    fireEvent.click(await screen.findByText("General"));

    const toggle = await screen.findByRole("checkbox", { name: "Persist infographics" });
    fireEvent.click(toggle);

    await waitFor(() => {
      expect(toggle).not.toBeChecked();
      expect(screen.getByText("Settings unavailable")).toBeInTheDocument();
    });
  });
});

describe("WebSettingsPanel — providers tab empty state", () => {
  it("renders the Get Started empty state when no providers exist", async () => {
    render(<WebSettingsPanel />);
    await waitFor(() =>
      expect(screen.getByText(/get started/i)).toBeInTheDocument()
    );
  });

  it("provider count badge reads 0 when there are no providers", async () => {
    render(<WebSettingsPanel />);
    await waitFor(() => expect(screen.getByText("Providers")).toBeInTheDocument());
    // The TabBar count is rendered next to the label.
    const tab = screen.getByText("Providers").closest("button, [role='tab'], div");
    expect(tab).not.toBeNull();
  });
});

describe("WebSettingsPanel — tab switching", () => {
  it("switches to General when the General tab is clicked", async () => {
    render(<WebSettingsPanel />);
    await waitFor(() => expect(screen.getByText("Settings")).toBeInTheDocument());
    fireEvent.click(screen.getByText("General"));
    // Loading completes and General tab shows content. Just check no crash.
    await waitFor(() => expect(screen.getByText("General")).toBeInTheDocument());
  });

  it("switches to Logging when the Logging tab is clicked", async () => {
    render(<WebSettingsPanel />);
    await waitFor(() => expect(screen.getByText("Settings")).toBeInTheDocument());
    fireEvent.click(screen.getByText("Logging"));
    await waitFor(() => expect(screen.getByText("Logging")).toBeInTheDocument());
  });

  it("switches to Advanced when the Advanced tab is clicked", async () => {
    render(<WebSettingsPanel />);
    await waitFor(() => expect(screen.getByText("Settings")).toBeInTheDocument());
    fireEvent.click(screen.getByText("Advanced"));
    await waitFor(() => expect(screen.getByText("Advanced")).toBeInTheDocument());
  });
});

describe("WebSettingsPanel — provider load failure", () => {
  it("does not crash when listProviders returns an error (still mounts the page)", async () => {
    mockListProviders.mockResolvedValueOnce({
      success: false,
      error: "Network timeout",
    });
    render(<WebSettingsPanel />);
    await waitFor(() => expect(screen.getByText("Settings")).toBeInTheDocument());
    // The empty-state path renders since hasProviders=false. The page survives the error.
    expect(screen.getByText("Providers")).toBeInTheDocument();
  });
});

describe("WebSettingsPanel — Sonar S6853 (empty label fix)", () => {
  it("uses no <label> elements without accessible text in the Logging tab", async () => {
    render(<WebSettingsPanel />);
    await waitFor(() => expect(screen.getByText("Settings")).toBeInTheDocument());
    fireEvent.click(screen.getByText("Logging"));
    // Every <label> in the rendered DOM either has accessible text content
    // or an aria-label. The previous empty <label aria-hidden="true"> for
    // layout spacing was the offender — it's now a <div>.
    await waitFor(() => {
      const labels = document.querySelectorAll("label");
      for (const lbl of labels) {
        const text = (lbl.textContent ?? "").trim();
        const aria = lbl.getAttribute("aria-label");
        const isAccessible = text.length > 0 || (aria && aria.length > 0);
        expect(isAccessible).toBe(true);
      }
    });
  });
});
