// ============================================================================
// MemoryTab (Command Deck) — delete handler wires to transport.deleteMemory
// ============================================================================

import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@/test/utils";
import { MemoryTab } from "./MemoryTab";
import type { WardContent } from "@/services/transport/types";

// ---------------------------------------------------------------------------
// Mock transport — returns one fact in the active ward so a delete button
// renders. We capture deleteMemory + getWardContent calls for assertions.
// ---------------------------------------------------------------------------

const mockListWards = vi.fn();
const mockGetWardContent = vi.fn();
const mockDeleteMemory = vi.fn();
const mockHybridSearch = vi.fn();

vi.mock("@/services/transport", async () => {
  const actual = await vi.importActual<Record<string, unknown>>("@/services/transport");
  return {
    ...actual,
    getTransport: async () => ({
      listWards: mockListWards,
      getWardContent: mockGetWardContent,
      deleteMemory: mockDeleteMemory,
      hybridSearch: mockHybridSearch,
      createMemory: vi.fn(),
    }),
  };
});

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

function emptyContent(wardId: string): WardContent {
  return {
    ward_id: wardId,
    summary: { description: "Test ward" } as WardContent["summary"],
    counts: { facts: 1, wiki: 0, procedures: 0, episodes: 0 },
    facts: [
      {
        id: "fact-1",
        content: "User prefers JWT.",
        category: "preference",
        confidence: 0.92,
        created_at: "2026-04-20T12:00:00Z",
        age_bucket: "today",
        ward_id: wardId,
      },
    ],
    wiki: [],
    procedures: [],
    episodes: [],
  } as unknown as WardContent;
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

describe("MemoryTab — delete fact wiring", () => {
  let confirmSpy: ReturnType<typeof vi.spyOn>;

  beforeEach(() => {
    vi.clearAllMocks();
    confirmSpy = vi.spyOn(window, "confirm").mockReturnValue(true) as unknown as ReturnType<typeof vi.spyOn>;
    mockListWards.mockResolvedValue({
      success: true,
      data: [{ id: "auth-system", count: 1 }],
    });
    mockGetWardContent.mockResolvedValue({
      success: true,
      data: emptyContent("auth-system"),
    });
    mockDeleteMemory.mockResolvedValue({ success: true });
    mockHybridSearch.mockResolvedValue({
      success: true,
      data: {
        facts: { hits: [], latency_ms: 1 },
        wiki: { hits: [], latency_ms: 1 },
        procedures: { hits: [], latency_ms: 1 },
        episodes: { hits: [], latency_ms: 1 },
      },
    });
  });

  afterEach(() => {
    confirmSpy.mockRestore();
  });

  it("renders the labelled scope, evidence, and curation workbench", async () => {
    render(<MemoryTab agentId="root" />);

    expect(
      screen.getByRole("region", { name: "Memory command deck" })
    ).toBeInTheDocument();
    expect(screen.getByRole("navigation", { name: "Memory scope" })).toBeInTheDocument();
    expect(screen.getByRole("region", { name: "Memory evidence" })).toBeInTheDocument();
    expect(screen.getByRole("complementary", { name: "Memory curation" })).toBeInTheDocument();
    expect(
      screen.getByRole("heading", { name: "Memory", level: 1 })
    ).toBeInTheDocument();
  });

  it("renders a delete button on each fact row in the Facts tab", async () => {
    render(<MemoryTab agentId="root" />);
    await waitFor(() => {
      expect(
        screen.getByRole("button", { name: /delete preference memory/i })
      ).toBeInTheDocument();
    });
  });

  it("calls transport.deleteMemory(agentId, factId) when delete is clicked + confirmed", async () => {
    render(<MemoryTab agentId="root" />);
    const btn = await screen.findByRole("button", { name: /delete preference memory/i });
    fireEvent.click(btn);
    await waitFor(() => {
      expect(mockDeleteMemory).toHaveBeenCalledWith("root", "fact-1");
    });
  });

  it("refreshes ward content after a successful delete", async () => {
    render(<MemoryTab agentId="root" />);
    const btn = await screen.findByRole("button", { name: /delete preference memory/i });
    // First call happens on initial load.
    await waitFor(() => expect(mockGetWardContent).toHaveBeenCalledTimes(1));
    fireEvent.click(btn);
    await waitFor(() => expect(mockGetWardContent).toHaveBeenCalledTimes(2));
  });

  it("skips transport.deleteMemory when the user cancels the confirm", async () => {
    confirmSpy.mockReturnValue(false);
    render(<MemoryTab agentId="root" />);
    const btn = await screen.findByRole("button", { name: /delete preference memory/i });
    fireEvent.click(btn);
    expect(mockDeleteMemory).not.toHaveBeenCalled();
  });
});
