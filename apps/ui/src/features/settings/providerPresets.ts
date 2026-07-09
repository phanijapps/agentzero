// ============================================================================
// PROVIDER PRESETS
// Pre-configured provider templates for quick setup
// ============================================================================

export interface ProviderPreset {
  name: string;
  baseUrl: string;
  models: string;
  apiKeyHint: string;
  apiKeyPlaceholder: string;
  /** If true, no API key is required (e.g., local Ollama) */
  noApiKey?: boolean;
  /** If true, shown in the top-3 prominent cards */
  featured?: boolean;
}

export const PROVIDER_PRESETS: ProviderPreset[] = [
  {
    name: "OpenAI",
    baseUrl: "https://api.openai.com/v1",
    models: "gpt-4o, gpt-4o-mini, o4-mini, gpt-4.1",
    apiKeyHint: "platform.openai.com/api-keys",
    apiKeyPlaceholder: "sk-...",
    featured: true,
  },
  {
    name: "Anthropic",
    baseUrl: "https://api.anthropic.com/v1",
    models: "claude-sonnet-4-20250514, claude-opus-4-20250514",
    apiKeyHint: "console.anthropic.com/settings/keys",
    apiKeyPlaceholder: "sk-ant-...",
    featured: true,
  },
  {
    name: "Ollama Cloud",
    baseUrl: "http://localhost:11434/v1",
    models: "gemma4:31b-cloud, nemotron-3-super:cloud, glm-5:cloud, gemini-3-flash-preview:cloud, deepseek-v3.2:cloud, kimi-k2.5:cloud, qwen3.5:cloud, devstral-2:cloud, minimax-m2.7:cloud",
    apiKeyHint: "",
    apiKeyPlaceholder: "",
    noApiKey: true,
    featured: true,
  },
  {
    name: "Ollama Local",
    baseUrl: "http://localhost:11434/v1",
    models: "gemma4, devstral-small-2, nemotron-3-nano, ministral-3, qwen3.5:4b, llama3.3",
    apiKeyHint: "",
    apiKeyPlaceholder: "",
    noApiKey: true,
  },
  {
    name: "Google Gemini",
    baseUrl: "https://generativelanguage.googleapis.com/v1beta/openai",
    models: "gemini-2.5-pro, gemini-2.5-flash, gemini-2.0-flash",
    apiKeyHint: "aistudio.google.com/apikey",
    apiKeyPlaceholder: "AIza...",
  },
  {
    name: "DeepSeek",
    baseUrl: "https://api.deepseek.com/v1",
    models: "deepseek-chat, deepseek-reasoner",
    apiKeyHint: "platform.deepseek.com/api_keys",
    apiKeyPlaceholder: "sk-...",
  },
  {
    name: "OpenRouter",
    baseUrl: "https://openrouter.ai/api/v1",
    models: "anthropic/claude-opus, openai/gpt-4-turbo, google/gemini-pro",
    apiKeyHint: "openrouter.ai/keys",
    apiKeyPlaceholder: "sk-or-...",
  },
  {
    name: "Z.AI",
    baseUrl: "https://api.z.ai/api/coding/paas/v4",
    models: "glm-5.1, glm-5, glm-5-turbo, glm-4.7, glm-4.6, glm-4.5",
    apiKeyHint: "z.ai dashboard",
    apiKeyPlaceholder: "your-api-key",
  },
  {
    name: "Mistral",
    baseUrl: "https://api.mistral.ai/v1",
    models: "mistral-large-latest, mistral-small-latest, codestral-latest",
    apiKeyHint: "console.mistral.ai/api-keys",
    apiKeyPlaceholder: "your-api-key",
  },
  {
    name: "Azure OpenAI",
    baseUrl: "https://YOUR-RESOURCE.openai.azure.com/openai/deployments/YOUR-DEPLOYMENT/v1",
    models: "gpt-4o, gpt-4.1, o4-mini",
    apiKeyHint: "portal.azure.com → OpenAI resource → Keys",
    apiKeyPlaceholder: "your-azure-api-key",
  },
];

/** Filter out presets where a provider with the same preset name already exists. */
export function getAvailablePresets(
  existingProviders: { baseUrl: string; name: string }[]
): ProviderPreset[] {
  return PROVIDER_PRESETS.filter(
    (preset) =>
      !existingProviders.some(
        (p) =>
          p.name.toLowerCase() === preset.name.toLowerCase()
      )
  );
}

// Re-export from shared utility for backwards compat
export { formatContextWindow } from "@/shared/utils/format";
