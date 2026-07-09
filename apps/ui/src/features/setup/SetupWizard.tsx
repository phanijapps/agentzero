import { useReducer, useCallback, useEffect, useState } from "react";
import { useNavigate } from "react-router-dom";
import { Loader2 } from "lucide-react";
import { getTransport } from "@/services/transport";
import type { ProviderResponse, McpServerConfig } from "@/services/transport";
import { DEFAULT_NAME_PRESET, NAME_PRESETS } from "./presets";
import { HelpBox } from "@/components/HelpBox";
import { StepIndicator } from "./components/StepIndicator";
import { WizardNav } from "./components/WizardNav";
import { NameStep } from "./steps/NameStep";
import { ProvidersStep } from "./steps/ProvidersStep";
import { SkillsStep } from "./steps/SkillsStep";
import { McpStep } from "./steps/McpStep";
import { AgentsStep } from "./steps/AgentsStep";
import { ReviewStep } from "./steps/ReviewStep";

interface AgentConfig {
  providerId: string;
  model: string;
  temperature: number;
  maxInputTokens: number;
  maxOutputTokens: number;
  /** Legacy alias for maxOutputTokens. */
  maxTokens: number;
}

interface WizardState {
  currentStep: number;
  agentName: string;
  namePreset: string | null;
  aboutMe: string;
  providers: ProviderResponse[];
  defaultProviderId: string;
  enabledSkillIds: string[];
  mcpConfigs: McpServerConfig[];
  globalDefault: AgentConfig;
  agentOverrides: Record<string, Partial<AgentConfig>>;
  // Original state for delta detection
  originalAgentName: string;
  originalAgentConfigs: Record<string, AgentConfig>;
  originalMcpIds: string[];
}

type WizardAction =
  | { type: "SET_STEP"; step: number }
  | { type: "SET_NAME"; name: string; preset: string | null }
  | { type: "SET_ABOUT_ME"; aboutMe: string }
  | { type: "SET_PROVIDERS"; providers: ProviderResponse[]; defaultId: string }
  | { type: "SET_SKILLS"; ids: string[] }
  | { type: "SET_MCPS"; configs: McpServerConfig[] }
  | { type: "SET_GLOBAL_DEFAULT"; defaults: WizardState["globalDefault"] }
  | { type: "SET_OVERRIDES"; overrides: WizardState["agentOverrides"] }
  | { type: "HYDRATE"; state: Partial<WizardState> };

function reducer(state: WizardState, action: WizardAction): WizardState {
  switch (action.type) {
    case "SET_STEP": return { ...state, currentStep: action.step };
    case "SET_NAME": return { ...state, agentName: action.name, namePreset: action.preset };
    case "SET_ABOUT_ME": return { ...state, aboutMe: action.aboutMe };
    case "SET_PROVIDERS": return { ...state, providers: action.providers, defaultProviderId: action.defaultId };
    case "SET_SKILLS": return { ...state, enabledSkillIds: action.ids };
    case "SET_MCPS": return { ...state, mcpConfigs: action.configs };
    case "SET_GLOBAL_DEFAULT": return { ...state, globalDefault: action.defaults };
    case "SET_OVERRIDES": return { ...state, agentOverrides: action.overrides };
    case "HYDRATE": return { ...state, ...action.state };
    default: return state;
  }
}

const LEGACY_DEFAULT_USER_PROFILE = "I am a private person, just call me Mr Z.";
const DEFAULT_MAX_INPUT_TOKENS = 200000;
const DEFAULT_MAX_OUTPUT_TOKENS = 32000;

const initialState: WizardState = {
  currentStep: 1,
  agentName: DEFAULT_NAME_PRESET.name,
  namePreset: DEFAULT_NAME_PRESET.id,
  aboutMe: "",
  providers: [],
  defaultProviderId: "",
  enabledSkillIds: [],
  mcpConfigs: [],
  globalDefault: {
    providerId: "",
    model: "",
    temperature: 0.7,
    maxInputTokens: DEFAULT_MAX_INPUT_TOKENS,
    maxOutputTokens: DEFAULT_MAX_OUTPUT_TOKENS,
    maxTokens: DEFAULT_MAX_OUTPUT_TOKENS,
  },
  agentOverrides: {},
  originalAgentName: "",
  originalAgentConfigs: {},
  originalMcpIds: [],
};

const STEP_TITLES: Record<number, { title: string; subtitle: string; help: string }> = {
  1: { title: "What should we call your agent?", subtitle: "Pick a personality or choose your own name.", help: "This name becomes your main AI assistant's identity. It appears in conversations and in the system prompt." },
  2: { title: "Connect your AI providers", subtitle: "Add at least one provider to power your agents.", help: "Providers are the AI services (OpenAI, Anthropic, etc.) that run your agents. You need at least one to get started." },
  3: { title: "Enable skills", subtitle: "Choose which skills your agents can use.", help: "Skills give your agents specialized abilities — coding, search, document processing, and more." },
  4: { title: "Configure tool servers", subtitle: "Connect external tools and services via MCP.", help: "MCP servers extend your agent with external tools like web search, GitHub access, and more. Some need API keys." },
  5: { title: "Configure your agents", subtitle: "Set a default model, then customize individual agents.", help: "Choose which AI model powers each agent. The global default applies to all agents unless you customize individually." },
  6: { title: "Review & Launch", subtitle: "Everything looks good? Hit launch to get started.", help: "Review your choices. Only changes from the current configuration will be applied." },
};

export function SetupWizard() {
  const [state, dispatch] = useReducer(reducer, initialState);
  const [isHydrating, setIsHydrating] = useState(true);
  const navigate = useNavigate();

  // On mount, load existing config for re-run mode
  useEffect(() => {
    const hydrate = async () => {
      try {
        const transport = await getTransport();
        const [providersRes, agentsRes, mcpsRes, execRes] = await Promise.all([
          transport.listProviders(),
          transport.listAgents(),
          transport.listMcps(),
          transport.getExecutionSettings(),
        ]);

        const hydrated: Partial<WizardState> = {};

        // Pre-fill agent name from settings (source of truth)
        if (execRes.success && execRes.data?.agentName) {
          const name = execRes.data.agentName;
          hydrated.agentName = name;
          hydrated.originalAgentName = name;
          const matchingPreset = NAME_PRESETS.find((p) => p.name === name && p.id !== "custom");
          hydrated.namePreset = matchingPreset?.id || "custom";
        }

        // Pre-fill About Me from memory facts (category=user, look for user.profile key)
        try {
          const memRes = await transport.listAllMemory({ category: "user" as any, limit: 20 });
          if (memRes.success && memRes.data && memRes.data.facts) {
            const profileFact = memRes.data.facts.find((f) => f.key === "user.profile");
            if (profileFact && profileFact.content.trim() !== LEGACY_DEFAULT_USER_PROFILE) {
              hydrated.aboutMe = profileFact.content;
            }
          }
        } catch { /* ignore */ }

        // Pre-fill providers
        if (providersRes.success && providersRes.data && providersRes.data.length > 0) {
          const providers = providersRes.data;
          hydrated.providers = providers;
          const defaultProvider = providers.find((p) => p.isDefault) || providers[0];
          hydrated.defaultProviderId = defaultProvider?.id || "";
        }

        // Pre-fill agent name, configs, and global default from existing agents
        if (agentsRes.success && agentsRes.data && agentsRes.data.length > 0) {
          const agents = agentsRes.data;
          const rootAgent = agents.find((a) => a.name === "root");

          // Fallback: agent name from root displayName (if not already set from settings)
          if (!hydrated.agentName && rootAgent && rootAgent.displayName && rootAgent.displayName !== "root") {
            const name = rootAgent.displayName;
            hydrated.agentName = name;
            hydrated.originalAgentName = name;
            const matchingPreset = NAME_PRESETS.find((p) => p.name === name && p.id !== "custom");
            hydrated.namePreset = matchingPreset?.id || "custom";
          }

          // Store original agent configs for delta detection
          const originalConfigs: Record<string, AgentConfig> = {};
          for (const agent of agents) {
            originalConfigs[agent.id] = {
              providerId: agent.providerId || "",
              model: agent.model || "",
              temperature: agent.temperature ?? 0.7,
              maxInputTokens: agent.maxInputTokens ?? DEFAULT_MAX_INPUT_TOKENS,
              maxOutputTokens: agent.maxOutputTokens ?? agent.maxTokens ?? DEFAULT_MAX_OUTPUT_TOKENS,
              maxTokens: agent.maxOutputTokens ?? agent.maxTokens ?? DEFAULT_MAX_OUTPUT_TOKENS,
            };
          }
          hydrated.originalAgentConfigs = originalConfigs;

          // Derive global default from root agent's actual config (or first agent)
          const baseAgent = rootAgent || agents[0];
          const globalDefault: AgentConfig = {
            providerId: baseAgent.providerId || hydrated.defaultProviderId || "",
            model: baseAgent.model || "",
            temperature: baseAgent.temperature ?? 0.7,
            maxInputTokens: baseAgent.maxInputTokens ?? DEFAULT_MAX_INPUT_TOKENS,
            maxOutputTokens: baseAgent.maxOutputTokens ?? baseAgent.maxTokens ?? DEFAULT_MAX_OUTPUT_TOKENS,
            maxTokens: baseAgent.maxOutputTokens ?? baseAgent.maxTokens ?? DEFAULT_MAX_OUTPUT_TOKENS,
          };
          hydrated.globalDefault = globalDefault;

          // Any specialist whose config differs from global gets an override
          const overrides: Record<string, Partial<AgentConfig>> = {};
          for (const agent of agents) {
            if (agent === baseAgent) continue;
            const config = originalConfigs[agent.id];
            if (
              config.providerId !== globalDefault.providerId ||
              config.model !== globalDefault.model ||
              config.temperature !== globalDefault.temperature ||
              config.maxInputTokens !== globalDefault.maxInputTokens ||
              config.maxOutputTokens !== globalDefault.maxOutputTokens
            ) {
              overrides[agent.id] = {
                ...(config.providerId !== globalDefault.providerId && { providerId: config.providerId }),
                ...(config.model !== globalDefault.model && { model: config.model }),
                ...(config.temperature !== globalDefault.temperature && { temperature: config.temperature }),
                ...(config.maxInputTokens !== globalDefault.maxInputTokens && { maxInputTokens: config.maxInputTokens }),
                ...(config.maxOutputTokens !== globalDefault.maxOutputTokens && {
                  maxOutputTokens: config.maxOutputTokens,
                  maxTokens: config.maxOutputTokens,
                }),
              };
            }
          }
          if (Object.keys(overrides).length > 0) {
            hydrated.agentOverrides = overrides;
          }
        } else if (hydrated.defaultProviderId) {
          // No agents yet (first run) — set global default from provider
          const defaultProvider = hydrated.providers?.find((p) => p.id === hydrated.defaultProviderId);
          if (defaultProvider) {
            hydrated.globalDefault = {
              providerId: defaultProvider.id!,
              model: defaultProvider.defaultModel || defaultProvider.models[0] || "",
              temperature: 0.7,
              maxInputTokens: DEFAULT_MAX_INPUT_TOKENS,
              maxOutputTokens: DEFAULT_MAX_OUTPUT_TOKENS,
              maxTokens: DEFAULT_MAX_OUTPUT_TOKENS,
            };
          }
        }

        // Pre-fill MCP configs from existing servers + track originals
        if (mcpsRes.success && mcpsRes.data && mcpsRes.data.servers && mcpsRes.data.servers.length > 0) {
          hydrated.mcpConfigs = mcpsRes.data.servers.map((s) => ({
            type: s.type as McpServerConfig["type"],
            id: s.id,
            name: s.name,
            description: s.description,
            enabled: s.enabled,
          }));
          hydrated.originalMcpIds = mcpsRes.data.servers.map((s) => s.id);
        }

        if (Object.keys(hydrated).length > 0) {
          dispatch({ type: "HYDRATE", state: hydrated });
        }
      } catch {
        // First run — no existing config, use defaults
      } finally {
        setIsHydrating(false);
      }
    };
    hydrate();
  }, []);

  const canNext = (): boolean => {
    switch (state.currentStep) {
      case 1: return state.agentName.trim().length > 0;
      case 2: return state.providers.some((p) => p.verified);
      case 3: return true;
      case 4: return true;
      case 5: return !!state.globalDefault.providerId && !!state.globalDefault.model;
      case 6: return true;
      default: return false;
    }
  };

  const handleNext = () => {
    if (state.currentStep < 6) {
      dispatch({ type: "SET_STEP", step: state.currentStep + 1 });
    }
  };

  const handleBack = () => {
    if (state.currentStep > 1) {
      dispatch({ type: "SET_STEP", step: state.currentStep - 1 });
    }
  };

  const handleSkip = useCallback(() => {
    navigate("/");
  }, [navigate]);

  const handleLaunchComplete = useCallback(() => {
    sessionStorage.setItem("setupComplete", "true");
    navigate("/");
  }, [navigate]);

  const isSkippable = state.currentStep === 3 || state.currentStep === 4;
  const stepInfo = STEP_TITLES[state.currentStep];

  if (isHydrating) {
    return (
      <div className="setup-wizard">
        <div className="settings-loading">
          <Loader2 className="loading-spinner__icon" />
        </div>
      </div>
    );
  }

  return (
    <div className="setup-wizard">
      <div className="setup-wizard__logo">
        <img src="/logo.svg" alt="z-Bot" className="setup-wizard__logo-light" />
        <img src="/logo-dark.svg" alt="z-Bot" className="setup-wizard__logo-dark" />
      </div>

      <div className="setup-wizard__container">
        <StepIndicator currentStep={state.currentStep} />

        {state.currentStep < 6 && (
          <WizardNav
            currentStep={state.currentStep}
            canNext={canNext()}
            onBack={handleBack}
            onNext={handleNext}
            onSkip={isSkippable || state.currentStep === 1 ? handleSkip : undefined}
          />
        )}

        <div className="setup-wizard__header">
          <h2 className="setup-wizard__title">{stepInfo.title}</h2>
          <p className="setup-wizard__subtitle">{stepInfo.subtitle}</p>
        </div>

        <div className="setup-wizard__body">
          {state.currentStep === 1 && (
            <NameStep
              agentName={state.agentName}
              namePreset={state.namePreset}
              aboutMe={state.aboutMe}
              onChange={(name, preset) => dispatch({ type: "SET_NAME", name, preset })}
              onAboutMeChange={(aboutMe) => dispatch({ type: "SET_ABOUT_ME", aboutMe })}
            />
          )}
          {state.currentStep === 2 && (
            <ProvidersStep
              providers={state.providers}
              defaultProviderId={state.defaultProviderId}
              onProvidersChanged={(providers, defaultId) =>
                dispatch({ type: "SET_PROVIDERS", providers, defaultId })
              }
            />
          )}
          {state.currentStep === 3 && (
            <SkillsStep
              enabledSkillIds={state.enabledSkillIds}
              onChange={(ids) => dispatch({ type: "SET_SKILLS", ids })}
            />
          )}
          {state.currentStep === 4 && (
            <McpStep
              mcpConfigs={state.mcpConfigs}
              onChange={(configs) => dispatch({ type: "SET_MCPS", configs })}
            />
          )}
          {state.currentStep === 5 && (
            <AgentsStep
              providers={state.providers}
              defaultProviderId={state.defaultProviderId}
              agentName={state.agentName}
              globalDefault={state.globalDefault}
              agentOverrides={state.agentOverrides}
              onGlobalChange={(defaults) => {
                dispatch({ type: "SET_GLOBAL_DEFAULT", defaults });
                // Keep defaultProviderId in sync — the global default provider IS the default
                if (defaults.providerId && defaults.providerId !== state.defaultProviderId) {
                  dispatch({ type: "SET_PROVIDERS", providers: state.providers, defaultId: defaults.providerId });
                }
              }}
              onOverrideChange={(overrides) => dispatch({ type: "SET_OVERRIDES", overrides })}
            />
          )}
          {state.currentStep === 6 && (
            <ReviewStep
              agentName={state.agentName}
              aboutMe={state.aboutMe}
              providers={state.providers}
              defaultProviderId={state.defaultProviderId}
              enabledSkillIds={state.enabledSkillIds}
              mcpConfigs={state.mcpConfigs}
              globalDefault={state.globalDefault}
              agentOverrides={state.agentOverrides}
              originalAgentName={state.originalAgentName}
              originalAgentConfigs={state.originalAgentConfigs}
              originalMcpIds={state.originalMcpIds}
              onLaunchComplete={handleLaunchComplete}
            />
          )}

          <div className="setup-wizard__help">
            <HelpBox>{stepInfo.help}</HelpBox>
          </div>
        </div>
      </div>
    </div>
  );
}
