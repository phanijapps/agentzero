//! Actor policy — who may use which tool capability, and the delegated
//! subagent guard hook.

use agent_runtime::{ContextActorKind, ToolDecision};
use serde_json::Value;

use super::setup::SubagentRole;

/// Runtime actor profile used to derive first-party tool capabilities.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeActorKind {
    Root,
    DelegatedExecutor,
    DelegatedReviewer,
    WardAgent,
    RemotePeer,
}

impl RuntimeActorKind {
    pub(crate) fn as_state_value(self) -> &'static str {
        match self {
            Self::Root => "root",
            Self::DelegatedExecutor => "delegated_executor",
            Self::DelegatedReviewer => "delegated_reviewer",
            Self::WardAgent => "ward_agent",
            Self::RemotePeer => "remote_peer",
        }
    }

    pub(crate) fn is_delegated_execution(self) -> bool {
        !matches!(self, Self::Root)
    }

    pub(crate) fn is_ordinary_subagent(self) -> bool {
        matches!(self, Self::DelegatedExecutor | Self::DelegatedReviewer)
    }

    pub(crate) fn subagent_role(self) -> Option<SubagentRole> {
        match self {
            Self::DelegatedExecutor => Some(SubagentRole::Executor),
            Self::DelegatedReviewer => Some(SubagentRole::Reviewer),
            Self::Root | Self::WardAgent | Self::RemotePeer => None,
        }
    }
}

impl From<SubagentRole> for RuntimeActorKind {
    fn from(role: SubagentRole) -> Self {
        match role {
            SubagentRole::Executor => Self::DelegatedExecutor,
            SubagentRole::Reviewer => Self::DelegatedReviewer,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ToolCapability {
    AgentControl,
    AgentDelegate,
    AgentReply,
    ConnectorInvoke,
    ConnectorQuery,
    ConnectorResourceRead,
    FileRead,
    FileWrite,
    GoalWrite,
    GraphRead,
    IngestWrite,
    MemoryRead,
    MemoryWrite,
    MultimodalAnalyze,
    PeerDelegate,
    PlanWrite,
    ProcedureRun,
    Respond,
    Shell,
    SkillLoad,
    SurfacePresent,
    WardRead,
    WardWrite,
}

impl ToolCapability {
    pub(crate) fn as_state_value(self) -> &'static str {
        match self {
            Self::AgentControl => "agent.control",
            Self::AgentDelegate => "agent.delegate",
            Self::AgentReply => "agent.reply",
            Self::ConnectorInvoke => "connector.invoke",
            Self::ConnectorQuery => "connector.query",
            Self::ConnectorResourceRead => "connector.resource.read",
            Self::FileRead => "fs.read",
            Self::FileWrite => "fs.write",
            Self::GoalWrite => "goal.write",
            Self::GraphRead => "graph.read",
            Self::IngestWrite => "ingest.write",
            Self::MemoryRead => "memory.read",
            Self::MemoryWrite => "memory.write",
            Self::MultimodalAnalyze => "multimodal.analyze",
            Self::PeerDelegate => "peer.delegate",
            Self::PlanWrite => "plan.write",
            Self::ProcedureRun => "procedure.run",
            Self::Respond => "respond",
            Self::Shell => "process.shell",
            Self::SkillLoad => "skill.load",
            Self::SurfacePresent => "surface.present",
            Self::WardRead => "ward.read",
            Self::WardWrite => "ward.write",
        }
    }
}

const ALL_CAPABILITIES: &[ToolCapability] = &[
    ToolCapability::AgentControl,
    ToolCapability::AgentDelegate,
    ToolCapability::AgentReply,
    ToolCapability::ConnectorInvoke,
    ToolCapability::ConnectorQuery,
    ToolCapability::ConnectorResourceRead,
    ToolCapability::FileRead,
    ToolCapability::FileWrite,
    ToolCapability::GoalWrite,
    ToolCapability::GraphRead,
    ToolCapability::IngestWrite,
    ToolCapability::MemoryRead,
    ToolCapability::MemoryWrite,
    ToolCapability::MultimodalAnalyze,
    ToolCapability::PeerDelegate,
    ToolCapability::PlanWrite,
    ToolCapability::ProcedureRun,
    ToolCapability::Respond,
    ToolCapability::Shell,
    ToolCapability::SkillLoad,
    ToolCapability::SurfacePresent,
    ToolCapability::WardRead,
    ToolCapability::WardWrite,
];

/// Root: orchestration + everything except skill loading and plain file-read
/// tools (those arrive through delegates). Delegated executors: implementation
/// tools without orchestration. Reviewers: read-only + reply. Ward agents:
/// everything. Remote peers: respond only.
pub(crate) fn actor_allows(actor: RuntimeActorKind, capability: ToolCapability) -> bool {
    match actor {
        RuntimeActorKind::Root => matches!(
            capability,
            ToolCapability::AgentControl
                | ToolCapability::AgentDelegate
                | ToolCapability::AgentReply
                | ToolCapability::ConnectorInvoke
                | ToolCapability::ConnectorQuery
                | ToolCapability::ConnectorResourceRead
                | ToolCapability::FileRead
                | ToolCapability::GoalWrite
                | ToolCapability::GraphRead
                | ToolCapability::IngestWrite
                | ToolCapability::MemoryRead
                | ToolCapability::MemoryWrite
                | ToolCapability::MultimodalAnalyze
                | ToolCapability::PeerDelegate
                | ToolCapability::PlanWrite
                | ToolCapability::ProcedureRun
                | ToolCapability::Respond
                | ToolCapability::Shell
                | ToolCapability::SurfacePresent
                | ToolCapability::WardRead
                | ToolCapability::WardWrite
        ),
        RuntimeActorKind::DelegatedExecutor => matches!(
            capability,
            ToolCapability::AgentReply
                | ToolCapability::FileRead
                | ToolCapability::FileWrite
                | ToolCapability::GoalWrite
                | ToolCapability::GraphRead
                | ToolCapability::IngestWrite
                | ToolCapability::MemoryRead
                | ToolCapability::MemoryWrite
                | ToolCapability::MultimodalAnalyze
                | ToolCapability::Respond
                | ToolCapability::Shell
                | ToolCapability::SkillLoad
                | ToolCapability::WardRead
                | ToolCapability::WardWrite
        ),
        RuntimeActorKind::DelegatedReviewer => matches!(
            capability,
            ToolCapability::AgentReply
                | ToolCapability::FileRead
                | ToolCapability::GraphRead
                | ToolCapability::MemoryRead
                | ToolCapability::MultimodalAnalyze
                | ToolCapability::Respond
                | ToolCapability::SkillLoad
                | ToolCapability::WardRead
        ),
        RuntimeActorKind::WardAgent => true,
        RuntimeActorKind::RemotePeer => matches!(capability, ToolCapability::Respond),
    }
}

pub(crate) fn actor_allows_all(actor: RuntimeActorKind, capabilities: &[ToolCapability]) -> bool {
    capabilities
        .iter()
        .copied()
        .all(|capability| actor_allows(actor, capability))
}

pub(crate) fn actor_capabilities(actor: RuntimeActorKind) -> Vec<&'static str> {
    ALL_CAPABILITIES
        .iter()
        .copied()
        .filter(|capability| actor_allows(actor, *capability))
        .map(ToolCapability::as_state_value)
        .collect()
}

pub(crate) fn context_actor_kind(actor: RuntimeActorKind) -> ContextActorKind {
    match actor {
        RuntimeActorKind::Root => ContextActorKind::Root,
        RuntimeActorKind::DelegatedExecutor => ContextActorKind::DelegatedExecutor,
        RuntimeActorKind::DelegatedReviewer => ContextActorKind::DelegatedReviewer,
        RuntimeActorKind::WardAgent => ContextActorKind::WardAgent,
        RuntimeActorKind::RemotePeer => ContextActorKind::RemotePeer,
    }
}

pub(crate) fn actor_policy_for_capabilities(
    fallback_actor: RuntimeActorKind,
    capabilities: &[ToolCapability],
) -> Vec<ContextActorKind> {
    if capabilities.is_empty() {
        return vec![context_actor_kind(fallback_actor)];
    }

    [
        RuntimeActorKind::Root,
        RuntimeActorKind::DelegatedExecutor,
        RuntimeActorKind::DelegatedReviewer,
        RuntimeActorKind::WardAgent,
        RuntimeActorKind::RemotePeer,
    ]
    .into_iter()
    .filter(|actor| actor_allows_all(*actor, capabilities))
    .map(context_actor_kind)
    .collect()
}

/// Delegated-executor guard hooks: block shell-as-file-writer bypass and
/// inject failure guidance to reduce fix-retry loops. Behavior preserved
/// verbatim from the previous closure hooks.
pub(crate) struct SubagentGuardHook;

#[async_trait::async_trait]
impl agent_runtime::EngineHook for SubagentGuardHook {
    async fn before_tool(&self, tool_name: &str, args: &Value) -> ToolDecision {
        if tool_name == "shell" {
            let cmd = args.get("command").and_then(|v| v.as_str()).unwrap_or("");
            // Block shell commands that create/write files — use write_file instead
            if cmd.contains("> ")
                || cmd.contains("cat <<")
                || cmd.contains("heredoc")
                || cmd.contains("echo \"") && cmd.contains("> ")
                || cmd.contains("printf") && cmd.contains("> ")
                || cmd.contains("tee ")
            {
                return ToolDecision::Block {
                    reason: "Use write_file to create files, not shell redirects. Shell is for running commands and reading output.".to_string() };
            }
        }
        ToolDecision::Allow
    }

    async fn after_tool(
        &self,
        tool_name: &str,
        _args: &Value,
        result: &str,
        succeeded: bool,
    ) -> Option<String> {
        if !succeeded && tool_name == "shell" {
            Some(format!(
                "{}\n\n[SYSTEM: Command failed. Read the error. Fix the ROOT CAUSE in your code, \
                         not the symptom. Do not retry the same command — fix the file first with edit_file.]",
                result
            ))
        } else if !succeeded {
            Some(format!(
                "{}\n\n[SYSTEM: Tool failed. Read the error carefully before retrying.]",
                result
            ))
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_keeps_orchestration_without_skill_load_or_plain_file_writes() {
        for capability in ALL_CAPABILITIES {
            let allowed = actor_allows(RuntimeActorKind::Root, *capability);
            match capability {
                ToolCapability::SkillLoad | ToolCapability::FileWrite => assert!(!allowed),
                _ => assert!(allowed, "root must allow {capability:?}"),
            }
        }
    }

    #[test]
    fn delegated_executor_keeps_implementation_without_orchestration() {
        for capability in ALL_CAPABILITIES {
            let allowed = actor_allows(RuntimeActorKind::DelegatedExecutor, *capability);
            match capability {
                ToolCapability::FileWrite | ToolCapability::Shell | ToolCapability::SkillLoad => {
                    assert!(allowed)
                }
                ToolCapability::AgentControl
                | ToolCapability::AgentDelegate
                | ToolCapability::ConnectorInvoke
                | ToolCapability::ConnectorQuery
                | ToolCapability::ConnectorResourceRead => assert!(!allowed),
                ToolCapability::PeerDelegate
                | ToolCapability::ProcedureRun
                | ToolCapability::SurfacePresent
                | ToolCapability::PlanWrite => assert!(!allowed),
                _ => {}
            }
        }
    }

    #[test]
    fn delegated_reviewer_is_read_only_and_non_orchestrating() {
        for capability in ALL_CAPABILITIES {
            let allowed = actor_allows(RuntimeActorKind::DelegatedReviewer, *capability);
            match capability {
                ToolCapability::AgentReply
                | ToolCapability::FileRead
                | ToolCapability::GraphRead
                | ToolCapability::MemoryRead
                | ToolCapability::MultimodalAnalyze
                | ToolCapability::Respond
                | ToolCapability::SkillLoad
                | ToolCapability::WardRead => assert!(allowed),
                _ => assert!(!allowed, "reviewer must not have {capability:?}"),
            }
        }
    }

    #[test]
    fn ward_agent_and_remote_peer_extremes() {
        for capability in ALL_CAPABILITIES {
            assert!(actor_allows(RuntimeActorKind::WardAgent, *capability));
        }
        for capability in ALL_CAPABILITIES {
            let allowed = actor_allows(RuntimeActorKind::RemotePeer, *capability);
            if *capability == ToolCapability::Respond {
                assert!(allowed);
            } else {
                assert!(!allowed);
            }
        }
    }

    #[test]
    fn actor_kind_conversions() {
        assert_eq!(
            RuntimeActorKind::from(SubagentRole::Executor),
            RuntimeActorKind::DelegatedExecutor
        );
        assert_eq!(
            RuntimeActorKind::from(SubagentRole::Reviewer),
            RuntimeActorKind::DelegatedReviewer
        );
        assert!(!RuntimeActorKind::Root.is_delegated_execution());
        assert!(RuntimeActorKind::DelegatedExecutor.is_ordinary_subagent());
        assert!(!RuntimeActorKind::WardAgent.is_ordinary_subagent());
        assert_eq!(RuntimeActorKind::Root.subagent_role(), None);
        assert_eq!(
            RuntimeActorKind::DelegatedExecutor.subagent_role(),
            Some(SubagentRole::Executor)
        );
    }
}
