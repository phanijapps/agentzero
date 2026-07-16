import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import type { Artifact } from "@/services/transport/types";

const getArtifactContentUrl = vi.fn();

vi.mock("@/services/transport", () => ({
  getTransport: async () => ({ getArtifactContentUrl }),
}));

import { ArtifactSlideOut } from "./ArtifactSlideOut";

const OFFICE_ARTIFACT: Artifact = {
  id: "art-office",
  sessionId: "sess-office",
  fileName: "report.docx",
  fileType: "docx",
  fileSize: 1024,
  createdAt: "2026-01-01T00:00:00Z",
};

describe("ArtifactSlideOut", () => {
  beforeEach(() => {
    getArtifactContentUrl.mockReset();
    getArtifactContentUrl.mockReturnValue("/api/artifacts/art-office/content?session_id=sess-office");
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue({ ok: true, status: 200 }));
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("keeps Office artifacts download-only instead of decompressing them in the browser", async () => {
    render(<ArtifactSlideOut artifact={OFFICE_ARTIFACT} onClose={vi.fn()} />);

    await screen.findByText(/Office previews are disabled for safety/i);
    expect(getArtifactContentUrl).toHaveBeenCalledWith("art-office", "sess-office");
    expect(screen.getByRole("link", { name: /Download report\.docx/i })).toBeTruthy();
    await waitFor(() => expect(fetch).toHaveBeenCalledTimes(1));
  });
});
