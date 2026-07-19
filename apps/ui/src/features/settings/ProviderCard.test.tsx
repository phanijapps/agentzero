// ============================================================================
// ProviderCard — render, status badges, capability badges
// ============================================================================

import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@/test/utils";
import { ProviderCard } from "./ProviderCard";
import type { ProviderResponse, ModelRegistryResponse } from "@/services/transport";

function makeProvider(overrides: Partial<ProviderResponse> = {}): ProviderResponse {
  return {
    id: "openai-1",
    name: "OpenAI",
    description: "OpenAI API",
    hasApiKey: true,
    baseUrl: "https://api.openai.com/v1",
    models: ["gpt-4o", "gpt-4o-mini"],
    verified: true,
    ...overrides,
  };
}

const REGISTRY: ModelRegistryResponse = {
  "gpt-4o": {
    capabilities: { tools: true, vision: true, thinking: false, embeddings: false },
  } as unknown as ModelRegistryResponse[string],
  "gpt-4o-mini": {
    capabilities: { tools: true, vision: false, thinking: false, embeddings: false },
  } as unknown as ModelRegistryResponse[string],
};

describe("ProviderCard", () => {
  it("renders provider name and shortened hostname", () => {
    render(
      <ProviderCard
        provider={makeProvider()}
        modelRegistry={REGISTRY}
        isActive={false}
        onClick={() => {}}
      />
    );
    expect(screen.getByText("OpenAI")).toBeInTheDocument();
    expect(screen.getByText(/api\.openai\.com/)).toBeInTheDocument();
  });

  it("shows the Connected badge when verified=true", () => {
    render(
      <ProviderCard
        provider={makeProvider({ verified: true })}
        modelRegistry={REGISTRY}
        isActive={false}
        onClick={() => {}}
      />
    );
    expect(screen.getByText(/connected/i)).toBeInTheDocument();
    expect(screen.queryByText(/not tested/i)).not.toBeInTheDocument();
  });

  it("shows the Not tested badge when verified is false/missing", () => {
    render(
      <ProviderCard
        provider={makeProvider({ verified: false })}
        modelRegistry={REGISTRY}
        isActive={false}
        onClick={() => {}}
      />
    );
    expect(screen.getByText(/not tested/i)).toBeInTheDocument();
  });

  it("shows the Active badge when isActive=true", () => {
    render(
      <ProviderCard
        provider={makeProvider()}
        modelRegistry={REGISTRY}
        isActive={true}
        onClick={() => {}}
      />
    );
    expect(screen.getByText(/active/i)).toBeInTheDocument();
  });

  it("renders Tools and Vision capability badges aggregated across models", () => {
    render(
      <ProviderCard
        provider={makeProvider()}
        modelRegistry={REGISTRY}
        isActive={false}
        onClick={() => {}}
      />
    );
    expect(screen.getByText(/^Tools$/)).toBeInTheDocument();
    expect(screen.getByText(/^Vision$/)).toBeInTheDocument();
    expect(screen.queryByText(/^Thinking$/)).not.toBeInTheDocument();
  });

  it("calls onClick when the card is clicked", () => {
    const onClick = vi.fn();
    render(
      <ProviderCard
        provider={makeProvider()}
        modelRegistry={REGISTRY}
        isActive={false}
        onClick={onClick}
      />
    );
    fireEvent.click(screen.getByRole("button"));
    expect(onClick).toHaveBeenCalledTimes(1);
  });

  it("calls onClick when Enter is pressed on the card", () => {
    const onClick = vi.fn();
    render(
      <ProviderCard
        provider={makeProvider()}
        modelRegistry={REGISTRY}
        isActive={false}
        onClick={onClick}
      />
    );
    fireEvent.keyDown(screen.getByRole("button"), { key: "Enter" });
    expect(onClick).toHaveBeenCalledTimes(1);
  });

  it("renders +N more chip when models exceed maxVisible (3)", () => {
    render(
      <ProviderCard
        provider={makeProvider({ models: ["a", "b", "c", "d", "e"] })}
        modelRegistry={REGISTRY}
        isActive={false}
        onClick={() => {}}
      />
    );
    expect(screen.getByText(/\+2 more/)).toBeInTheDocument();
  });

  it("renders 'No models' chip when models list is empty", () => {
    render(
      <ProviderCard
        provider={makeProvider({ models: [] })}
        modelRegistry={REGISTRY}
        isActive={false}
        onClick={() => {}}
      />
    );
    expect(screen.getByText(/no models/i)).toBeInTheDocument();
  });
});
