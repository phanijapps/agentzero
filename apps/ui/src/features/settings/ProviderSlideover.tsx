// ============================================================================
// PROVIDER SLIDE-OVER
// Detail panel with view/edit modes + create mode
// ============================================================================

import { useState, useEffect, useRef, useCallback } from "react";
import { X, Pencil, Loader2, AlertCircle, Plus, Play, Trash2 } from "lucide-react";
import { getTransport } from "@/services/transport";
import type { ProviderResponse, ModelRegistryResponse } from "@/services/transport";
import { ModelChip } from "@/shared/ui/ModelChip";
import type { ProviderPreset } from "./providerPresets";

// ============================================================================
// Types
// ============================================================================

interface ProviderSlideoverProps {
  provider?: ProviderResponse | null;
  modelRegistry: ModelRegistryResponse;
  isActive: boolean;
  isOpen: boolean;
  /** "view" for existing provider, "create" for new */
  mode: "view" | "create";
  /** Preset to pre-fill in create mode */
  preset?: ProviderPreset | null;
  onClose: () => void;
  onSaved: () => void;
  onDeleted: () => void;
  onSetActive: (id: string) => void;
}

interface EditForm {
  name: string;
  description: string;
  apiKey: string;
  baseUrl: string;
  models: string[];
  defaultModel: string;
  rateLimitsRpm: string;
  rateLimitsConcurrent: string;
}

// ============================================================================
// Component
// ============================================================================

export function ProviderSlideover({
  provider,
  modelRegistry,
  isActive,
  isOpen,
  mode,
  preset,
  onClose,
  onSaved,
  onDeleted,
  onSetActive,
}: ProviderSlideoverProps) {
  const [isEditing, setIsEditing] = useState(mode === "create");
  const [form, setForm] = useState<EditForm>({ name: "", description: "", apiKey: "", baseUrl: "", models: [], defaultModel: "", rateLimitsRpm: "30", rateLimitsConcurrent: "2" });
  const [isSaving, setIsSaving] = useState(false);
  const [isTesting, setIsTesting] = useState(false);
  const [isDeleting, setIsDeleting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [testResult, setTestResult] = useState<{ success: boolean; message: string } | null>(null);
  const [isDirty, setIsDirty] = useState(false);
  const [showApiKey, setShowApiKey] = useState(false);
  const [modelInput, setModelInput] = useState("");
  const [showDeleteConfirm, setShowDeleteConfirm] = useState(false);
  const panelRef = useRef<HTMLDivElement>(null);

  // Initialize form from provider or preset
  useEffect(() => {
    if (mode === "create" && preset) {
      const models = preset.models.split(",").map((m) => m.trim()).filter(Boolean);
      setForm({
        name: preset.name,
        description: `${preset.name} API`,
        apiKey: preset.noApiKey ? "ollama" : "",
        baseUrl: preset.baseUrl,
        models,
        defaultModel: models[0] || "",
        rateLimitsRpm: "30",
        rateLimitsConcurrent: "2",
      });
      setIsEditing(true);
      setIsDirty(false);
    } else if (provider) {
      setForm({
        name: provider.name,
        description: provider.description,
        apiKey: "",
        baseUrl: provider.baseUrl,
        models: [...provider.models],
        defaultModel: provider.defaultModel || provider.models[0] || "",
        rateLimitsRpm: String(provider?.rateLimits?.requestsPerMinute ?? 30),
        rateLimitsConcurrent: String(provider?.rateLimits?.concurrentRequests ?? 2),
      });
      setIsEditing(false);
      setIsDirty(false);
    }
    setError(null);
    setTestResult(null);
    setShowApiKey(false);
    setShowDeleteConfirm(false);
  }, [provider, preset, mode, isOpen]);

  // Escape key handler
  useEffect(() => {
    const handleEscape = (e: KeyboardEvent) => {
      if (e.key === "Escape" && isOpen) {
        if (isEditing && isDirty) {
          if (confirm("You have unsaved changes. Discard them?")) {
            handleClose();
          }
        } else {
          handleClose();
        }
      }
    };
    document.addEventListener("keydown", handleEscape);
    return () => document.removeEventListener("keydown", handleEscape);
  }, [isOpen, isEditing, isDirty]);

  const handleClose = useCallback(() => {
    setIsEditing(false);
    setIsDirty(false);
    onClose();
  }, [onClose]);

  const handleBackdropClick = () => {
    if (isEditing && isDirty) {
      if (confirm("You have unsaved changes. Discard them?")) {
        handleClose();
      }
    } else {
      handleClose();
    }
  };

  const handleFormChange = (updates: Partial<EditForm>) => {
    setForm((prev) => ({ ...prev, ...updates }));
    setIsDirty(true);
  };

  const handleAddModel = () => {
    const model = modelInput.trim();
    if (model && !form.models.includes(model)) {
      handleFormChange({ models: [...form.models, model] });
      if (!form.defaultModel) handleFormChange({ defaultModel: model });
    }
    setModelInput("");
  };

  const handleRemoveModel = (model: string) => {
    const updated = form.models.filter((m) => m !== model);
    const updates: Partial<EditForm> = { models: updated };
    if (form.defaultModel === model) updates.defaultModel = updated[0] || "";
    handleFormChange(updates);
  };

  const handleTest = async () => {
    setIsTesting(true);
    setTestResult(null);
    try {
      const transport = await getTransport();
      let result;
      if (provider?.id && mode === "view") {
        // Existing provider — use ID-based test that persists verified + models
        result = await transport.testProviderById(provider.id);
      } else {
        // New provider (create mode) — inline test
        result = await transport.testProvider({
          name: form.name,
          description: form.description,
          apiKey: form.apiKey,
          baseUrl: form.baseUrl,
          models: form.models,
        });
      }
      if (result.success && result.data) {
        setTestResult(result.data);
        // Refresh parent to show updated verified status + models
        if (result.data.success) {
          onSaved();
        }
      } else {
        setTestResult({ success: false, message: result.error || "Test failed" });
      }
    } catch {
      setTestResult({ success: false, message: "Could not reach provider" });
    } finally {
      setIsTesting(false);
    }
  };

  const handleSave = async () => {
    if (!form.name || !form.baseUrl || (mode === "create" && !form.apiKey)) return;
    setIsSaving(true);
    setError(null);
    try {
      const transport = await getTransport();

      if (mode === "create") {
        // Test first, then create
        const testRes = await transport.testProvider({
          name: form.name,
          description: form.description,
          apiKey: form.apiKey,
          baseUrl: form.baseUrl,
          models: form.models,
        });
        if (!testRes.success || !testRes.data?.success) {
          setError(testRes.data?.message || testRes.error || "Connection test failed. Check your API key.");
          setIsSaving(false);
          return;
        }

        const result = await transport.createProvider({
          name: form.name,
          description: form.description,
          apiKey: form.apiKey,
          baseUrl: form.baseUrl,
          models: form.models,
          defaultModel: form.defaultModel || undefined,
          rateLimits: {
            requestsPerMinute: Number.parseInt(form.rateLimitsRpm) || 30,
            concurrentRequests: Number.parseInt(form.rateLimitsConcurrent) || 2,
          },
        });
        if (!result.success) {
          setError(result.error || "Failed to create provider");
          setIsSaving(false);
          return;
        }
      } else if (provider?.id) {
        const result = await transport.updateProvider(provider.id, {
          name: form.name,
          description: form.description,
          apiKey: form.apiKey || undefined,
          baseUrl: form.baseUrl,
          models: form.models,
          defaultModel: form.defaultModel || undefined,
          rateLimits: {
            requestsPerMinute: Number.parseInt(form.rateLimitsRpm) || 30,
            concurrentRequests: Number.parseInt(form.rateLimitsConcurrent) || 2,
          },
        });
        if (!result.success) {
          setError(result.error || "Failed to update provider");
          setIsSaving(false);
          return;
        }
      }
      setIsDirty(false);
      setIsEditing(false);
      onSaved();
    } catch {
      setError("Something went wrong. Please try again.");
    } finally {
      setIsSaving(false);
    }
  };

  const handleDelete = async () => {
    if (!provider?.id) return;
    setIsDeleting(true);
    try {
      const transport = await getTransport();
      const result = await transport.deleteProvider(provider.id);
      if (result.success) {
        onDeleted();
      } else {
        setError(result.error || "Failed to delete provider");
      }
    } catch {
      setError("Failed to delete provider");
    } finally {
      setIsDeleting(false);
      setShowDeleteConfirm(false);
    }
  };

  const maskedKey = form.apiKey
    ? `${form.apiKey.slice(0, 4)}${"•".repeat(8)}${form.apiKey.slice(-4)}`
    : provider?.hasApiKey ? "Configured" : "Not set";

  return (
    <>
      {/* Backdrop */}
      <div
        className={`provider-slideover__backdrop${isOpen ? " provider-slideover__backdrop--open" : ""}`}
        role="button"
        tabIndex={0}
        aria-label="Close panel"
        onClick={handleBackdropClick}
        onKeyDown={(e) => { if (e.key === "Enter" || e.key === " ") handleBackdropClick(); }}
      />

      {/* Panel */}
      <div
        ref={panelRef}
        className={`provider-slideover${isOpen ? " provider-slideover--open" : ""}`}
        role="dialog"
        aria-modal="true"
        aria-label={mode === "create" ? "Add Provider" : `${provider?.name || "Provider"} details`}
      >
        {/* Header */}
        <div className="provider-slideover__header">
          <div>
            <h2 style={{ fontSize: "var(--text-lg)", fontWeight: 600, color: "var(--foreground)" }}>
              {mode === "create" ? "Add Provider" : form.name}
            </h2>
            {mode === "view" && (
              <div style={{ display: "flex", gap: "var(--spacing-1-5)", marginTop: "var(--spacing-1-5)" }}>
                {provider?.verified ? (
                  <span className="badge badge--success">Connected</span>
                ) : (
                  <span className="badge badge--warning">Not tested</span>
                )}
                {isActive && <span className="badge badge--primary">Active</span>}
              </div>
            )}
          </div>
          <div style={{ display: "flex", gap: "var(--spacing-2)", alignItems: "center" }}>
            {mode === "view" && !isEditing && (
              <button className="btn btn--outline btn--sm" onClick={() => setIsEditing(true)}>
                <Pencil size={14} /> Edit
              </button>
            )}
            {isEditing && mode === "view" && (
              <>
                <button className="btn btn--ghost btn--sm" onClick={() => { setIsEditing(false); setIsDirty(false); setError(null); }}>
                  Cancel
                </button>
                <button className="btn btn--primary btn--sm" disabled={isSaving} onClick={handleSave}>
                  {isSaving ? <Loader2 className="w-4 h-4 animate-spin" /> : "Save"}
                </button>
              </>
            )}
            <button className="btn btn--ghost btn--icon" onClick={handleBackdropClick} aria-label="Close">
              <X size={18} />
            </button>
          </div>
        </div>

        {/* Connection bar */}
        {mode === "view" && (
          <div className={`connection-bar${provider?.verified ? " connection-bar--ok" : " connection-bar--warn"}`}>
            <div className={`connection-dot${provider?.verified ? " connection-dot--ok" : " connection-dot--warn"}`} />
            <span>{provider?.verified ? "Connection verified" : "Not yet tested"}</span>
            <button
              className="btn btn--ghost btn--sm"
              style={{ marginLeft: "auto" }}
              disabled={isTesting}
              onClick={handleTest}
            >
              {isTesting ? <Loader2 className="w-3.5 h-3.5 animate-spin" /> : <Play size={14} />}
              Test
            </button>
          </div>
        )}

        {/* Test result */}
        {testResult && (
          <div className={`alert ${testResult.success ? "alert--success" : "alert--error"}`} style={{ margin: "0 var(--spacing-6)", marginTop: "var(--spacing-3)" }}>
            <span>{testResult.message}</span>
          </div>
        )}

        {/* Error */}
        {error && (
          <div className="alert alert--error" style={{ margin: "0 var(--spacing-6)", marginTop: "var(--spacing-3)" }}>
            <AlertCircle size={14} />
            <span>{error}</span>
          </div>
        )}

        {/* Body */}
        <div className="provider-slideover__body">
          {/* API Key hint for create mode */}
          {mode === "create" && preset?.apiKeyHint && (
            <div style={{ background: "var(--primary-muted)", borderRadius: "var(--radius-md)", padding: "var(--spacing-3)", fontSize: "var(--text-xs)", color: "var(--muted-foreground)" }}>
              Get your API key from <span style={{ color: "var(--primary)" }}>{preset.apiKeyHint}</span>
            </div>
          )}

          {/* API Key */}
          <div>
            <div className="field-label">API Key</div>
            {isEditing ? (
              <input
                className="form-input"
                type={showApiKey ? "text" : "password"}
                value={form.apiKey}
                onChange={(e) => handleFormChange({ apiKey: e.target.value })}
                placeholder={preset?.apiKeyPlaceholder || "sk-..."}
                autoFocus={mode === "create"}
              />
            ) : (
              <div className="field-value field-value--mono">{maskedKey}</div>
            )}
          </div>

          {/* Base URL */}
          <div>
            <div className="field-label">Base URL</div>
            {isEditing ? (
              <input
                className="form-input"
                type="text"
                value={form.baseUrl}
                onChange={(e) => handleFormChange({ baseUrl: e.target.value })}
              />
            ) : (
              <div className="field-value field-value--mono">{form.baseUrl}</div>
            )}
          </div>

          {/* Rate Limits */}
          <div>
            <div className="field-label">Rate Limits</div>
            {isEditing ? (
              <div className="provider-slideover__rate-grid">
                <div>
                  <div className="field-label">Requests/min</div>
                  <input
                    className="form-input"
                    type="number"
                    min="1"
                    max="1000"
                    value={form.rateLimitsRpm}
                    onChange={(e) => handleFormChange({ rateLimitsRpm: e.target.value })}
                  />
                </div>
                <div>
                  <div className="field-label">Max concurrent</div>
                  <input
                    className="form-input"
                    type="number"
                    min="1"
                    max="20"
                    value={form.rateLimitsConcurrent}
                    onChange={(e) => handleFormChange({ rateLimitsConcurrent: e.target.value })}
                  />
                </div>
              </div>
            ) : (
              <div className="provider-slideover__rate-grid">
                <div className="field-value">{provider?.rateLimits?.requestsPerMinute ?? 30} req/min</div>
                <div className="field-value">{provider?.rateLimits?.concurrentRequests ?? 2} concurrent</div>
              </div>
            )}
          </div>

          {/* Default Model */}
          <div>
            <div className="field-label">Default Model</div>
            {isEditing ? (
              <select
                className="form-input"
                value={form.defaultModel}
                onChange={(e) => handleFormChange({ defaultModel: e.target.value })}
              >
                {form.models.map((m) => (
                  <option key={m} value={m}>{m}</option>
                ))}
              </select>
            ) : (
              <div className="field-value">{form.defaultModel || "None"}</div>
            )}
          </div>

          {/* Models - View mode: enriched rows */}
          {!isEditing && (
            <div>
              <div className="field-label">Models ({provider?.models?.length ?? 0})</div>
              {provider?.models?.map((modelId) => {
                const config = provider.modelConfigs?.[modelId];
                const registryProfile = modelRegistry[modelId];
                const caps = config?.capabilities ?? registryProfile?.capabilities;
                const maxIn = config?.maxInput ?? registryProfile?.context?.input;
                const maxOut = config?.maxOutput ?? registryProfile?.context?.output;
                const isDefault = modelId === (provider.defaultModel || provider.models[0]);

                return (
                  <div key={modelId} className="provider-slideover__model-row">
                    <div className="provider-slideover__model-header">
                      <span className="provider-slideover__model-name">{modelId}</span>
                      {isDefault && <span className="badge badge--success badge--xs">default</span>}
                    </div>
                    <div className="provider-slideover__model-meta">
                      {caps?.tools && <span className="cap-badge cap-badge--tools">tools</span>}
                      {caps?.vision && <span className="cap-badge cap-badge--vision">vision</span>}
                      {caps?.thinking && <span className="cap-badge cap-badge--thinking">thinking</span>}
                      {caps?.embeddings && <span className="cap-badge cap-badge--embed">embeddings</span>}
                      {maxIn != null && maxIn > 0 && (
                        <span className="provider-slideover__model-context">
                          {Math.round(maxIn / 1000)}K in{maxOut ? ` / ${Math.round(maxOut / 1000)}K out` : ""}
                        </span>
                      )}
                    </div>
                  </div>
                );
              })}
            </div>
          )}

          {/* Models - Edit mode: chips */}
          {isEditing && (
            <div>
              <div className="field-label">Models ({form.models.length})</div>
              <div style={{ display: "flex", flexWrap: "wrap", gap: "var(--spacing-1-5)" }}>
                {form.models.map((model) => (
                  <ModelChip
                    key={model}
                    modelId={model}
                    profile={modelRegistry[model]}
                    showContext
                    removable={isEditing}
                    onRemove={() => handleRemoveModel(model)}
                  />
                ))}
                <span className="model-chip model-chip--removable" style={{ border: "1px dashed var(--border)" }}>
                  <input
                    style={{ background: "transparent", border: "none", outline: "none", width: 80, fontSize: "var(--text-xs)", color: "var(--foreground)" }}
                    placeholder="Add model..."
                    value={modelInput}
                    onChange={(e) => setModelInput(e.target.value)}
                    onKeyDown={(e) => { if (e.key === "Enter") { e.preventDefault(); handleAddModel(); } }}
                  />
                  <button onClick={handleAddModel} style={{ color: "var(--primary)", background: "none", border: "none", cursor: "pointer", padding: 0 }}>
                    <Plus size={12} />
                  </button>
                </span>
              </div>
            </div>
          )}
        </div>

        {/* Footer */}
        <div className="provider-slideover__footer">
          {mode === "create" ? (
            <>
              <div />
              <div style={{ display: "flex", gap: "var(--spacing-2)" }}>
                <button className="btn btn--ghost btn--sm" onClick={handleClose}>Cancel</button>
                <button className="btn btn--primary btn--sm" disabled={isSaving || !form.apiKey || !form.baseUrl} onClick={handleSave}>
                  {isSaving ? <Loader2 className="w-4 h-4 animate-spin" /> : "Test & Connect"}
                </button>
              </div>
            </>
          ) : (
            <>
              {showDeleteConfirm ? (
                <div style={{ display: "flex", gap: "var(--spacing-2)", alignItems: "center" }}>
                  <span style={{ fontSize: "var(--text-xs)", color: "var(--muted-foreground)" }}>Are you sure?</span>
                  <button className="btn btn--destructive btn--sm" disabled={isDeleting} onClick={handleDelete}>
                    {isDeleting ? <Loader2 className="w-3.5 h-3.5 animate-spin" /> : <><Trash2 size={14} /> Delete</>}
                  </button>
                  <button className="btn btn--ghost btn--sm" onClick={() => setShowDeleteConfirm(false)}>Cancel</button>
                </div>
              ) : (
                <button className="btn btn--ghost btn--sm" style={{ color: "var(--destructive)" }} onClick={() => setShowDeleteConfirm(true)}>
                  Delete provider
                </button>
              )}
              {!isActive && provider?.id && (
                <button className="btn btn--outline btn--sm" onClick={() => onSetActive(provider.id!)}>
                  Set as active
                </button>
              )}
            </>
          )}
        </div>
      </div>
    </>
  );
}
