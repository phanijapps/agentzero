// ============================================================================
// WEB AGENTS PANEL
// Consolidated Agents page: My Agents | Skills Library | Schedules
// ============================================================================

import { useState, useEffect, useCallback, useMemo } from "react";
import { useSearchParams } from "react-router-dom";
import {
  Bot, Plus, Trash2, Pencil, Loader2, Zap, Play, Pause,
  Calendar, Server, X, RefreshCw,
} from "lucide-react";
import {
  getTransport,
  getProviderDefaultModel,
  type AgentResponse,
  type CreateAgentRequest,
  type ProviderResponse,
  type ModelRegistryResponse,
  type SkillResponse,
  type CreateSkillRequest,
  type CronJobResponse,
  type CreateCronJobRequest,
  type UpdateCronJobRequest,
  type Transport,
} from "@/services/transport";
import { TabBar, TabPanel } from "@/components/TabBar";
import { HelpBox } from "@/components/HelpBox";
import { ActionBar } from "@/components/ActionBar";
import { MetaChip } from "@/components/MetaChip";
import { Slideover } from "@/components/Slideover";
import { EmptyState } from "@/shared/ui/EmptyState";
import { ModelChip } from "@/shared/ui/ModelChip";
import { AgentEditPanel } from "./AgentEditPanel";

// ============================================================================
// Constants
// ============================================================================

const DEFAULT_MAX_INPUT_TOKENS = 200000;
const DEFAULT_MAX_OUTPUT_TOKENS = 32000;

// 6-field cron (sec min hour day month weekday) — the format
// `tokio-cron-scheduler` requires. The backend also accepts 5-field
// input and prepends `0 ` automatically, but we send canonical 6-field
// from here so the UI matches what gets persisted.
const CRON_PRESETS = [
  { label: "Every minute", value: "0 * * * * *" },
  { label: "Every 5 minutes", value: "0 */5 * * * *" },
  { label: "Every 15 minutes", value: "0 */15 * * * *" },
  { label: "Every hour", value: "0 0 * * * *" },
  { label: "Every 6 hours", value: "0 0 */6 * * *" },
  { label: "Daily at midnight", value: "0 0 0 * * *" },
  { label: "Daily at 9 AM", value: "0 0 9 * * *" },
  { label: "Weekly on Monday", value: "0 0 0 * * 1" },
  { label: "Monthly on the 1st", value: "0 0 0 1 * *" },
];

const AGENT_EMOJIS = [
  "\u{1F916}", "\u{1F9E0}", "\u{26A1}", "\u{1F4A1}", "\u{1F680}", "\u{2B50}", "\u{1F3AF}", "\u{1F525}", "\u{1F48E}", "\u{1F50D}",
  "\u{1F4DD}", "\u{1F517}", "\u{1F30D}", "\u{1F4CA}", "\u{1F527}",
];

function getAgentEmoji(id: string): string {
  let hash = 0;
  for (let i = 0; i < id.length; i++) {
    hash = Math.trunc((hash << 5) - hash + (id.codePointAt(i) ?? 0));
  }
  return AGENT_EMOJIS[Math.abs(hash) % AGENT_EMOJIS.length];
}

function describeCron(schedule: string): string {
  const preset = CRON_PRESETS.find((p) => p.value === schedule);
  if (preset) return preset.label;
  return schedule;
}

function formatTime(timestamp?: string): string {
  if (!timestamp) return "Never";
  return new Date(timestamp).toLocaleString();
}

function generateId(name: string): string {
  return name
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-|-$/g, "");
}

// ============================================================================
// Component
// ============================================================================

export function WebAgentsPanel() {
  const [searchParams, setSearchParams] = useSearchParams();
  const activeTab = searchParams.get("tab") || "agents";

  const setActiveTab = useCallback(
    (tab: string) => {
      setSearchParams(tab === "agents" ? {} : { tab });
    },
    [setSearchParams],
  );

  // ── Shared state ──
  const [transport, setTransport] = useState<Transport | null>(null);
  const [isLoading, setIsLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  // ── Agent state ──
  const [agents, setAgents] = useState<AgentResponse[]>([]);
  const [providers, setProviders] = useState<ProviderResponse[]>([]);
  const [modelRegistry, setModelRegistry] = useState<ModelRegistryResponse>({});
  const [agentSearch, setAgentSearch] = useState("");
  const [editingAgent, setEditingAgent] = useState<AgentResponse | null>(null);
  const [isCreatingAgent, setIsCreatingAgent] = useState(false);
  const [newAgent, setNewAgent] = useState<Partial<CreateAgentRequest>>({
    name: "",
    displayName: "",
    description: "",
    providerId: "",
    model: "",
    temperature: 0.7,
    maxInputTokens: DEFAULT_MAX_INPUT_TOKENS,
    maxOutputTokens: DEFAULT_MAX_OUTPUT_TOKENS,
    maxTokens: DEFAULT_MAX_OUTPUT_TOKENS,
  });

  // ── Skills state ──
  const [skills, setSkills] = useState<SkillResponse[]>([]);
  const [skillSearch, setSkillSearch] = useState("");
  const [selectedSkill, setSelectedSkill] = useState<SkillResponse | null>(null);
  const [isCreatingSkill, setIsCreatingSkill] = useState(false);
  const [newSkill, setNewSkill] = useState<Partial<CreateSkillRequest>>({
    name: "",
    displayName: "",
    description: "",
    category: "general",
    instructions: "You are a helpful skill.",
  });

  // ── Schedules state ──
  const [schedules, setSchedules] = useState<CronJobResponse[]>([]);
  const [scheduleSearch, setScheduleSearch] = useState("");
  const [editingSchedule, setEditingSchedule] = useState<CronJobResponse | null>(null);
  const [isCreatingSchedule, setIsCreatingSchedule] = useState(false);
  const [scheduleForm, setScheduleForm] = useState({
    id: "",
    name: "",
    schedule: "0 0 * * * *",
    agent_id: "root",
    message: "",
    enabled: true,
    timezone: "",
  });
  const [triggeringJob, setTriggeringJob] = useState<string | null>(null);

  // ── Initialize ──
  useEffect(() => {
    getTransport().then(setTransport);
  }, []);

  const applyProvidersResult = (data: typeof providers) => {
    setProviders(data);
    if (!newAgent.providerId && data.length > 0) {
      const def = data[0];
      setNewAgent((prev) => ({
        ...prev,
        providerId: def.id || "",
        model: getProviderDefaultModel(def),
      }));
    }
  };

  const loadAllData = useCallback(async () => {
    if (!transport) return;
    setIsLoading(true);
    try {
      const [agentsRes, providersRes, modelsRes, skillsRes, schedulesRes] = await Promise.all([
        transport.listAgents(),
        transport.listProviders(),
        transport.listModels(),
        transport.listSkills(),
        transport.listCronJobs(),
      ]);
      if (agentsRes.success && agentsRes.data) setAgents(agentsRes.data);
      if (providersRes.success && providersRes.data) applyProvidersResult(providersRes.data);
      if (modelsRes.data) setModelRegistry(modelsRes.data);
      if (skillsRes.success && skillsRes.data) setSkills(skillsRes.data);
      if (schedulesRes.success && schedulesRes.data) setSchedules(schedulesRes.data);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to load data");
    } finally {
      setIsLoading(false);
    }
  }, [transport]); // eslint-disable-line react-hooks/exhaustive-deps

  useEffect(() => {
    loadAllData();
  }, [loadAllData]);

  // ── Agent CRUD ──
  const reloadAgents = useCallback(async () => {
    if (!transport) return;
    try {
      const res = await transport.listAgents();
      if (res.success && res.data) setAgents(res.data);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to reload agents");
    }
  }, [transport]);

  const handleCreateAgent = async () => {
    if (!transport || !newAgent.name) return;
    try {
      const result = await transport.createAgent({
        name: newAgent.name,
        displayName: newAgent.displayName || newAgent.name,
        description: newAgent.description,
        providerId: newAgent.providerId || "openai",
        model: newAgent.model || "gpt-4o",
        temperature: newAgent.temperature,
        maxInputTokens: newAgent.maxInputTokens ?? DEFAULT_MAX_INPUT_TOKENS,
        maxOutputTokens: newAgent.maxOutputTokens ?? newAgent.maxTokens ?? DEFAULT_MAX_OUTPUT_TOKENS,
      });
      if (result.success) {
        setIsCreatingAgent(false);
        const defaultProvider = providers[0];
        setNewAgent({
          name: "",
          displayName: "",
          description: "",
          providerId: defaultProvider?.id || "",
          model: defaultProvider ? getProviderDefaultModel(defaultProvider) : "",
          temperature: 0.7,
          maxInputTokens: DEFAULT_MAX_INPUT_TOKENS,
          maxOutputTokens: DEFAULT_MAX_OUTPUT_TOKENS,
          maxTokens: DEFAULT_MAX_OUTPUT_TOKENS,
        });
        reloadAgents();
      } else {
        setError(result.error || "Failed to create agent");
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to create agent");
    }
  };

  const handleDeleteAgent = async (id: string) => {
    if (!transport) return;
    if (!confirm("Are you sure you want to delete this agent?")) return;
    try {
      const result = await transport.deleteAgent(id);
      if (result.success) {
        reloadAgents();
      } else {
        setError(result.error || "Failed to delete agent");
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to delete agent");
    }
  };

  // ── Skill CRUD ──
  const reloadSkills = useCallback(async () => {
    if (!transport) return;
    try {
      const res = await transport.listSkills();
      if (res.success && res.data) setSkills(res.data);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to reload skills");
    }
  }, [transport]);

  const handleCreateSkill = async () => {
    if (!transport || !newSkill.name) return;
    try {
      const result = await transport.createSkill({
        name: newSkill.name,
        displayName: newSkill.displayName || newSkill.name,
        description: newSkill.description,
        category: newSkill.category || "general",
        instructions: newSkill.instructions,
      });
      if (result.success) {
        setIsCreatingSkill(false);
        setNewSkill({
          name: "",
          displayName: "",
          description: "",
          category: "general",
          instructions: "You are a helpful skill.",
        });
        reloadSkills();
      } else {
        setError(result.error || "Failed to create skill");
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to create skill");
    }
  };

  const handleDeleteSkill = async (id: string) => {
    if (!transport) return;
    if (!confirm("Are you sure you want to delete this skill?")) return;
    try {
      const result = await transport.deleteSkill(id);
      if (result.success) {
        setSelectedSkill(null);
        reloadSkills();
      } else {
        setError(result.error || "Failed to delete skill");
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to delete skill");
    }
  };

  // ── Schedule CRUD ──
  const reloadSchedules = useCallback(async () => {
    if (!transport) return;
    try {
      const res = await transport.listCronJobs();
      if (res.success && res.data) setSchedules(res.data);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to reload schedules");
    }
  }, [transport]);

  const openCreateSchedule = () => {
    setScheduleForm({
      id: "",
      name: "",
      schedule: "0 0 * * * *",
      agent_id: "root",
      message: "",
      enabled: true,
      timezone: "",
    });
    setEditingSchedule(null);
    setIsCreatingSchedule(true);
  };

  const openEditSchedule = (job: CronJobResponse) => {
    setScheduleForm({
      id: job.id,
      name: job.name,
      schedule: job.schedule,
      agent_id: job.agent_id,
      message: job.message,
      enabled: job.enabled,
      timezone: job.timezone || "",
    });
    setEditingSchedule(job);
    setIsCreatingSchedule(true);
  };

  const handleScheduleNameChange = (name: string) => {
    setScheduleForm((prev) => ({
      ...prev,
      name,
      id: !editingSchedule ? generateId(name) : prev.id,
    }));
  };

  const handleSubmitSchedule = async () => {
    if (!transport) return;
    try {
      if (!editingSchedule) {
        const request: CreateCronJobRequest = {
          id: scheduleForm.id,
          name: scheduleForm.name,
          schedule: scheduleForm.schedule,
          agent_id: scheduleForm.agent_id,
          message: scheduleForm.message,
          enabled: scheduleForm.enabled,
          timezone: scheduleForm.timezone || undefined,
        };
        const result = await transport.createCronJob(request);
        if (result.success) {
          setIsCreatingSchedule(false);
          reloadSchedules();
        } else {
          setError(result.error || "Failed to create schedule");
        }
      } else {
        const request: UpdateCronJobRequest = {
          name: scheduleForm.name,
          schedule: scheduleForm.schedule,
          agent_id: scheduleForm.agent_id,
          message: scheduleForm.message,
          timezone: scheduleForm.timezone || undefined,
        };
        const result = await transport.updateCronJob(editingSchedule.id, request);
        if (result.success) {
          setIsCreatingSchedule(false);
          reloadSchedules();
        } else {
          setError(result.error || "Failed to update schedule");
        }
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : "Schedule operation failed");
    }
  };

  const handleDeleteSchedule = async (id: string) => {
    if (!transport) return;
    if (!confirm("Are you sure you want to delete this schedule?")) return;
    try {
      const result = await transport.deleteCronJob(id);
      if (result.success) {
        reloadSchedules();
      } else {
        setError(result.error || "Failed to delete schedule");
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to delete schedule");
    }
  };

  const handleToggleSchedule = async (job: CronJobResponse) => {
    if (!transport) return;
    try {
      const result = job.enabled
        ? await transport.disableCronJob(job.id)
        : await transport.enableCronJob(job.id);
      if (result.success && result.data) {
        setSchedules((prev) => prev.map((j) => (j.id === job.id ? result.data! : j)));
      } else {
        setError(result.error || "Failed to toggle schedule");
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : "Toggle failed");
    }
  };

  const handleTriggerSchedule = async (id: string) => {
    if (!transport) return;
    setTriggeringJob(id);
    try {
      const result = await transport.triggerCronJob(id);
      if (result.success) {
        await reloadSchedules();
      } else {
        setError(result.error || "Failed to trigger schedule");
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : "Trigger failed");
    } finally {
      setTriggeringJob(null);
    }
  };

  // ── Filtered lists ──
  const filteredAgents = useMemo(() => {
    if (!agentSearch) return agents;
    const q = agentSearch.toLowerCase();
    return agents.filter(
      (a) =>
        a.name.toLowerCase().includes(q) ||
        a.displayName?.toLowerCase().includes(q) ||
        a.description?.toLowerCase().includes(q),
    );
  }, [agents, agentSearch]);

  const filteredSkills = useMemo(() => {
    if (!skillSearch) return skills;
    const q = skillSearch.toLowerCase();
    return skills.filter(
      (s) =>
        s.name.toLowerCase().includes(q) ||
        s.displayName?.toLowerCase().includes(q) ||
        s.description?.toLowerCase().includes(q) ||
        s.category?.toLowerCase().includes(q),
    );
  }, [skills, skillSearch]);

  const filteredSchedules = useMemo(() => {
    if (!scheduleSearch) return schedules;
    const q = scheduleSearch.toLowerCase();
    return schedules.filter(
      (s) =>
        s.name.toLowerCase().includes(q) ||
        s.message?.toLowerCase().includes(q) ||
        s.schedule.includes(q),
    );
  }, [schedules, scheduleSearch]);

  // ── Loading ──
  if (isLoading) {
    return (
      <div className="page" style={{ display: "flex", alignItems: "center", justifyContent: "center" }}>
        <Loader2 style={{ width: 24, height: 24, animation: "spin 1s linear infinite" }} />
      </div>
    );
  }

  // ── Render ──
  return (
    <div className="page" style={{ display: "flex", flexDirection: "column" }}>
      <div className="page-header-v2">
        <h1 className="page-title-v2">Agents</h1>
        <p className="page-subtitle-v2">
          Create and manage your AI assistants. Each agent has its own personality, model, skills, and tools.
        </p>
      </div>

      <TabBar
        tabs={[
          { id: "agents", label: "My Agents", count: agents.length },
          { id: "skills", label: "Skills Library", count: skills.length },
          { id: "schedules", label: "Schedules", count: schedules.length },
        ]}
        activeTab={activeTab}
        onTabChange={setActiveTab}
      />

      {/* Error banner */}
      {error && (
        <div className="alert alert--error" style={{ margin: "0 var(--spacing-9)" }}>
          <span>{error}</span>
          <button className="btn btn--icon-ghost" onClick={() => setError(null)}>
            <X style={{ width: 14, height: 14 }} />
          </button>
        </div>
      )}

      {/* ────────────── My Agents Tab ────────────── */}
      <TabPanel id="agents" activeTab={activeTab}>
        <div className="page-content-v2">
          <ActionBar
            searchPlaceholder="Search agents..."
            searchValue={agentSearch}
            onSearchChange={setAgentSearch}
            actions={
              <button className="btn btn--primary btn--md" onClick={() => setIsCreatingAgent(true)}>
                <Plus style={{ width: 16, height: 16 }} />
                Create Agent
              </button>
            }
          />

          <HelpBox>
            Think of agents like team members — each one has a role, a model (brain), skills (expertise), and tools (MCP connections).
          </HelpBox>

          {filteredAgents.length === 0 ? (
            <EmptyState
              icon={Bot}
              title={agentSearch ? "No matching agents" : "No agents yet"}
              description="Agents are your AI assistants. Each agent can have its own personality, skills, and model."
              action={
                !agentSearch
                  ? { label: "Create Agent", onClick: () => setIsCreatingAgent(true) }
                  : undefined
              }
            />
          ) : (
            <div className="card-grid">
              {filteredAgents.map((agent, i) => {
                const emoji = getAgentEmoji(agent.id);
                const provider = providers.find((p) => p.id === agent.providerId);
                const skillCount = agent.skills?.length || 0;
                const mcpCount = agent.mcps?.length || 0;
                return (
                  <div
                    key={agent.id}
                    className={`agent-card animate-fade-in-up animate-delay-${Math.min(i + 1, 4)}`}
                    role="button"
                    tabIndex={0}
                    onClick={() => setEditingAgent(agent)}
                    onKeyDown={(e) => { if (e.key === "Enter" || e.key === " ") setEditingAgent(agent); }}
                  >
                    <div className="agent-card__top">
                      <div className="agent-card__avatar">
                        {emoji}
                        <span className="agent-card__online-dot" />
                      </div>
                      <div className="agent-card__info">
                        <div className="agent-card__name">
                          {agent.displayName || agent.name}
                        </div>
                        <div className="agent-card__id">{agent.id}</div>
                      </div>
                    </div>

                    {agent.description && (
                      <div className="agent-card__desc">{agent.description}</div>
                    )}

                    <div className="agent-card__meta">
                      <ModelChip
                        modelId={agent.model}
                        profile={modelRegistry[agent.model]}
                      />
                      {skillCount > 0 && (
                        <MetaChip variant="skills" icon={<Zap style={{ width: 11, height: 11 }} />}>
                          {skillCount} skill{skillCount !== 1 ? "s" : ""}
                        </MetaChip>
                      )}
                      {mcpCount > 0 && (
                        <MetaChip variant="mcps" icon={<Server style={{ width: 11, height: 11 }} />}>
                          {mcpCount} MCP{mcpCount !== 1 ? "s" : ""}
                        </MetaChip>
                      )}
                    </div>

                    <div className="agent-card__footer">
                      <div className="agent-card__footer-left">
                        {provider?.name || agent.providerId}
                      </div>
                      <div className="agent-card__footer-actions">
                        <button
                          className="btn btn--icon-ghost"
                          onClick={(e) => {
                            e.stopPropagation();
                            setEditingAgent(agent);
                          }}
                        >
                          <Pencil style={{ width: 14, height: 14 }} />
                        </button>
                        <button
                          className="btn btn--icon-ghost"
                          onClick={(e) => {
                            e.stopPropagation();
                            handleDeleteAgent(agent.id);
                          }}
                        >
                          <Trash2 style={{ width: 14, height: 14 }} />
                        </button>
                      </div>
                    </div>
                  </div>
                );
              })}
            </div>
          )}
        </div>
      </TabPanel>

      {/* ────────────── Skills Library Tab ────────────── */}
      <TabPanel id="skills" activeTab={activeTab}>
        <div className="page-content-v2">
          <ActionBar
            searchPlaceholder="Search skills..."
            searchValue={skillSearch}
            onSearchChange={setSkillSearch}
            actions={
              <button className="btn btn--primary btn--md" onClick={() => setIsCreatingSkill(true)}>
                <Plus style={{ width: 16, height: 16 }} />
                Create Skill
              </button>
            }
          />

          <HelpBox>
            Skills are reusable instruction packages — create once, assign to any agent.
          </HelpBox>

          {filteredSkills.length === 0 ? (
            <EmptyState
              icon={Zap}
              title={skillSearch ? "No matching skills" : "No skills yet"}
              description="Skills teach your agents how to handle specific tasks."
              action={
                !skillSearch
                  ? { label: "Create Skill", onClick: () => setIsCreatingSkill(true) }
                  : undefined
              }
            />
          ) : (
            <div className="card-grid">
              {filteredSkills.map((skill, i) => (
                <div
                  key={skill.id}
                  className={`skill-card animate-fade-in-up animate-delay-${Math.min(i + 1, 4)}`}
                  role="button"
                  tabIndex={0}
                  onClick={() => setSelectedSkill(skill)}
                  onKeyDown={(e) => { if (e.key === "Enter" || e.key === " ") setSelectedSkill(skill); }}
                >
                  <div className="skill-card__header">
                    <span className="skill-card__name">{skill.displayName || skill.name}</span>
                    <MetaChip variant="skills">{skill.category}</MetaChip>
                  </div>
                  <div className="skill-card__desc">
                    {skill.description || "No description"}
                  </div>
                </div>
              ))}
            </div>
          )}
        </div>
      </TabPanel>

      {/* ────────────── Schedules Tab ────────────── */}
      <TabPanel id="schedules" activeTab={activeTab}>
        <div className="page-content-v2">
          <ActionBar
            searchPlaceholder="Search schedules..."
            searchValue={scheduleSearch}
            onSearchChange={setScheduleSearch}
            actions={
              <button className="btn btn--primary btn--md" onClick={openCreateSchedule}>
                <Plus style={{ width: 16, height: 16 }} />
                Create Schedule
              </button>
            }
          />

          <HelpBox>
            Schedules run your agents automatically on a timer.
          </HelpBox>

          {filteredSchedules.length === 0 ? (
            <EmptyState
              icon={Calendar}
              title={scheduleSearch ? "No matching schedules" : "No schedules yet"}
              description="Run your agents on autopilot — create schedules to trigger them automatically."
              action={
                !scheduleSearch
                  ? { label: "Create Schedule", onClick: openCreateSchedule }
                  : undefined
              }
            />
          ) : (
            <div style={{ display: "flex", flexDirection: "column", gap: "var(--spacing-3)" }}>
              {filteredSchedules.map((job, i) => {
                const agentName = agents.find((a) => a.id === job.agent_id)?.displayName || job.agent_id;
                return (
                  <div
                    key={job.id}
                    className={`schedule-card animate-fade-in-up animate-delay-${Math.min(i + 1, 4)}`}
                  >
                    <div className={`schedule-card__icon ${job.enabled ? "schedule-card__icon--active" : "schedule-card__icon--paused"}`}>
                      {job.enabled ? <Play style={{ width: 18, height: 18 }} /> : <Pause style={{ width: 18, height: 18 }} />}
                    </div>

                    <div className="schedule-card__info">
                      <div className="schedule-card__name">{job.name}</div>
                      <div className="schedule-card__cron">{describeCron(job.schedule)}</div>
                      <div className="schedule-card__agent">Agent: {agentName}</div>
                    </div>

                    <div className="schedule-card__meta">
                      <div className="schedule-card__time">
                        Last: {formatTime(job.last_run)}
                      </div>
                      <div className="schedule-card__time">
                        Next: {job.enabled ? formatTime(job.next_run) : "Paused"}
                      </div>
                    </div>

                    <button
                      className={`toggle-switch ${job.enabled ? "toggle-switch--on" : "toggle-switch--off"}`}
                      onClick={() => handleToggleSchedule(job)}
                      aria-label={job.enabled ? "Disable schedule" : "Enable schedule"}
                    />

                    <div style={{ display: "flex", gap: 4 }}>
                      <button
                        className="btn btn--icon-ghost"
                        onClick={() => handleTriggerSchedule(job.id)}
                        disabled={triggeringJob === job.id}
                        title="Trigger now"
                      >
                        {triggeringJob === job.id ? (
                          <RefreshCw style={{ width: 14, height: 14, animation: "spin 1s linear infinite" }} />
                        ) : (
                          <Zap style={{ width: 14, height: 14 }} />
                        )}
                      </button>
                      <button
                        className="btn btn--icon-ghost"
                        onClick={() => openEditSchedule(job)}
                        title="Edit"
                      >
                        <Pencil style={{ width: 14, height: 14 }} />
                      </button>
                      <button
                        className="btn btn--icon-ghost"
                        onClick={() => handleDeleteSchedule(job.id)}
                        title="Delete"
                      >
                        <Trash2 style={{ width: 14, height: 14 }} />
                      </button>
                    </div>
                  </div>
                );
              })}
            </div>
          )}
        </div>
      </TabPanel>

      {/* ────────────── Create Agent Slideover ────────────── */}
      <Slideover
        open={isCreatingAgent}
        onClose={() => setIsCreatingAgent(false)}
        title="Create Agent"
        icon={<Bot style={{ width: 18, height: 18 }} />}
        footer={
          <>
            <button className="btn btn--secondary btn--md" onClick={() => setIsCreatingAgent(false)}>
              Cancel
            </button>
            <button
              className="btn btn--primary btn--md"
              onClick={handleCreateAgent}
              disabled={!newAgent.name || !newAgent.providerId || !newAgent.model}
            >
              Create
            </button>
          </>
        }
      >
        <div className="form-group">
          <label className="form-label" htmlFor="create-agent-name">Name (ID)</label>
          <input
            id="create-agent-name"
            className="form-input"
            type="text"
            value={newAgent.name}
            onChange={(e) =>
              setNewAgent({ ...newAgent, name: e.target.value.toLowerCase().replace(/[^a-z0-9-]/g, "-") })
            }
            placeholder="my-agent"
          />
        </div>
        <div className="form-group">
          <label className="form-label" htmlFor="create-agent-display-name">Display Name</label>
          <input
            id="create-agent-display-name"
            className="form-input"
            type="text"
            value={newAgent.displayName}
            onChange={(e) => setNewAgent({ ...newAgent, displayName: e.target.value })}
            placeholder="My Agent"
          />
        </div>
        <div className="form-group">
          <label className="form-label" htmlFor="create-agent-description">Description</label>
          <textarea
            id="create-agent-description"
            className="form-textarea"
            value={newAgent.description}
            onChange={(e) => setNewAgent({ ...newAgent, description: e.target.value })}
            placeholder="What does this agent do?"
            rows={2}
          />
        </div>
        <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: "var(--spacing-4)" }}>
          <div className="form-group">
            <label className="form-label" htmlFor="create-agent-provider">Provider</label>
            <select
              id="create-agent-provider"
              className="form-select"
              value={newAgent.providerId}
              onChange={(e) => {
                const provider = providers.find((p) => p.id === e.target.value);
                setNewAgent({
                  ...newAgent,
                  providerId: e.target.value,
                  model: provider ? getProviderDefaultModel(provider) : "",
                });
              }}
            >
              {providers.length === 0 ? (
                <option value="">No providers configured</option>
              ) : (
                providers.map((p) => (
                  <option key={p.id} value={p.id}>{p.name}</option>
                ))
              )}
            </select>
          </div>
          <div className="form-group">
            <label className="form-label" htmlFor="create-agent-model">Model</label>
            <select
              id="create-agent-model"
              className="form-select"
              value={newAgent.model}
              onChange={(e) => setNewAgent({ ...newAgent, model: e.target.value })}
            >
              {providers
                .find((p) => p.id === newAgent.providerId)
                ?.models.map((m) => (
                  <option key={m} value={m}>{m}</option>
                )) || <option value="">Select provider first</option>}
            </select>
            {newAgent.model && modelRegistry[newAgent.model] && (
              <div style={{ marginTop: "var(--spacing-1)" }}>
                <ModelChip
                  modelId={newAgent.model}
                  profile={modelRegistry[newAgent.model]}
                  showContext
                />
              </div>
            )}
          </div>
        </div>
      </Slideover>

      {/* ────────────── Edit Agent Slideover ────────────── */}
      {editingAgent && (
        <AgentEditPanel
          agent={editingAgent}
          providers={providers}
          modelRegistry={modelRegistry}
          onClose={() => setEditingAgent(null)}
          onSave={() => {
            reloadAgents();
            reloadSkills();
          }}
        />
      )}

      {/* ────────────── Skill Detail Slideover ────────────── */}
      <Slideover
        open={!!selectedSkill}
        onClose={() => setSelectedSkill(null)}
        title={selectedSkill?.displayName || selectedSkill?.name || ""}
        subtitle={selectedSkill?.id}
        icon={<Zap style={{ width: 18, height: 18 }} />}
        footer={
          <>
            <button className="btn btn--secondary btn--md" onClick={() => setSelectedSkill(null)}>
              Close
            </button>
            <button
              className="btn btn--destructive btn--md"
              onClick={() => selectedSkill && handleDeleteSkill(selectedSkill.id)}
            >
              <Trash2 style={{ width: 14, height: 14 }} />
              Delete
            </button>
          </>
        }
      >
        {selectedSkill && (
          <>
            <div className="form-group">
              <label className="form-label" htmlFor="skill-description-value">Description</label>
              <p id="skill-description-value" style={{ fontSize: "var(--text-sm)", color: "var(--foreground)" }}>
                {selectedSkill.description || "No description"}
              </p>
            </div>
            <div className="form-group">
              <label className="form-label" htmlFor="skill-category-value">Category</label>
              <div id="skill-category-value">
                <MetaChip variant="skills">{selectedSkill.category}</MetaChip>
              </div>
            </div>
            <div className="form-group">
              <label className="form-label" htmlFor="skill-instructions-value">Instructions</label>
              <pre
                id="skill-instructions-value"
                style={{
                  background: "var(--background-elevated)",
                  borderRadius: "var(--radius-md)",
                  padding: "var(--spacing-4)",
                  fontSize: "var(--text-sm)",
                  color: "var(--foreground)",
                  whiteSpace: "pre-wrap",
                  fontFamily: "var(--font-mono)",
                  maxHeight: 400,
                  overflow: "auto",
                  border: "1px solid var(--border)",
                }}
              >
                {selectedSkill.instructions}
              </pre>
            </div>
          </>
        )}
      </Slideover>

      {/* ────────────── Create Skill Slideover ────────────── */}
      <Slideover
        open={isCreatingSkill}
        onClose={() => setIsCreatingSkill(false)}
        title="Create Skill"
        icon={<Zap style={{ width: 18, height: 18 }} />}
        footer={
          <>
            <button className="btn btn--secondary btn--md" onClick={() => setIsCreatingSkill(false)}>
              Cancel
            </button>
            <button
              className="btn btn--primary btn--md"
              onClick={handleCreateSkill}
              disabled={!newSkill.name}
            >
              Create Skill
            </button>
          </>
        }
      >
        <div className="form-group">
          <label className="form-label" htmlFor="create-skill-name">Name (ID)</label>
          <input
            id="create-skill-name"
            className="form-input"
            type="text"
            value={newSkill.name}
            onChange={(e) =>
              setNewSkill({
                ...newSkill,
                name: e.target.value.toLowerCase().replace(/[^a-z0-9-]/g, "-"),
              })
            }
            placeholder="my-skill"
          />
        </div>
        <div className="form-group">
          <label className="form-label" htmlFor="create-skill-display-name">Display Name</label>
          <input
            id="create-skill-display-name"
            className="form-input"
            type="text"
            value={newSkill.displayName}
            onChange={(e) => setNewSkill({ ...newSkill, displayName: e.target.value })}
            placeholder="My Skill"
          />
        </div>
        <div className="form-group">
          <label className="form-label" htmlFor="create-skill-description">Description</label>
          <input
            id="create-skill-description"
            className="form-input"
            type="text"
            value={newSkill.description}
            onChange={(e) => setNewSkill({ ...newSkill, description: e.target.value })}
            placeholder="What does this skill do?"
          />
        </div>
        <div className="form-group">
          <label className="form-label" htmlFor="create-skill-category">Category</label>
          <input
            id="create-skill-category"
            className="form-input"
            type="text"
            value={newSkill.category}
            onChange={(e) => setNewSkill({ ...newSkill, category: e.target.value })}
            placeholder="general"
          />
        </div>
        <div className="form-group">
          <label className="form-label" htmlFor="create-skill-instructions">Instructions</label>
          <textarea
            id="create-skill-instructions"
            className="form-textarea"
            value={newSkill.instructions}
            onChange={(e) => setNewSkill({ ...newSkill, instructions: e.target.value })}
            placeholder="Instructions for the agent when using this skill..."
            rows={10}
            style={{ fontFamily: "var(--font-mono)" }}
          />
        </div>
      </Slideover>

      {/* ────────────── Create / Edit Schedule Slideover ────────────── */}
      <Slideover
        open={isCreatingSchedule}
        onClose={() => setIsCreatingSchedule(false)}
        title={editingSchedule ? "Edit Schedule" : "Create Schedule"}
        icon={<Calendar style={{ width: 18, height: 18 }} />}
        footer={
          <>
            <button className="btn btn--secondary btn--md" onClick={() => setIsCreatingSchedule(false)}>
              Cancel
            </button>
            <button
              className="btn btn--primary btn--md"
              onClick={handleSubmitSchedule}
              disabled={!scheduleForm.name || !scheduleForm.id || !scheduleForm.schedule || !scheduleForm.message}
            >
              {editingSchedule ? "Save Changes" : "Create"}
            </button>
          </>
        }
      >
        <div className="form-group">
          <label className="form-label" htmlFor="schedule-name">Name</label>
          <input
            id="schedule-name"
            className="form-input"
            type="text"
            value={scheduleForm.name}
            onChange={(e) => handleScheduleNameChange(e.target.value)}
            placeholder="My Scheduled Task"
          />
        </div>
        <div className="form-group">
          <label className="form-label" htmlFor="schedule-id">ID</label>
          <input
            id="schedule-id"
            className="form-input"
            type="text"
            value={scheduleForm.id}
            onChange={(e) =>
              !editingSchedule && setScheduleForm((prev) => ({ ...prev, id: e.target.value }))
            }
            placeholder="my-scheduled-task"
            disabled={!!editingSchedule}
          />
        </div>
        <div className="form-group">
          <label className="form-label" htmlFor="schedule-agent">Agent</label>
          <select
            id="schedule-agent"
            className="form-select"
            value={scheduleForm.agent_id}
            onChange={(e) => setScheduleForm((prev) => ({ ...prev, agent_id: e.target.value }))}
          >
            <option value="root">root (default)</option>
            {agents.map((a) =>
              a.id !== "root" ? (
                <option key={a.id} value={a.id}>
                  {a.displayName || a.name}
                </option>
              ) : null,
            )}
          </select>
        </div>
        <div className="form-group">
          <label className="form-label" htmlFor="schedule-cron-preset">Schedule (Cron Expression)</label>
          <select
            id="schedule-cron-preset"
            className="form-select"
            value={
              CRON_PRESETS.find((p) => p.value === scheduleForm.schedule)
                ? scheduleForm.schedule
                : "custom"
            }
            onChange={(e) => {
              if (e.target.value !== "custom") {
                setScheduleForm((prev) => ({ ...prev, schedule: e.target.value }));
              }
            }}
          >
            {CRON_PRESETS.map((p) => (
              <option key={p.value} value={p.value}>{p.label}</option>
            ))}
            <option value="custom">Custom...</option>
          </select>
          <input
            className="form-input"
            type="text"
            value={scheduleForm.schedule}
            onChange={(e) => setScheduleForm((prev) => ({ ...prev, schedule: e.target.value }))}
            placeholder="* * * * *"
            style={{ fontFamily: "var(--font-mono)", marginTop: "var(--spacing-2)" }}
          />
          <span style={{ fontSize: "var(--text-xs)", color: "var(--dim-foreground)" }}>
            Format: minute hour day-of-month month day-of-week
          </span>
        </div>
        <div className="form-group">
          <label className="form-label" htmlFor="schedule-message">Message</label>
          <textarea
            id="schedule-message"
            className="form-textarea"
            value={scheduleForm.message}
            onChange={(e) => setScheduleForm((prev) => ({ ...prev, message: e.target.value }))}
            placeholder="The message to send to the agent..."
            rows={3}
          />
        </div>
        <div className="form-group">
          <label className="form-label" htmlFor="schedule-timezone">Timezone (optional)</label>
          <input
            id="schedule-timezone"
            className="form-input"
            type="text"
            value={scheduleForm.timezone}
            onChange={(e) => setScheduleForm((prev) => ({ ...prev, timezone: e.target.value }))}
            placeholder="America/New_York"
          />
          <span style={{ fontSize: "var(--text-xs)", color: "var(--dim-foreground)" }}>
            Leave empty to use system timezone
          </span>
        </div>
        {!editingSchedule && (
          <div style={{
            display: "flex",
            alignItems: "center",
            justifyContent: "space-between",
            padding: "var(--spacing-3)",
            background: "var(--background-elevated)",
            borderRadius: "var(--radius-md)",
          }}>
            <div>
              <div style={{ fontSize: "var(--text-sm)", fontWeight: 500, color: "var(--foreground)" }}>
                Enable on Create
              </div>
              <div style={{ fontSize: "var(--text-xs)", color: "var(--dim-foreground)" }}>
                Start running immediately after creation
              </div>
            </div>
            <button
              type="button"
              className={`toggle-switch ${scheduleForm.enabled ? "toggle-switch--on" : "toggle-switch--off"}`}
              onClick={() => setScheduleForm((prev) => ({ ...prev, enabled: !prev.enabled }))}
            />
          </div>
        )}
      </Slideover>
    </div>
  );
}
