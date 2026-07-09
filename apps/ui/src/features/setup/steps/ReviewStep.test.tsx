// ============================================================================
// ReviewStep — render and interaction tests
// ============================================================================

import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import type { Transport } from "@/services/transport";

const listAgents = vi.fn<Transport["listAgents"]>();
const updateAgent = vi.fn<Transport["updateAgent"]>();
const setDefaultProvider = vi.fn<Transport["setDefaultProvider"]>();
const createMcp = vi.fn<Transport["createMcp"]>();
const createMemory = vi.fn<Transport["createMemory"]>();
const getExecutionSettings = vi.fn<Transport["getExecutionSettings"]>();
const updateExecutionSettings = vi.fn<Transport["updateExecutionSettings"]>();

vi.mock("@/services/transport", () => ({
  getTransport: async () => ({
    listAgents,
    updateAgent,
    setDefaultProvider,
    createMcp,
    createMemory,
    getExecutionSettings,
    updateExecutionSettings,
  }),
}));

import { ReviewStep } from "./ReviewStep";

const defaultProps = {
  agentName: "z-Bot",
  aboutMe: "",
  providers: [
    {
      id: "anthropic",
      name: "Anthropic",
      models: ["claude-3-sonnet"],
      defaultModel: "claude-3-sonnet",
      enabled: true,
      isDefault: true,
    },
  ],
  defaultProviderId: "anthropic",
  enabledSkillIds: ["skill-1"],
  mcpConfigs: [],
  globalDefault: {
    providerId: "anthropic",
    model: "claude-3-sonnet",
    temperature: 0.7,
    maxInputTokens: 200000,
    maxOutputTokens: 32000,
    maxTokens: 32000,
  },
  agentOverrides: {},
  originalAgentName: "z-Bot",
  originalAgentConfigs: {},
  originalMcpIds: [],
  onLaunchComplete: vi.fn(),
};

describe("ReviewStep", () => {
  beforeEach(() => {
    listAgents.mockReset();
    updateAgent.mockReset();
    setDefaultProvider.mockReset();
    createMcp.mockReset();
    createMemory.mockReset();
    getExecutionSettings.mockReset();
    updateExecutionSettings.mockReset();
    defaultProps.onLaunchComplete = vi.fn();
  });

  it("renders Launch button", () => {
    render(<ReviewStep {...defaultProps} />);
    expect(screen.getByRole("button", { name: /launch/i })).toBeInTheDocument();
  });

  it("renders agent identity section with agent name", () => {
    render(<ReviewStep {...defaultProps} />);
    // The identity section header
    expect(screen.getByText("Agent Identity")).toBeInTheDocument();
    // The name value (appears in the body since identity section is open by default)
    expect(screen.getAllByText("z-Bot").length).toBeGreaterThan(0);
  });

  it("renders providers section", () => {
    render(<ReviewStep {...defaultProps} />);
    expect(screen.getByText("Providers")).toBeInTheDocument();
  });

  it("renders skills section with count", () => {
    render(<ReviewStep {...defaultProps} />);
    expect(screen.getByText("Skills")).toBeInTheDocument();
    expect(screen.getByText("1 enabled")).toBeInTheDocument();
  });

  it("renders 'No skills selected' when enabledSkillIds is empty — section opens on click", () => {
    render(<ReviewStep {...defaultProps} enabledSkillIds={[]} />);
    // Skills section starts closed — open it
    fireEvent.click(screen.getByText("Skills").closest("[role='button']")!);
    expect(screen.getByText("No skills selected")).toBeInTheDocument();
  });

  it("collapses and expands a section on click", () => {
    render(<ReviewStep {...defaultProps} />);
    // Identity section is initially OPEN — click to close
    const identityHeader = screen.getByText("Agent Identity").closest("[role='button']")!;
    fireEvent.click(identityHeader);
    // Name content should be gone now
    expect(screen.queryByText("Name")).toBeNull();

    // Click to expand again
    fireEvent.click(identityHeader);
    expect(screen.getByText("Name")).toBeInTheDocument();
  });

  it("renders aboutMe section content when provided", () => {
    render(<ReviewStep {...defaultProps} aboutMe="I am a developer" />);
    expect(screen.getByText("I am a developer")).toBeInTheDocument();
  });

  it("shows error message on launch failure", async () => {
    listAgents.mockRejectedValue(new Error("Failed to load agents"));
    render(<ReviewStep {...defaultProps} />);
    fireEvent.click(screen.getByRole("button", { name: /launch/i }));
    await waitFor(() => {
      expect(screen.getByText(/failed to load agents/i)).toBeInTheDocument();
    });
  });

  it("calls onLaunchComplete after successful launch", async () => {
    const onLaunchComplete = vi.fn();
    listAgents.mockResolvedValue({
      success: true,
      data: [{ id: "root", name: "root", displayName: "z-Bot", providerId: "anthropic", model: "claude-3-sonnet", temperature: 0.7, maxInputTokens: 200000, maxOutputTokens: 32000, maxTokens: 32000, thinkingEnabled: false, voiceRecordingEnabled: false, instructions: "", mcps: [], skills: [] }],
    });
    setDefaultProvider.mockResolvedValue({ success: true, data: {} as never });
    updateAgent.mockResolvedValue({ success: true, data: {} as never });
    getExecutionSettings.mockResolvedValue({ success: true, data: { maxParallelAgents: 2, setupComplete: false } as never });
    updateExecutionSettings.mockResolvedValue({ success: true, data: {} as never });

    render(<ReviewStep {...defaultProps} onLaunchComplete={onLaunchComplete} />);
    fireEvent.click(screen.getByRole("button", { name: /launch/i }));
    await waitFor(() => {
      expect(onLaunchComplete).toHaveBeenCalled();
    });
  });

  it("shows '...' text while launching", async () => {
    // Never resolve so the loading state persists
    listAgents.mockReturnValue(new Promise(() => {}));
    render(<ReviewStep {...defaultProps} />);
    fireEvent.click(screen.getByRole("button", { name: /launch/i }));
    expect(screen.getByText(/launching/i)).toBeInTheDocument();
  });

  it("handles section toggle via keyboard Enter", () => {
    render(<ReviewStep {...defaultProps} />);
    // Agent Identity section is open by default — close it with Enter
    const identityHeader = screen.getByText("Agent Identity").closest("[role='button']")!;
    fireEvent.keyDown(identityHeader, { key: "Enter" });
    expect(screen.queryByText("Name")).toBeNull();
  });

  it("renders override info in Agent Config section when overrides present", () => {
    const props = {
      ...defaultProps,
      agentOverrides: {
        "specialist-1": {
          providerId: "anthropic",
          model: "claude-3-opus",
        },
      },
    };
    render(<ReviewStep {...props} />);
    expect(screen.getByText("1 customized")).toBeInTheDocument();
  });

  it("renders 'No MCP servers enabled' when mcpConfigs is empty — section opens on click", () => {
    render(<ReviewStep {...defaultProps} />);
    // MCP Servers section starts closed — open it
    fireEvent.click(screen.getByText("MCP Servers").closest("[role='button']")!);
    expect(screen.getByText("No MCP servers enabled")).toBeInTheDocument();
  });

  it("renders enabled MCP servers when section is opened", () => {
    const props = {
      ...defaultProps,
      mcpConfigs: [
        {
          id: "mcp-1",
          name: "Browser MCP",
          type: "stdio" as const,
          enabled: true,
          description: "Browser automation",
          command: "npx",
          args: [],
        },
      ],
    };
    render(<ReviewStep {...props} />);
    // MCP Servers section starts closed — open it
    fireEvent.click(screen.getByText("MCP Servers").closest("[role='button']")!);
    expect(screen.getByText("Browser MCP")).toBeInTheDocument();
  });
});
