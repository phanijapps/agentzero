// =============================================================================
// artifact-poll — unit tests for the surviving pure helpers after R14f.
//
// The pre-R14f timer (`startArtifactPolling` + `sameArtifactIdSet` +
// `ARTIFACT_POLL_INTERVAL_MS`) is gone — snapshotSession() fetches once on
// open and again on agent_completed. The tests for the deleted helpers were
// dropped alongside them.
// =============================================================================

import { describe, it, expect, vi, beforeEach } from "vitest";
import type { Artifact } from "@/services/transport/types";
import type { ResearchArtifactRef } from "./types";

// --- Mocks ------------------------------------------------------------------
// vi.mock factories are hoisted; use vi.hoisted so the spies resolve at
// parse time (otherwise the top-level consts are TDZ-undefined).

const { listSessionArtifacts, toastError } = vi.hoisted(() => ({
  listSessionArtifacts: vi.fn<(sessionId: string) => Promise<{ success: boolean; data?: Artifact[]; error?: string }>>(),
  toastError: vi.fn(),
}));

vi.mock("@/services/transport", () => ({
  getTransport: async () => ({ listSessionArtifacts }),
}));

vi.mock("sonner", () => ({ toast: { error: toastError } }));

// Import AFTER mocks.
import { fetchArtifactsOnce, toArtifactRef } from "./artifact-poll";

// --- Fixtures ---------------------------------------------------------------

function makeArtifact(id: string, overrides: Partial<Artifact> = {}): Artifact {
  return {
    id,
    sessionId: "sess-1",
    filePath: `/tmp/${id}.md`,
    fileName: `${id}.md`,
    fileType: "md",
    fileSize: 100,
    isGoalArtifact: true,
    createdAt: "2026-04-19T00:00:00Z",
    ...overrides,
  };
}

beforeEach(() => {
  listSessionArtifacts.mockReset();
  toastError.mockReset();
});

// --- toArtifactRef ----------------------------------------------------------

describe("toArtifactRef", () => {
  it("maps an Artifact to a lightweight ResearchArtifactRef", () => {
    const full = makeArtifact("a1", { label: "Plan", fileSize: 2048 });
    const ref = toArtifactRef(full);
    expect(ref).toEqual({
      id: "a1",
      fileName: "a1.md",
      fileType: "md",
      fileSize: 2048,
      label: "Plan",
    });
  });

  it("omits undefined optional fields without dropping them from the shape", () => {
    const full = makeArtifact("a2", { fileType: undefined, fileSize: undefined, label: undefined });
    const ref = toArtifactRef(full);
    expect(ref.id).toBe("a2");
    expect(ref.fileName).toBe("a2.md");
    expect(ref.fileType).toBeUndefined();
  });
});

// --- fetchArtifactsOnce -----------------------------------------------------

describe("fetchArtifactsOnce", () => {
  it("dispatches SET_ARTIFACTS with the mapped refs on success", async () => {
    const dispatch = vi.fn();
    const latest = { current: [] as Artifact[] };
    const next = [makeArtifact("a1"), makeArtifact("a2")];
    listSessionArtifacts.mockResolvedValueOnce({ success: true, data: next });

    await fetchArtifactsOnce("sess-1", [], dispatch, latest);

    expect(listSessionArtifacts).toHaveBeenCalledWith("sess-1", {
      goalArtifactsOnly: true,
      limit: 24,
    });
    expect(dispatch).toHaveBeenCalledTimes(1);
    const call = dispatch.mock.calls[0][0];
    expect(call.type).toBe("SET_ARTIFACTS");
    expect(call.artifacts.map((a: ResearchArtifactRef) => a.id)).toEqual(["a1", "a2"]);
    expect(latest.current).toEqual(next);
  });

  it("defensively discards an undesignated row returned by the server", async () => {
    const dispatch = vi.fn();
    const latest = { current: [] as Artifact[] };
    listSessionArtifacts.mockResolvedValueOnce({
      success: true,
      data: [
        makeArtifact("deliverable", { isGoalArtifact: true }),
        makeArtifact("scratch", { isGoalArtifact: false }),
      ],
    });

    await fetchArtifactsOnce("sess-1", [], dispatch, latest);

    expect(latest.current.map((artifact) => artifact.id)).toEqual(["deliverable"]);
    expect(dispatch.mock.calls[0][0].artifacts.map((artifact: ResearchArtifactRef) => artifact.id))
      .toEqual(["deliverable"]);
  });

  it("no-op on a failed transport call; no dispatch, no toast", async () => {
    const dispatch = vi.fn();
    const latest = { current: [] as Artifact[] };
    listSessionArtifacts.mockResolvedValueOnce({ success: false, error: "offline" });

    await fetchArtifactsOnce("sess-1", [], dispatch, latest);

    expect(dispatch).not.toHaveBeenCalled();
    expect(toastError).not.toHaveBeenCalled();
    expect(latest.current).toEqual([]);
  });

  it("surfaces a sonner toast when the fetch throws", async () => {
    const dispatch = vi.fn();
    const latest = { current: [] as Artifact[] };
    listSessionArtifacts.mockRejectedValueOnce(new Error("boom"));

    await fetchArtifactsOnce("sess-1", [], dispatch, latest);

    expect(dispatch).not.toHaveBeenCalled();
    expect(toastError).toHaveBeenCalledTimes(1);
    expect(String(toastError.mock.calls[0][0])).toContain("boom");
  });
});
