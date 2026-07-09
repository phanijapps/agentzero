import { useEffect, useState } from "react";
import { Loader2 } from "lucide-react";
import { getTransport } from "@/services/transport";
import type { McpServerConfig } from "@/services/transport";

interface McpStepProps {
  mcpConfigs: McpServerConfig[];
  onChange: (configs: McpServerConfig[]) => void;
}

export function McpStep({ mcpConfigs, onChange }: McpStepProps) {
  const [isLoading, setIsLoading] = useState(true);

  useEffect(() => {
    const load = async () => {
      try {
        const transport = await getTransport();
        const result = await transport.getMcpDefaults();
        if (result.success && result.data && mcpConfigs.length === 0) {
          const configs = result.data.map((c: McpServerConfig) => ({
            ...c,
            enabled: Boolean(c.enabled) && !hasEnvKeys(c),
          }));
          onChange(configs);
        }
      } finally {
        setIsLoading(false);
      }
    };
    load();
  }, []);

  if (isLoading) {
    return <div className="settings-loading"><Loader2 className="loading-spinner__icon" /></div>;
  }

  const keyless = mcpConfigs.filter((c) => !hasEnvKeys(c));
  const needsKey = mcpConfigs.filter((c) => hasEnvKeys(c));

  const toggleServer = (id: string) => {
    onChange(
      mcpConfigs.map((c) =>
        c.id === id ? { ...c, enabled: !c.enabled } : c
      )
    );
  };

  const updateEnvKey = (serverId: string, envKey: string, value: string) => {
    onChange(
      mcpConfigs.map((c) =>
        c.id === serverId
          ? { ...c, env: { ...c.env, [envKey]: value } }
          : c
      )
    );
  };

  return (
    <div>
      {keyless.length > 0 && (
        <div className="mcp-section">
          <div className="mcp-section__title">Ready to use</div>
          {keyless.map((server) => (
            <div key={server.id} className="mcp-row">
              <div className="mcp-row__info">
                <div className="mcp-row__name">{server.name}</div>
                <div className="mcp-row__desc">{server.description}</div>
              </div>
              <div className="mcp-row__toggle">
                <div
                  className={`toggle-switch ${server.enabled ? "toggle-switch--on" : "toggle-switch--off"}`}
                  onClick={() => toggleServer(server.id!)}
                  role="button"
                  tabIndex={0}
                  onKeyDown={(e: React.KeyboardEvent) => { if (e.key === "Enter" || e.key === " ") toggleServer(server.id!); }}
                />
              </div>
            </div>
          ))}
        </div>
      )}

      {needsKey.length > 0 && (
        <div className="mcp-section">
          <div className="mcp-section__title">Requires API key</div>
          {needsKey.map((server) => {
            const envKeys = getEmptyEnvKeys(server);
            return (
              <div key={server.id} className="mcp-row">
                <div className="mcp-row__info">
                  <div className="mcp-row__name">{server.name}</div>
                  <div className="mcp-row__desc">{server.description}</div>
                  {envKeys.map((envKey) => (
                    <div key={envKey} className="mcp-row__key-input">
                      <input
                        className="form-input"
                        placeholder={envKey}
                        type="password"
                        value={server.env?.[envKey] || ""}
                        onChange={(e) => updateEnvKey(server.id!, envKey, e.target.value)}
                      />
                    </div>
                  ))}
                </div>
                <div className="mcp-row__toggle">
                  <div
                    className={`toggle-switch ${server.enabled ? "toggle-switch--on" : "toggle-switch--off"}`}
                    role="button"
                    tabIndex={0}
                    aria-label={`Toggle ${server.name || server.id}`}
                    onClick={() => toggleServer(server.id!)}
                    onKeyDown={(e) => { if (e.key === "Enter" || e.key === " ") toggleServer(server.id!); }}
                  />
                </div>
              </div>
            );
          })}
        </div>
      )}

      {mcpConfigs.length === 0 && (
        <p className="settings-hint">No MCP server templates available.</p>
      )}
    </div>
  );
}

function hasEnvKeys(config: McpServerConfig): boolean {
  if (!config.env) return false;
  return Object.values(config.env).some((v) => v === "" || v === undefined);
}

function getEmptyEnvKeys(config: McpServerConfig): string[] {
  if (!config.env) return [];
  return Object.entries(config.env)
    .filter(([, v]) => v === "" || v === undefined)
    .map(([k]) => k);
}
