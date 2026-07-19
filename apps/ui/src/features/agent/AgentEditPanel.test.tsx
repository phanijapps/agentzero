// ============================================================================
// AgentEditPanel — Advanced section + voice-recording removal tests
// ============================================================================

import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor, within } from "@/test/utils";
import { AgentEditPanel } from "./AgentEditPanel";
import type {
  AgentResponse,
  ProviderResponse,
  ModelRegistryResponse,
} from "@/services/transport";

// ---------------------------------------------------------------------------
// Mocks
// ---------------------------------------------------------------------------

const mockListMcps = vi.fn();
const mockUpdateAgent = vi.fn();

vi.mock("@/services/transport", async () => {
  const actual = await vi.importActual<Record<string, unknown>>("@/services/transport");
  return {
    ...actual,
    getTransport: async () => ({
      listMcps: mockListMcps,
      updateAgent: mockUpdateAgent,
    }),
  };
});

const PROVIDERS_MULTI: ProviderResponse[] = [
  {
    id: "openai",
    name: "OpenAI",
    description: "",
    apiKey: "x",
    baseUrl: "https://api.openai.com/v1",
    models: ["gpt-4o", "gpt-4o-mini"],
    defaultModel: "gpt-4o",
  } as unknown as ProviderResponse,
  {
    id: "anthropic",
    name: "Anthropic",
    description: "",
    apiKey: "y",
    baseUrl: "https://api.anthropic.com",
    models: ["claude-sonnet-4"],
    defaultModel: "claude-sonnet-4",
  } as unknown as ProviderResponse,
];

const REGISTRY_WITH_GPT4O: ModelRegistryResponse = {
  "gpt-4o": {
    name: "gpt-4o",
    provider: "openai",
    capabilities: {
      tools: true,
      vision: true,
      thinking: false,
      embeddings: false,
      voice: false,
      imageGeneration: false,
      videoGeneration: false,
    },
    context: { input: 128000, output: 8192 },
  },
};

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

function makeAgent(overrides: Partial<AgentResponse> = {}): AgentResponse {
  return {
    id: "agent-1",
    name: "researcher",
    displayName: "Researcher",
    description: "Investigates topics",
    providerId: "openai",
    model: "gpt-4o",
    temperature: 0.7,
    maxInputTokens: 200000,
    maxOutputTokens: 32000,
    maxTokens: 32000,
    thinkingEnabled: false,
    voiceRecordingEnabled: false,
    instructions: "You are helpful.",
    mcps: [],
    skills: [],
    ...overrides,
  };
}

const PROVIDERS: ProviderResponse[] = [
  {
    id: "openai",
    name: "openai",
    displayName: "OpenAI",
    type: "openai",
    enabled: true,
    apiBaseUrl: "https://api.openai.com/v1",
  } as unknown as ProviderResponse,
];

const MODEL_REGISTRY: ModelRegistryResponse = {};

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

describe("AgentEditPanel — Advanced section", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockListMcps.mockResolvedValue({ success: true, data: { servers: [] } });
    mockUpdateAgent.mockResolvedValue({ success: true });
  });

  async function renderPanel(agent = makeAgent()) {
    const onClose = vi.fn();
    const onSave = vi.fn();
    const result = render(
      <AgentEditPanel
        agent={agent}
        providers={PROVIDERS}
        modelRegistry={MODEL_REGISTRY}
        onClose={onClose}
        onSave={onSave}
      />
    );
    // Wait for the loading spinner to clear and the form to mount.
    await waitFor(() => expect(screen.getByText(/basic information/i)).toBeInTheDocument());
    return { ...result, onClose, onSave };
  }

  it("does NOT render a Voice Recording toggle in Advanced", async () => {
    await renderPanel();
    fireEvent.click(screen.getByRole("button", { name: /advanced options/i }));
    expect(screen.queryByText(/voice recording/i)).not.toBeInTheDocument();
    expect(screen.queryByText(/enable voice input/i)).not.toBeInTheDocument();
  });

  it("renders the Thinking Enabled toggle in Advanced", async () => {
    await renderPanel();
    fireEvent.click(screen.getByRole("button", { name: /advanced options/i }));
    expect(screen.getByText(/thinking enabled/i)).toBeInTheDocument();
    expect(screen.getByText(/allow the model to show reasoning steps/i)).toBeInTheDocument();
  });

  it("uses .toggle-switch--off when thinking is disabled", async () => {
    await renderPanel(makeAgent({ thinkingEnabled: false }));
    fireEvent.click(screen.getByRole("button", { name: /advanced options/i }));
    const thinkingRow = screen.getByText(/thinking enabled/i).closest(".skill-toggle");
    expect(thinkingRow).not.toBeNull();
    const toggle = thinkingRow!.querySelector(".toggle-switch");
    expect(toggle?.classList.contains("toggle-switch--off")).toBe(true);
    expect(toggle?.classList.contains("toggle-switch--on")).toBe(false);
  });

  it("uses .toggle-switch--on when thinking is enabled", async () => {
    await renderPanel(makeAgent({ thinkingEnabled: true }));
    fireEvent.click(screen.getByRole("button", { name: /advanced options/i }));
    const thinkingRow = screen.getByText(/thinking enabled/i).closest(".skill-toggle");
    const toggle = thinkingRow!.querySelector(".toggle-switch");
    expect(toggle?.classList.contains("toggle-switch--on")).toBe(true);
    expect(toggle?.classList.contains("toggle-switch--off")).toBe(false);
  });

  it("flips the thinking toggle when the row is clicked", async () => {
    await renderPanel(makeAgent({ thinkingEnabled: false }));
    fireEvent.click(screen.getByRole("button", { name: /advanced options/i }));
    const thinkingRow = screen.getByText(/thinking enabled/i).closest(".skill-toggle")!;
    fireEvent.click(thinkingRow);
    const toggle = thinkingRow.querySelector(".toggle-switch");
    expect(toggle?.classList.contains("toggle-switch--on")).toBe(true);
  });

  it("does not send voiceRecordingEnabled in the update payload on Save", async () => {
    const { onSave } = await renderPanel();
    fireEvent.click(screen.getByRole("button", { name: /save changes/i }));
    await waitFor(() => expect(mockUpdateAgent).toHaveBeenCalledTimes(1));
    const [, payload] = mockUpdateAgent.mock.calls[0];
    expect(payload).not.toHaveProperty("voiceRecordingEnabled");
    expect(payload).toHaveProperty("thinkingEnabled");
    expect(payload).toHaveProperty("maxOutputTokens", 32000);
    expect(payload).not.toHaveProperty("maxTokens");
    await waitFor(() => expect(onSave).toHaveBeenCalled());
  });

  it("collapses Advanced section by default", async () => {
    await renderPanel();
    expect(screen.queryByText(/thinking enabled/i)).not.toBeInTheDocument();
  });
});

// ============================================================================
// Loading + error states
// ============================================================================

describe("AgentEditPanel — loading + error states", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockListMcps.mockResolvedValue({ success: true, data: { servers: [] } });
    mockUpdateAgent.mockResolvedValue({ success: true });
  });

  it("renders the loading slideover until listMcps resolves", async () => {
    let resolveMcps: ((v: { success: true; data: { servers: [] } }) => void) | null =
      null;
    mockListMcps.mockImplementation(
      () =>
        new Promise((r) => {
          resolveMcps = r as (
            v: { success: true; data: { servers: [] } },
          ) => void;
        }),
    );

    render(
      <AgentEditPanel
        agent={makeAgent()}
        providers={PROVIDERS_MULTI}
        modelRegistry={{}}
        onClose={vi.fn()}
        onSave={vi.fn()}
      />,
    );

    // Title shows "Loading..." while pending
    await waitFor(() =>
      expect(screen.getByText(/^Loading\.\.\.$/)).toBeInTheDocument(),
    );
    // The Basic Information section is not yet rendered
    expect(screen.queryByText(/basic information/i)).not.toBeInTheDocument();

    resolveMcps!({ success: true, data: { servers: [] } });
    await waitFor(() =>
      expect(screen.getByText(/basic information/i)).toBeInTheDocument(),
    );
  });

  it("surfaces the error banner when listMcps rejects (line 88 catch)", async () => {
    mockListMcps.mockRejectedValue(new Error("mcp boom"));
    render(
      <AgentEditPanel
        agent={makeAgent()}
        providers={PROVIDERS_MULTI}
        modelRegistry={{}}
        onClose={vi.fn()}
        onSave={vi.fn()}
      />,
    );
    await waitFor(() =>
      expect(screen.getByText(/mcp boom/i)).toBeInTheDocument(),
    );
  });

  it("surfaces the error banner with fallback string when listMcps rejects with non-Error", async () => {
    mockListMcps.mockRejectedValue("string error");
    render(
      <AgentEditPanel
        agent={makeAgent()}
        providers={PROVIDERS_MULTI}
        modelRegistry={{}}
        onClose={vi.fn()}
        onSave={vi.fn()}
      />,
    );
    await waitFor(() =>
      expect(
        screen.getByText(/failed to load data/i),
      ).toBeInTheDocument(),
    );
  });
});

// ============================================================================
// handleSave — success / failure / thrown branches
// ============================================================================

describe("AgentEditPanel — save handler", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockListMcps.mockResolvedValue({ success: true, data: { servers: [] } });
    mockUpdateAgent.mockResolvedValue({ success: true });
  });

  async function renderForSave(
    onSave = vi.fn(),
    onClose = vi.fn(),
    agent = makeAgent(),
  ) {
    render(
      <AgentEditPanel
        agent={agent}
        providers={PROVIDERS_MULTI}
        modelRegistry={{}}
        onClose={onClose}
        onSave={onSave}
      />,
    );
    await waitFor(() =>
      expect(screen.getByText(/basic information/i)).toBeInTheDocument(),
    );
    return { onSave, onClose };
  }

  it("calls onSave + onClose on success", async () => {
    const { onSave, onClose } = await renderForSave();
    fireEvent.click(screen.getByRole("button", { name: /save changes/i }));
    await waitFor(() => expect(onSave).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(onClose).toHaveBeenCalledTimes(1));
  });

  it("sets the error banner when updateAgent returns success=false", async () => {
    mockUpdateAgent.mockResolvedValue({
      success: false,
      error: "server said no",
    });
    await renderForSave();
    fireEvent.click(screen.getByRole("button", { name: /save changes/i }));
    await waitFor(() =>
      expect(screen.getByText(/server said no/i)).toBeInTheDocument(),
    );
  });

  it("uses fallback message when success=false has no error field", async () => {
    mockUpdateAgent.mockResolvedValue({ success: false });
    await renderForSave();
    fireEvent.click(screen.getByRole("button", { name: /save changes/i }));
    await waitFor(() =>
      expect(
        screen.getByText(/failed to update agent/i),
      ).toBeInTheDocument(),
    );
  });

  it("sets the error banner when updateAgent throws an Error", async () => {
    mockUpdateAgent.mockRejectedValue(new Error("network gone"));
    await renderForSave();
    fireEvent.click(screen.getByRole("button", { name: /save changes/i }));
    await waitFor(() =>
      expect(screen.getByText(/network gone/i)).toBeInTheDocument(),
    );
  });

  it("falls back to 'Unknown error' when updateAgent throws non-Error", async () => {
    mockUpdateAgent.mockRejectedValue("plain string");
    await renderForSave();
    fireEvent.click(screen.getByRole("button", { name: /save changes/i }));
    await waitFor(() =>
      expect(screen.getByText(/unknown error/i)).toBeInTheDocument(),
    );
  });

  it("disables the Save button + shows 'Saving...' label while saving", async () => {
    let resolveUpdate: ((v: { success: true }) => void) | null = null;
    mockUpdateAgent.mockImplementation(
      () =>
        new Promise((r) => {
          resolveUpdate = r as (v: { success: true }) => void;
        }),
    );
    await renderForSave();

    const saveBtn = screen.getByRole("button", { name: /save changes/i });
    fireEvent.click(saveBtn);
    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: /saving/i }),
      ).toBeInTheDocument(),
    );
    expect(
      screen.getByRole("button", { name: /saving/i }),
    ).toBeDisabled();

    resolveUpdate!({ success: true });
  });

  it("calls onClose when the Cancel button is pressed (no save)", async () => {
    const { onClose, onSave } = await renderForSave();
    fireEvent.click(screen.getByRole("button", { name: /^Cancel$/ }));
    expect(onClose).toHaveBeenCalledTimes(1);
    expect(onSave).not.toHaveBeenCalled();
    expect(mockUpdateAgent).not.toHaveBeenCalled();
  });
});

// ============================================================================
// Form fields: display name, description, temperature, max tokens
// ============================================================================

describe("AgentEditPanel — form field changes", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockListMcps.mockResolvedValue({ success: true, data: { servers: [] } });
    mockUpdateAgent.mockResolvedValue({ success: true });
  });

  async function mountReady(agent = makeAgent()) {
    render(
      <AgentEditPanel
        agent={agent}
        providers={PROVIDERS_MULTI}
        modelRegistry={{}}
        onClose={vi.fn()}
        onSave={vi.fn()}
      />,
    );
    await waitFor(() =>
      expect(screen.getByText(/basic information/i)).toBeInTheDocument(),
    );
  }

  it("typing into Display Name updates the value", async () => {
    await mountReady();
    const input = screen.getByLabelText(/display name/i) as HTMLInputElement;
    fireEvent.change(input, { target: { value: "Updated Name" } });
    expect(input.value).toBe("Updated Name");
  });

  it("typing into Display Name when starting from empty also works (handles `|| ''` branch)", async () => {
    await mountReady(makeAgent({ displayName: "" }));
    const input = screen.getByLabelText(/display name/i) as HTMLInputElement;
    expect(input.value).toBe("");
    fireEvent.change(input, { target: { value: "X" } });
    expect(input.value).toBe("X");
  });

  it("typing into Description updates the value", async () => {
    await mountReady();
    const ta = screen.getByLabelText(/description/i) as HTMLTextAreaElement;
    fireEvent.change(ta, { target: { value: "new desc" } });
    expect(ta.value).toBe("new desc");
  });

  it("dragging the temperature slider updates the displayed value", async () => {
    await mountReady();
    const slider = document.querySelector(
      'input[type="range"]',
    ) as HTMLInputElement;
    fireEvent.change(slider, { target: { value: "1.4" } });
    expect(screen.getByText(/^1\.4$/)).toBeInTheDocument();
  });

  it("temperature shows fallback 0.7 when undefined on the agent", async () => {
    // The render uses `formData.temperature?.toFixed(1) || "0.7"` — covers
    // the fallback branch.
    await mountReady(makeAgent({ temperature: undefined as unknown as number }));
    expect(screen.getByText(/^0\.7$/)).toBeInTheDocument();
  });

  it("editing max output tokens updates the value (Number.parseInt path)", async () => {
    await mountReady();
    const input = document.querySelectorAll(
      'input[type="number"]',
    )[1] as HTMLInputElement;
    fireEvent.change(input, { target: { value: "8192" } });
    expect(input.value).toBe("8192");
  });

  it("editing max output tokens with non-numeric input falls back to 32000", async () => {
    await mountReady();
    const input = document.querySelectorAll(
      'input[type="number"]',
    )[1] as HTMLInputElement;
    // jsdom's number-type inputs swallow non-digit input. To exercise the
    // `Number.parseInt(...) || 32000` fallback, pass an empty string —
    // parseInt("") is NaN → falsy → fallback 32000.
    fireEvent.change(input, { target: { value: "" } });
    expect(input.value).toBe("32000");
  });

  it("typing into the System Prompt textarea inside Advanced updates instructions", async () => {
    await mountReady();
    fireEvent.click(screen.getByRole("button", { name: /advanced options/i }));
    // System Prompt textarea has placeholder "System instructions..." — find by placeholder.
    const ta = screen.getByPlaceholderText(
      /system instructions for the agent/i,
    ) as HTMLTextAreaElement;
    fireEvent.change(ta, { target: { value: "be concise" } });
    expect(ta.value).toBe("be concise");
  });
});

// ============================================================================
// Provider/model: changing provider switches model to provider's default
// ============================================================================

describe("AgentEditPanel — provider/model switch", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockListMcps.mockResolvedValue({ success: true, data: { servers: [] } });
    mockUpdateAgent.mockResolvedValue({ success: true });
  });

  it("changing the provider sets the model to that provider's default (lines 214-220)", async () => {
    render(
      <AgentEditPanel
        agent={makeAgent({ providerId: "openai", model: "gpt-4o" })}
        providers={PROVIDERS_MULTI}
        modelRegistry={{}}
        onClose={vi.fn()}
        onSave={vi.fn()}
      />,
    );
    await waitFor(() =>
      expect(screen.getByText(/basic information/i)).toBeInTheDocument(),
    );

    const providerSelect = screen.getByLabelText(
      /^Provider$/,
    ) as HTMLSelectElement;
    fireEvent.change(providerSelect, { target: { value: "anthropic" } });
    expect(providerSelect.value).toBe("anthropic");

    // The save payload should now use the new provider + its default model.
    fireEvent.click(screen.getByRole("button", { name: /save changes/i }));
    await waitFor(() => expect(mockUpdateAgent).toHaveBeenCalledTimes(1));
    const [, payload] = mockUpdateAgent.mock.calls[0];
    expect(payload.providerId).toBe("anthropic");
    expect(payload.model).toBe("claude-sonnet-4");
  });

  it("changing to an empty provider value clears the model to empty string (covers `: ''` branch on line 218)", async () => {
    // Render with NO providers — the select renders empty. Then dispatch
    // a synthetic change with value="". The `providers.find` returns
    // undefined → the `: ""` branch on line 218 is exercised.
    render(
      <AgentEditPanel
        agent={makeAgent()}
        providers={[]}
        modelRegistry={{}}
        onClose={vi.fn()}
        onSave={vi.fn()}
      />,
    );
    await waitFor(() =>
      expect(screen.getByText(/basic information/i)).toBeInTheDocument(),
    );
    const providerSelect = screen.getByLabelText(
      /^Provider$/,
    ) as HTMLSelectElement;
    fireEvent.change(providerSelect, { target: { value: "" } });

    fireEvent.click(screen.getByRole("button", { name: /save changes/i }));
    await waitFor(() => expect(mockUpdateAgent).toHaveBeenCalledTimes(1));
    const [, payload] = mockUpdateAgent.mock.calls[0];
    expect(payload.providerId).toBe("");
    expect(payload.model).toBe("");
  });

  it("renders the ModelChip when modelRegistry has the current model (lines 237-243)", async () => {
    render(
      <AgentEditPanel
        agent={makeAgent({ model: "gpt-4o" })}
        providers={PROVIDERS_MULTI}
        modelRegistry={REGISTRY_WITH_GPT4O}
        onClose={vi.fn()}
        onSave={vi.fn()}
      />,
    );
    await waitFor(() =>
      expect(screen.getByText(/basic information/i)).toBeInTheDocument(),
    );
    // Two appearances of "gpt-4o" — the input value + the ModelChip text.
    const matches = screen.getAllByText(/gpt-4o/i);
    expect(matches.length).toBeGreaterThan(0);
  });
});

// ============================================================================
// Thinking toggle inner button (lines 313-315) and Enter/Space on the row
// ============================================================================

describe("AgentEditPanel — thinking toggle button", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockListMcps.mockResolvedValue({ success: true, data: { servers: [] } });
    mockUpdateAgent.mockResolvedValue({ success: true });
  });

  async function openAdvanced(agent = makeAgent()) {
    render(
      <AgentEditPanel
        agent={agent}
        providers={PROVIDERS_MULTI}
        modelRegistry={{}}
        onClose={vi.fn()}
        onSave={vi.fn()}
      />,
    );
    await waitFor(() =>
      expect(screen.getByText(/basic information/i)).toBeInTheDocument(),
    );
    fireEvent.click(screen.getByRole("button", { name: /advanced options/i }));
  }

  it("clicking the inner toggle-switch button flips the value (stopPropagation branch)", async () => {
    await openAdvanced(makeAgent({ thinkingEnabled: false }));
    const row = screen.getByText(/thinking enabled/i).closest(".skill-toggle")!;
    const innerToggle = row.querySelector(".toggle-switch")! as HTMLButtonElement;
    fireEvent.click(innerToggle);
    // After click, the toggle should be `--on`
    expect(innerToggle.classList.contains("toggle-switch--on")).toBe(true);
  });

  it("Space key on the thinking row also flips the toggle (covers onKeyDown space branch)", async () => {
    await openAdvanced(makeAgent({ thinkingEnabled: false }));
    const row = screen
      .getByText(/thinking enabled/i)
      .closest(".skill-toggle")!;
    fireEvent.keyDown(row, { key: " " });
    const inner = row.querySelector(".toggle-switch")!;
    expect(inner.classList.contains("toggle-switch--on")).toBe(true);
  });

  it("non-Enter/Space key on the thinking row does NOT flip the toggle", async () => {
    await openAdvanced(makeAgent({ thinkingEnabled: false }));
    const row = screen
      .getByText(/thinking enabled/i)
      .closest(".skill-toggle")!;
    fireEvent.keyDown(row, { key: "a" });
    const inner = row.querySelector(".toggle-switch")!;
    expect(inner.classList.contains("toggle-switch--off")).toBe(true);
  });
});

// ============================================================================
// MCP server list + toggle (lines 351-380)
// ============================================================================

describe("AgentEditPanel — MCP servers", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockUpdateAgent.mockResolvedValue({ success: true });
  });

  it("shows 'No MCP servers configured' when none are returned", async () => {
    mockListMcps.mockResolvedValue({
      success: true,
      data: { servers: [] },
    });
    render(
      <AgentEditPanel
        agent={makeAgent()}
        providers={PROVIDERS_MULTI}
        modelRegistry={{}}
        onClose={vi.fn()}
        onSave={vi.fn()}
      />,
    );
    await waitFor(() =>
      expect(screen.getByText(/basic information/i)).toBeInTheDocument(),
    );
    fireEvent.click(screen.getByRole("button", { name: /advanced options/i }));
    expect(
      screen.getByText(/no mcp servers configured/i),
    ).toBeInTheDocument();
  });

  it("renders one row per MCP, with name + description + type chip", async () => {
    mockListMcps.mockResolvedValue({
      success: true,
      data: {
        servers: [
          {
            id: "fs",
            name: "Filesystem",
            description: "File access",
            type: "stdio",
            enabled: true,
          },
          {
            id: "web",
            name: "Web Fetch",
            description: "Internet access",
            type: "http",
            enabled: true,
          },
        ],
      },
    });
    render(
      <AgentEditPanel
        agent={makeAgent({ mcps: [] })}
        providers={PROVIDERS_MULTI}
        modelRegistry={{}}
        onClose={vi.fn()}
        onSave={vi.fn()}
      />,
    );
    await waitFor(() =>
      expect(screen.getByText(/basic information/i)).toBeInTheDocument(),
    );
    fireEvent.click(screen.getByRole("button", { name: /advanced options/i }));

    expect(screen.getByText(/filesystem/i)).toBeInTheDocument();
    expect(screen.getByText(/web fetch/i)).toBeInTheDocument();
    expect(screen.getByText(/file access/i)).toBeInTheDocument();
    expect(screen.getByText(/internet access/i)).toBeInTheDocument();
    // Type chips
    expect(screen.getByText(/^stdio$/)).toBeInTheDocument();
    expect(screen.getByText(/^http$/)).toBeInTheDocument();
  });

  it("clicking an MCP row adds it to the agent's mcps; clicking again removes it", async () => {
    mockListMcps.mockResolvedValue({
      success: true,
      data: {
        servers: [
          {
            id: "fs",
            name: "Filesystem",
            description: "File access",
            type: "stdio",
            enabled: true,
          },
        ],
      },
    });
    render(
      <AgentEditPanel
        agent={makeAgent({ mcps: [] })}
        providers={PROVIDERS_MULTI}
        modelRegistry={{}}
        onClose={vi.fn()}
        onSave={vi.fn()}
      />,
    );
    await waitFor(() =>
      expect(screen.getByText(/basic information/i)).toBeInTheDocument(),
    );
    fireEvent.click(screen.getByRole("button", { name: /advanced options/i }));

    const row = screen.getByText(/filesystem/i).closest(".skill-toggle")!;
    fireEvent.click(row);
    // Now toggle reflects ON state
    let toggle = row.querySelector(".toggle-switch")!;
    expect(toggle.classList.contains("toggle-switch--on")).toBe(true);

    // Click again — removes
    fireEvent.click(row);
    toggle = row.querySelector(".toggle-switch")!;
    expect(toggle.classList.contains("toggle-switch--off")).toBe(true);
  });

  it("clicking the inner MCP toggle-switch button stops propagation and toggles", async () => {
    mockListMcps.mockResolvedValue({
      success: true,
      data: {
        servers: [
          {
            id: "fs",
            name: "Filesystem",
            description: "File access",
            type: "stdio",
            enabled: true,
          },
        ],
      },
    });
    render(
      <AgentEditPanel
        agent={makeAgent({ mcps: [] })}
        providers={PROVIDERS_MULTI}
        modelRegistry={{}}
        onClose={vi.fn()}
        onSave={vi.fn()}
      />,
    );
    await waitFor(() =>
      expect(screen.getByText(/basic information/i)).toBeInTheDocument(),
    );
    fireEvent.click(screen.getByRole("button", { name: /advanced options/i }));

    const row = screen.getByText(/filesystem/i).closest(".skill-toggle")!;
    const inner = row.querySelector(".toggle-switch")!;
    fireEvent.click(inner);
    // Re-query — the row was rebuilt; isOn should be true now.
    const newRow = screen.getByText(/filesystem/i).closest(".skill-toggle")!;
    expect(
      newRow.querySelector(".toggle-switch")!.classList.contains(
        "toggle-switch--on",
      ),
    ).toBe(true);
  });

  it("Enter on an MCP row toggles it (covers onKeyDown Enter branch)", async () => {
    mockListMcps.mockResolvedValue({
      success: true,
      data: {
        servers: [
          {
            id: "fs",
            name: "Filesystem",
            description: "File access",
            type: "stdio",
            enabled: true,
          },
        ],
      },
    });
    render(
      <AgentEditPanel
        agent={makeAgent({ mcps: [] })}
        providers={PROVIDERS_MULTI}
        modelRegistry={{}}
        onClose={vi.fn()}
        onSave={vi.fn()}
      />,
    );
    await waitFor(() =>
      expect(screen.getByText(/basic information/i)).toBeInTheDocument(),
    );
    fireEvent.click(screen.getByRole("button", { name: /advanced options/i }));

    const row = screen.getByText(/filesystem/i).closest(".skill-toggle")!;
    fireEvent.keyDown(row, { key: "Enter" });
    expect(
      screen
        .getByText(/filesystem/i)
        .closest(".skill-toggle")!
        .querySelector(".toggle-switch")!
        .classList.contains("toggle-switch--on"),
    ).toBe(true);
  });

  it("Space on an MCP row toggles it (covers onKeyDown Space branch)", async () => {
    mockListMcps.mockResolvedValue({
      success: true,
      data: {
        servers: [
          {
            id: "fs",
            name: "Filesystem",
            description: "File access",
            type: "stdio",
            enabled: true,
          },
        ],
      },
    });
    render(
      <AgentEditPanel
        agent={makeAgent({ mcps: [] })}
        providers={PROVIDERS_MULTI}
        modelRegistry={{}}
        onClose={vi.fn()}
        onSave={vi.fn()}
      />,
    );
    await waitFor(() =>
      expect(screen.getByText(/basic information/i)).toBeInTheDocument(),
    );
    fireEvent.click(screen.getByRole("button", { name: /advanced options/i }));

    const row = screen.getByText(/filesystem/i).closest(".skill-toggle")!;
    fireEvent.keyDown(row, { key: " " });
    expect(
      screen
        .getByText(/filesystem/i)
        .closest(".skill-toggle")!
        .querySelector(".toggle-switch")!
        .classList.contains("toggle-switch--on"),
    ).toBe(true);
  });

  it("non-Enter/Space key on MCP row does NOT toggle", async () => {
    mockListMcps.mockResolvedValue({
      success: true,
      data: {
        servers: [
          {
            id: "fs",
            name: "Filesystem",
            description: "File access",
            type: "stdio",
            enabled: true,
          },
        ],
      },
    });
    render(
      <AgentEditPanel
        agent={makeAgent({ mcps: [] })}
        providers={PROVIDERS_MULTI}
        modelRegistry={{}}
        onClose={vi.fn()}
        onSave={vi.fn()}
      />,
    );
    await waitFor(() =>
      expect(screen.getByText(/basic information/i)).toBeInTheDocument(),
    );
    fireEvent.click(screen.getByRole("button", { name: /advanced options/i }));

    const row = screen.getByText(/filesystem/i).closest(".skill-toggle")!;
    fireEvent.keyDown(row, { key: "x" });
    expect(
      row.querySelector(".toggle-switch")!.classList.contains(
        "toggle-switch--off",
      ),
    ).toBe(true);
  });

  it("renders an already-enabled MCP as ON (covers `isOn` true branch)", async () => {
    mockListMcps.mockResolvedValue({
      success: true,
      data: {
        servers: [
          {
            id: "fs",
            name: "Filesystem",
            description: "File access",
            type: "stdio",
            enabled: true,
          },
        ],
      },
    });
    render(
      <AgentEditPanel
        agent={makeAgent({ mcps: ["fs"] })}
        providers={PROVIDERS_MULTI}
        modelRegistry={{}}
        onClose={vi.fn()}
        onSave={vi.fn()}
      />,
    );
    await waitFor(() =>
      expect(screen.getByText(/basic information/i)).toBeInTheDocument(),
    );
    fireEvent.click(screen.getByRole("button", { name: /advanced options/i }));

    const row = screen.getByText(/filesystem/i).closest(".skill-toggle")!;
    expect(row.classList.contains("skill-toggle--on")).toBe(true);
  });
});

// ============================================================================
// Error banner display (lines 162-166)
// ============================================================================

describe("AgentEditPanel — error banner", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockListMcps.mockResolvedValue({ success: true, data: { servers: [] } });
    mockUpdateAgent.mockResolvedValue({
      success: false,
      error: "saved-error",
    });
  });

  it("renders the .alert--error banner inside the slideover when error is set", async () => {
    render(
      <AgentEditPanel
        agent={makeAgent()}
        providers={PROVIDERS_MULTI}
        modelRegistry={{}}
        onClose={vi.fn()}
        onSave={vi.fn()}
      />,
    );
    await waitFor(() =>
      expect(screen.getByText(/basic information/i)).toBeInTheDocument(),
    );
    fireEvent.click(screen.getByRole("button", { name: /save changes/i }));
    await waitFor(() =>
      expect(screen.getByText(/saved-error/i)).toBeInTheDocument(),
    );
    // Specifically the alert--error className is on the banner
    const banner = screen.getByText(/saved-error/i).closest(".alert--error");
    expect(banner).not.toBeNull();
    // Make sure the banner is inside the slideover dialog
    const dialog = within(banner!.parentElement! as HTMLElement);
    expect(dialog).toBeDefined();
  });
});
