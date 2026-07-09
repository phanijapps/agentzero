import { useState } from "react";
import { ChevronDown, ChevronRight, Loader2 } from "lucide-react";
import { getTransport } from "@/services/transport";
import type { ProviderResponse, McpServerConfig } from "@/services/transport";

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

interface ReviewStepProps {
  agentName: string;
  aboutMe: string;
  providers: ProviderResponse[];
  defaultProviderId: string;
  enabledSkillIds: string[];
  mcpConfigs: McpServerConfig[];
  globalDefault: GlobalDefault;
  agentOverrides: Record<string, AgentOverride>;
  originalAgentName: string;
  originalAgentConfigs: Record<string, GlobalDefault>;
  originalMcpIds: string[];
  onLaunchComplete: () => void;
}

export function ReviewStep({
  agentName,
  aboutMe,
  providers,
  defaultProviderId,
  enabledSkillIds,
  mcpConfigs,
  globalDefault,
  agentOverrides,
  originalAgentName,
  originalAgentConfigs,
  originalMcpIds,
  onLaunchComplete,
}: ReviewStepProps) {
  const [isLaunching, setIsLaunching] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [openSections, setOpenSections] = useState<Set<string>>(new Set(["identity", "providers", "agents"]));

  const toggleSection = (id: string) => {
    const next = new Set(openSections);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    setOpenSections(next);
  };

  const enabledMcps = mcpConfigs.filter((c) => c.enabled);
  const overrideCount = Object.keys(agentOverrides).length;

  const hasAgentConfigChanged = (
    original: GlobalDefault | undefined,
    desired: GlobalDefault,
  ): boolean => {
    if (!original) return true;
    return (
      original.providerId !== desired.providerId
      || original.model !== desired.model
      || original.temperature !== desired.temperature
      || original.maxInputTokens !== desired.maxInputTokens
      || original.maxOutputTokens !== desired.maxOutputTokens
    );
  };

  const handleLaunch = async () => {
    setIsLaunching(true);
    setError(null);
    try {
      const transport = await getTransport();

      // 1. Get all agents
      const agentsResult = await transport.listAgents();
      if (!agentsResult.success || !agentsResult.data) {
        throw new Error("Failed to load agents");
      }
      const agents = agentsResult.data;
      const rootAgent = agents.find((a) => a.name === "root" || a.id === "root" || a.displayName === originalAgentName);

      // 2. Set root agent display name — only if name changed
      // IMPORTANT: only update displayName, never change name/id from "root"
      // The entire system (memory, recall, delegation) keys on agent_id = "root"
      if (rootAgent && agentName !== originalAgentName) {
        await transport.updateAgent(rootAgent.id, {
          displayName: agentName,
        });
      }

      // 3. Set default provider
      if (defaultProviderId) {
        await transport.setDefaultProvider(defaultProviderId);
      }

      // 4. Update agent configs — only agents whose config changed
      for (const agent of agents) {
        const override = agentOverrides[agent.id];
        const desired = override
          ? {
              providerId: override.providerId || globalDefault.providerId,
              model: override.model || globalDefault.model,
              temperature: override.temperature ?? globalDefault.temperature,
              maxInputTokens: override.maxInputTokens ?? globalDefault.maxInputTokens,
              maxOutputTokens: override.maxOutputTokens ?? override.maxTokens ?? globalDefault.maxOutputTokens ?? globalDefault.maxTokens,
              maxTokens: override.maxOutputTokens ?? override.maxTokens ?? globalDefault.maxOutputTokens ?? globalDefault.maxTokens,
            }
          : globalDefault;

        const original = originalAgentConfigs[agent.id];
        if (hasAgentConfigChanged(original, desired)) {
          await transport.updateAgent(agent.id, {
            providerId: desired.providerId,
            model: desired.model,
            temperature: desired.temperature,
            maxInputTokens: desired.maxInputTokens,
            maxOutputTokens: desired.maxOutputTokens,
          });
        }
      }

      // 5. Create MCP servers — only new ones (not already existing)
      const existingMcpSet = new Set(originalMcpIds);
      for (const mcp of enabledMcps) {
        if (!existingMcpSet.has(mcp.id || "")) {
          await transport.createMcp({
            type: mcp.type,
            id: mcp.id,
            name: mcp.name,
            description: mcp.description,
            command: mcp.command,
            args: mcp.args,
            env: mcp.env,
            url: mcp.url,
            headers: mcp.headers,
            enabled: true,
          });
        }
      }

      // 6. Save About Me as a pinned user memory fact
      if (aboutMe.trim()) {
        await transport.createMemory("root", {
          category: "user",
          key: "user.profile",
          content: aboutMe.trim(),
          confidence: 0.95,
          pinned: true,
        });
      }

      // 7. Mark setup complete + persist agent name (also updates SOUL.md via gateway)
      const execResult = await transport.getExecutionSettings();
      const currentExec = execResult.data || { maxParallelAgents: 2, setupComplete: false };
      await transport.updateExecutionSettings({
        ...currentExec,
        setupComplete: true,
        agentName: agentName,
      });

      onLaunchComplete();
    } catch (err) {
      setError(err instanceof Error ? err.message : "Something went wrong");
    } finally {
      setIsLaunching(false);
    }
  };

  const getProviderName = (id: string) => providers.find((p) => p.id === id)?.name || id;

  return (
    <div>
      <Section id="identity" title="Agent Identity" count={agentName} open={openSections} onToggle={toggleSection}>
        <div className="review-item">
          <span className="review-item__label">Name</span>
          <span className="review-item__value">{agentName}</span>
        </div>
        {aboutMe && (
          <div className="review-item">
            <span className="review-item__label">About You</span>
            <span className="review-item__value">{aboutMe}</span>
          </div>
        )}
      </Section>

      <Section id="providers" title="Providers" count={`${providers.length} connected`} open={openSections} onToggle={toggleSection}>
        {providers.map((p) => (
          <div key={p.id} className="review-item">
            <span className="review-item__label">{p.name}</span>
            <span className="review-item__value">
              {p.models.length} models
              {p.id === defaultProviderId ? " (default)" : ""}
            </span>
          </div>
        ))}
      </Section>

      <Section id="skills" title="Skills" count={`${enabledSkillIds.length} enabled`} open={openSections} onToggle={toggleSection}>
        {enabledSkillIds.length > 0 ? (
          <p className="settings-hint">{enabledSkillIds.join(", ")}</p>
        ) : (
          <p className="settings-hint">No skills selected</p>
        )}
      </Section>

      <Section id="mcps" title="MCP Servers" count={`${enabledMcps.length} enabled`} open={openSections} onToggle={toggleSection}>
        {enabledMcps.length > 0 ? (
          enabledMcps.map((m) => (
            <div key={m.id} className="review-item">
              <span className="review-item__label">{m.name}</span>
              <span className="review-item__value">{m.type}</span>
            </div>
          ))
        ) : (
          <p className="settings-hint">No MCP servers enabled</p>
        )}
      </Section>

      <Section id="agents" title="Agent Config" count={overrideCount > 0 ? `${overrideCount} customized` : "all default"} open={openSections} onToggle={toggleSection}>
        <div className="review-item">
          <span className="review-item__label">Default</span>
          <span className="review-item__value">
            {getProviderName(globalDefault.providerId)} / {globalDefault.model} / {globalDefault.temperature} / {globalDefault.maxInputTokens} in / {globalDefault.maxOutputTokens} out
          </span>
        </div>
        {Object.entries(agentOverrides).map(([agentId, override]) => (
          <div key={agentId} className="review-item">
            <span className="review-item__label">{agentId}</span>
            <span className="review-item__value">
              {getProviderName(override.providerId || globalDefault.providerId)} / {override.model || globalDefault.model}
            </span>
          </div>
        ))}
      </Section>

      {error && <div className="alert alert--error">{error}</div>}

      <div>
        <button
          className="btn btn--primary btn--lg"
          onClick={handleLaunch}
          disabled={isLaunching}
        >
          {isLaunching ? <><Loader2 className="loading-spinner__icon" /> Launching...</> : "Launch"}
        </button>
      </div>
    </div>
  );
}

function Section({ id, title, count, open, onToggle, children }: {
  id: string; title: string; count: string;
  open: Set<string>; onToggle: (id: string) => void;
  children: React.ReactNode;
}) {
  const isOpen = open.has(id);
  return (
    <div className="review-section">
      <div className="review-section__header" onClick={() => onToggle(id)} role="button" tabIndex={0} onKeyDown={(e) => { if (e.key === "Enter" || e.key === " ") onToggle(id); }}>
        <span className="review-section__title">{title}</span>
        <div className="flex items-center gap-2">
          <span className="review-section__count">{count}</span>
          {isOpen ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
        </div>
      </div>
      {isOpen && <div className="review-section__body">{children}</div>}
    </div>
  );
}
