import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { CommissioningGuard } from "./CommissioningGuard";

const getCommissioningStatus = vi.fn();

vi.mock("@/services/transport", () => ({
  getTransport: async () => ({ getCommissioningStatus }),
}));

beforeEach(() => {
  getCommissioningStatus.mockReset();
});

function renderGuard(path = "/") {
  return render(
    <MemoryRouter initialEntries={[path]}>
      <Routes>
        <Route path="/commission" element={<div>Commissioning screen</div>} />
        <Route path="*" element={<CommissioningGuard><div>Application shell</div></CommissioningGuard>} />
      </Routes>
    </MemoryRouter>,
  );
}

describe("CommissioningGuard", () => {
  it("allows a completed installation into the application", async () => {
    getCommissioningStatus.mockResolvedValue({
      success: true,
      data: { state: "complete", semanticProfile: { version: 1, basePackIds: [], domainPackIds: [], provisioning: "deferred" } },
    });
    renderGuard();
    expect(await screen.findByText("Application shell")).toBeInTheDocument();
  });

  it("redirects an uncommissioned installation to the commissioning route", async () => {
    getCommissioningStatus.mockResolvedValue({
      success: true,
      data: { state: "not_started", semanticProfile: { version: 1, basePackIds: [], domainPackIds: [], provisioning: "deferred" } },
    });
    renderGuard();
    expect(await screen.findByText("Commissioning screen")).toBeInTheDocument();
  });

  it("does not fail open when readiness cannot be checked", async () => {
    getCommissioningStatus.mockResolvedValue({ success: false, error: "gateway unavailable" });
    renderGuard();
    await waitFor(() => {
      expect(screen.getByRole("alert")).toHaveTextContent(/can’t confirm z-Bot is ready/i);
    });
    expect(screen.queryByText("Application shell")).toBeNull();
  });
});
