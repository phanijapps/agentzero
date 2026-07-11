import { describe, it, expect, vi } from "vitest";
import { render, screen, waitFor, fireEvent } from "@testing-library/react";
import { MemoryTab } from "../MemoryTab";

vi.mock("@/services/transport", () => ({
  getTransport: async () => ({
    listWards: async () => ({ success: true, data: [{ id: "wardA", count: 2 }] }),
    getWardContent: async () => ({
      success: true,
      data: {
        ward_id: "wardA",
        summary: { title: "wardA" },
        facts: [
          {
            id: "f1",
            content: "alpha",
            category: "instruction",
            confidence: 0.9,
            created_at: new Date().toISOString(),
            age_bucket: "today",
          },
        ],
        wiki: [],
        procedures: [],
        episodes: [],
        counts: { facts: 1, wiki: 0, procedures: 0, episodes: 0 },
      },
    }),
    searchMemoryHybrid: async () => ({
      success: true,
      data: {
        facts: { hits: [], latency_ms: 0 },
        wiki: { hits: [], latency_ms: 0 },
        procedures: { hits: [], latency_ms: 0 },
        episodes: { hits: [], latency_ms: 0 },
      },
    }),
    createMemory: vi.fn(async () => ({ success: true })),
  }),
}));

describe("Memory Tab e2e smoke", () => {
  it("write flow: add fact via right rail", async () => {
    render(<MemoryTab agentId="root" />);
    // Ward appears in rail
    await waitFor(() =>
      expect(screen.getAllByText(/wardA/).length).toBeGreaterThan(0),
    );
    // Open AddDrawer via "+ Fact"
    fireEvent.click(screen.getByRole("button", { name: /\+ fact/i }));
    // Fill content
    fireEvent.change(screen.getByRole("textbox", { name: /memory content/i }), {
      target: { value: "new fact" },
    });
    // Save
    fireEvent.click(screen.getByRole("button", { name: /^save$/i }));
    // Drawer closes
    await waitFor(() =>
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument(),
    );
  });
});
