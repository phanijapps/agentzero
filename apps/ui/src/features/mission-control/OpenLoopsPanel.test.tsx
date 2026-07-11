import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@/test/utils";

const listAutonomyItems = vi.fn();
const transitionAutonomyItem = vi.fn();

vi.mock("@/services/transport", () => ({
  getTransport: () => Promise.resolve({ listAutonomyItems, transitionAutonomyItem }),
}));

import { OpenLoopsPanel } from "./OpenLoopsPanel";

beforeEach(() => {
  listAutonomyItems.mockResolvedValue({
    success: true,
    data: [{
      id: "aut-1", title: "Compare engines", objective: "Choose an engine",
      next_action: "Review migration evidence", state: "proposed", approval_policy: "manual",
      dedupe_key: "engines", created_at: "2026-07-09T00:00:00Z", updated_at: "2026-07-09T00:00:00Z",
    }],
  });
  transitionAutonomyItem.mockResolvedValue({
    success: true,
    data: {
      id: "aut-1", title: "Compare engines", objective: "Choose an engine",
      next_action: "Review migration evidence", state: "approved", approval_policy: "manual",
      dedupe_key: "engines", created_at: "2026-07-09T00:00:00Z", updated_at: "2026-07-09T00:00:00Z",
    },
  });
});

describe("OpenLoopsPanel", () => {
  it("shows a durable open loop and applies only an explicit transition", async () => {
    render(<OpenLoopsPanel />);
    expect(await screen.findByText("Compare engines")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Approve" }));
    await waitFor(() => expect(transitionAutonomyItem).toHaveBeenCalledWith("aut-1", "approved"));
    expect(screen.getByText("approved")).toBeInTheDocument();
  });
});
