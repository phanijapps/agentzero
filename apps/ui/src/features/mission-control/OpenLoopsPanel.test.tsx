import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@/test/utils";

const listAutonomyItems = vi.fn();
const transitionAutonomyItem = vi.fn();
const getAutonomyItem = vi.fn();
const resumeAutonomyItem = vi.fn();

vi.mock("@/services/transport", () => ({
  getTransport: () => Promise.resolve({ listAutonomyItems, transitionAutonomyItem, getAutonomyItem, resumeAutonomyItem }),
}));

import { OpenLoopsPanel } from "./OpenLoopsPanel";

beforeEach(() => {
  listAutonomyItems.mockResolvedValue({
    success: true,
    data: [{
      id: "aut-1", title: "Compare engines", objective: "Choose an engine",
      next_action: "Review migration evidence", state: "proposed", approval_policy: "manual",
      source_session_id: "sess-source", dedupe_key: "engines", created_at: "2026-07-09T00:00:00Z", updated_at: "2026-07-09T00:00:00Z",
    }],
  });
  transitionAutonomyItem.mockResolvedValue({
    success: true,
    data: {
      id: "aut-1", title: "Compare engines", objective: "Choose an engine",
      next_action: "Review migration evidence", state: "approved", approval_policy: "manual",
      source_session_id: "sess-source", dedupe_key: "engines", created_at: "2026-07-09T00:00:00Z", updated_at: "2026-07-09T00:00:00Z",
    },
  });
  getAutonomyItem.mockResolvedValue({
    success: true,
    data: {
      id: "aut-1", title: "Compare engines", objective: "Choose an engine",
      next_action: "Review migration evidence", state: "approved", approval_policy: "manual",
      source_session_id: "sess-source", dedupe_key: "engines", created_at: "2026-07-09T00:00:00Z", updated_at: "2026-07-09T00:00:00Z",
      evidence: [{ id: "ae-1", item_id: "aut-1", kind: "session", reference_id: "sess-source", label: "Source", created_at: "2026-07-09T00:00:00Z" }],
    },
  });
  resumeAutonomyItem.mockResolvedValue({ success: true, data: { item_id: "aut-1", session_id: "sess-new" } });
});

describe("OpenLoopsPanel", () => {
  it("shows a durable decision thread and applies only an explicit transition", async () => {
    render(<OpenLoopsPanel />);
    expect(screen.getByRole("region", { name: "Decision threads" })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Decision threads" })).toBeInTheDocument();
    expect(await screen.findByText("Compare engines")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Approve" }));
    await waitFor(() => expect(transitionAutonomyItem).toHaveBeenCalledWith("aut-1", "approved"));
    expect(screen.getByText("approved")).toBeInTheDocument();
  });

  it("inspects reference-only evidence and resumes only the approved selected thread", async () => {
    listAutonomyItems.mockResolvedValueOnce({
      success: true,
      data: [{
        id: "aut-1", title: "Compare engines", objective: "Choose an engine",
        next_action: "Review migration evidence", state: "approved", approval_policy: "manual",
        source_session_id: "sess-source", dedupe_key: "engines", created_at: "2026-07-09T00:00:00Z", updated_at: "2026-07-09T00:00:00Z",
      }],
    });
    render(<OpenLoopsPanel />);
    await screen.findByText("Compare engines");
    fireEvent.click(screen.getByRole("button", { name: "Inspect" }));
    await waitFor(() => expect(getAutonomyItem).toHaveBeenCalledWith("aut-1"));
    expect(screen.getByText("Source session")).toBeInTheDocument();
    expect(screen.getByText("sess-source")).toBeInTheDocument();
    expect(screen.getByText("Evidence references (1)")).toBeInTheDocument();
    expect(screen.getByText("session: sess-source")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Resume" }));
    await waitFor(() => expect(resumeAutonomyItem).toHaveBeenCalledWith("aut-1"));
    expect(screen.getByText("Started new session sess-new.")).toBeInTheDocument();
  });

  it("keeps the source session visible when a thread has no linked evidence", async () => {
    getAutonomyItem.mockResolvedValueOnce({
      success: true,
      data: {
        id: "aut-1", title: "Compare engines", objective: "Choose an engine",
        next_action: "Review migration evidence", state: "proposed", approval_policy: "manual",
        source_session_id: "sess-source", dedupe_key: "engines", created_at: "2026-07-09T00:00:00Z", updated_at: "2026-07-09T00:00:00Z",
        evidence: [],
      },
    });
    render(<OpenLoopsPanel />);
    await screen.findByText("Compare engines");
    fireEvent.click(screen.getByRole("button", { name: "Inspect" }));
    await waitFor(() => expect(screen.getByText("No linked evidence.")).toBeInTheDocument());
    expect(screen.getByText("sess-source")).toBeInTheDocument();
  });
});
