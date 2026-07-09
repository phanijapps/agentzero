import { useEffect, useState, useRef } from "react";
import { Loader2 } from "lucide-react";
import { getTransport } from "@/services/transport";
import type { ProviderResponse, AgentResponse } from "@/services/transport";

interface GlobalDefault {
  providerId: string;
  model: string;
  temperature: number;
  maxInputTokens: number;
  maxOutputTokens: number;
  /** Legacy alias for maxOutputTokens. */
  maxTokens: number;
}

interface AgentOverride {
  providerId?: string;
  model?: string;
  temperature?: number;
  maxInputTokens?: number;
  maxOutputTokens?: number;
  /** Legacy alias for maxOutputTokens. */
  maxTokens?: number;
}

interface AgentsStepProps {
  providers: ProviderResponse[];
  defaultProviderId: string;
  agentName: string;
  globalDefault: GlobalDefault;
  agentOverrides: Record<string, AgentOverride>;
  onGlobalChange: (defaults: GlobalDefault) => void;
  onOverrideChange: (overrides: Record<string, AgentOverride>) => void;
}

export function AgentsStep({
  providers,
  defaultProviderId,
  agentName,
  globalDefault,
  agentOverrides,
  onGlobalChange,
  onOverrideChange,
}: AgentsStepProps) {
  const [agents, setAgents] = useState<AgentResponse[]>([]);
  const [loadedProviders, setLoadedProviders] = useState<ProviderResponse[]>([]);
  const [isLoading, setIsLoading] = useState(true);
  const [expandedAgent, setExpandedAgent] = useState<string | null>(null);
  const initDone = useRef(false);
  const defaultMaxInputTokens = 200000;
  const defaultMaxOutputTokens = 32000;

  // The providers we actually use for rendering — loaded from API, fallback to props
  const allProviders = loadedProviders.length > 0 ? loadedProviders : providers;

  // Derive models for the currently selected provider
  const selectedProvider = allProviders.find((p) => p.id === globalDefault.providerId);
  const globalModels = selectedProvider?.models || [];

  // Load agents + fresh providers on mount
  useEffect(() => {
    const load = async () => {
      try {
        const transport = await getTransport();
        const [agentsRes, providersRes] = await Promise.all([
          transport.listAgents(),
          transport.listProviders(),
        ]);

        if (agentsRes.success && agentsRes.data) {
          setAgents(agentsRes.data);
        }

        if (providersRes.success && providersRes.data && providersRes.data.length > 0) {
          setLoadedProviders(providersRes.data);
        }
      } finally {
        setIsLoading(false);
      }
    };
    load();
  }, []);

  // Once loading is done AND we have providers, initialize globalDefault if needed
  useEffect(() => {
    if (isLoading || initDone.current || allProviders.length === 0) return;
    initDone.current = true;

    const pid = globalDefault.providerId || defaultProviderId;
    const matched = allProviders.find((p) => p.id === pid);
    const provider = matched || allProviders[0];
    const models = provider?.models || [];
    const modelValid = globalDefault.model && models.includes(globalDefault.model);

    if (!matched || !modelValid) {
      onGlobalChange({
        providerId: provider.id!,
        model: modelValid ? globalDefault.model : (provider.defaultModel || models[0] || ""),
        temperature: globalDefault.temperature || 0.7,
        maxInputTokens: globalDefault.maxInputTokens || defaultMaxInputTokens,
        maxOutputTokens: globalDefault.maxOutputTokens || globalDefault.maxTokens || defaultMaxOutputTokens,
        maxTokens: globalDefault.maxOutputTokens || globalDefault.maxTokens || defaultMaxOutputTokens,
      });
    }
  }, [isLoading, allProviders.length]);

  // When user changes provider in the dropdown, sync model to first of that provider
  useEffect(() => {
    if (isLoading || !initDone.current) return;
    if (globalModels.length > 0 && globalDefault.model && !globalModels.includes(globalDefault.model)) {
      onGlobalChange({ ...globalDefault, model: globalModels[0] });
    }
  }, [globalDefault.providerId]);

  if (isLoading) {
    return <div className="settings-loading"><Loader2 className="loading-spinner__icon" /></div>;
  }

  const getProviderModels = (providerId: string) => {
    return allProviders.find((p) => p.id === providerId)?.models || [];
  };

  const getProviderName = (providerId: string) => {
    return allProviders.find((p) => p.id === providerId)?.name || providerId;
  };

  const getEffectiveConfig = (agentId: string) => {
    const override = agentOverrides[agentId];
    if (!override) return globalDefault;
    return {
      providerId: override.providerId || globalDefault.providerId,
      model: override.model || globalDefault.model,
      temperature: override.temperature ?? globalDefault.temperature,
      maxInputTokens: override.maxInputTokens ?? globalDefault.maxInputTokens,
      maxOutputTokens: override.maxOutputTokens ?? override.maxTokens ?? globalDefault.maxOutputTokens ?? globalDefault.maxTokens,
      maxTokens: override.maxOutputTokens ?? override.maxTokens ?? globalDefault.maxOutputTokens ?? globalDefault.maxTokens,
    };
  };

  const handleOverride = (agentId: string, field: string, value: string | number) => {
    const current = agentOverrides[agentId] || {};
    onOverrideChange({
      ...agentOverrides,
      [agentId]: { ...current, [field]: value },
    });
  };

  const handleResetOverride = (agentId: string) => {
    const { [agentId]: _, ...rest } = agentOverrides;
    onOverrideChange(rest);
    setExpandedAgent(null);
  };

  const rootAgent = agents.find((a) => a.name === "root" || a.id === "root");
  const specialists = agents.filter((a) => a !== rootAgent).sort((a, b) => a.name.localeCompare(b.name));
  const sortedAgents = rootAgent ? [rootAgent, ...specialists] : specialists;

  return (
    <div>
      {/* Global default card */}
      <div className="agent-global-card">
        <div className="agent-global-card__label">Default for all</div>
        <div className="agent-global-card__fields">
          <div className="form-group">
            <label className="form-label" htmlFor="agents-global-provider">Provider</label>
            <select
              id="agents-global-provider"
              className="form-input form-select"
              value={globalDefault.providerId}
              onChange={(e) => {
                const pid = e.target.value;
                const models = getProviderModels(pid);
                onGlobalChange({ ...globalDefault, providerId: pid, model: models[0] || "" });
              }}
            >
              {allProviders.length === 0 && (
                <option value="">No providers configured</option>
              )}
              {allProviders.map((p) => (
                <option key={p.id} value={p.id}>{p.name}</option>
              ))}
            </select>
          </div>
          <div className="form-group">
            <label className="form-label" htmlFor="agents-global-model">Model</label>
            <select
              id="agents-global-model"
              className="form-input form-select"
              value={globalDefault.model}
              onChange={(e) => onGlobalChange({ ...globalDefault, model: e.target.value })}
            >
              {globalModels.length === 0 && (
                <option value="">No models available</option>
              )}
              {globalModels.map((m) => (
                <option key={m} value={m}>{m}</option>
              ))}
            </select>
          </div>
          <div className="form-group">
            <label className="form-label" htmlFor="agents-global-temp">Temperature</label>
            <input
              id="agents-global-temp"
              className="form-input"
              type="number"
              value={globalDefault.temperature}
              onChange={(e) => onGlobalChange({ ...globalDefault, temperature: Number.parseFloat(e.target.value) || 0 })}
              min={0} max={2} step={0.1}
            />
          </div>
          <div className="form-group">
            <label className="form-label" htmlFor="agents-global-tokens">Max Output Tokens</label>
            <input
              id="agents-global-tokens"
              className="form-input"
              type="number"
              value={globalDefault.maxOutputTokens ?? globalDefault.maxTokens}
              onChange={(e) => {
                const maxOutputTokens = Number.parseInt(e.target.value) || defaultMaxOutputTokens;
                onGlobalChange({ ...globalDefault, maxOutputTokens, maxTokens: maxOutputTokens });
              }}
              min={256} step={1024}
            />
          </div>
          <div className="form-group">
            <label className="form-label" htmlFor="agents-global-input-tokens">Max Input Tokens</label>
            <input
              id="agents-global-input-tokens"
              className="form-input"
              type="number"
              value={globalDefault.maxInputTokens}
              onChange={(e) => onGlobalChange({ ...globalDefault, maxInputTokens: Number.parseInt(e.target.value) || defaultMaxInputTokens })}
              min={256} step={1024}
            />
          </div>
        </div>
      </div>

      {/* Agent list */}
      <div className="settings-field-label">Agents</div>
      <div className="agent-list">
        {sortedAgents.map((agent) => {
          const isRoot = agent === rootAgent;
          const isCustomized = !!agentOverrides[agent.id];
          const isExpanded = expandedAgent === agent.id;
          const effective = getEffectiveConfig(agent.id);
          const overrideModels = isCustomized && agentOverrides[agent.id]?.providerId
            ? getProviderModels(agentOverrides[agent.id].providerId!)
            : globalModels;

          return (
            <div key={agent.id} className={`agent-row ${isCustomized ? "agent-row--customized" : ""}`}>
              <div className="agent-row__summary">
                <div className="agent-row__info">
                  <span className="agent-row__name">{isRoot ? agentName : agent.displayName || agent.name}</span>
                  {isRoot && <span className="badge badge--primary badge--xs">root</span>}
                  {isCustomized && <span className="badge badge--warning badge--xs">customized</span>}
                </div>
                {!isExpanded ? (
                  <div className="flex items-center gap-2">
                    <span className="agent-row__config">
                      {getProviderName(effective.providerId)} &middot; {effective.model} &middot; {effective.temperature} &middot; {effective.maxInputTokens}/{effective.maxOutputTokens}
                    </span>
                    <button className="btn btn--outline btn--sm" onClick={() => setExpandedAgent(agent.id)}>
                      Customize
                    </button>
                  </div>
                ) : (
                  <button className="btn btn--ghost btn--sm" onClick={() => handleResetOverride(agent.id)}>
                    Reset to default
                  </button>
                )}
              </div>
              {isExpanded && (
                <div className="agent-row__fields">
                  <div>
                    <div className="agent-row__field-label">Provider</div>
                    <select
                      className="form-input form-select"
                      value={agentOverrides[agent.id]?.providerId || globalDefault.providerId}
                      onChange={(e) => handleOverride(agent.id, "providerId", e.target.value)}
                    >
                      {allProviders.map((p) => (
                        <option key={p.id} value={p.id}>{p.name}</option>
                      ))}
                    </select>
                  </div>
                  <div>
                    <div className="agent-row__field-label">Model</div>
                    <select
                      className="form-input form-select"
                      value={agentOverrides[agent.id]?.model || globalDefault.model}
                      onChange={(e) => handleOverride(agent.id, "model", e.target.value)}
                    >
                      {overrideModels.map((m) => (
                        <option key={m} value={m}>{m}</option>
                      ))}
                    </select>
                  </div>
                  <div>
                    <div className="agent-row__field-label">Temp</div>
                    <input
                      className="form-input"
                      type="number"
                      value={agentOverrides[agent.id]?.temperature ?? globalDefault.temperature}
                      onChange={(e) => handleOverride(agent.id, "temperature", Number.parseFloat(e.target.value) || 0)}
                      min={0} max={2} step={0.1}
                    />
                  </div>
                  <div>
                    <div className="agent-row__field-label">Input Tokens</div>
                    <input
                      className="form-input"
                      type="number"
                      value={agentOverrides[agent.id]?.maxInputTokens ?? globalDefault.maxInputTokens}
                      onChange={(e) => handleOverride(agent.id, "maxInputTokens", Number.parseInt(e.target.value) || defaultMaxInputTokens)}
                      min={256} step={1024}
                    />
                  </div>
                  <div>
                    <div className="agent-row__field-label">Output Tokens</div>
                    <input
                      className="form-input"
                      type="number"
                      value={agentOverrides[agent.id]?.maxOutputTokens ?? agentOverrides[agent.id]?.maxTokens ?? globalDefault.maxOutputTokens ?? globalDefault.maxTokens}
                      onChange={(e) => {
                        const maxOutputTokens = Number.parseInt(e.target.value) || defaultMaxOutputTokens;
                        const current = agentOverrides[agent.id] || {};
                        onOverrideChange({
                          ...agentOverrides,
                          [agent.id]: { ...current, maxOutputTokens, maxTokens: maxOutputTokens },
                        });
                      }}
                      min={256} step={1024}
                    />
                  </div>
                </div>
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
}
