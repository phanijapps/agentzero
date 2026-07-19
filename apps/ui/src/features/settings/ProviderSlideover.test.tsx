// ============================================================================
// ProviderSlideover — open/close, view mode, edit toggle, delete + test calls
// ============================================================================

import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@/test/utils";
import { ProviderSlideover } from "./ProviderSlideover";
import type { ProviderResponse } from "@/services/transport";

// ---------------------------------------------------------------------------
// Mocks
// ---------------------------------------------------------------------------

const mockTestProviderById = vi.fn();
const mockTestProvider = vi.fn();
const mockUpdateProvider = vi.fn();
const mockDeleteProvider = vi.fn();
const mockCreateProvider = vi.fn();

vi.mock("@/services/transport", async () => {
  const actual = await vi.importActual<Record<string, unknown>>("@/services/transport");
  return {
    ...actual,
    getTransport: async () => ({
      testProviderById: mockTestProviderById,
      testProvider: mockTestProvider,
      updateProvider: mockUpdateProvider,
      deleteProvider: mockDeleteProvider,
      createProvider: mockCreateProvider,
    }),
  };
});

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

function makeProvider(): ProviderResponse {
  return {
    id: "openai-1",
    name: "OpenAI",
    description: "OpenAI API",
    hasApiKey: true,
    baseUrl: "https://api.openai.com/v1",
    models: ["gpt-4o", "gpt-4o-mini"],
    defaultModel: "gpt-4o",
    verified: true,
    rateLimits: { requestsPerMinute: 60, concurrentRequests: 4 },
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  mockTestProviderById.mockResolvedValue({
    success: true,
    data: { success: true, message: "ok" },
  });
  mockTestProvider.mockResolvedValue({
    success: true,
    data: { success: true, message: "ok" },
  });
  mockUpdateProvider.mockResolvedValue({ success: true });
  mockDeleteProvider.mockResolvedValue({ success: true });
  mockCreateProvider.mockResolvedValue({ success: true, data: { id: "new-id" } });
});

afterEach(() => {
  // jsdom keeps body around between tests; clear listeners by unmounting via render's cleanup.
});

describe("ProviderSlideover — view mode", () => {
  it("renders the provider name in the header", () => {
    render(
      <ProviderSlideover
        provider={makeProvider()}
        modelRegistry={{}}
        isActive={false}
        isOpen={true}
        mode="view"
        onClose={() => {}}
        onSaved={() => {}}
        onDeleted={() => {}}
        onSetActive={() => {}}
      />
    );
    expect(screen.getByRole("heading", { name: "OpenAI" })).toBeInTheDocument();
  });

  it("shows the Edit button in view mode (not editing)", () => {
    render(
      <ProviderSlideover
        provider={makeProvider()}
        modelRegistry={{}}
        isActive={false}
        isOpen={true}
        mode="view"
        onClose={() => {}}
        onSaved={() => {}}
        onDeleted={() => {}}
        onSetActive={() => {}}
      />
    );
    expect(screen.getByRole("button", { name: /edit/i })).toBeInTheDocument();
  });

  it("calls onClose when the close (X) button is clicked", () => {
    const onClose = vi.fn();
    render(
      <ProviderSlideover
        provider={makeProvider()}
        modelRegistry={{}}
        isActive={false}
        isOpen={true}
        mode="view"
        onClose={onClose}
        onSaved={() => {}}
        onDeleted={() => {}}
        onSetActive={() => {}}
      />
    );
    fireEvent.click(screen.getByRole("button", { name: /^close$/i }));
    expect(onClose).toHaveBeenCalled();
  });

  it("calls testProviderById when Test is clicked in view mode", async () => {
    render(
      <ProviderSlideover
        provider={makeProvider()}
        modelRegistry={{}}
        isActive={false}
        isOpen={true}
        mode="view"
        onClose={() => {}}
        onSaved={() => {}}
        onDeleted={() => {}}
        onSetActive={() => {}}
      />
    );
    fireEvent.click(screen.getByRole("button", { name: /^test$/i }));
    await waitFor(() => expect(mockTestProviderById).toHaveBeenCalledWith("openai-1"));
  });

  it("shows the Connected badge when verified=true", () => {
    render(
      <ProviderSlideover
        provider={makeProvider()}
        modelRegistry={{}}
        isActive={false}
        isOpen={true}
        mode="view"
        onClose={() => {}}
        onSaved={() => {}}
        onDeleted={() => {}}
        onSetActive={() => {}}
      />
    );
    expect(screen.getByText(/connected/i)).toBeInTheDocument();
  });

  it("shows the Active badge when isActive=true", () => {
    render(
      <ProviderSlideover
        provider={makeProvider()}
        modelRegistry={{}}
        isActive={true}
        isOpen={true}
        mode="view"
        onClose={() => {}}
        onSaved={() => {}}
        onDeleted={() => {}}
        onSetActive={() => {}}
      />
    );
    expect(screen.getByText(/^active$/i)).toBeInTheDocument();
  });

  it("clicking Edit reveals Cancel + Save buttons", () => {
    render(
      <ProviderSlideover
        provider={makeProvider()}
        modelRegistry={{}}
        isActive={false}
        isOpen={true}
        mode="view"
        onClose={() => {}}
        onSaved={() => {}}
        onDeleted={() => {}}
        onSetActive={() => {}}
      />
    );
    fireEvent.click(screen.getByRole("button", { name: /edit/i }));
    expect(screen.getByRole("button", { name: /cancel/i })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /^save$/i })).toBeInTheDocument();
  });
});

describe("ProviderSlideover — create mode", () => {
  it("renders the Add Provider header in create mode", () => {
    render(
      <ProviderSlideover
        provider={null}
        modelRegistry={{}}
        isActive={false}
        isOpen={true}
        mode="create"
        preset={null}
        onClose={() => {}}
        onSaved={() => {}}
        onDeleted={() => {}}
        onSetActive={() => {}}
      />
    );
    expect(screen.getByRole("heading", { name: /add provider/i })).toBeInTheDocument();
  });

  it("starts in editing state in create mode (no Edit button)", () => {
    render(
      <ProviderSlideover
        provider={null}
        modelRegistry={{}}
        isActive={false}
        isOpen={true}
        mode="create"
        preset={null}
        onClose={() => {}}
        onSaved={() => {}}
        onDeleted={() => {}}
        onSetActive={() => {}}
      />
    );
    expect(screen.queryByRole("button", { name: /^edit$/i })).not.toBeInTheDocument();
  });

  it("pre-fills the form (header name + base URL input) from a preset", () => {
    render(
      <ProviderSlideover
        provider={null}
        modelRegistry={{}}
        isActive={false}
        isOpen={true}
        mode="create"
        preset={{
          name: "OpenAI",
          baseUrl: "https://api.openai.com/v1",
          models: "gpt-4o, gpt-4o-mini",
          apiKeyHint: "platform.openai.com",
          apiKeyPlaceholder: "sk-...",
        }}
        onClose={() => {}}
        onSaved={() => {}}
        onDeleted={() => {}}
        onSetActive={() => {}}
      />
    );
    // Heading stays "Add Provider" in create mode (form.name is shown elsewhere).
    expect(screen.getByRole("heading", { name: /add provider/i })).toBeInTheDocument();
    // Base URL is an editable input in create mode and is pre-filled from the preset.
    expect(screen.getByDisplayValue("https://api.openai.com/v1")).toBeInTheDocument();
  });
});
