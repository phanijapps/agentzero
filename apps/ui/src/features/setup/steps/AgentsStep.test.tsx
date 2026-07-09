// ============================================================================
// AgentsStep — render and interaction tests
// ============================================================================

import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import type { Transport } from "@/services/transport";

const listAgents = vi.fn<Transport["listAgents"]>();
const listProviders = vi.fn<Transport["listProviders"]>();

vi.mock("@/services/transport", () => ({
  getTransport: async () => ({ listAgents, listProviders }),
}));

import { AgentsStep } from "./AgentsStep";

function makeProvider(id: string, name: string, models: string[]) {
  return { id, name, models, defaultModel: models[0], enabled: true, isDefault: false };
}

function makeAgent(id: string, name: string) {
  return {
    id,
    name,
    displayName: name,
    providerId: "anthropic",
    model: "claude-3-sonnet",
    temperature: 0.7,
    maxInputTokens: 200000,
    maxOutputTokens: 32000,
    maxTokens: 32000,
    thinkingEnabled: false,
    voiceRecordingEnabled: false,
    instructions: "",
    mcps: [],
    skills: [],
  };
}

const globalDefault = {
  providerId: "anthropic",
  model: "claude-3-sonnet",
  temperature: 0.7,
  maxInputTokens: 200000,
  maxOutputTokens: 32000,
  maxTokens: 32000,
};

const defaultProps = {
  providers: [makeProvider("anthropic", "Anthropic", ["claude-3-sonnet", "claude-3-opus"])],
  defaultProviderId: "anthropic",
  agentName: "z-Bot",
  globalDefault,
  agentOverrides: {},
  onGlobalChange: vi.fn(),
  onOverrideChange: vi.fn(),
};

describe("AgentsStep", () => {
  beforeEach(() => {
    listAgents.mockReset();
    listProviders.mockReset();
    defaultProps.onGlobalChange = vi.fn();
    defaultProps.onOverrideChange = vi.fn();

    listAgents.mockResolvedValue({
      success: true,
      data: [makeAgent("root", "root")],
    });
    listProviders.mockResolvedValue({
      success: true,
      data: [makeProvider("anthropic", "Anthropic", ["claude-3-sonnet", "claude-3-opus"])],
    });
  });

  it("shows loading spinner initially", () => {
    listAgents.mockReturnValue(new Promise(() => {}));
    listProviders.mockReturnValue(new Promise(() => {}));
    const { container } = render(<AgentsStep {...defaultProps} />);
    expect(container.querySelector(".settings-loading")).toBeInTheDocument();
  });

  it("renders global default card after loading", async () => {
    render(<AgentsStep {...defaultProps} />);
    await waitFor(() => {
      expect(screen.getByText("Default for all")).toBeInTheDocument();
    });
  });

  it("renders provider select with options", async () => {
    render(<AgentsStep {...defaultProps} />);
    await waitFor(() => {
      expect(screen.getByLabelText("Provider")).toBeInTheDocument();
    });
    expect(screen.getByRole("option", { name: "Anthropic" })).toBeInTheDocument();
  });

  it("renders model select", async () => {
    render(<AgentsStep {...defaultProps} />);
    await waitFor(() => {
      expect(screen.getByLabelText("Model")).toBeInTheDocument();
    });
  });

  it("renders temperature and token limit inputs", async () => {
    render(<AgentsStep {...defaultProps} />);
    await waitFor(() => {
      expect(screen.getByLabelText("Temperature")).toBeInTheDocument();
      expect(screen.getByLabelText("Max Input Tokens")).toBeInTheDocument();
      expect(screen.getByLabelText("Max Output Tokens")).toBeInTheDocument();
    });
  });

  it("renders agent list with root agent", async () => {
    render(<AgentsStep {...defaultProps} />);
    await waitFor(() => {
      expect(screen.getByText("Agents")).toBeInTheDocument();
    });
    // The root agent shows the agentName (z-Bot) instead of the raw name
    expect(screen.getAllByText("z-Bot").length).toBeGreaterThan(0);
  });

  it("calls onGlobalChange when provider changes", async () => {
    const onGlobalChange = vi.fn();
    render(<AgentsStep {...defaultProps} onGlobalChange={onGlobalChange} />);
    await waitFor(() => screen.getByLabelText("Provider"));
    // Clear calls from initialization
    onGlobalChange.mockClear();
    const select = screen.getByLabelText("Provider");
    fireEvent.change(select, { target: { value: "anthropic" } });
    expect(onGlobalChange).toHaveBeenCalled();
  });

  it("calls onGlobalChange when model changes", async () => {
    const onGlobalChange = vi.fn();
    render(<AgentsStep {...defaultProps} onGlobalChange={onGlobalChange} />);
    await waitFor(() => screen.getByLabelText("Model"));
    onGlobalChange.mockClear();
    const select = screen.getByLabelText("Model");
    fireEvent.change(select, { target: { value: "claude-3-opus" } });
    expect(onGlobalChange).toHaveBeenCalled();
  });

  it("calls onGlobalChange when temperature changes", async () => {
    const onGlobalChange = vi.fn();
    render(<AgentsStep {...defaultProps} onGlobalChange={onGlobalChange} />);
    await waitFor(() => screen.getByLabelText("Temperature"));
    onGlobalChange.mockClear();
    const input = screen.getByLabelText("Temperature");
    fireEvent.change(input, { target: { value: "0.5" } });
    expect(onGlobalChange).toHaveBeenCalled();
  });

  it("calls onGlobalChange when maxOutputTokens changes", async () => {
    const onGlobalChange = vi.fn();
    render(<AgentsStep {...defaultProps} onGlobalChange={onGlobalChange} />);
    await waitFor(() => screen.getByLabelText("Max Output Tokens"));
    onGlobalChange.mockClear();
    const input = screen.getByLabelText("Max Output Tokens");
    fireEvent.change(input, { target: { value: "8192" } });
    expect(onGlobalChange).toHaveBeenCalled();
  });

  it("expands agent override fields when Customize is clicked", async () => {
    render(<AgentsStep {...defaultProps} />);
    await waitFor(() => screen.getByText("Customize"));
    fireEvent.click(screen.getByText("Customize"));
    // After expanding, provider/model selects appear in the expanded row
    await waitFor(() => {
      expect(screen.getByText("Reset to default")).toBeInTheDocument();
    });
  });

  it("resets override when 'Reset to default' is clicked", async () => {
    const onOverrideChange = vi.fn();
    render(
      <AgentsStep
        {...defaultProps}
        agentOverrides={{ root: { model: "claude-3-opus" } }}
        onOverrideChange={onOverrideChange}
      />
    );
    // First expand the agent row, then click Reset to default
    await waitFor(() => screen.getByText("Customize"));
    fireEvent.click(screen.getByText("Customize"));
    await waitFor(() => screen.getByText("Reset to default"));
    fireEvent.click(screen.getByText("Reset to default"));
    expect(onOverrideChange).toHaveBeenCalledWith({});
  });

  it("shows no-providers message when providers list is empty", async () => {
    listProviders.mockResolvedValue({ success: true, data: [] });
    render(<AgentsStep {...defaultProps} providers={[]} />);
    await waitFor(() => {
      expect(screen.getByText("No providers configured")).toBeInTheDocument();
    });
  });
});
