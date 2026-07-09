// ============================================================================
// providerPresets — preset list shape + getAvailablePresets filter
// ============================================================================

import { describe, it, expect } from "vitest";
import { PROVIDER_PRESETS, getAvailablePresets } from "./providerPresets";

describe("PROVIDER_PRESETS", () => {
  it("ships at least the four canonical providers", () => {
    const names = PROVIDER_PRESETS.map((p) => p.name);
    expect(names).toContain("OpenAI");
    expect(names).toContain("Anthropic");
    expect(names).toContain("Ollama Cloud");
    expect(names).toContain("Ollama Local");
  });

  it("has exactly three featured providers", () => {
    const featured = PROVIDER_PRESETS.filter((p) => p.featured);
    expect(featured).toHaveLength(3);
    expect(featured.map((p) => p.name)).toEqual(["OpenAI", "Anthropic", "Ollama Cloud"]);
  });

  it("marks both Ollama presets as noApiKey", () => {
    const ollama = PROVIDER_PRESETS.filter((p) => p.name.startsWith("Ollama"));
    expect(ollama).toHaveLength(2);
    expect(ollama.every((p) => p.noApiKey === true)).toBe(true);
  });

  it("every preset has a non-empty baseUrl and at least one model", () => {
    for (const p of PROVIDER_PRESETS) {
      expect(p.baseUrl).toMatch(/^https?:\/\//);
      expect(p.models.split(",").length).toBeGreaterThan(0);
    }
  });
});

describe("getAvailablePresets", () => {
  it("returns the full preset list when no providers exist", () => {
    expect(getAvailablePresets([])).toEqual(PROVIDER_PRESETS);
  });

  it("does not filter out presets only because baseUrl is already used", () => {
    const result = getAvailablePresets([
      { baseUrl: "https://api.openai.com/v1/", name: "My OpenAI" },
    ]);
    expect(result.find((p) => p.name === "OpenAI")).toBeDefined();
    expect(result.find((p) => p.name === "Anthropic")).toBeDefined();
  });

  it("filters out presets whose name matches case-insensitively", () => {
    const result = getAvailablePresets([
      { baseUrl: "https://example.com", name: "ANTHROPIC" },
    ]);
    expect(result.find((p) => p.name === "Anthropic")).toBeUndefined();
  });

  it("preserves presets that share neither baseUrl nor name with existing providers", () => {
    const result = getAvailablePresets([
      { baseUrl: "https://example.com/v1", name: "Custom" },
    ]);
    expect(result).toEqual(PROVIDER_PRESETS);
  });
});
