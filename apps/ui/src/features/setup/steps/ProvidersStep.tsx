import { useState } from "react";
import { Loader2 } from "lucide-react";
import { getTransport } from "@/services/transport";
import type { ProviderResponse } from "@/services/transport";
import { PROVIDER_PRESETS, type ProviderPreset } from "@/features/settings/providerPresets";

/** Remove trailing slashes from a URL without using a regex */
const trimSlashes = (s: string): string => { let r = s; while (r.endsWith("/")) r = r.slice(0, -1); return r; };

const providerPresetKey = (name: string, baseUrl: string): string =>
  `${name.toLowerCase()}|${trimSlashes(baseUrl)}`;

interface ProvidersStepProps {
  providers: ProviderResponse[];
  defaultProviderId: string;
  onProvidersChanged: (providers: ProviderResponse[], defaultId: string) => void;
}

export function ProvidersStep({ providers, defaultProviderId, onProvidersChanged }: ProvidersStepProps) {
  const [expandedPreset, setExpandedPreset] = useState<string | null>(null);
  const [apiKey, setApiKey] = useState("");
  const [isTesting, setIsTesting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const addedPresetKeys = new Set(providers.map((p) => providerPresetKey(p.name, p.baseUrl)));

  const handleAddProvider = async (preset: ProviderPreset) => {
    if (preset.noApiKey) {
      await testAndAdd(preset, "ollama");
      return;
    }
    setExpandedPreset(preset.name);
    setApiKey("");
    setError(null);
  };

  const testAndAdd = async (preset: ProviderPreset, key: string) => {
    setIsTesting(true);
    setError(null);
    try {
      const transport = await getTransport();
      const models = preset.models.split(",").map((m) => m.trim()).filter(Boolean);
      const createResult = await transport.createProvider({
        name: preset.name,
        description: `${preset.name} API`,
        apiKey: key,
        baseUrl: preset.baseUrl,
        models,
      });
      if (!createResult.success || !createResult.data) {
        setError(createResult.error || "Failed to create provider");
        setIsTesting(false);
        return;
      }
      const id = createResult.data.id!;
      const testResult = await transport.testProviderById(id);
      if (!testResult.success || !testResult.data?.success) {
        await transport.deleteProvider(id);
        setError(testResult.data?.message || "Connection test failed");
        setIsTesting(false);
        return;
      }
      // Refresh provider list
      const listResult = await transport.listProviders();
      if (listResult.success && listResult.data) {
        const newProviders = listResult.data;
        const newDefault = newProviders.length === 1 ? id : defaultProviderId || id;
        onProvidersChanged(newProviders, newDefault);
      }
      setExpandedPreset(null);
      setApiKey("");
    } catch {
      setError("Something went wrong. Please try again.");
    } finally {
      setIsTesting(false);
    }
  };

  const handleRemove = async (id: string) => {
    try {
      const transport = await getTransport();
      await transport.deleteProvider(id);
      const listResult = await transport.listProviders();
      if (listResult.success && listResult.data) {
        const remaining = listResult.data;
        const newDefault = id === defaultProviderId
          ? remaining[0]?.id || ""
          : defaultProviderId;
        onProvidersChanged(remaining, newDefault);
      }
    } catch { /* ignore */ }
  };

  const handleSetDefault = (id: string) => {
    onProvidersChanged(providers, id);
  };

  return (
    <div>
      {/* Added providers */}
      {providers.length > 0 && (
        <div className="provider-added-list">
          {providers.map((p) => (
            <div key={p.id} className={`provider-added-row ${p.verified ? "provider-added-row--verified" : ""}`}>
              <div className="provider-added-row__info">
                {p.verified && <div className="provider-added-row__dot" />}
                <div>
                  <div className="provider-added-row__name">{p.name}</div>
                  <div className="provider-added-row__models">
                    {p.models.slice(0, 3).join(", ")}{p.models.length > 3 ? ` + ${p.models.length - 3} more` : ""}
                  </div>
                </div>
              </div>
              <div className="provider-added-row__actions">
                {p.verified && <span className="badge badge--success badge--xs">verified</span>}
                {p.id === defaultProviderId
                  ? <span className="badge badge--primary badge--xs">default</span>
                  : <button className="btn btn--ghost btn--sm" onClick={() => handleSetDefault(p.id!)}>set default</button>}
                <button className="btn btn--ghost btn--sm" onClick={() => handleRemove(p.id!)}>remove</button>
              </div>
            </div>
          ))}
        </div>
      )}

      {/* Preset grid */}
      <div className="settings-field-label">Add a provider</div>
      <div className="provider-add-grid">
        {PROVIDER_PRESETS.map((preset) => {
          const isAdded = addedPresetKeys.has(providerPresetKey(preset.name, preset.baseUrl));
          return (
            <div
              key={preset.name}
              className={`provider-add-card ${isAdded ? "provider-add-card--added" : ""}`}
              onClick={() => !isAdded && handleAddProvider(preset)}
              role="button"
              tabIndex={0}
              onKeyDown={(e) => { if (e.key === "Enter" || e.key === " ") { if (!isAdded) handleAddProvider(preset); } }}
            >
              <div className="provider-add-card__name">{preset.name}</div>
              <div className="provider-add-card__hint">
                {isAdded ? "added" : preset.noApiKey ? "no key needed" : preset.apiKeyPlaceholder}
              </div>
            </div>
          );
        })}
      </div>

      {/* Inline add form */}
      {expandedPreset && (
        <div className="provider-add-form">
          <div className="settings-field-label">Add {expandedPreset}</div>
          <div className="flex gap-2">
            <input
              className="form-input flex-1"
              value={apiKey}
              onChange={(e) => setApiKey(e.target.value)}
              placeholder={PROVIDER_PRESETS.find((p) => p.name === expandedPreset)?.apiKeyPlaceholder}
              type="password"
              autoFocus
            />
            <button
              className="btn btn--primary btn--sm"
              onClick={() => {
                const preset = PROVIDER_PRESETS.find((p) => p.name === expandedPreset);
                if (preset && apiKey.trim()) testAndAdd(preset, apiKey.trim());
              }}
              disabled={!apiKey.trim() || isTesting}
            >
              {isTesting ? <Loader2 className="loading-spinner__icon" /> : "Test & Add"}
            </button>
          </div>
          {error && <div className="alert alert--error">{error}</div>}
          <p className="settings-hint">
            {PROVIDER_PRESETS.find((p) => p.name === expandedPreset)?.apiKeyHint}
          </p>
        </div>
      )}
    </div>
  );
}
