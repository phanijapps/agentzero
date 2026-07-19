import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { WebIntegrationsPanel } from "./WebIntegrationsPanel";

const transport = {
  listMcps: vi.fn(),
  getMcp: vi.fn(),
  createMcp: vi.fn(),
  updateMcp: vi.fn(),
  deleteMcp: vi.fn(),
  testMcp: vi.fn(),
  getMcpOAuthStatus: vi.fn(),
  startMcpOAuth: vi.fn(),
  disconnectMcpOAuth: vi.fn(),
  listBridgeWorkers: vi.fn(),
  listPlugins: vi.fn(),
};

vi.mock("@/services/transport", async () => {
  const actual = await vi.importActual<typeof import("@/services/transport")>(
    "@/services/transport",
  );
  return {
    ...actual,
    getTransport: vi.fn(async () => transport),
  };
});

function renderPanel() {
  return render(
    <MemoryRouter>
      <WebIntegrationsPanel />
    </MemoryRouter>,
  );
}

describe("WebIntegrationsPanel OAuth MCP flow", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    transport.listMcps.mockResolvedValue({ success: true, data: { servers: [] } });
    transport.listBridgeWorkers.mockResolvedValue({ success: true, data: [] });
    transport.listPlugins.mockResolvedValue({ success: true, data: { plugins: [] } });
    transport.createMcp.mockResolvedValue({
      success: true,
      data: {
        type: "streamable-http",
        id: "robinhood-trading",
        name: "Robinhood Trading",
        description: "Trading MCP",
        url: "https://agent.robinhood.com/mcp/trading",
        auth: { type: "oauth2" },
        enabled: false,
      },
    });
    transport.getMcpOAuthStatus.mockResolvedValue({
      success: true,
      data: { status: "not_connected" },
    });
    transport.startMcpOAuth.mockResolvedValue({
      success: true,
      data: {
        authUrl: "https://robinhood.example/oauth/authorize",
        state: "state-1",
      },
    });
    vi.spyOn(window, "open").mockReturnValue({
      location: { href: "" },
      opener: null,
    } as Window);
  });

  it("offers authorization after adding an OAuth MCP server", async () => {
    transport.getMcpOAuthStatus
      .mockResolvedValueOnce({
        success: true,
        data: { status: "not_connected" },
      })
      .mockResolvedValueOnce({
        success: true,
        data: { status: "connected" },
      });

    renderPanel();

    await screen.findByText("No tool servers");
    fireEvent.click(screen.getAllByRole("button", { name: /add tool server/i })[0]);
    fireEvent.change(screen.getByLabelText("Type"), {
      target: { value: "streamable-http" },
    });
    fireEvent.change(screen.getByLabelText("Name"), {
      target: { value: "Robinhood Trading" },
    });
    fireEvent.change(screen.getByLabelText("Description"), {
      target: { value: "Trading MCP" },
    });
    fireEvent.change(screen.getByLabelText("URL"), {
      target: { value: "https://agent.robinhood.com/mcp/trading" },
    });
    fireEvent.click(screen.getByLabelText("OAuth 2.0"));
    expect(screen.queryByLabelText(/client id/i)).not.toBeInTheDocument();
    expect(screen.queryByLabelText(/client secret/i)).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /add server/i }));

    await waitFor(() => {
      expect(transport.createMcp).toHaveBeenCalledWith(
        expect.objectContaining({
          type: "streamable-http",
          auth: { type: "oauth2" },
        }),
      );
    });
    expect(transport.createMcp).toHaveBeenCalledWith(
      expect.not.objectContaining({
        auth: expect.objectContaining({ clientId: expect.any(String) }),
      }),
    );

    const authorize = await screen.findByRole("button", { name: /authorize/i });
    fireEvent.click(authorize);

    await waitFor(() => {
      expect(transport.startMcpOAuth).toHaveBeenCalledWith("robinhood-trading", {
        redirectUri: "http://localhost:3000/api/mcps/oauth/callback",
      });
    });
    expect(await screen.findByText(/authorization complete/i)).toBeInTheDocument();
  });
});
