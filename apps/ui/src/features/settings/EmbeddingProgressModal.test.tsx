// ============================================================================
// EmbeddingProgressModal tests
// ============================================================================

import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor, fireEvent, act } from "@/test/utils";

import type {
  ConfigureProgressEvent,
  EmbeddingConfig,
  EmbeddingsHealth,
} from "@/services/transport";

// ---------------------------------------------------------------------------
// Transport mock that captures the onProgress callback so tests can drive it.
// ---------------------------------------------------------------------------

type ProgressFn = (event: ConfigureProgressEvent) => void;

let capturedProgress: ProgressFn | null = null;
let resolveConfigure: ((result: { success: boolean; data?: EmbeddingsHealth; error?: string }) => void) | null = null;
const configureCalls: EmbeddingConfig[] = [];

const mockConfigure = vi.fn(
  (config: EmbeddingConfig, onProgress: ProgressFn) =>
    new Promise((resolve) => {
      capturedProgress = onProgress;
      resolveConfigure = resolve;
      configureCalls.push(config);
    }),
);

vi.mock("@/services/transport", async () => {
  const actual = await vi.importActual<Record<string, unknown>>("@/services/transport");
  return {
    ...actual,
    getTransport: async () => ({ configureEmbeddings: mockConfigure }),
  };
});

import { EmbeddingProgressModal } from "./EmbeddingProgressModal";

const CONFIG: EmbeddingConfig = {
  internal: false,
  ollama: { url: "http://localhost:11434", model: "nomic-embed-text", dimensions: 768 },
};

beforeEach(() => {
  capturedProgress = null;
  resolveConfigure = null;
  configureCalls.length = 0;
  mockConfigure.mockClear();
});

async function waitForProgress() {
  await waitFor(() => expect(capturedProgress).not.toBeNull());
}

describe("EmbeddingProgressModal", () => {
  it("renders pulling phase when pulling event arrives", async () => {
    render(
      <EmbeddingProgressModal
        config={CONFIG}
        indexedCount={50}
        onClose={() => {}}
        onSuccess={() => {}}
      />,
    );
    await waitForProgress();
    act(() => {
      capturedProgress!({ kind: "pulling", mb_done: 100, mb_total: 400 });
    });
    expect(screen.getByTestId("phase-pulling")).toBeInTheDocument();
    expect(screen.getByTestId("phase-pulling").textContent).toMatch(/100 MB \/ 400 MB/);
    expect(screen.getByTestId("phase-pulling").textContent).toMatch(/25%/);
  });

  it("transitions to reindexing phase when event arrives", async () => {
    render(
      <EmbeddingProgressModal
        config={CONFIG}
        indexedCount={200}
        onClose={() => {}}
        onSuccess={() => {}}
      />,
    );
    await waitForProgress();
    act(() => {
      capturedProgress!({ kind: "reindexing", table: "engram_vectors", current: 50, total: 200 });
    });
    expect(screen.getByTestId("phase-reindexing")).toBeInTheDocument();
    expect(screen.getByTestId("phase-reindexing").textContent).toMatch(/engram_vectors: 50 \/ 200/);
  });

  it("shows ready state with Close button on success", async () => {
    const onSuccess = vi.fn();
    render(
      <EmbeddingProgressModal
        config={CONFIG}
        indexedCount={0}
        onClose={() => {}}
        onSuccess={onSuccess}
      />,
    );
    await waitForProgress();
    act(() => {
      capturedProgress!({ kind: "ready", backend: "ollama", model: "nomic-embed-text", dim: 768 });
    });
    await act(async () => {
      resolveConfigure!({
        success: true,
        data: {
          backend: "ollama",
          model: "nomic-embed-text",
          dim: 768,
          status: "ready",
          indexed_count: 0,
        },
      });
    });
    expect(screen.getByTestId("phase-ready")).toBeInTheDocument();
    const closeButtons = screen.getAllByRole("button", { name: /close/i });
    // One inside our phase body + the ModalOverlay's built-in close icon.
    expect(closeButtons.length).toBeGreaterThanOrEqual(1);
    expect(onSuccess).toHaveBeenCalledTimes(1);
  });

  it("shows error state with Retry that re-invokes configureEmbeddings", async () => {
    render(
      <EmbeddingProgressModal
        config={CONFIG}
        indexedCount={0}
        onClose={() => {}}
        onSuccess={() => {}}
      />,
    );
    await waitForProgress();
    act(() => {
      capturedProgress!({ kind: "error", reason: "ollama unreachable" });
    });
    await act(async () => {
      resolveConfigure!({ success: false, error: "ollama unreachable" });
    });
    expect(screen.getByTestId("phase-error")).toBeInTheDocument();
    expect(screen.getByTestId("phase-error").textContent).toMatch(/ollama unreachable/);

    expect(mockConfigure).toHaveBeenCalledTimes(1);
    fireEvent.click(screen.getByRole("button", { name: /retry/i }));
    await waitFor(() => expect(mockConfigure).toHaveBeenCalledTimes(2));
    expect(configureCalls[1]).toEqual(CONFIG);
  });
});
