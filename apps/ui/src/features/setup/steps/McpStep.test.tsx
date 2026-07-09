// ============================================================================
// McpStep — render and interaction tests
// ============================================================================

import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import type { Transport } from "@/services/transport";
import type { McpServerConfig } from "@/services/transport/types";

const getMcpDefaults = vi.fn<Transport["getMcpDefaults"]>();

vi.mock("@/services/transport", () => ({
  getTransport: async () => ({ getMcpDefaults }),
}));

import { McpStep } from "./McpStep";

function makeServer(overrides: Partial<McpServerConfig> = {}): McpServerConfig {
  return {
    id: "mcp-1",
    name: "Test MCP",
    description: "A test MCP server",
    type: "stdio",
    command: "npx",
    args: ["-y", "mcp-server"],
    enabled: true,
    env: {},
    ...overrides,
  } as McpServerConfig;
}

describe("McpStep", () => {
  beforeEach(() => {
    getMcpDefaults.mockReset();
  });

  it("shows loading spinner initially", () => {
    getMcpDefaults.mockReturnValue(new Promise(() => { /* never resolves */ }));
    const { container } = render(<McpStep mcpConfigs={[]} onChange={vi.fn()} />);
    expect(container.querySelector(".settings-loading")).toBeInTheDocument();
  });

  it("shows empty hint when no MCP configs", async () => {
    getMcpDefaults.mockResolvedValue({ success: true, data: [] });
    render(<McpStep mcpConfigs={[]} onChange={vi.fn()} />);
    await waitFor(() => {
      expect(screen.getByText(/no mcp server templates/i)).toBeInTheDocument();
    });
  });

  it("renders keyless servers in 'Ready to use' section", async () => {
    getMcpDefaults.mockResolvedValue({ success: true, data: [] });
    const servers = [makeServer({ id: "s1", name: "Ready Server" })];
    render(<McpStep mcpConfigs={servers} onChange={vi.fn()} />);
    await waitFor(() => {
      expect(screen.getByText("Ready to use")).toBeInTheDocument();
      expect(screen.getByText("Ready Server")).toBeInTheDocument();
    });
  });

  it("renders servers needing API key in 'Requires API key' section", async () => {
    getMcpDefaults.mockResolvedValue({ success: true, data: [] });
    const servers = [
      makeServer({ id: "s2", name: "Key Server", env: { API_KEY: "" } }),
    ];
    render(<McpStep mcpConfigs={servers} onChange={vi.fn()} />);
    await waitFor(() => {
      expect(screen.getByText("Requires API key")).toBeInTheDocument();
      expect(screen.getByText("Key Server")).toBeInTheDocument();
    });
  });

  it("loads defaults from transport when mcpConfigs is empty", async () => {
    const onChange = vi.fn();
    const defaultServer = makeServer({
      id: "default-1",
      name: "Default MCP",
      enabled: false,
    });
    getMcpDefaults.mockResolvedValue({ success: true, data: [defaultServer] });
    render(<McpStep mcpConfigs={[]} onChange={onChange} />);
    await waitFor(() => {
      expect(onChange).toHaveBeenCalledWith([
        expect.objectContaining({ id: "default-1", enabled: false }),
      ]);
    });
  });

  it("does not overwrite existing configs when mcpConfigs is non-empty", async () => {
    const onChange = vi.fn();
    const existingServer = makeServer({ id: "existing-1", name: "Existing" });
    getMcpDefaults.mockResolvedValue({ success: true, data: [makeServer()] });
    render(<McpStep mcpConfigs={[existingServer]} onChange={onChange} />);
    await waitFor(() => {
      expect(screen.getByText("Existing")).toBeInTheDocument();
    });
    // onChange should not be called by the default loader (configs already populated)
    expect(onChange).not.toHaveBeenCalled();
  });

  it("toggles server enabled state when toggle clicked", async () => {
    const onChange = vi.fn();
    getMcpDefaults.mockResolvedValue({ success: true, data: [] });
    const servers = [makeServer({ id: "s1", name: "Server One", enabled: true })];
    render(<McpStep mcpConfigs={servers} onChange={onChange} />);
    await waitFor(() => screen.getByText("Server One"));
    const toggle = screen.getByRole("button", { hidden: true });
    fireEvent.click(toggle);
    expect(onChange).toHaveBeenCalledWith([
      expect.objectContaining({ id: "s1", enabled: false }),
    ]);
  });

  it("handles Enter keydown on toggle", async () => {
    const onChange = vi.fn();
    getMcpDefaults.mockResolvedValue({ success: true, data: [] });
    const servers = [makeServer({ id: "s1", name: "Server One", enabled: true })];
    render(<McpStep mcpConfigs={servers} onChange={onChange} />);
    await waitFor(() => screen.getByText("Server One"));
    const toggle = screen.getByRole("button", { hidden: true });
    fireEvent.keyDown(toggle, { key: "Enter" });
    expect(onChange).toHaveBeenCalled();
  });
});
