// ============================================================================
// LearningHealthBar — counts, progress label, backfill button, error state
// ============================================================================

import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent } from "@/test/utils";

const mockUseGraphStats = vi.fn();
const mockUseDistillationStatus = vi.fn();
const mockUseBackfill = vi.fn();

vi.mock("./graph-hooks", () => ({
  useGraphStats: () => mockUseGraphStats(),
  useDistillationStatus: () => mockUseDistillationStatus(),
  useBackfill: () => mockUseBackfill(),
}));

import { LearningHealthBar } from "./LearningHealthBar";

beforeEach(() => {
  vi.clearAllMocks();
  mockUseGraphStats.mockReturnValue({
    stats: { facts: 12, entities: 7, relationships: 4, episodes: 3 },
    loading: false,
    error: null,
  });
  mockUseDistillationStatus.mockReturnValue({
    status: {
      success_count: 5,
      failed_count: 0,
      skipped_count: 0,
      permanently_failed_count: 0,
    },
    loading: false,
    error: null,
    refetch: vi.fn(),
  });
  mockUseBackfill.mockReturnValue({
    run: vi.fn(),
    isRunning: false,
    isDone: false,
    progress: { current: 0, total: 0 },
    error: null,
  });
});

describe("LearningHealthBar", () => {
  it("renders nothing when both stats + distillation are loading", () => {
    mockUseGraphStats.mockReturnValueOnce({ stats: null, loading: true, error: null });
    mockUseDistillationStatus.mockReturnValueOnce({
      status: null,
      loading: true,
      error: null,
      refetch: vi.fn(),
    });
    const { container } = render(<LearningHealthBar />);
    expect(container).toBeEmptyDOMElement();
  });

  it("renders sessions distilled count (success / total)", () => {
    render(<LearningHealthBar />);
    expect(screen.getByText(/sessions distilled/i)).toBeInTheDocument();
    expect(screen.getByText(/5 \/ 5/)).toBeInTheDocument();
  });

  it("renders facts/entities/relationships/episodes from stats", () => {
    render(<LearningHealthBar />);
    expect(screen.getByText("12")).toBeInTheDocument(); // facts
    expect(screen.getByText("7")).toBeInTheDocument(); // entities
    expect(screen.getByText("4")).toBeInTheDocument(); // relationships
    expect(screen.getByText("3")).toBeInTheDocument(); // episodes
  });

  it("renders additive governance health when present", () => {
    mockUseGraphStats.mockReturnValueOnce({
      stats: {
        facts: 12,
        entities: 7,
        relationships: 4,
        episodes: 3,
        governance: {
          supported: true,
          ontologyIds: ["zbot.base:v1"],
          taxonomySchemeIds: ["zbot.general:v1"],
          validationMode: "advisory",
          allowUnclassified: "allow",
          skosExpansion: { maxDepth: 2, maxFanOut: 4, maxCandidates: 10 },
          findingCount: 2,
          findingCodes: ["unknown_relationship_type"],
        },
      },
      loading: false,
      error: null,
    });

    render(<LearningHealthBar />);

    expect(screen.getByText(/governance/i)).toBeInTheDocument();
    expect(screen.getByText(/active .* 2 findings/i)).toBeInTheDocument();
  });

  it("renders Failed and Skipped chips when those counts are > 0", () => {
    mockUseDistillationStatus.mockReturnValueOnce({
      status: {
        success_count: 5,
        failed_count: 2,
        skipped_count: 1,
        permanently_failed_count: 0,
      },
      loading: false,
      error: null,
      refetch: vi.fn(),
    });
    render(<LearningHealthBar />);
    expect(screen.getByText(/failed/i)).toBeInTheDocument();
    expect(screen.getByText(/skipped/i)).toBeInTheDocument();
  });

  it("shows the Backfill button and calls run() on click", () => {
    const run = vi.fn();
    mockUseBackfill.mockReturnValueOnce({
      run,
      isRunning: false,
      isDone: false,
      progress: { current: 0, total: 0 },
      error: null,
    });
    render(<LearningHealthBar />);
    const btn = screen.getByRole("button", { name: /backfill/i });
    fireEvent.click(btn);
    expect(run).toHaveBeenCalledTimes(1);
  });

  it("disables Backfill while running and shows the in-progress label", () => {
    mockUseBackfill.mockReturnValueOnce({
      run: vi.fn(),
      isRunning: true,
      isDone: false,
      progress: { current: 3, total: 10 },
      error: null,
    });
    render(<LearningHealthBar />);
    expect(screen.getByText(/distilling 3\/10/i)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /backfill/i })).toBeDisabled();
  });

  it("hides the Backfill button when isDone=true", () => {
    mockUseBackfill.mockReturnValueOnce({
      run: vi.fn(),
      isRunning: false,
      isDone: true,
      progress: { current: 0, total: 0 },
      error: null,
    });
    render(<LearningHealthBar />);
    expect(screen.queryByRole("button", { name: /backfill/i })).not.toBeInTheDocument();
  });

  it("shows 'Backfill failed' when backfill error is set", () => {
    mockUseBackfill.mockReturnValueOnce({
      run: vi.fn(),
      isRunning: false,
      isDone: false,
      progress: { current: 0, total: 0 },
      error: "request timeout",
    });
    render(<LearningHealthBar />);
    expect(screen.getByText(/backfill failed/i)).toBeInTheDocument();
  });
});
