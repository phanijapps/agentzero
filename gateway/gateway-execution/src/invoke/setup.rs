//! # Setup Module
//!
//! Provider resolution and agent loading utilities for execution setup.

use crate::delegation::DelegationMode;
use crate::errors::ExecutionError;
use agent_tools::ToolSettings;
use gateway_services::providers::Provider;
use gateway_services::{
    load_bounded_vault_utf8_file, AgentService, BoundedFileError, ProviderService, SettingsService,
    SharedVaultPaths, WARD_AGENT_TEMPLATE_MAX_BYTES,
};
use std::sync::Arc;

// ============================================================================
// SETUP CONTEXT
// ============================================================================

/// Context for setting up agent execution.
///
/// Contains all services and configuration needed to prepare an agent
/// for execution.
#[derive(Clone)]
pub struct SetupContext {
    /// Agent service for loading agent configurations
    pub agent_service: Arc<AgentService>,
    /// Provider service for loading provider configurations
    pub provider_service: Arc<ProviderService>,
    /// Vault paths for accessing configuration and data directories
    pub paths: SharedVaultPaths,
}

impl SetupContext {
    /// Create a new setup context.
    pub fn new(
        agent_service: Arc<AgentService>,
        provider_service: Arc<ProviderService>,
        paths: SharedVaultPaths,
    ) -> Self {
        Self {
            agent_service,
            provider_service,
            paths,
        }
    }

    /// Get tool settings from the configuration directory.
    pub fn tool_settings(&self) -> ToolSettings {
        let settings_service = SettingsService::new(self.paths.clone());
        settings_service.get_tool_settings().unwrap_or_default()
    }
}

// ============================================================================
// PROVIDER RESOLVER
// ============================================================================

/// Resolves providers for agent execution.
pub struct ProviderResolver<'a> {
    provider_service: &'a ProviderService,
}

impl<'a> ProviderResolver<'a> {
    /// Create a new provider resolver.
    pub fn new(provider_service: &'a ProviderService) -> Self {
        Self { provider_service }
    }

    /// Get the default provider (marked as is_default) or fall back to first.
    pub fn get_default(&self) -> Result<Provider, ExecutionError> {
        let providers = self
            .provider_service
            .list()
            .map_err(|e| format!("Failed to list providers: {}", e))?;

        // First try to find the provider marked as default
        if let Some(default_provider) = providers.iter().find(|p| p.is_default).cloned() {
            return Ok(default_provider);
        }

        // Fall back to first provider
        providers.into_iter().next().ok_or_else(|| {
            ExecutionError::from(
                "No providers configured. Add a provider in Integrations.".to_string(),
            )
        })
    }

    /// Get provider by ID, falling back to default if not found.
    pub fn get_or_default(&self, provider_id: &str) -> Result<Provider, ExecutionError> {
        if !provider_id.is_empty() {
            match self.provider_service.get(provider_id) {
                Ok(provider) => return Ok(provider),
                Err(_) => {
                    tracing::warn!(
                        "Provider {} not found, falling back to default",
                        provider_id
                    );
                }
            }
        }
        self.get_default()
    }
}

// ============================================================================
// AGENT LOADER
// ============================================================================

/// Loads and prepares agent configurations.
pub struct AgentLoader<'a> {
    agent_service: &'a AgentService,
    provider_resolver: ProviderResolver<'a>,
    paths: SharedVaultPaths,
    settings: Option<&'a SettingsService>,
    chat_mode: bool,
}

impl<'a> AgentLoader<'a> {
    /// Create a new agent loader.
    pub fn new(
        agent_service: &'a AgentService,
        provider_service: &'a ProviderService,
        paths: SharedVaultPaths,
    ) -> Self {
        Self {
            agent_service,
            provider_resolver: ProviderResolver::new(provider_service),
            paths,
            settings: None,
            chat_mode: false,
        }
    }

    /// Set settings service for reading orchestrator config.
    pub fn with_settings(mut self, settings: &'a SettingsService) -> Self {
        self.settings = Some(settings);
        self
    }

    /// Enable chat mode (lean prompt, higher temperature, skip pipeline).
    ///
    /// Memory injection still runs in chat mode — only intent analysis / planning /
    /// delegation / ward transitions are skipped.
    pub fn with_chat_mode(mut self, chat_mode: bool) -> Self {
        self.chat_mode = chat_mode;
        self
    }

    /// Load an agent by ID.
    ///
    /// Returns the agent and its resolved provider.
    pub async fn load(
        &self,
        agent_id: &str,
    ) -> Result<(gateway_services::agents::Agent, Provider), ExecutionError> {
        let mut agent = self
            .agent_service
            .get(agent_id)
            .await
            .map_err(|e| format!("Failed to load agent {}: {}", agent_id, e))?;

        // Append OS context and shards to agent instructions
        // so subagents know platform commands and tool syntax
        agent.instructions =
            append_system_context(&agent.instructions, &self.paths, SubagentRole::Executor);

        let provider = self.provider_resolver.get_or_default(&agent.provider_id)?;

        Ok((agent, provider))
    }

    /// Load an agent by ID, creating a default root agent if the ID is "root" and not found.
    pub async fn load_or_create_root(
        &self,
        agent_id: &str,
    ) -> Result<(gateway_services::agents::Agent, Provider), ExecutionError> {
        if agent_id != "root" {
            let agent = self.agent_service.get(agent_id).await?;
            let provider = self.provider_resolver.get_or_default(&agent.provider_id)?;
            return Ok((agent, provider));
        }

        // Root is a system agent. Always synthesize it from orchestrator
        // settings; never let an on-disk agents/root shadow the root prompt.
        let orch = self
            .settings
            .and_then(|s| s.get_execution_settings().ok())
            .map(|s| s.orchestrator)
            .unwrap_or_default();
        let max_input_tokens = orch.effective_max_input_tokens();
        let max_input_tokens_explicit = orch.max_input_tokens_explicit();

        // Resolve provider: orchestrator config → default provider
        let provider = match &orch.provider_id {
            Some(id) if !id.is_empty() => self.provider_resolver.get_or_default(id)?,
            _ => self.provider_resolver.get_default()?,
        };

        // Resolve model: orchestrator config → provider default
        let model = orch
            .model
            .filter(|m| !m.is_empty())
            .unwrap_or_else(|| provider.default_model().to_string());

        // Quick Chat should answer immediately. Research mode respects the
        // orchestrator thinking toggle; chat mode keeps reasoning off even
        // when the root agent uses a reasoning-capable model.
        let thinking_enabled = !self.chat_mode && orch.thinking_enabled;

        tracing::info!(
            provider = %provider.name,
            model = %model,
            temperature = orch.temperature,
            max_tokens = orch.max_tokens,
            thinking = thinking_enabled,
            chat_mode = self.chat_mode,
            "Creating root agent from orchestrator config"
        );

        // Chat mode uses a lean prompt; research mode uses the full system prompt
        let instructions = if self.chat_mode {
            gateway_templates::load_chat_prompt_from_paths(&self.paths)
        } else {
            gateway_templates::load_system_prompt_from_paths(&self.paths)
        };

        // Chat mode: higher temperature for creative, personality-forward responses
        let temperature = if self.chat_mode {
            1.0
        } else {
            orch.temperature
        };

        let agent = gateway_services::agents::Agent {
            id: "root".to_string(),
            name: "root".to_string(),
            display_name: "Root Agent".to_string(),
            description: "System root agent that handles all conversations".to_string(),
            agent_type: Some("orchestrator".to_string()),
            provider_id: provider.id.clone().unwrap_or_default(),
            model,
            temperature,
            max_input_tokens,
            max_input_tokens_explicit,
            max_tokens: orch.max_tokens,
            thinking_enabled,
            voice_recording_enabled: false,
            system_instruction: None,
            instructions,
            mcps: vec![],
            skills: vec![],
            middleware: None,
            created_at: None,
        };

        Ok((agent, provider))
    }

    /// Load an agent by ID. If not found, auto-create a specialist agent
    /// with the same provider/model as root and role-specific instructions.
    pub async fn load_or_create_specialist(
        &self,
        agent_id: &str,
    ) -> Result<(gateway_services::agents::Agent, Provider), ExecutionError> {
        if matches!(agent_id, "root" | "orchestrator") {
            return Err(ExecutionError::from(format!(
                "Reserved system agent id cannot be delegated: {agent_id}"
            )));
        }

        // Ward-as-agent: a `ward:<name>` id synthesizes the agent from the
        // ward directory instead of loading `agents/<id>/`. The ward IS the
        // agent — no on-disk agent folder.
        if let Some(ward_name) = agent_id.strip_prefix("ward:") {
            return self.synthesize_ward_agent(ward_name);
        }

        // Try loading existing agent first
        match self.agent_service.get(agent_id).await {
            Ok(mut agent) => {
                // Append OS context and shards so pre-configured agents
                // also know platform commands (PowerShell vs bash, etc.)
                agent.instructions =
                    append_system_context_without_rules(&agent.instructions, &self.paths);
                let provider = self.provider_resolver.get_or_default(&agent.provider_id)?;
                Ok((agent, provider))
            }
            Err(_) => {
                // Agent doesn't exist — auto-create with default provider
                let provider = self.provider_resolver.get_default()?;
                let model = provider.default_model().to_string();

                let instructions = build_specialist_instructions(agent_id, &self.paths);

                tracing::info!(
                    agent_id = %agent_id,
                    "Auto-creating specialist agent (not found in config)"
                );

                let agent = gateway_services::agents::Agent {
                    id: agent_id.to_string(),
                    name: agent_id.to_string(),
                    display_name: format_agent_display_name(agent_id),
                    description: format!("Auto-created specialist: {}", agent_id),
                    agent_type: Some("specialist".to_string()),
                    provider_id: provider.id.clone().unwrap_or_default(),
                    model,
                    temperature: 0.7,
                    max_input_tokens: gateway_services::models::DEFAULT_MAX_INPUT_TOKENS,
                    max_input_tokens_explicit: false,
                    max_tokens: gateway_services::models::DEFAULT_MAX_OUTPUT_TOKENS,
                    thinking_enabled: false,
                    voice_recording_enabled: false,
                    system_instruction: None,
                    instructions,
                    mcps: vec![],
                    skills: vec![],
                    middleware: None,
                    created_at: None,
                };

                Ok((agent, provider))
            }
        }
    }

    /// Synthesize a ward-agent for a `ward:<name>` delegation target.
    ///
    /// The ward IS the agent — no on-disk `agents/<id>/` folder. The system
    /// prompt is a generated identity line + the standard system-context
    /// shards + the ward's `AGENTS.md` doctrine. There is no `ZBOT.md`.
    ///
    /// P1 scope: the ward directory must already exist; creating a ward is
    /// the cold path's job. A missing directory is an error here.
    fn synthesize_ward_agent(
        &self,
        ward_name: &str,
    ) -> Result<(gateway_services::agents::Agent, Provider), ExecutionError> {
        let ward_dir = self.paths.ward_dir(ward_name);
        if !ward_dir.is_dir() {
            return Err(ExecutionError::from(format!(
                "ward '{}' has no directory at {}",
                ward_name,
                ward_dir.display()
            )));
        }

        let loaded_doctrine = load_ward_doctrine(&self.paths, ward_name);

        let mut identity = format!(
            "You are the `{ward}` ward-agent — you own the `{ward}` domain end \
             to end. Given ONE task you plan AND execute it to completion in \
             this single delegation, then return a result. The caller does not \
             orchestrate your steps.\n",
            ward = ward_name
        );
        identity.push_str(
            "\n## First-turn protocol\n\
             You are already bound to the ward named in your identity and supplied ward_snapshot. \
             Do not call the ward tool to enter it again.\n\
             1. recall — pull this ward's procedures, facts and past episodes \
             for the task.\n\
             2. Plan before execution — call update_plan with the concrete steps before any \
             write, shell mutation, procedure execution, or sub-delegation. Take the cheapest \
             route recall supports: replay a \
             matching promoted procedure with run_procedure; adapt a partial \
             match into a step plan; or, if nothing matches, decompose the \
             task into steps yourself, binding each step to a tool, skill, or \
             sub-delegation.\n\
             3. Execute the plan step by step — act, observe, adjust.\n\
             4. respond using the Handoff schema in your doctrine below.\n\
             \n\
             If the task falls outside your Purpose / Scope, do not attempt \
             it — call `respond` with a single line: \
             `RESULT: OUT_OF_SCOPE — <one-line reason>`.\n\
             If the task is within your Purpose / Scope but you lack a \
             tool, skill, or MCP needed to finish it, do not fake a \
             partial result — call `respond` with a single line: \
             `RESULT: CAPABILITY_MISSING — <the missing capability>`.\n",
        );
        if let Some(diagnostic) = loaded_doctrine.diagnostic {
            identity.push('\n');
            identity.push_str(diagnostic);
            identity.push('\n');
        }
        let instructions = compose_ward_agent_instructions(
            &identity,
            &self.paths,
            ward_name,
            &loaded_doctrine.doctrine,
        );

        // Ward-agent LLM config resolves in three tiers: the ward's own
        // `config.yaml` overrides the orchestrator (Settings > Advanced >
        // Orchestrator), which in turn falls back to the provider default.
        // A `null` provider/model in `config.yaml` inherits the orchestrator,
        // so an untouched ward behaves exactly as before.
        let orch = self
            .settings
            .and_then(|s| s.get_execution_settings().ok())
            .map(|s| s.orchestrator)
            .unwrap_or_default();
        let max_input_tokens = orch.effective_max_input_tokens();
        let max_input_tokens_explicit = orch.max_input_tokens_explicit();
        let ward_cfg = load_or_seed_ward_config(&ward_dir);
        let provider_id = ward_cfg
            .provider
            .filter(|p| !p.is_empty())
            .or_else(|| orch.provider_id.filter(|p| !p.is_empty()));
        let provider = match &provider_id {
            Some(id) => self.provider_resolver.get_or_default(id)?,
            None => self.provider_resolver.get_default()?,
        };
        let model = ward_cfg
            .model
            .filter(|m| !m.is_empty())
            .or_else(|| orch.model.filter(|m| !m.is_empty()))
            .unwrap_or_else(|| provider.default_model().to_string());

        let agent = gateway_services::agents::Agent {
            id: format!("ward:{ward_name}"),
            name: ward_name.to_string(),
            display_name: format!("Ward Agent: {ward_name}"),
            description: format!("Ward-agent for the {ward_name} ward"),
            agent_type: Some("ward".to_string()),
            provider_id: provider.id.clone().unwrap_or_default(),
            model,
            temperature: orch.temperature,
            max_input_tokens,
            max_input_tokens_explicit,
            max_tokens: orch.max_tokens,
            thinking_enabled: orch.thinking_enabled,
            voice_recording_enabled: false,
            system_instruction: None,
            instructions,
            mcps: vec![],
            skills: vec![],
            middleware: None,
            created_at: None,
        };

        tracing::info!(
            ward = ward_name,
            instructions_bytes = agent.instructions.len(),
            doctrine_bytes = loaded_doctrine.doctrine.len(),
            "Synthesized ward-agent for delegation"
        );
        Ok((agent, provider))
    }

    /// Get the provider resolver.
    pub fn provider_resolver(&self) -> &ProviderResolver<'a> {
        &self.provider_resolver
    }
}

// ============================================================================
// SPECIALIST AGENT HELPERS
// ============================================================================

/// Subagent execution role — determines which rules are injected.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SubagentRole {
    /// Write code, build things, run scripts. Strict rules.
    Executor,
    /// Review code, validate output, evaluate quality. Relaxed rules.
    Reviewer,
}

/// Detect subagent role from task description.
pub fn detect_subagent_role(agent_id: &str, task: &str) -> SubagentRole {
    let agent_lower = agent_id.to_lowercase();
    let task_lower = task.to_lowercase();

    if is_reviewer_agent(&agent_lower) {
        return SubagentRole::Reviewer;
    }
    if has_execution_signal(&task_lower) {
        return SubagentRole::Executor;
    }
    if has_read_only_review_signal(&task_lower) {
        return SubagentRole::Reviewer;
    }
    if is_execution_agent(&agent_lower) {
        return SubagentRole::Executor;
    }
    if has_review_signal(&task_lower) {
        return SubagentRole::Reviewer;
    }

    SubagentRole::Executor
}

fn is_reviewer_agent(agent_lower: &str) -> bool {
    agent_lower == "reviewer-agent"
        || agent_lower.contains("reviewer")
        || agent_lower.contains("review-agent")
}

fn is_execution_agent(agent_lower: &str) -> bool {
    [
        "builder-agent",
        "research-agent",
        "planner-agent",
        "writing-agent",
        "code-agent",
    ]
    .contains(&agent_lower)
}

fn has_execution_signal(task_lower: &str) -> bool {
    [
        "find ", "fetch", "scrape", "write", "create", "run ", "parse", "extract", "generate",
        "save", "build", "load", "search", "gather", "collect",
    ]
    .iter()
    .any(|signal| task_lower.contains(signal))
}

fn has_read_only_review_signal(task_lower: &str) -> bool {
    [
        "read-only review",
        "review without changing",
        "review without editing",
        "inspect and report",
        "report defects",
        "do not write",
        "no shell",
        "no file changes",
        "result: approved",
        "result: defects",
        "quality review",
    ]
    .iter()
    .any(|signal| task_lower.contains(signal))
}

fn has_review_signal(task_lower: &str) -> bool {
    ["review", "audit"]
        .iter()
        .any(|signal| task_lower.contains(signal))
}

pub fn subagent_rules(role: SubagentRole, mode: DelegationMode) -> &'static str {
    match role {
        SubagentRole::Executor => executor_rules(mode),
        SubagentRole::Reviewer => {
            "\n\n# --- SUBAGENT RULES ---\n\
            You are reviewing work produced by another agent. Think critically and independently.\n\
            1. Read the specs and the implementation carefully before forming opinions.\n\
            2. Inspect source files, provided context, and command output already produced by executor/root agents.\n\
            3. Evaluate with domain expertise — are values reasonable? Is data complete?\n\
            4. Report your findings in structured format.\n\n\
            ## Report Format\n\
            End your response with EXACTLY one of:\n\
            RESULT: APPROVED\n\
            or\n\
            RESULT: DEFECTS\n\
            - {file_or_output}: {issue} (severity: high|medium|low)\n"
        }
    }
}

fn executor_rules(mode: DelegationMode) -> &'static str {
    match mode {
        DelegationMode::DirectArtifact => {
            "\n\n# RULES: direct_artifact\n\
            Create the exact requested output files first. Do not read unrelated documentation or root workspace docs.\n\
            Use write_file/edit_file/shell as needed, verify the files exist, and respond with artifact paths, commands run, and errors.\n"
        }
        DelegationMode::WardBackedBuild => {
            "\n\n# RULES: ward_backed_build\n\
            Read the supplied ward_snapshot and only the relevant ward files before coding. Reuse registered primitives before creating new ones.\n\
            Execute with write_file/edit_file/shell and follow the exact paths supplied in the delegated task.\n\
            Respond with files changed, commands run, memory updates, and errors.\n"
        }
        DelegationMode::StepExecutor => {
            "\n\n# RULES: step_executor\n\
            Execute the delegated step spec exactly: read Goal, Inputs, Outputs, Acceptance, and any Paths table before writing.\n\
            Write outputs to the specified paths, run acceptance checks, update required summaries/manifests, and respond with step result paths and errors.\n"
        }
    }
}

/// Build specialist instructions from OS.md + tooling shard + role preamble.
/// Does NOT include orchestration/planning instructions — specialists execute, they don't orchestrate.
/// Append OS context and tooling shard to agent instructions.
/// This ensures ALL agents (pre-configured and auto-created) know:
/// - Platform commands (PowerShell vs bash)
/// - Tool syntax (write_file/edit_file, shell, etc.)
pub fn append_system_context(
    instructions: &str,
    paths: &SharedVaultPaths,
    role: SubagentRole,
) -> String {
    // OS context: platform-correct commands (bash vs PowerShell). ~500B.
    let os_context = std::fs::read_to_string(paths.os()).unwrap_or_default();

    // Rules: only append if not already present (delegated agents prepend
    // mode-specific rules in spawn.rs). Direct non-root loads use the
    // conservative ward-backed posture.
    let rules = if instructions.contains("# RULES") {
        "" // Already prepended by spawn.rs
    } else {
        subagent_rules(role, DelegationMode::WardBackedBuild)
    };

    // Memory shard for all agents — subagents are knowledge readers AND writers
    // (Phase 7: subagents can now save facts they learn during execution).
    let memory_shard = gateway_templates::Templates::get("shards/memory_learning.md")
        .map(|f| String::from_utf8_lossy(&f.data).to_string())
        .unwrap_or_default();

    // Ward curation shard — reuse hierarchy + spec-driven development rules.
    // Subagents are the ones writing files inside wards; they need the curation
    // policy as much as (or more than) the root agent.
    let curation_shard = gateway_templates::Templates::get("shards/ward_curation.md")
        .map(|f| String::from_utf8_lossy(&f.data).to_string())
        .unwrap_or_default();

    // Session-ctx shard — teaches every subagent how to read shared session
    // state via memory(get_fact, key="ctx.<sid>.<field>"). Same static text
    // for every subagent, so the provider's prompt cache keeps it warm.
    // Phase 4a of the session-ctx bundle.
    let ctx_shard = gateway_templates::Templates::get("shards/session_ctx.md")
        .map(|f| String::from_utf8_lossy(&f.data).to_string())
        .unwrap_or_default();

    format!(
        "{}\n\n# --- SYSTEM CONTEXT ---\n\n{}\n\n{}\n\n{}\n\n{}{}",
        instructions, os_context, memory_shard, curation_shard, ctx_shard, rules
    )
}

fn append_system_context_without_rules(instructions: &str, paths: &SharedVaultPaths) -> String {
    let os_context = std::fs::read_to_string(paths.os()).unwrap_or_default();
    let memory_shard = gateway_templates::Templates::get("shards/memory_learning.md")
        .map(|f| String::from_utf8_lossy(&f.data).to_string())
        .unwrap_or_default();
    let curation_shard = gateway_templates::Templates::get("shards/ward_curation.md")
        .map(|f| String::from_utf8_lossy(&f.data).to_string())
        .unwrap_or_default();
    let ctx_shard = gateway_templates::Templates::get("shards/session_ctx.md")
        .map(|f| String::from_utf8_lossy(&f.data).to_string())
        .unwrap_or_default();

    format!(
        "{}\n\n# --- SYSTEM CONTEXT ---\n\n{}\n\n{}\n\n{}\n\n{}",
        instructions, os_context, memory_shard, curation_shard, ctx_shard
    )
}

fn build_specialist_instructions(agent_id: &str, paths: &SharedVaultPaths) -> String {
    let role_preamble = generate_role_preamble(agent_id);

    // Load OS context for platform-native commands
    let os_context = std::fs::read_to_string(paths.os()).unwrap_or_default();

    // Load tooling shard for write_file/edit_file syntax and tool docs
    let tooling = gateway_templates::Templates::get("shards/tooling_skills.md")
        .map(|f| String::from_utf8_lossy(&f.data).to_string())
        .unwrap_or_default();

    // Session-ctx shard — auto-created specialists need to know how to
    // read shared session state too. Keep the shard identical to what
    // pre-configured agents get via append_system_context.
    let ctx_shard = gateway_templates::Templates::get("shards/session_ctx.md")
        .map(|f| String::from_utf8_lossy(&f.data).to_string())
        .unwrap_or_default();

    format!(
        "{}\n\n{}\n\n{}\n\n{}",
        role_preamble, os_context, tooling, ctx_shard
    )
}

/// Compose a ward-agent's system prompt: identity line → system-context
/// shards → ward doctrine (`AGENTS.md`). The doctrine is framed under a
/// delimited header so the model can tell identity from conventions; an
/// empty doctrine (fresh ward) omits the section. Free function so the
/// composition is unit-testable without the agent/provider plumbing.
fn compose_ward_agent_instructions(
    identity: &str,
    paths: &SharedVaultPaths,
    ward_name: &str,
    doctrine: &str,
) -> String {
    let mut instructions = append_system_context_without_rules(identity.trim(), paths);
    if !doctrine.trim().is_empty() {
        instructions.push_str(&format!(
            "\n\n# --- WARD DOCTRINE: {ward_name} ---\n\n{}\n",
            doctrine.trim()
        ));
    }
    instructions
}

#[derive(Debug, Default, PartialEq, Eq)]
struct LoadedWardDoctrine {
    doctrine: String,
    diagnostic: Option<&'static str>,
}

const DOCTRINE_TOO_LARGE: &str = "[WARD_DOCTRINE_UNAVAILABLE:too_large] Ward doctrine was not loaded. Ask the user to repair AGENTS.md.";
const DOCTRINE_INVALID_UTF8: &str = "[WARD_DOCTRINE_UNAVAILABLE:invalid_utf8] Ward doctrine was not loaded. Ask the user to repair AGENTS.md.";
const DOCTRINE_UNSAFE_FILE: &str = "[WARD_DOCTRINE_UNAVAILABLE:unsafe_file] Ward doctrine was not loaded. Ask the user to repair AGENTS.md.";
const DOCTRINE_IO_ERROR: &str = "[WARD_DOCTRINE_UNAVAILABLE:io_error] Ward doctrine was not loaded. Ask the user to repair AGENTS.md.";

fn load_ward_doctrine(paths: &SharedVaultPaths, ward_name: &str) -> LoadedWardDoctrine {
    match load_bounded_vault_utf8_file(
        paths.vault_dir(),
        &["wards", ward_name, "AGENTS.md"],
        WARD_AGENT_TEMPLATE_MAX_BYTES,
    ) {
        Ok(doctrine) => LoadedWardDoctrine {
            doctrine,
            diagnostic: None,
        },
        Err(BoundedFileError::Missing) => LoadedWardDoctrine::default(),
        Err(error) => {
            let (code, diagnostic) = match error {
                BoundedFileError::TooLarge => ("too_large", DOCTRINE_TOO_LARGE),
                BoundedFileError::InvalidUtf8 => ("invalid_utf8", DOCTRINE_INVALID_UTF8),
                BoundedFileError::Unsafe => ("unsafe_file", DOCTRINE_UNSAFE_FILE),
                BoundedFileError::Io(_) => ("io_error", DOCTRINE_IO_ERROR),
                BoundedFileError::Missing => unreachable!("missing handled above"),
            };
            tracing::warn!(ward = ward_name, code, "Ward doctrine rejected");
            LoadedWardDoctrine {
                doctrine: String::new(),
                diagnostic: Some(diagnostic),
            }
        }
    }
}

/// Per-ward LLM config from `wards/<ward>/config.yaml`. Each `None` field
/// inherits the orchestrator setting — see [`load_or_seed_ward_config`].
#[derive(Debug, Default, serde::Deserialize)]
struct WardConfig {
    #[serde(default)]
    provider: Option<String>,
    #[serde(default)]
    model: Option<String>,
}

/// Default `config.yaml` seeded into a ward the first time it runs as a
/// ward-agent. Every field is `null`, so the ward inherits the orchestrator
/// until the user edits it.
const DEFAULT_WARD_CONFIG_YAML: &str = "\
# Per-ward LLM config. A null field inherits the orchestrator setting
# (Settings > Advanced > Orchestrator). Set provider/model to override.
provider: null
model: null
";

/// Read `wards/<ward>/config.yaml`, or seed it with [`DEFAULT_WARD_CONFIG_YAML`]
/// and return the all-`None` default. A malformed file logs a warning and
/// falls back to the default rather than failing the delegation.
fn load_or_seed_ward_config(ward_dir: &std::path::Path) -> WardConfig {
    let path = ward_dir.join("config.yaml");
    match std::fs::read_to_string(&path) {
        Ok(raw) => serde_yaml::from_str(&raw).unwrap_or_else(|e| {
            tracing::warn!(
                path = %path.display(),
                error = %e,
                "Malformed ward config.yaml; inheriting orchestrator config"
            );
            WardConfig::default()
        }),
        Err(_) => {
            if let Err(e) = std::fs::write(&path, DEFAULT_WARD_CONFIG_YAML) {
                tracing::warn!(
                    path = %path.display(),
                    error = %e,
                    "Failed to seed ward config.yaml"
                );
            }
            WardConfig::default()
        }
    }
}

/// Generate a role-specific preamble based on the agent name.
fn generate_role_preamble(agent_id: &str) -> String {
    let name_lower = agent_id.to_lowercase();

    let role_description = if name_lower.contains("coder")
        || name_lower.contains("code")
        || name_lower.contains("developer")
        || name_lower.contains("programmer")
    {
        "You are a coding specialist. Write clean, modular, reusable code.\n\
         Use write_file/edit_file for all file creation and editing.\n\
         Follow the coding skill protocol: explore ward, plan, build core/ first, then task scripts.\n\
         Fix broken code — never create _v2 or _improved copies.\n\
         Load the 'coding' skill for detailed instructions."
    } else if name_lower.contains("research") || name_lower.contains("search") {
        "You are a research specialist. Gather, analyze, and synthesize information.\n\
         Use web search and available tools to find accurate, up-to-date information.\n\
         Save findings as structured JSON/markdown files in the ward.\n\
         Cite sources and cross-reference facts."
    } else if name_lower.contains("writ") || name_lower.contains("report") {
        "You are a writing specialist. Create clear, professional documents and reports.\n\
         Use write_file/edit_file to create well-formatted HTML, markdown, or text files.\n\
         Put all output in the output/ directory of the ward.\n\
         Focus on clarity, structure, and visual presentation."
    } else if name_lower.contains("analy") || name_lower.contains("data") {
        "You are a data analysis specialist. Analyze data, compute metrics, and generate insights.\n\
         Write clean Python scripts that import from core/ modules.\n\
         Save results as JSON/CSV in the task subdirectory.\n\
         Load the 'coding' skill for file organization guidelines."
    } else {
        "You are a specialist agent. Execute the task you are given precisely.\n\
         Use write_file/edit_file for file operations. Work in the ward specified in your task.\n\
         Read AGENTS.md before writing code. Follow existing patterns."
    };

    format!("You are **{}**.\n\n{}", agent_id, role_description)
}

/// Format agent ID as display name: "python-coder" → "Python Coder"
fn format_agent_display_name(agent_id: &str) -> String {
    agent_id
        .split('-')
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(c) => format!("{}{}", c.to_uppercase(), chars.collect::<String>()),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ward_config_seeds_default_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = load_or_seed_ward_config(dir.path());
        assert!(cfg.provider.is_none());
        assert!(cfg.model.is_none());
        // The seed file is written so the user can edit it to override.
        let written = std::fs::read_to_string(dir.path().join("config.yaml")).unwrap();
        assert!(written.contains("provider: null"));
        assert!(written.contains("model: null"));
    }

    #[test]
    fn ward_config_seeded_file_reparses_to_all_none() {
        // The seeded `null` fields must round-trip back to `None`.
        let dir = tempfile::tempdir().unwrap();
        let _ = load_or_seed_ward_config(dir.path());
        let cfg = load_or_seed_ward_config(dir.path());
        assert!(cfg.provider.is_none() && cfg.model.is_none());
    }

    #[test]
    fn ward_config_reads_overrides() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("config.yaml"),
            "provider: provider-openai\nmodel: gpt-4o\n",
        )
        .unwrap();
        let cfg = load_or_seed_ward_config(dir.path());
        assert_eq!(cfg.provider.as_deref(), Some("provider-openai"));
        assert_eq!(cfg.model.as_deref(), Some("gpt-4o"));
    }

    #[test]
    fn ward_config_malformed_falls_back_to_default() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("config.yaml"),
            "provider: [not, a, string\n",
        )
        .unwrap();
        let cfg = load_or_seed_ward_config(dir.path());
        assert!(cfg.provider.is_none() && cfg.model.is_none());
    }

    #[test]
    fn test_format_agent_display_name() {
        assert_eq!(format_agent_display_name("python-coder"), "Python Coder");
        assert_eq!(
            format_agent_display_name("research-agent"),
            "Research Agent"
        );
        assert_eq!(format_agent_display_name("analyst"), "Analyst");
    }

    #[test]
    fn test_generate_role_preamble_coder() {
        let preamble = generate_role_preamble("python-coder");
        assert!(preamble.contains("**python-coder**"));
        assert!(preamble.contains("coding specialist"));
    }

    #[test]
    fn test_generate_role_preamble_researcher() {
        let preamble = generate_role_preamble("research-agent");
        assert!(preamble.contains("research specialist"));
    }

    #[test]
    fn test_generate_role_preamble_writer() {
        let preamble = generate_role_preamble("writing-agent");
        assert!(preamble.contains("writing specialist"));
    }

    #[test]
    fn test_generate_role_preamble_analyst() {
        let preamble = generate_role_preamble("data-analyst");
        assert!(preamble.contains("data analysis specialist"));
    }

    #[test]
    fn test_generate_role_preamble_generic() {
        let preamble = generate_role_preamble("helper-bot");
        assert!(preamble.contains("specialist agent"));
    }

    #[test]
    fn compose_ward_agent_instructions_places_identity_then_doctrine() {
        let tmp = tempfile::TempDir::new().unwrap();
        let paths: SharedVaultPaths =
            Arc::new(gateway_services::VaultPaths::new(tmp.path().to_path_buf()));
        paths.ensure_dirs_exist().unwrap();
        let out = compose_ward_agent_instructions(
            "You are the maritime ward-agent.",
            &paths,
            "maritime",
            "## Identity\nPersistent maritime steward.\n\n## Persona\nEvidence-led and explicit about uncertainty.\n\n## Self-Maintenance\nPropose durable doctrine changes and edit only with explicit user direction.",
        );
        assert!(out.starts_with("You are the maritime ward-agent."));
        assert!(out.contains("# --- WARD DOCTRINE: maritime ---"));
        assert!(out.contains("Persistent maritime steward."));
        assert!(out.contains("Evidence-led and explicit about uncertainty."));
        assert!(out.contains(
            "Propose durable doctrine changes and edit only with explicit user direction."
        ));
    }

    #[test]
    fn compose_ward_agent_instructions_omits_empty_doctrine() {
        let tmp = tempfile::TempDir::new().unwrap();
        let paths: SharedVaultPaths =
            Arc::new(gateway_services::VaultPaths::new(tmp.path().to_path_buf()));
        paths.ensure_dirs_exist().unwrap();
        let out = compose_ward_agent_instructions("identity line", &paths, "maritime", "   ");
        assert!(!out.contains("WARD DOCTRINE"));
    }

    // STUB: AC5
    #[test]
    fn load_ward_doctrine_preserves_complete_valid_content() {
        let tmp = tempfile::TempDir::new().unwrap();
        let paths: SharedVaultPaths =
            Arc::new(gateway_services::VaultPaths::new(tmp.path().to_path_buf()));
        paths.ensure_dirs_exist().unwrap();
        let ward = paths.ward_dir("maritime");
        std::fs::create_dir_all(&ward).unwrap();
        std::fs::write(ward.join("AGENTS.md"), "  exact doctrine\nbody  \n").unwrap();

        let loaded = load_ward_doctrine(&paths, "maritime");
        assert_eq!(loaded.doctrine, "  exact doctrine\nbody  \n");
        assert!(loaded.diagnostic.is_none());
        let out = compose_ward_agent_instructions("identity", &paths, "maritime", &loaded.doctrine);
        assert!(out.contains("# --- WARD DOCTRINE: maritime ---\n\nexact doctrine\nbody\n"));
    }

    // STUB: AC6
    #[test]
    fn load_ward_doctrine_rejects_oversized_without_partial_content() {
        let tmp = tempfile::TempDir::new().unwrap();
        let paths: SharedVaultPaths =
            Arc::new(gateway_services::VaultPaths::new(tmp.path().to_path_buf()));
        paths.ensure_dirs_exist().unwrap();
        let ward = paths.ward_dir("oversized");
        std::fs::create_dir_all(&ward).unwrap();
        let payload = "secret-marker".repeat(2048);
        std::fs::write(ward.join("AGENTS.md"), &payload).unwrap();

        let loaded = load_ward_doctrine(&paths, "oversized");
        assert!(loaded.doctrine.is_empty());
        let diagnostic = loaded.diagnostic.expect("fixed diagnostic");
        assert!(diagnostic.is_ascii());
        assert!(diagnostic.len() <= 160);
        assert!(diagnostic.contains("WARD_DOCTRINE_UNAVAILABLE:too_large"));
        assert!(!diagnostic.contains("secret-marker"));
        assert!(!diagnostic.contains('/'));
    }

    // STUB: AC7
    #[test]
    fn load_ward_doctrine_treats_missing_as_empty_without_diagnostic() {
        let tmp = tempfile::TempDir::new().unwrap();
        let paths: SharedVaultPaths =
            Arc::new(gateway_services::VaultPaths::new(tmp.path().to_path_buf()));
        paths.ensure_dirs_exist().unwrap();
        std::fs::create_dir_all(paths.ward_dir("empty")).unwrap();

        let loaded = load_ward_doctrine(&paths, "empty");
        assert!(loaded.doctrine.is_empty());
        assert!(loaded.diagnostic.is_none());
    }

    // STUB: AC6
    #[cfg(unix)]
    #[test]
    fn load_ward_doctrine_rejects_symlink_hardlink_special_and_non_utf8() {
        use std::os::unix::fs::symlink;

        for case in ["symlink", "hardlink", "fifo", "non-utf8"] {
            let tmp = tempfile::TempDir::new().unwrap();
            let paths: SharedVaultPaths =
                Arc::new(gateway_services::VaultPaths::new(tmp.path().to_path_buf()));
            paths.ensure_dirs_exist().unwrap();
            let ward = paths.ward_dir(case);
            std::fs::create_dir_all(&ward).unwrap();
            let doctrine = ward.join("AGENTS.md");
            match case {
                "symlink" => {
                    let source = tmp.path().join("outside.md");
                    std::fs::write(&source, "secret-symlink").unwrap();
                    symlink(source, &doctrine).unwrap();
                }
                "hardlink" => {
                    let source = tmp.path().join("outside.md");
                    std::fs::write(&source, "secret-hardlink").unwrap();
                    std::fs::hard_link(source, &doctrine).unwrap();
                }
                "fifo" => {
                    assert!(std::process::Command::new("mkfifo")
                        .arg(&doctrine)
                        .status()
                        .unwrap()
                        .success());
                }
                "non-utf8" => std::fs::write(&doctrine, [0xff, 0xfe]).unwrap(),
                _ => unreachable!(),
            }

            let loaded = load_ward_doctrine(&paths, case);
            assert!(loaded.doctrine.is_empty(), "case {case}");
            let diagnostic = loaded.diagnostic.expect("fixed diagnostic");
            assert!(diagnostic.is_ascii(), "case {case}");
            assert!(diagnostic.len() <= 160, "case {case}");
            assert!(!diagnostic.contains("secret"), "case {case}");
            assert!(!diagnostic.contains('/'), "case {case}");
        }
    }

    #[test]
    fn scaffolded_ward_persona_is_preserved_as_synthesized_doctrine() {
        let tmp = tempfile::TempDir::new().unwrap();
        let paths: SharedVaultPaths =
            Arc::new(gateway_services::VaultPaths::new(tmp.path().to_path_buf()));
        paths.ensure_dirs_exist().unwrap();
        gateway_services::seed_default_ward_layout_template(&paths).unwrap();
        gateway_services::seed_default_ward_agent_template(&paths).unwrap();
        let created = gateway_services::create_ward_from_template(&paths, "market-research")
            .expect("scaffold ward");
        let doctrine = std::fs::read_to_string(created.path.join("AGENTS.md")).unwrap();

        let out = compose_ward_agent_instructions(
            "You are the market-research ward-agent.",
            &paths,
            "market-research",
            &doctrine,
        );

        assert!(out.contains("# --- WARD DOCTRINE: market-research ---"));
        assert!(out.contains("# Market Research Ward Agent"));
        assert!(out.contains("## Identity"));
        assert!(out.contains("## Persona"));
        assert!(out.contains("## Self-Maintenance"));
        assert!(out.contains("Never delete or rewrite existing persona text"));
    }
}
