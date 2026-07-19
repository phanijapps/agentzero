// ============================================================================
// ProvidersGrid — renders one ProviderCard per provider, fires onSelect
// ============================================================================

import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@/test/utils";
import { ProvidersGrid } from "./ProvidersGrid";
import type { ProviderResponse } from "@/services/transport";

function makeProvider(id: string, name: string): ProviderResponse {
  return {
    id,
    name,
    description: "",
    hasApiKey: false,
    baseUrl: `https://${id}.example.com/v1`,
    models: [],
    verified: false,
  };
}

describe("ProvidersGrid", () => {
  it("renders a card per provider", () => {
    const providers = [makeProvider("a", "Alpha"), makeProvider("b", "Beta")];
    render(
      <ProvidersGrid
        providers={providers}
        modelRegistry={{}}
        onSelect={() => {}}
      />
    );
    expect(screen.getByText("Alpha")).toBeInTheDocument();
    expect(screen.getByText("Beta")).toBeInTheDocument();
    expect(screen.getAllByRole("button")).toHaveLength(2);
  });

  it("marks the provider whose id matches defaultProviderId as Active", () => {
    const providers = [makeProvider("a", "Alpha"), makeProvider("b", "Beta")];
    render(
      <ProvidersGrid
        providers={providers}
        modelRegistry={{}}
        defaultProviderId="b"
        onSelect={() => {}}
      />
    );
    const activeBadges = screen.getAllByText(/^active$/i);
    expect(activeBadges).toHaveLength(1);
  });

  it("calls onSelect with the clicked provider", () => {
    const onSelect = vi.fn();
    const providers = [makeProvider("a", "Alpha"), makeProvider("b", "Beta")];
    render(
      <ProvidersGrid
        providers={providers}
        modelRegistry={{}}
        onSelect={onSelect}
      />
    );
    fireEvent.click(screen.getByText("Beta").closest('[role="button"]') as HTMLElement);
    expect(onSelect).toHaveBeenCalledWith(providers[1]);
  });

  it("renders an empty grid when no providers", () => {
    render(<ProvidersGrid providers={[]} modelRegistry={{}} onSelect={() => {}} />);
    expect(screen.queryAllByRole("button")).toHaveLength(0);
  });
});
