import { describe, expect, it, vi, beforeEach } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { CommissioningScreen } from "./CommissioningScreen";

const diagnoseLocalRuntime = vi.fn();
const completeCommissioning = vi.fn();

vi.mock("@/services/transport", () => ({
  getTransport: async () => ({ diagnoseLocalRuntime, completeCommissioning }),
}));

function renderScreen() {
  return render(<MemoryRouter><CommissioningScreen /></MemoryRouter>);
}

beforeEach(() => {
  diagnoseLocalRuntime.mockReset();
  completeCommissioning.mockReset();
});

describe("CommissioningScreen", () => {
  it("requires a focus before the user can continue", () => {
    renderScreen();
    expect(screen.getByText(/your profile stays in local z-bot data/i)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /continue/i })).toBeDisabled();

    fireEvent.click(screen.getByRole("button", { name: /think & organize/i }));

    expect(screen.getByRole("button", { name: /continue/i })).toBeEnabled();
    expect(screen.getByRole("button", { name: /^personal knowledge$/i })).toHaveAttribute("aria-pressed", "true");
  });

  it("shows an actionable local-runtime diagnosis and available models", async () => {
    diagnoseLocalRuntime.mockResolvedValue({
      success: true,
      data: { state: "ready", recoveryCode: "local_runtime_ready", models: ["llama3.3"] },
    });
    renderScreen();

    fireEvent.click(screen.getByRole("button", { name: /build & code/i }));
    fireEvent.click(screen.getByRole("button", { name: /continue/i }));
    fireEvent.click(screen.getByRole("button", { name: /local model/i }));
    fireEvent.click(screen.getByRole("button", { name: /check local setup/i }));

    await waitFor(() => {
      expect(screen.getByText(/your local runtime is ready/i)).toBeInTheDocument();
    });
    expect(screen.getByRole("option", { name: "llama3.3" })).toBeInTheDocument();
  });

  it("submits a portable commission and clears the entered API key", async () => {
    completeCommissioning.mockResolvedValue({
      success: true,
      data: { state: "complete", semanticProfile: { version: 1, basePackIds: [], domainPackIds: [], provisioning: "deferred" } },
    });
    renderScreen();

    fireEvent.click(screen.getByRole("button", { name: /research & learn/i }));
    fireEvent.click(screen.getByRole("button", { name: /continue/i }));
    fireEvent.change(screen.getByLabelText(/api key/i), { target: { value: "sensitive-key" } });
    fireEvent.click(screen.getByRole("button", { name: /continue/i }));
    expect(screen.getByRole("button", { name: /commission my agent/i })).toBeDisabled();
    fireEvent.change(screen.getByLabelText(/your name/i), { target: { value: "Ada" } });
    fireEvent.click(screen.getByRole("button", { name: /learning & ideas/i }));
    fireEvent.change(screen.getByLabelText(/hobbies & pastimes/i), { target: { value: "Reading, Hiking" } });
    fireEvent.change(screen.getByLabelText(/date of birth/i), { target: { value: "1990-01-01" } });
    expect(screen.getByText(/execution is not sandboxed yet/i)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /commission my agent/i }));

    await waitFor(() => expect(completeCommissioning).toHaveBeenCalledTimes(1));
    expect(completeCommissioning.mock.calls[0][0].provider.apiKey).toBe("sensitive-key");
    expect(completeCommissioning.mock.calls[0][0]).toMatchObject({
      userName: "Ada",
      interests: ["Learning & ideas"],
      hobbies: ["Reading", "Hiking"],
      dateOfBirth: "1990-01-01",
    });
    expect(screen.queryByDisplayValue("sensitive-key")).toBeNull();
  });
});
