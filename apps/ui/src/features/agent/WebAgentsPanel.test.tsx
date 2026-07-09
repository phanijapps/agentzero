// ============================================================================
// WebAgentsPanel — comprehensive coverage of the 3-tab page.
// Mocks the transport surface and the AgentEditPanel child to keep these
// tests focused on this component's logic (loading, tabs, list/search/CRUD
// per tab, slideovers, error handling).
// ============================================================================

import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, waitFor, fireEvent, within } from "@/test/utils";
import userEvent from "@testing-library/user-event";
import type {
  AgentResponse,
  ProviderResponse,
  SkillResponse,
  CronJobResponse,
} from "@/services/transport";

// ---------------------------------------------------------------------------
// Mock the AgentEditPanel — it owns its own transport calls. We render a
// stub that exposes the agent prop so the parent's wiring can be asserted.
// ---------------------------------------------------------------------------

vi.mock("./AgentEditPanel", () => ({
  AgentEditPanel: ({
    agent,
    onClose,
    onSave,
  }: {
    agent: AgentResponse;
    onClose: () => void;
    onSave: () => void;
  }) => (
    <div data-testid="agent-edit-panel">
      <span data-testid="agent-edit-panel-name">{agent.name}</span>
      <button onClick={onClose} type="button" aria-label="stub-close">
        stub-close
      </button>
      <button onClick={onSave} type="button" aria-label="stub-save">
        stub-save
      </button>
    </div>
  ),
}));

// ---------------------------------------------------------------------------
// Mock the transport. mockTransport is rebuilt in beforeEach so each test
// gets a fresh set of vi.fn()s. getTransport() returns the live ref.
// ---------------------------------------------------------------------------

interface MockTransport {
  listAgents: ReturnType<typeof vi.fn>;
  createAgent: ReturnType<typeof vi.fn>;
  deleteAgent: ReturnType<typeof vi.fn>;
  listProviders: ReturnType<typeof vi.fn>;
  listModels: ReturnType<typeof vi.fn>;
  listSkills: ReturnType<typeof vi.fn>;
  createSkill: ReturnType<typeof vi.fn>;
  deleteSkill: ReturnType<typeof vi.fn>;
  listCronJobs: ReturnType<typeof vi.fn>;
  createCronJob: ReturnType<typeof vi.fn>;
  updateCronJob: ReturnType<typeof vi.fn>;
  deleteCronJob: ReturnType<typeof vi.fn>;
  enableCronJob: ReturnType<typeof vi.fn>;
  disableCronJob: ReturnType<typeof vi.fn>;
  triggerCronJob: ReturnType<typeof vi.fn>;
}

let mockTransport: MockTransport;
// `transportPromise` is awaited by getTransport(). Tests that exercise the
// loading state can hold this promise unresolved.
let transportResolver: ((t: MockTransport | null) => void) | null = null;

vi.mock("@/services/transport", async () => {
  const actual = await vi.importActual<Record<string, unknown>>(
    "@/services/transport",
  );
  return {
    ...actual,
    getTransport: () =>
      new Promise((resolve) => {
        transportResolver = resolve;
        // resolved synchronously by default — see beforeEach
      }),
    getProviderDefaultModel: () => "gpt-4o",
  };
});

// Now import the panel AFTER vi.mock declarations (Vitest hoists vi.mock,
// so the order doesn't matter functionally, but this keeps things explicit).
import { WebAgentsPanel } from "./WebAgentsPanel";

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

function makeAgent(overrides: Partial<AgentResponse> = {}): AgentResponse {
  return {
    id: "researcher",
    name: "researcher",
    displayName: "Researcher",
    description: "Investigates topics deeply",
    providerId: "openai",
    model: "gpt-4o",
    temperature: 0.7,
    maxInputTokens: 200000,
    maxOutputTokens: 32000,
    maxTokens: 32000,
    thinkingEnabled: false,
    voiceRecordingEnabled: false,
    instructions: "Be helpful.",
    mcps: [],
    skills: [],
    ...overrides,
  };
}

function makeSkill(overrides: Partial<SkillResponse> = {}): SkillResponse {
  return {
    id: "summarize",
    name: "summarize",
    displayName: "Summarize",
    description: "Boil down long text",
    category: "general",
    instructions: "You summarize text.",
    ...overrides,
  };
}

function makeJob(overrides: Partial<CronJobResponse> = {}): CronJobResponse {
  return {
    id: "morning-briefing",
    name: "Morning Briefing",
    schedule: "0 0 9 * * *",
    agent_id: "researcher",
    message: "Brief me on today",
    respond_to: [],
    enabled: true,
    timezone: "",
    last_run: undefined,
    next_run: undefined,
    ...overrides,
  };
}

const PROVIDERS: ProviderResponse[] = [
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
    models: ["claude-sonnet-4", "claude-haiku-4"],
    defaultModel: "claude-sonnet-4",
  } as unknown as ProviderResponse,
];

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function buildTransport(overrides: Partial<MockTransport> = {}): MockTransport {
  return {
    listAgents: vi.fn().mockResolvedValue({ success: true, data: [] }),
    createAgent: vi.fn().mockResolvedValue({ success: true }),
    deleteAgent: vi.fn().mockResolvedValue({ success: true }),
    listProviders: vi.fn().mockResolvedValue({ success: true, data: [] }),
    listModels: vi.fn().mockResolvedValue({ success: true, data: {} }),
    listSkills: vi.fn().mockResolvedValue({ success: true, data: [] }),
    createSkill: vi.fn().mockResolvedValue({ success: true }),
    deleteSkill: vi.fn().mockResolvedValue({ success: true }),
    listCronJobs: vi.fn().mockResolvedValue({ success: true, data: [] }),
    createCronJob: vi.fn().mockResolvedValue({ success: true }),
    updateCronJob: vi.fn().mockResolvedValue({ success: true }),
    deleteCronJob: vi.fn().mockResolvedValue({ success: true }),
    enableCronJob: vi.fn(),
    disableCronJob: vi.fn(),
    triggerCronJob: vi.fn().mockResolvedValue({ success: true }),
    ...overrides,
  };
}

/**
 * Render the panel. By default the transport promise is resolved
 * synchronously so the loading spinner clears before the test continues.
 * Pass `resolveTransport: false` to keep it pending.
 */
async function mountPanel({
  resolveTransport = true,
}: { resolveTransport?: boolean } = {}) {
  const result = render(<WebAgentsPanel />);
  if (resolveTransport) {
    // Drain microtasks until the resolver is set, then resolve it.
    await waitFor(() => expect(transportResolver).not.toBeNull());
    transportResolver!(mockTransport);
  }
  return result;
}

// ---------------------------------------------------------------------------
// beforeEach / afterEach
// ---------------------------------------------------------------------------

beforeEach(() => {
  vi.clearAllMocks();
  transportResolver = null;
  mockTransport = buildTransport();
  // Default-resolve the URL search-params back to the agents tab.
  window.history.replaceState({}, "", "/");
});

afterEach(() => {
  vi.restoreAllMocks();
});

// ===========================================================================
// 1 — Loading state
// ===========================================================================

describe("WebAgentsPanel — loading state", () => {
  it("renders a spinner before the transport resolves and not the page header", async () => {
    const { container } = await mountPanel({ resolveTransport: false });

    // No page-header-v2 yet — only the loading container.
    expect(container.querySelector(".page-header-v2")).toBeNull();
    // The Loader2 icon is the only child while loading.
    expect(container.querySelector('[class*="lucide-loader"]')).not.toBeNull();
  });

  it("clears the spinner once transport resolves and shows the page header", async () => {
    await mountPanel();
    await waitFor(() =>
      expect(screen.getByText(/^Agents$/)).toBeInTheDocument(),
    );
  });
});

// ===========================================================================
// 2 — Tab switching
// ===========================================================================

describe("WebAgentsPanel — tabs", () => {
  it("starts on My Agents and switches to Skills then Schedules", async () => {
    await mountPanel();

    await waitFor(() =>
      expect(screen.getByText(/no agents yet/i)).toBeInTheDocument(),
    );

    // Click "Skills Library" tab — the empty state for skills appears.
    fireEvent.click(screen.getByRole("tab", { name: /skills library/i }));
    await waitFor(() =>
      expect(screen.getByText(/no skills yet/i)).toBeInTheDocument(),
    );

    // Click "Schedules" tab — the empty state for schedules appears.
    fireEvent.click(screen.getByRole("tab", { name: /schedules/i }));
    await waitFor(() =>
      expect(screen.getByText(/no schedules yet/i)).toBeInTheDocument(),
    );
  });

  it("encodes the active tab in the URL search-params (skills/schedules) but not for agents", async () => {
    await mountPanel();
    await waitFor(() =>
      expect(screen.getByText(/no agents yet/i)).toBeInTheDocument(),
    );

    fireEvent.click(screen.getByRole("tab", { name: /skills library/i }));
    await waitFor(() =>
      expect(window.location.search).toContain("tab=skills"),
    );

    fireEvent.click(screen.getByRole("tab", { name: /my agents/i }));
    await waitFor(() => expect(window.location.search).toBe(""));
  });
});

// ===========================================================================
// 3 — Agent list rendering + provider name + chips
// ===========================================================================

describe("WebAgentsPanel — agent list", () => {
  it("renders one card per agent with provider name + skill/MCP chips", async () => {
    mockTransport.listAgents.mockResolvedValue({
      success: true,
      data: [
        makeAgent({
          id: "researcher",
          name: "researcher",
          displayName: "Researcher",
          providerId: "openai",
          skills: ["a", "b"],
          mcps: ["fs"],
        }),
        makeAgent({
          id: "coder",
          name: "coder",
          displayName: "Coder",
          providerId: "anthropic",
          skills: [],
          mcps: [],
        }),
      ],
    });
    mockTransport.listProviders.mockResolvedValue({
      success: true,
      data: PROVIDERS,
    });

    await mountPanel();

    await waitFor(() =>
      expect(screen.getByText(/^Researcher$/)).toBeInTheDocument(),
    );

    // Both agent cards present.
    expect(screen.getByText(/^Researcher$/)).toBeInTheDocument();
    expect(screen.getByText(/^Coder$/)).toBeInTheDocument();

    // Provider names resolve from the providers map (footer text).
    expect(screen.getByText(/^OpenAI$/)).toBeInTheDocument();
    expect(screen.getByText(/^Anthropic$/)).toBeInTheDocument();

    // Skill + MCP MetaChips render only when count > 0.
    expect(screen.getByText(/2 skills/i)).toBeInTheDocument();
    expect(screen.getByText(/1 MCP$/i)).toBeInTheDocument();
    // Coder has zero — no chip for it.
    expect(screen.queryByText(/0 skills/i)).not.toBeInTheDocument();
  });

  it("renders the agent description if present", async () => {
    mockTransport.listAgents.mockResolvedValue({
      success: true,
      data: [makeAgent({ description: "Investigates topics deeply" })],
    });
    await mountPanel();
    await waitFor(() =>
      expect(
        screen.getByText(/investigates topics deeply/i),
      ).toBeInTheDocument(),
    );
  });
});

// ===========================================================================
// 4 — Agent search filter
// ===========================================================================

describe("WebAgentsPanel — agent search", () => {
  it("filters agents by name / display name / description", async () => {
    mockTransport.listAgents.mockResolvedValue({
      success: true,
      data: [
        makeAgent({ id: "researcher", name: "researcher", displayName: "Researcher" }),
        makeAgent({
          id: "coder",
          name: "coder",
          displayName: "Code Agent",
          description: "writes code",
        }),
      ],
    });
    await mountPanel();
    await waitFor(() =>
      expect(screen.getByText(/^Researcher$/)).toBeInTheDocument(),
    );

    const search = screen.getByPlaceholderText(/search agents/i);
    fireEvent.change(search, { target: { value: "code" } });

    await waitFor(() =>
      expect(screen.queryByText(/^Researcher$/)).not.toBeInTheDocument(),
    );
    expect(screen.getByText(/code agent/i)).toBeInTheDocument();
  });

  it("shows the 'no matching agents' empty state when filter excludes all", async () => {
    mockTransport.listAgents.mockResolvedValue({
      success: true,
      data: [makeAgent()],
    });
    await mountPanel();
    await waitFor(() =>
      expect(screen.getByText(/^Researcher$/)).toBeInTheDocument(),
    );

    fireEvent.change(screen.getByPlaceholderText(/search agents/i), {
      target: { value: "nope" },
    });
    await waitFor(() =>
      expect(screen.getByText(/no matching agents/i)).toBeInTheDocument(),
    );
  });
});

// ===========================================================================
// 5 — Click agent card → opens edit panel (mouse + keyboard)
// ===========================================================================

describe("WebAgentsPanel — open edit panel", () => {
  it("opens AgentEditPanel when an agent card is clicked", async () => {
    mockTransport.listAgents.mockResolvedValue({
      success: true,
      data: [makeAgent({ name: "researcher" })],
    });
    await mountPanel();
    await waitFor(() =>
      expect(screen.getByText(/^Researcher$/)).toBeInTheDocument(),
    );

    // The card is a role=button — click it.
    const card = screen.getByText(/^Researcher$/).closest(".agent-card");
    expect(card).not.toBeNull();
    fireEvent.click(card!);

    await waitFor(() =>
      expect(screen.getByTestId("agent-edit-panel")).toBeInTheDocument(),
    );
    expect(screen.getByTestId("agent-edit-panel-name")).toHaveTextContent(
      "researcher",
    );
  });

  it("opens AgentEditPanel via Enter key on the card", async () => {
    mockTransport.listAgents.mockResolvedValue({
      success: true,
      data: [makeAgent({ name: "researcher" })],
    });
    await mountPanel();
    await waitFor(() =>
      expect(screen.getByText(/^Researcher$/)).toBeInTheDocument(),
    );

    const card = screen.getByText(/^Researcher$/).closest(".agent-card")!;
    fireEvent.keyDown(card, { key: "Enter" });
    await waitFor(() =>
      expect(screen.getByTestId("agent-edit-panel")).toBeInTheDocument(),
    );
  });

  it("opens AgentEditPanel via Space key on the card", async () => {
    mockTransport.listAgents.mockResolvedValue({
      success: true,
      data: [makeAgent({ name: "researcher" })],
    });
    await mountPanel();
    await waitFor(() =>
      expect(screen.getByText(/^Researcher$/)).toBeInTheDocument(),
    );

    const card = screen.getByText(/^Researcher$/).closest(".agent-card")!;
    fireEvent.keyDown(card, { key: " " });
    await waitFor(() =>
      expect(screen.getByTestId("agent-edit-panel")).toBeInTheDocument(),
    );
  });

  it("ignores other key presses on the card (no edit panel)", async () => {
    mockTransport.listAgents.mockResolvedValue({
      success: true,
      data: [makeAgent({ name: "researcher" })],
    });
    await mountPanel();
    await waitFor(() =>
      expect(screen.getByText(/^Researcher$/)).toBeInTheDocument(),
    );

    const card = screen.getByText(/^Researcher$/).closest(".agent-card")!;
    fireEvent.keyDown(card, { key: "a" });
    expect(screen.queryByTestId("agent-edit-panel")).not.toBeInTheDocument();
  });

  it("Edit (pencil) button opens the edit panel and stops propagation", async () => {
    mockTransport.listAgents.mockResolvedValue({
      success: true,
      data: [makeAgent({ name: "researcher" })],
    });
    await mountPanel();
    await waitFor(() =>
      expect(screen.getByText(/^Researcher$/)).toBeInTheDocument(),
    );

    const card = screen.getByText(/^Researcher$/).closest(".agent-card")!;
    const actions = card.querySelector(".agent-card__footer-actions")!;
    const pencil = actions.querySelectorAll("button")[0];
    fireEvent.click(pencil);
    await waitFor(() =>
      expect(screen.getByTestId("agent-edit-panel")).toBeInTheDocument(),
    );
  });
});

// ===========================================================================
// 6 — Delete agent
// ===========================================================================

describe("WebAgentsPanel — delete agent", () => {
  it("calls deleteAgent + reloads list when confirm=true", async () => {
    mockTransport.listAgents
      .mockResolvedValueOnce({
        success: true,
        data: [makeAgent({ id: "researcher" })],
      })
      .mockResolvedValueOnce({ success: true, data: [] });

    await mountPanel();
    await waitFor(() =>
      expect(screen.getByText(/^Researcher$/)).toBeInTheDocument(),
    );

    vi.spyOn(window, "confirm").mockReturnValue(true);

    const card = screen.getByText(/^Researcher$/).closest(".agent-card")!;
    const actions = card.querySelector(".agent-card__footer-actions")!;
    const trash = actions.querySelectorAll("button")[1];
    fireEvent.click(trash);

    await waitFor(() =>
      expect(mockTransport.deleteAgent).toHaveBeenCalledWith("researcher"),
    );
    // listAgents is called twice: initial load + reload after delete.
    await waitFor(() =>
      expect(mockTransport.listAgents).toHaveBeenCalledTimes(2),
    );
  });

  it("does NOT call deleteAgent when confirm=false", async () => {
    mockTransport.listAgents.mockResolvedValue({
      success: true,
      data: [makeAgent()],
    });
    await mountPanel();
    await waitFor(() =>
      expect(screen.getByText(/^Researcher$/)).toBeInTheDocument(),
    );

    vi.spyOn(window, "confirm").mockReturnValue(false);

    const card = screen.getByText(/^Researcher$/).closest(".agent-card")!;
    const trash = card.querySelectorAll(
      ".agent-card__footer-actions button",
    )[1];
    fireEvent.click(trash);

    expect(mockTransport.deleteAgent).not.toHaveBeenCalled();
  });

  it("shows error banner when delete fails", async () => {
    mockTransport.listAgents.mockResolvedValue({
      success: true,
      data: [makeAgent()],
    });
    mockTransport.deleteAgent.mockResolvedValue({
      success: false,
      error: "Boom: agent in use",
    });
    await mountPanel();
    await waitFor(() =>
      expect(screen.getByText(/^Researcher$/)).toBeInTheDocument(),
    );

    vi.spyOn(window, "confirm").mockReturnValue(true);

    const card = screen.getByText(/^Researcher$/).closest(".agent-card")!;
    const trash = card.querySelectorAll(
      ".agent-card__footer-actions button",
    )[1];
    fireEvent.click(trash);

    await waitFor(() =>
      expect(screen.getByText(/boom: agent in use/i)).toBeInTheDocument(),
    );
  });

  it("shows error banner when deleteAgent throws", async () => {
    mockTransport.listAgents.mockResolvedValue({
      success: true,
      data: [makeAgent()],
    });
    mockTransport.deleteAgent.mockRejectedValue(new Error("network down"));
    await mountPanel();
    await waitFor(() =>
      expect(screen.getByText(/^Researcher$/)).toBeInTheDocument(),
    );

    vi.spyOn(window, "confirm").mockReturnValue(true);

    const card = screen.getByText(/^Researcher$/).closest(".agent-card")!;
    const trash = card.querySelectorAll(
      ".agent-card__footer-actions button",
    )[1];
    fireEvent.click(trash);

    await waitFor(() =>
      expect(screen.getByText(/network down/i)).toBeInTheDocument(),
    );
  });
});

// ===========================================================================
// 7 — Create Agent flow
// ===========================================================================

describe("WebAgentsPanel — create agent", () => {
  it("opens slideover, fills the form, and submits", async () => {
    mockTransport.listProviders.mockResolvedValue({
      success: true,
      data: PROVIDERS,
    });
    await mountPanel();
    await waitFor(() =>
      expect(screen.getByText(/no agents yet/i)).toBeInTheDocument(),
    );

    // Click the primary "Create Agent" button in the action bar.
    const createButtons = screen.getAllByRole("button", {
      name: /create agent/i,
    });
    fireEvent.click(createButtons[0]);

    // Slideover heading
    await waitFor(() =>
      expect(
        screen.getByRole("heading", { name: /create agent/i }),
      ).toBeInTheDocument(),
    );

    // Type into Name with mixed case + special chars — auto-lowercases &
    // replaces non [a-z0-9-] chars.
    const nameInput = screen.getByLabelText(/^Name \(ID\)$/);
    fireEvent.change(nameInput, { target: { value: "My Agent!" } });
    expect((nameInput as HTMLInputElement).value).toBe("my-agent-");

    // Display name
    const displayNameInput = screen.getByLabelText(/Display Name/i);
    fireEvent.change(displayNameInput, { target: { value: "My Agent" } });

    // Description
    fireEvent.change(screen.getByLabelText(/Description/i), {
      target: { value: "Helpful" },
    });

    // Submit
    fireEvent.click(
      screen.getByRole("button", { name: /^Create$/ }),
    );

    await waitFor(() =>
      expect(mockTransport.createAgent).toHaveBeenCalledTimes(1),
    );
    expect(mockTransport.createAgent).toHaveBeenCalledWith(
      expect.objectContaining({
        name: "my-agent-",
        displayName: "My Agent",
        description: "Helpful",
        providerId: "openai",
        model: "gpt-4o",
      }),
    );
  });

  it("disables Create button until name + provider + model are present", async () => {
    // No providers — model & providerId stay empty even after the user
    // enters a name, so Create stays disabled.
    mockTransport.listProviders.mockResolvedValue({
      success: true,
      data: [],
    });
    await mountPanel();
    await waitFor(() =>
      expect(screen.getByText(/no agents yet/i)).toBeInTheDocument(),
    );

    fireEvent.click(screen.getAllByRole("button", { name: /create agent/i })[0]);
    await waitFor(() =>
      expect(
        screen.getByRole("heading", { name: /create agent/i }),
      ).toBeInTheDocument(),
    );

    const submit = screen.getByRole("button", { name: /^Create$/ });
    expect(submit).toBeDisabled();

    // Even after filling name, no provider means still disabled.
    fireEvent.change(screen.getByLabelText(/^Name \(ID\)$/), {
      target: { value: "x" },
    });
    expect(submit).toBeDisabled();
  });

  it("changing provider updates the model to the new provider's default", async () => {
    mockTransport.listProviders.mockResolvedValue({
      success: true,
      data: PROVIDERS,
    });
    await mountPanel();
    await waitFor(() =>
      expect(screen.getByText(/no agents yet/i)).toBeInTheDocument(),
    );

    fireEvent.click(
      screen.getAllByRole("button", { name: /create agent/i })[0],
    );
    await waitFor(() =>
      expect(screen.getByLabelText(/^Provider$/)).toBeInTheDocument(),
    );

    const providerSelect = screen.getByLabelText(
      /^Provider$/,
    ) as HTMLSelectElement;
    fireEvent.change(providerSelect, { target: { value: "anthropic" } });
    // getProviderDefaultModel mock returns "gpt-4o" regardless — but we
    // still verify the providerId moved.
    expect(providerSelect.value).toBe("anthropic");
  });

  it("model select shows the provider's models when a provider is chosen", async () => {
    mockTransport.listProviders.mockResolvedValue({
      success: true,
      data: PROVIDERS,
    });
    await mountPanel();
    await waitFor(() =>
      expect(screen.getByText(/no agents yet/i)).toBeInTheDocument(),
    );

    fireEvent.click(
      screen.getAllByRole("button", { name: /create agent/i })[0],
    );
    await waitFor(() =>
      expect(screen.getByLabelText(/^Model$/)).toBeInTheDocument(),
    );

    const modelSelect = screen.getByLabelText(/^Model$/) as HTMLSelectElement;
    const optionValues = Array.from(modelSelect.options).map((o) => o.value);
    expect(optionValues).toContain("gpt-4o");
    expect(optionValues).toContain("gpt-4o-mini");
  });

  it("create failure surfaces error", async () => {
    mockTransport.listProviders.mockResolvedValue({
      success: true,
      data: PROVIDERS,
    });
    mockTransport.createAgent.mockResolvedValue({
      success: false,
      error: "duplicate name",
    });

    await mountPanel();
    await waitFor(() =>
      expect(screen.getByText(/no agents yet/i)).toBeInTheDocument(),
    );

    fireEvent.click(
      screen.getAllByRole("button", { name: /create agent/i })[0],
    );
    await waitFor(() =>
      expect(screen.getByLabelText(/^Name \(ID\)$/)).toBeInTheDocument(),
    );
    fireEvent.change(screen.getByLabelText(/^Name \(ID\)$/), {
      target: { value: "dupe" },
    });
    fireEvent.click(screen.getByRole("button", { name: /^Create$/ }));

    await waitFor(() =>
      expect(screen.getByText(/duplicate name/i)).toBeInTheDocument(),
    );
  });

  it("create thrown error surfaces error", async () => {
    mockTransport.listProviders.mockResolvedValue({
      success: true,
      data: PROVIDERS,
    });
    mockTransport.createAgent.mockRejectedValue(new Error("net glitch"));

    await mountPanel();
    await waitFor(() =>
      expect(screen.getByText(/no agents yet/i)).toBeInTheDocument(),
    );

    fireEvent.click(
      screen.getAllByRole("button", { name: /create agent/i })[0],
    );
    await waitFor(() =>
      expect(screen.getByLabelText(/^Name \(ID\)$/)).toBeInTheDocument(),
    );
    fireEvent.change(screen.getByLabelText(/^Name \(ID\)$/), {
      target: { value: "x" },
    });
    fireEvent.click(screen.getByRole("button", { name: /^Create$/ }));

    await waitFor(() =>
      expect(screen.getByText(/net glitch/i)).toBeInTheDocument(),
    );
  });

  it("does nothing when the empty-state action is clicked then submitted with empty name", async () => {
    await mountPanel();
    await waitFor(() =>
      expect(screen.getByText(/no agents yet/i)).toBeInTheDocument(),
    );
    // Empty state action also opens the slideover.
    const emptyStateBtns = screen.getAllByRole("button", {
      name: /create agent/i,
    });
    fireEvent.click(emptyStateBtns[emptyStateBtns.length - 1]);
    await waitFor(() =>
      expect(
        screen.getByRole("heading", { name: /create agent/i }),
      ).toBeInTheDocument(),
    );
    // Submit while name is empty — handler short-circuits.
    const submit = screen.getByRole("button", { name: /^Create$/ });
    expect(submit).toBeDisabled();
    expect(mockTransport.createAgent).not.toHaveBeenCalled();
  });
});

// ===========================================================================
// 8 — Skills tab
// ===========================================================================

describe("WebAgentsPanel — skills tab", () => {
  async function mountAndSwitchToSkills() {
    await mountPanel();
    // Wait for the page header to render (i.e. loading is done).
    await waitFor(() =>
      expect(screen.getByText(/^Agents$/)).toBeInTheDocument(),
    );
    fireEvent.click(screen.getByRole("tab", { name: /skills library/i }));
  }

  it("renders skill cards and filters by search", async () => {
    mockTransport.listSkills.mockResolvedValue({
      success: true,
      data: [
        makeSkill({
          id: "summarize",
          name: "summarize",
          displayName: "Summarize",
        }),
        makeSkill({
          id: "translate",
          name: "translate",
          displayName: "Translate",
        }),
      ],
    });
    await mountAndSwitchToSkills();

    await waitFor(() =>
      expect(screen.getByText(/^Summarize$/)).toBeInTheDocument(),
    );
    expect(screen.getByText(/^Translate$/)).toBeInTheDocument();

    fireEvent.change(screen.getByPlaceholderText(/search skills/i), {
      target: { value: "trans" },
    });
    await waitFor(() =>
      expect(screen.queryByText(/^Summarize$/)).not.toBeInTheDocument(),
    );
    expect(screen.getByText(/^Translate$/)).toBeInTheDocument();
  });

  it("filtering with no matches shows the no-matching empty state", async () => {
    mockTransport.listSkills.mockResolvedValue({
      success: true,
      data: [makeSkill()],
    });
    await mountAndSwitchToSkills();
    await waitFor(() =>
      expect(screen.getByText(/^Summarize$/)).toBeInTheDocument(),
    );
    fireEvent.change(screen.getByPlaceholderText(/search skills/i), {
      target: { value: "zzzz" },
    });
    await waitFor(() =>
      expect(screen.getByText(/no matching skills/i)).toBeInTheDocument(),
    );
  });

  it("clicking a skill card opens the detail slideover", async () => {
    mockTransport.listSkills.mockResolvedValue({
      success: true,
      data: [
        makeSkill({
          id: "summarize",
          name: "summarize",
          displayName: "Summarize",
          description: "Boil down long text",
          category: "writing",
          instructions: "Step 1. Read.\nStep 2. Summarize.",
        }),
      ],
    });
    await mountAndSwitchToSkills();
    await waitFor(() =>
      expect(screen.getByText(/^Summarize$/)).toBeInTheDocument(),
    );

    const card = screen.getByText(/^Summarize$/).closest(".skill-card")!;
    fireEvent.click(card);
    // The slideover renders an instructions <pre>.
    await waitFor(() =>
      expect(screen.getByText(/Step 1\. Read\./)).toBeInTheDocument(),
    );
    // Description shows in the slideover (id="skill-description-value").
    expect(
      document.getElementById("skill-description-value"),
    ).toHaveTextContent(/Boil down long text/);
  });

  it("opens detail slideover via Space key", async () => {
    mockTransport.listSkills.mockResolvedValue({
      success: true,
      data: [makeSkill()],
    });
    await mountAndSwitchToSkills();
    await waitFor(() =>
      expect(screen.getByText(/^Summarize$/)).toBeInTheDocument(),
    );

    const card = screen.getByText(/^Summarize$/).closest(".skill-card")!;
    fireEvent.keyDown(card, { key: " " });
    await waitFor(() =>
      expect(
        document.getElementById("skill-instructions-value"),
      ).not.toBeNull(),
    );
  });

  it("delete from detail slideover confirms + reloads", async () => {
    mockTransport.listSkills
      .mockResolvedValueOnce({ success: true, data: [makeSkill()] })
      .mockResolvedValueOnce({ success: true, data: [] });
    await mountAndSwitchToSkills();
    await waitFor(() =>
      expect(screen.getByText(/^Summarize$/)).toBeInTheDocument(),
    );

    fireEvent.click(screen.getByText(/^Summarize$/).closest(".skill-card")!);
    await waitFor(() =>
      expect(
        document.getElementById("skill-instructions-value"),
      ).not.toBeNull(),
    );

    vi.spyOn(window, "confirm").mockReturnValue(true);
    fireEvent.click(screen.getByRole("button", { name: /delete/i }));

    await waitFor(() =>
      expect(mockTransport.deleteSkill).toHaveBeenCalledWith("summarize"),
    );
    await waitFor(() =>
      expect(mockTransport.listSkills).toHaveBeenCalledTimes(2),
    );
  });

  it("delete from detail slideover with confirm=false aborts", async () => {
    mockTransport.listSkills.mockResolvedValue({
      success: true,
      data: [makeSkill()],
    });
    await mountAndSwitchToSkills();
    await waitFor(() =>
      expect(screen.getByText(/^Summarize$/)).toBeInTheDocument(),
    );

    fireEvent.click(screen.getByText(/^Summarize$/).closest(".skill-card")!);
    await waitFor(() =>
      expect(
        document.getElementById("skill-instructions-value"),
      ).not.toBeNull(),
    );

    vi.spyOn(window, "confirm").mockReturnValue(false);
    fireEvent.click(screen.getByRole("button", { name: /delete/i }));

    expect(mockTransport.deleteSkill).not.toHaveBeenCalled();
  });

  it("delete failure surfaces error banner", async () => {
    mockTransport.listSkills.mockResolvedValue({
      success: true,
      data: [makeSkill()],
    });
    mockTransport.deleteSkill.mockResolvedValue({
      success: false,
      error: "skill referenced by agent",
    });
    await mountAndSwitchToSkills();
    await waitFor(() =>
      expect(screen.getByText(/^Summarize$/)).toBeInTheDocument(),
    );

    fireEvent.click(screen.getByText(/^Summarize$/).closest(".skill-card")!);
    await waitFor(() =>
      expect(
        document.getElementById("skill-instructions-value"),
      ).not.toBeNull(),
    );

    vi.spyOn(window, "confirm").mockReturnValue(true);
    fireEvent.click(screen.getByRole("button", { name: /delete/i }));

    await waitFor(() =>
      expect(
        screen.getByText(/skill referenced by agent/i),
      ).toBeInTheDocument(),
    );
  });

  it("delete throw surfaces error banner", async () => {
    mockTransport.listSkills.mockResolvedValue({
      success: true,
      data: [makeSkill()],
    });
    mockTransport.deleteSkill.mockRejectedValue(new Error("kaboom"));
    await mountAndSwitchToSkills();
    await waitFor(() =>
      expect(screen.getByText(/^Summarize$/)).toBeInTheDocument(),
    );

    fireEvent.click(screen.getByText(/^Summarize$/).closest(".skill-card")!);
    await waitFor(() =>
      expect(
        document.getElementById("skill-instructions-value"),
      ).not.toBeNull(),
    );

    vi.spyOn(window, "confirm").mockReturnValue(true);
    fireEvent.click(screen.getByRole("button", { name: /delete/i }));

    await waitFor(() =>
      expect(screen.getByText(/kaboom/i)).toBeInTheDocument(),
    );
  });

  it("create skill: fills form + submits + creates", async () => {
    await mountAndSwitchToSkills();
    await waitFor(() =>
      expect(screen.getByText(/no skills yet/i)).toBeInTheDocument(),
    );

    fireEvent.click(
      screen.getAllByRole("button", { name: /create skill/i })[0],
    );
    await waitFor(() =>
      expect(
        screen.getByRole("heading", { name: /create skill/i }),
      ).toBeInTheDocument(),
    );

    const nameInput = screen.getByLabelText(/^Name \(ID\)$/);
    fireEvent.change(nameInput, { target: { value: "My Skill!" } });
    expect((nameInput as HTMLInputElement).value).toBe("my-skill-");

    fireEvent.change(screen.getByLabelText(/Display Name/i), {
      target: { value: "My Skill" },
    });
    fireEvent.change(screen.getByLabelText(/Description/i), {
      target: { value: "Does X" },
    });
    fireEvent.change(screen.getByLabelText(/Category/i), {
      target: { value: "writing" },
    });
    fireEvent.change(screen.getByLabelText(/Instructions/i), {
      target: { value: "Step 1." },
    });

    // Submit via the slideover's footer button (scoped to .slideover--open)
    const slideover = document.querySelector(".slideover--open")!;
    const submit = within(slideover as HTMLElement).getByRole("button", {
      name: /create skill/i,
    });
    fireEvent.click(submit);

    await waitFor(() =>
      expect(mockTransport.createSkill).toHaveBeenCalledTimes(1),
    );
    expect(mockTransport.createSkill).toHaveBeenCalledWith(
      expect.objectContaining({
        name: "my-skill-",
        displayName: "My Skill",
        category: "writing",
      }),
    );
  });

  it("create skill failure surfaces error", async () => {
    mockTransport.createSkill.mockResolvedValue({
      success: false,
      error: "no good",
    });
    await mountAndSwitchToSkills();
    await waitFor(() =>
      expect(screen.getByText(/no skills yet/i)).toBeInTheDocument(),
    );

    fireEvent.click(
      screen.getAllByRole("button", { name: /create skill/i })[0],
    );
    await waitFor(() =>
      expect(screen.getByLabelText(/^Name \(ID\)$/)).toBeInTheDocument(),
    );
    fireEvent.change(screen.getByLabelText(/^Name \(ID\)$/), {
      target: { value: "x" },
    });
    const slideover = document.querySelector(".slideover--open")!;
    fireEvent.click(
      within(slideover as HTMLElement).getByRole("button", {
        name: /create skill/i,
      }),
    );

    await waitFor(() =>
      expect(screen.getByText(/no good/i)).toBeInTheDocument(),
    );
  });

  it("create skill thrown error surfaces error", async () => {
    mockTransport.createSkill.mockRejectedValue(new Error("hard fail"));
    await mountAndSwitchToSkills();
    await waitFor(() =>
      expect(screen.getByText(/no skills yet/i)).toBeInTheDocument(),
    );

    fireEvent.click(
      screen.getAllByRole("button", { name: /create skill/i })[0],
    );
    await waitFor(() =>
      expect(screen.getByLabelText(/^Name \(ID\)$/)).toBeInTheDocument(),
    );
    fireEvent.change(screen.getByLabelText(/^Name \(ID\)$/), {
      target: { value: "x" },
    });
    const slideover = document.querySelector(".slideover--open")!;
    fireEvent.click(
      within(slideover as HTMLElement).getByRole("button", {
        name: /create skill/i,
      }),
    );

    await waitFor(() =>
      expect(screen.getByText(/hard fail/i)).toBeInTheDocument(),
    );
  });
});

// ===========================================================================
// 9 — Schedules tab
// ===========================================================================

describe("WebAgentsPanel — schedules tab", () => {
  async function mountAndSwitchToSchedules() {
    await mountPanel();
    await waitFor(() =>
      expect(screen.getByText(/^Agents$/)).toBeInTheDocument(),
    );
    fireEvent.click(screen.getByRole("tab", { name: /schedules/i }));
  }

  it("renders enabled (Play) and disabled (Pause) icons + agent name + paused next-run", async () => {
    mockTransport.listAgents.mockResolvedValue({
      success: true,
      data: [
        makeAgent({ id: "researcher", displayName: "Researcher" }),
      ],
    });
    mockTransport.listCronJobs.mockResolvedValue({
      success: true,
      data: [
        makeJob({
          id: "j1",
          name: "Job One",
          enabled: true,
          last_run: "2026-05-01T00:00:00Z",
          next_run: "2026-05-02T00:00:00Z",
        }),
        makeJob({
          id: "j2",
          name: "Job Two",
          enabled: false,
          agent_id: "unknown",
        }),
      ],
    });

    await mountAndSwitchToSchedules();
    await waitFor(() =>
      expect(screen.getByText(/^Job One$/)).toBeInTheDocument(),
    );
    expect(screen.getByText(/^Job Two$/)).toBeInTheDocument();

    // Last/Next text rendered for the enabled job
    expect(screen.getAllByText(/^Last:/)[0]).toBeInTheDocument();
    expect(screen.getByText(/Next: Paused/)).toBeInTheDocument();

    // Agent name resolves through the agents list (Researcher) for j1
    expect(screen.getByText(/Agent: Researcher/)).toBeInTheDocument();
    // Unknown agent_id falls back to the raw id
    expect(screen.getByText(/Agent: unknown/)).toBeInTheDocument();

    // Cron description: "0 0 9 * * *" → "Daily at 9 AM" preset label
    expect(screen.getAllByText(/Daily at 9 AM/i).length).toBeGreaterThan(0);
  });

  it("filters schedules by name / message / cron string", async () => {
    mockTransport.listCronJobs.mockResolvedValue({
      success: true,
      data: [
        makeJob({ id: "j1", name: "Morning Briefing" }),
        makeJob({ id: "j2", name: "Nightly Cleanup", message: "tidy up" }),
      ],
    });
    await mountAndSwitchToSchedules();
    await waitFor(() =>
      expect(screen.getByText(/^Morning Briefing$/)).toBeInTheDocument(),
    );

    fireEvent.change(screen.getByPlaceholderText(/search schedules/i), {
      target: { value: "tidy" },
    });
    await waitFor(() =>
      expect(
        screen.queryByText(/^Morning Briefing$/),
      ).not.toBeInTheDocument(),
    );
    expect(screen.getByText(/^Nightly Cleanup$/)).toBeInTheDocument();
  });

  it("toggles a schedule on/off via the toggle switch", async () => {
    const enabled = makeJob({ id: "j1", name: "X", enabled: true });
    const disabled = { ...enabled, enabled: false };

    mockTransport.listCronJobs.mockResolvedValue({
      success: true,
      data: [enabled],
    });
    mockTransport.disableCronJob.mockResolvedValue({
      success: true,
      data: disabled,
    });
    mockTransport.enableCronJob.mockResolvedValue({
      success: true,
      data: enabled,
    });

    await mountAndSwitchToSchedules();
    await waitFor(() =>
      expect(screen.getByText(/^X$/)).toBeInTheDocument(),
    );

    // Click the toggle (aria-label "Disable schedule" when currently enabled)
    fireEvent.click(
      screen.getByRole("button", { name: /disable schedule/i }),
    );
    await waitFor(() =>
      expect(mockTransport.disableCronJob).toHaveBeenCalledWith("j1"),
    );

    // Now the label flips to "Enable schedule"
    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: /enable schedule/i }),
      ).toBeInTheDocument(),
    );

    fireEvent.click(
      screen.getByRole("button", { name: /enable schedule/i }),
    );
    await waitFor(() =>
      expect(mockTransport.enableCronJob).toHaveBeenCalledWith("j1"),
    );
  });

  it("toggle failure surfaces error banner (success=false)", async () => {
    mockTransport.listCronJobs.mockResolvedValue({
      success: true,
      data: [makeJob({ id: "j1", enabled: true })],
    });
    mockTransport.disableCronJob.mockResolvedValue({
      success: false,
      error: "scheduler offline",
    });
    await mountAndSwitchToSchedules();
    await waitFor(() =>
      expect(screen.getByText(/^Morning Briefing$/)).toBeInTheDocument(),
    );

    fireEvent.click(
      screen.getByRole("button", { name: /disable schedule/i }),
    );
    await waitFor(() =>
      expect(screen.getByText(/scheduler offline/i)).toBeInTheDocument(),
    );
  });

  it("toggle thrown error surfaces error banner", async () => {
    mockTransport.listCronJobs.mockResolvedValue({
      success: true,
      data: [makeJob({ id: "j1", enabled: true })],
    });
    mockTransport.disableCronJob.mockRejectedValue(new Error("toggle bang"));
    await mountAndSwitchToSchedules();
    await waitFor(() =>
      expect(screen.getByText(/^Morning Briefing$/)).toBeInTheDocument(),
    );

    fireEvent.click(
      screen.getByRole("button", { name: /disable schedule/i }),
    );
    await waitFor(() =>
      expect(screen.getByText(/toggle bang/i)).toBeInTheDocument(),
    );
  });

  it("trigger schedule: shows spinner during the call + reloads on success", async () => {
    mockTransport.listCronJobs.mockResolvedValue({
      success: true,
      data: [makeJob({ id: "j1" })],
    });
    let resolveTrigger: ((v: { success: true }) => void) | null = null;
    mockTransport.triggerCronJob.mockImplementation(
      () =>
        new Promise((r) => {
          resolveTrigger = r as (v: { success: true }) => void;
        }),
    );

    await mountAndSwitchToSchedules();
    await waitFor(() =>
      expect(screen.getByText(/^Morning Briefing$/)).toBeInTheDocument(),
    );

    const triggerBtn = screen.getByTitle(/trigger now/i);
    fireEvent.click(triggerBtn);

    // While trigger is pending, the button is disabled.
    await waitFor(() => expect(triggerBtn).toBeDisabled());

    // Resolve it — the panel reloads schedules.
    resolveTrigger!({ success: true });
    await waitFor(() =>
      expect(mockTransport.listCronJobs).toHaveBeenCalledTimes(2),
    );
  });

  it("trigger failure surfaces error", async () => {
    mockTransport.listCronJobs.mockResolvedValue({
      success: true,
      data: [makeJob({ id: "j1" })],
    });
    mockTransport.triggerCronJob.mockResolvedValue({
      success: false,
      error: "queue full",
    });
    await mountAndSwitchToSchedules();
    await waitFor(() =>
      expect(screen.getByText(/^Morning Briefing$/)).toBeInTheDocument(),
    );

    fireEvent.click(screen.getByTitle(/trigger now/i));
    await waitFor(() =>
      expect(screen.getByText(/queue full/i)).toBeInTheDocument(),
    );
  });

  it("trigger thrown error surfaces error", async () => {
    mockTransport.listCronJobs.mockResolvedValue({
      success: true,
      data: [makeJob({ id: "j1" })],
    });
    mockTransport.triggerCronJob.mockRejectedValue(new Error("trigger blew"));
    await mountAndSwitchToSchedules();
    await waitFor(() =>
      expect(screen.getByText(/^Morning Briefing$/)).toBeInTheDocument(),
    );

    fireEvent.click(screen.getByTitle(/trigger now/i));
    await waitFor(() =>
      expect(screen.getByText(/trigger blew/i)).toBeInTheDocument(),
    );
  });

  it("edit schedule: opens slideover pre-populated, ID disabled, submits update", async () => {
    const job = makeJob({
      id: "morning-briefing",
      name: "Morning Briefing",
      schedule: "0 0 9 * * *",
      message: "Brief me",
    });
    mockTransport.listCronJobs.mockResolvedValue({ success: true, data: [job] });
    await mountAndSwitchToSchedules();
    await waitFor(() =>
      expect(screen.getByText(/^Morning Briefing$/)).toBeInTheDocument(),
    );

    fireEvent.click(screen.getByTitle(/^Edit$/));
    await waitFor(() =>
      expect(
        screen.getByRole("heading", { name: /edit schedule/i }),
      ).toBeInTheDocument(),
    );

    // Pre-populated values
    expect((screen.getByLabelText(/^Name$/) as HTMLInputElement).value).toBe(
      "Morning Briefing",
    );
    const idInput = screen.getByLabelText(/^ID$/) as HTMLInputElement;
    expect(idInput.value).toBe("morning-briefing");
    expect(idInput).toBeDisabled();

    // Tweak the name
    fireEvent.change(screen.getByLabelText(/^Name$/), {
      target: { value: "Morning Update" },
    });

    fireEvent.click(screen.getByRole("button", { name: /save changes/i }));
    await waitFor(() =>
      expect(mockTransport.updateCronJob).toHaveBeenCalledWith(
        "morning-briefing",
        expect.objectContaining({
          name: "Morning Update",
        }),
      ),
    );
  });

  it("edit schedule failure surfaces error", async () => {
    const job = makeJob();
    mockTransport.listCronJobs.mockResolvedValue({ success: true, data: [job] });
    mockTransport.updateCronJob.mockResolvedValue({
      success: false,
      error: "invalid cron",
    });
    await mountAndSwitchToSchedules();
    await waitFor(() =>
      expect(screen.getByText(/^Morning Briefing$/)).toBeInTheDocument(),
    );

    fireEvent.click(screen.getByTitle(/^Edit$/));
    await waitFor(() =>
      expect(
        screen.getByRole("heading", { name: /edit schedule/i }),
      ).toBeInTheDocument(),
    );

    fireEvent.click(screen.getByRole("button", { name: /save changes/i }));
    await waitFor(() =>
      expect(screen.getByText(/invalid cron/i)).toBeInTheDocument(),
    );
  });

  it("create schedule: cron preset select changes form value; Custom does not overwrite; ID auto-generated", async () => {
    await mountAndSwitchToSchedules();
    await waitFor(() =>
      expect(screen.getByText(/no schedules yet/i)).toBeInTheDocument(),
    );

    fireEvent.click(
      screen.getAllByRole("button", { name: /create schedule/i })[0],
    );
    await waitFor(() =>
      expect(
        screen.getByRole("heading", { name: /create schedule/i }),
      ).toBeInTheDocument(),
    );

    // Type a name → ID auto-derived
    fireEvent.change(screen.getByLabelText(/^Name$/), {
      target: { value: "My Job" },
    });
    expect((screen.getByLabelText(/^ID$/) as HTMLInputElement).value).toBe(
      "my-job",
    );

    // Cron preset select
    const cronPreset = screen.getByLabelText(
      /Schedule \(Cron Expression\)/i,
    ) as HTMLSelectElement;
    fireEvent.change(cronPreset, { target: { value: "0 */5 * * * *" } });
    // The text input below the preset reflects the new schedule value.
    const cronText = cronPreset.parentElement!.querySelector(
      'input[type="text"]',
    ) as HTMLInputElement;
    expect(cronText.value).toBe("0 */5 * * * *");

    // Custom — does not overwrite the schedule. Switch back to a known
    // preset, then Custom, and verify the schedule is unchanged.
    fireEvent.change(cronPreset, { target: { value: "0 0 9 * * *" } });
    expect(cronText.value).toBe("0 0 9 * * *");
    fireEvent.change(cronPreset, { target: { value: "custom" } });
    expect(cronText.value).toBe("0 0 9 * * *"); // unchanged

    // Custom edit via the text input directly
    fireEvent.change(cronText, { target: { value: "0 30 8 * * 1-5" } });
    expect(cronText.value).toBe("0 30 8 * * 1-5");

    // Fill remaining required fields
    fireEvent.change(screen.getByLabelText(/^Message$/), {
      target: { value: "do thing" },
    });

    // Toggle the "Enable on Create" switch (covers the !editingSchedule branch).
    // Walk up to the wrapper that holds the toggle-switch sibling.
    const enableLabel = screen.getByText(/enable on create/i);
    let wrapper: HTMLElement | null = enableLabel as HTMLElement;
    let toggle: Element | null = null;
    while (wrapper && !toggle) {
      toggle = wrapper.querySelector(":scope > .toggle-switch");
      wrapper = wrapper.parentElement;
    }
    expect(toggle).not.toBeNull();
    fireEvent.click(toggle!);
    fireEvent.click(toggle!); // back to enabled

    fireEvent.click(screen.getByRole("button", { name: /^Create$/ }));
    await waitFor(() =>
      expect(mockTransport.createCronJob).toHaveBeenCalledWith(
        expect.objectContaining({
          id: "my-job",
          name: "My Job",
          schedule: "0 30 8 * * 1-5",
          message: "do thing",
        }),
      ),
    );
  });

  it("create schedule failure surfaces error", async () => {
    mockTransport.createCronJob.mockResolvedValue({
      success: false,
      error: "no go",
    });
    await mountAndSwitchToSchedules();
    await waitFor(() =>
      expect(screen.getByText(/no schedules yet/i)).toBeInTheDocument(),
    );

    fireEvent.click(
      screen.getAllByRole("button", { name: /create schedule/i })[0],
    );
    await waitFor(() =>
      expect(screen.getByLabelText(/^Name$/)).toBeInTheDocument(),
    );
    fireEvent.change(screen.getByLabelText(/^Name$/), {
      target: { value: "X" },
    });
    fireEvent.change(screen.getByLabelText(/^Message$/), {
      target: { value: "msg" },
    });
    fireEvent.click(screen.getByRole("button", { name: /^Create$/ }));

    await waitFor(() =>
      expect(screen.getByText(/no go/i)).toBeInTheDocument(),
    );
  });

  it("create/update schedule thrown error surfaces error", async () => {
    mockTransport.createCronJob.mockRejectedValue(new Error("net dead"));
    await mountAndSwitchToSchedules();
    await waitFor(() =>
      expect(screen.getByText(/no schedules yet/i)).toBeInTheDocument(),
    );

    fireEvent.click(
      screen.getAllByRole("button", { name: /create schedule/i })[0],
    );
    await waitFor(() =>
      expect(screen.getByLabelText(/^Name$/)).toBeInTheDocument(),
    );
    fireEvent.change(screen.getByLabelText(/^Name$/), {
      target: { value: "X" },
    });
    fireEvent.change(screen.getByLabelText(/^Message$/), {
      target: { value: "msg" },
    });
    fireEvent.click(screen.getByRole("button", { name: /^Create$/ }));

    await waitFor(() =>
      expect(screen.getByText(/net dead/i)).toBeInTheDocument(),
    );
  });

  it("delete schedule with confirm + reload", async () => {
    mockTransport.listCronJobs
      .mockResolvedValueOnce({
        success: true,
        data: [makeJob({ id: "j1" })],
      })
      .mockResolvedValueOnce({ success: true, data: [] });
    await mountAndSwitchToSchedules();
    await waitFor(() =>
      expect(screen.getByText(/^Morning Briefing$/)).toBeInTheDocument(),
    );

    vi.spyOn(window, "confirm").mockReturnValue(true);
    fireEvent.click(screen.getByTitle(/^Delete$/));

    await waitFor(() =>
      expect(mockTransport.deleteCronJob).toHaveBeenCalledWith("j1"),
    );
    await waitFor(() =>
      expect(mockTransport.listCronJobs).toHaveBeenCalledTimes(2),
    );
  });

  it("delete schedule with confirm=false aborts", async () => {
    mockTransport.listCronJobs.mockResolvedValue({
      success: true,
      data: [makeJob({ id: "j1" })],
    });
    await mountAndSwitchToSchedules();
    await waitFor(() =>
      expect(screen.getByText(/^Morning Briefing$/)).toBeInTheDocument(),
    );

    vi.spyOn(window, "confirm").mockReturnValue(false);
    fireEvent.click(screen.getByTitle(/^Delete$/));

    expect(mockTransport.deleteCronJob).not.toHaveBeenCalled();
  });

  it("delete schedule failure surfaces error", async () => {
    mockTransport.listCronJobs.mockResolvedValue({
      success: true,
      data: [makeJob({ id: "j1" })],
    });
    mockTransport.deleteCronJob.mockResolvedValue({
      success: false,
      error: "still running",
    });
    await mountAndSwitchToSchedules();
    await waitFor(() =>
      expect(screen.getByText(/^Morning Briefing$/)).toBeInTheDocument(),
    );

    vi.spyOn(window, "confirm").mockReturnValue(true);
    fireEvent.click(screen.getByTitle(/^Delete$/));

    await waitFor(() =>
      expect(screen.getByText(/still running/i)).toBeInTheDocument(),
    );
  });

  it("delete schedule thrown error surfaces error", async () => {
    mockTransport.listCronJobs.mockResolvedValue({
      success: true,
      data: [makeJob({ id: "j1" })],
    });
    mockTransport.deleteCronJob.mockRejectedValue(new Error("oops"));
    await mountAndSwitchToSchedules();
    await waitFor(() =>
      expect(screen.getByText(/^Morning Briefing$/)).toBeInTheDocument(),
    );

    vi.spyOn(window, "confirm").mockReturnValue(true);
    fireEvent.click(screen.getByTitle(/^Delete$/));

    await waitFor(() =>
      expect(screen.getByText(/oops/i)).toBeInTheDocument(),
    );
  });
});

// ===========================================================================
// 10 — Error banner dismissal + load failure
// ===========================================================================

describe("WebAgentsPanel — error banner", () => {
  it("dismissable via the X button", async () => {
    mockTransport.listAgents.mockResolvedValue({
      success: true,
      data: [makeAgent()],
    });
    mockTransport.deleteAgent.mockResolvedValue({
      success: false,
      error: "kaboom",
    });
    await mountPanel();
    await waitFor(() =>
      expect(screen.getByText(/^Researcher$/)).toBeInTheDocument(),
    );

    vi.spyOn(window, "confirm").mockReturnValue(true);
    const card = screen.getByText(/^Researcher$/).closest(".agent-card")!;
    fireEvent.click(
      card.querySelectorAll(".agent-card__footer-actions button")[1],
    );
    await waitFor(() =>
      expect(screen.getByText(/kaboom/i)).toBeInTheDocument(),
    );

    // Find the dismiss button within the alert.
    const banner = screen.getByText(/kaboom/i).closest(".alert--error")!;
    const dismiss = within(banner as HTMLElement).getByRole("button");
    fireEvent.click(dismiss);
    await waitFor(() =>
      expect(screen.queryByText(/kaboom/i)).not.toBeInTheDocument(),
    );
  });

  it("loadAllData rejection sets the error banner", async () => {
    mockTransport.listAgents.mockRejectedValue(new Error("server gone"));
    await mountPanel();

    await waitFor(() =>
      expect(screen.getByText(/server gone/i)).toBeInTheDocument(),
    );
  });

  it("reloadAgents error sets the banner (skill list reload error path)", async () => {
    // First load succeeds; second listAgents rejects.
    mockTransport.listAgents
      .mockResolvedValueOnce({
        success: true,
        data: [makeAgent()],
      })
      .mockRejectedValueOnce(new Error("reload fail"));
    mockTransport.deleteAgent.mockResolvedValue({ success: true });

    await mountPanel();
    await waitFor(() =>
      expect(screen.getByText(/^Researcher$/)).toBeInTheDocument(),
    );

    vi.spyOn(window, "confirm").mockReturnValue(true);
    const card = screen.getByText(/^Researcher$/).closest(".agent-card")!;
    fireEvent.click(
      card.querySelectorAll(".agent-card__footer-actions button")[1],
    );

    await waitFor(() =>
      expect(screen.getByText(/reload fail/i)).toBeInTheDocument(),
    );
  });
});

// ===========================================================================
// 11 — Branch coverage extras (small uncovered pockets)
// ===========================================================================

describe("WebAgentsPanel — branch coverage extras", () => {
  it("renders the raw cron string when it is not a known preset (describeCron fallback)", async () => {
    mockTransport.listCronJobs.mockResolvedValue({
      success: true,
      data: [
        makeJob({
          id: "j1",
          name: "Custom Cron",
          schedule: "0 1 2 3 4 5", // non-preset
        }),
      ],
    });
    await mountPanel();
    await waitFor(() =>
      expect(screen.getByText(/^Agents$/)).toBeInTheDocument(),
    );
    fireEvent.click(screen.getByRole("tab", { name: /schedules/i }));
    await waitFor(() =>
      expect(screen.getByText(/^Custom Cron$/)).toBeInTheDocument(),
    );
    expect(screen.getByText("0 1 2 3 4 5")).toBeInTheDocument();
  });

  it("filtered-empty schedules list does NOT show the create-schedule action button (action=undefined)", async () => {
    mockTransport.listCronJobs.mockResolvedValue({
      success: true,
      data: [makeJob({ name: "Morning Briefing" })],
    });
    await mountPanel();
    await waitFor(() =>
      expect(screen.getByText(/^Agents$/)).toBeInTheDocument(),
    );
    fireEvent.click(screen.getByRole("tab", { name: /schedules/i }));
    await waitFor(() =>
      expect(screen.getByText(/^Morning Briefing$/)).toBeInTheDocument(),
    );

    fireEvent.change(screen.getByPlaceholderText(/search schedules/i), {
      target: { value: "zzzzzz" },
    });
    await waitFor(() =>
      expect(
        screen.getByText(/no matching schedules/i),
      ).toBeInTheDocument(),
    );
    // The empty state's CTA button is gone (action=undefined when search active).
    // The action-bar's Create Schedule button still exists, but the empty
    // state renders without its own action button.
    const emptyState = document.querySelector(".empty-state")!;
    expect(
      within(emptyState as HTMLElement).queryByRole("button", {
        name: /create schedule/i,
      }),
    ).toBeNull();
  });

  it("filtered-empty skills list also hides its CTA action button", async () => {
    mockTransport.listSkills.mockResolvedValue({
      success: true,
      data: [makeSkill()],
    });
    await mountPanel();
    await waitFor(() =>
      expect(screen.getByText(/^Agents$/)).toBeInTheDocument(),
    );
    fireEvent.click(screen.getByRole("tab", { name: /skills library/i }));
    await waitFor(() =>
      expect(screen.getByText(/^Summarize$/)).toBeInTheDocument(),
    );

    fireEvent.change(screen.getByPlaceholderText(/search skills/i), {
      target: { value: "zzzzzz" },
    });
    await waitFor(() =>
      expect(screen.getByText(/no matching skills/i)).toBeInTheDocument(),
    );
    const emptyState = document.querySelector(".empty-state")!;
    expect(
      within(emptyState as HTMLElement).queryByRole("button", {
        name: /create skill/i,
      }),
    ).toBeNull();
  });

  it("renders the ModelChip in the create-agent slideover when the chosen model has a registry profile", async () => {
    mockTransport.listProviders.mockResolvedValue({
      success: true,
      data: PROVIDERS,
    });
    mockTransport.listModels.mockResolvedValue({
      success: true,
      data: {
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
      },
    });
    await mountPanel();
    await waitFor(() =>
      expect(screen.getByText(/^Agents$/)).toBeInTheDocument(),
    );

    fireEvent.click(
      screen.getAllByRole("button", { name: /create agent/i })[0],
    );
    await waitFor(() =>
      expect(screen.getByLabelText(/^Model$/)).toBeInTheDocument(),
    );

    // The slideover renders a ModelChip below the model select when the
    // chosen model has a profile in modelRegistry. Asserting the presence
    // of "gpt-4o" inside the slideover is enough.
    const slideover = document.querySelector(".slideover--open")!;
    expect(
      within(slideover as HTMLElement).getAllByText(/gpt-4o/).length,
    ).toBeGreaterThan(0);
  });

  it("schedule-id input is editable when creating (writable through onChange)", async () => {
    await mountPanel();
    await waitFor(() =>
      expect(screen.getByText(/^Agents$/)).toBeInTheDocument(),
    );
    fireEvent.click(screen.getByRole("tab", { name: /schedules/i }));
    await waitFor(() =>
      expect(screen.getByText(/no schedules yet/i)).toBeInTheDocument(),
    );

    fireEvent.click(
      screen.getAllByRole("button", { name: /create schedule/i })[0],
    );
    await waitFor(() =>
      expect(screen.getByLabelText(/^ID$/)).toBeInTheDocument(),
    );

    const idInput = screen.getByLabelText(/^ID$/) as HTMLInputElement;
    expect(idInput).not.toBeDisabled();
    fireEvent.change(idInput, { target: { value: "manual-id" } });
    expect(idInput.value).toBe("manual-id");
  });

  it("agent dropdown in the schedule slideover lists non-root agents", async () => {
    mockTransport.listAgents.mockResolvedValue({
      success: true,
      data: [
        makeAgent({ id: "root", name: "root", displayName: "Root" }),
        makeAgent({ id: "researcher", displayName: "Researcher" }),
      ],
    });
    await mountPanel();
    await waitFor(() =>
      expect(screen.getByText(/^Agents$/)).toBeInTheDocument(),
    );

    fireEvent.click(screen.getByRole("tab", { name: /schedules/i }));
    await waitFor(() =>
      expect(screen.getByText(/no schedules yet/i)).toBeInTheDocument(),
    );

    fireEvent.click(
      screen.getAllByRole("button", { name: /create schedule/i })[0],
    );
    await waitFor(() =>
      expect(screen.getByLabelText(/^Agent$/)).toBeInTheDocument(),
    );

    const agentSelect = screen.getByLabelText(/^Agent$/) as HTMLSelectElement;
    const optionValues = Array.from(agentSelect.options).map((o) => o.value);
    expect(optionValues).toContain("root");
    expect(optionValues).toContain("researcher");
    // The hardcoded "root (default)" option appears once — the agent named
    // "root" in the list is filtered out by the `a.id !== "root"` ternary.
    expect(optionValues.filter((v) => v === "root").length).toBe(1);
  });

  it("AgentEditPanel onSave callback triggers reloadAgents + reloadSkills", async () => {
    mockTransport.listAgents.mockResolvedValue({
      success: true,
      data: [makeAgent()],
    });
    mockTransport.listSkills.mockResolvedValue({
      success: true,
      data: [],
    });
    await mountPanel();
    await waitFor(() =>
      expect(screen.getByText(/^Researcher$/)).toBeInTheDocument(),
    );

    fireEvent.click(screen.getByText(/^Researcher$/).closest(".agent-card")!);
    await waitFor(() =>
      expect(screen.getByTestId("agent-edit-panel")).toBeInTheDocument(),
    );

    // Click the stub's "stub-save" → triggers onSave → reloadAgents +
    // reloadSkills. Both lists already had a list call from initial load,
    // so each will be called twice total.
    fireEvent.click(screen.getByLabelText(/stub-save/));
    await waitFor(() =>
      expect(mockTransport.listAgents).toHaveBeenCalledTimes(2),
    );
    await waitFor(() =>
      expect(mockTransport.listSkills).toHaveBeenCalledTimes(2),
    );
  });

  it("AgentEditPanel onClose closes the panel", async () => {
    mockTransport.listAgents.mockResolvedValue({
      success: true,
      data: [makeAgent()],
    });
    await mountPanel();
    await waitFor(() =>
      expect(screen.getByText(/^Researcher$/)).toBeInTheDocument(),
    );

    fireEvent.click(screen.getByText(/^Researcher$/).closest(".agent-card")!);
    await waitFor(() =>
      expect(screen.getByTestId("agent-edit-panel")).toBeInTheDocument(),
    );

    fireEvent.click(screen.getByLabelText(/stub-close/));
    await waitFor(() =>
      expect(screen.queryByTestId("agent-edit-panel")).not.toBeInTheDocument(),
    );
  });

  it("reloadSkills error path: rejected reload after delete-skill success surfaces banner", async () => {
    mockTransport.listSkills
      .mockResolvedValueOnce({
        success: true,
        data: [makeSkill()],
      })
      .mockRejectedValueOnce(new Error("skills reload bang"));
    mockTransport.deleteSkill.mockResolvedValue({ success: true });

    await mountPanel();
    await waitFor(() =>
      expect(screen.getByText(/^Agents$/)).toBeInTheDocument(),
    );
    fireEvent.click(screen.getByRole("tab", { name: /skills library/i }));
    await waitFor(() =>
      expect(screen.getByText(/^Summarize$/)).toBeInTheDocument(),
    );

    fireEvent.click(screen.getByText(/^Summarize$/).closest(".skill-card")!);
    await waitFor(() =>
      expect(
        document.getElementById("skill-instructions-value"),
      ).not.toBeNull(),
    );

    vi.spyOn(window, "confirm").mockReturnValue(true);
    fireEvent.click(screen.getByRole("button", { name: /delete/i }));
    await waitFor(() =>
      expect(screen.getByText(/skills reload bang/i)).toBeInTheDocument(),
    );
  });

  it("reloadSchedules error path: rejected reload after delete-schedule surfaces banner", async () => {
    mockTransport.listCronJobs
      .mockResolvedValueOnce({
        success: true,
        data: [makeJob({ id: "j1" })],
      })
      .mockRejectedValueOnce(new Error("schedules reload bang"));
    mockTransport.deleteCronJob.mockResolvedValue({ success: true });

    await mountPanel();
    await waitFor(() =>
      expect(screen.getByText(/^Agents$/)).toBeInTheDocument(),
    );
    fireEvent.click(screen.getByRole("tab", { name: /schedules/i }));
    await waitFor(() =>
      expect(screen.getByText(/^Morning Briefing$/)).toBeInTheDocument(),
    );

    vi.spyOn(window, "confirm").mockReturnValue(true);
    fireEvent.click(screen.getByTitle(/^Delete$/));
    await waitFor(() =>
      expect(
        screen.getByText(/schedules reload bang/i),
      ).toBeInTheDocument(),
    );
  });
});

// ===========================================================================
// 12 — Userevent typing path (smoke test for typing interactions)
// ===========================================================================

describe("WebAgentsPanel — userEvent integration", () => {
  it("types into the agent search field via userEvent.setup()", async () => {
    mockTransport.listAgents.mockResolvedValue({
      success: true,
      data: [makeAgent()],
    });
    const user = userEvent.setup();
    await mountPanel();
    await waitFor(() =>
      expect(screen.getByText(/^Researcher$/)).toBeInTheDocument(),
    );

    const search = screen.getByPlaceholderText(/search agents/i);
    await user.type(search, "z");
    await waitFor(() =>
      expect(screen.getByText(/no matching agents/i)).toBeInTheDocument(),
    );
  });
});
