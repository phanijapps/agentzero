//! Scope translation between AgentZero host concepts and Engram scopes.

use engram_domain::Scope;
use serde::{Deserialize, Serialize};

use crate::error::{AdapterError, AdapterResult};

/// Engram scope field targeted by an AgentZero identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScopeTarget {
    /// Map to `Scope.workspace`.
    Workspace,
    /// Map to `Scope.session`.
    Session,
    /// Map to `Scope.environment`.
    Environment,
    /// Map to `Scope.subject`.
    Subject,
}

/// Maps AgentZero ward/session/partition identifiers into Engram `Scope`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopeMapper {
    tenant: String,
    ward_target: ScopeTarget,
    partition_target: ScopeTarget,
}

impl ScopeMapper {
    /// Construct a mapper after validating tenant isolation input.
    pub(crate) fn new(
        tenant: String,
        ward_target: ScopeTarget,
        partition_target: ScopeTarget,
    ) -> AdapterResult<Self> {
        let tenant = validated_component("tenant", &tenant)?.to_string();

        Ok(Self {
            tenant,
            ward_target,
            partition_target,
        })
    }

    /// Map an AgentZero ward ID, including `__global__`, without widening scope.
    pub fn ward_scope(&self, ward_id: &str) -> AdapterResult<Scope> {
        let ward_id = validated_component("ward_id", ward_id)?;
        reject_session_target("ward_scope_target", self.ward_target)?;
        Ok(self.scope_with(self.ward_target, ward_id, None))
    }

    /// Map a belief partition ID without hard-coding `root`.
    pub fn partition_scope(&self, partition_id: &str) -> AdapterResult<Scope> {
        let partition_id = validated_component("partition_id", partition_id)?;
        reject_session_target("partition_scope_target", self.partition_target)?;
        Ok(self.scope_with(self.partition_target, partition_id, None))
    }

    /// Map a memory fact scope with optional session locality.
    pub fn memory_fact_scope(
        &self,
        ward_id: &str,
        session_id: Option<&str>,
    ) -> AdapterResult<Scope> {
        let ward_id = validated_component("ward_id", ward_id)?;
        reject_session_target("ward_scope_target", self.ward_target)?;
        let session_id = session_id
            .map(|value| validated_component("session_id", value))
            .transpose()?;
        Ok(self.scope_with(self.ward_target, ward_id, session_id))
    }

    fn scope_with(&self, target: ScopeTarget, value: &str, session_id: Option<&str>) -> Scope {
        let mut scope = Scope {
            tenant: self.tenant.clone(),
            subject: None,
            workspace: None,
            session: session_id.map(ToOwned::to_owned),
            environment: None,
        };
        match target {
            ScopeTarget::Workspace => scope.workspace = Some(value.to_string()),
            ScopeTarget::Session => scope.session = Some(value.to_string()),
            ScopeTarget::Environment => scope.environment = Some(value.to_string()),
            ScopeTarget::Subject => scope.subject = Some(value.to_string()),
        }
        scope
    }
}

fn reject_session_target(component: &'static str, target: ScopeTarget) -> AdapterResult<()> {
    if target == ScopeTarget::Session {
        return Err(AdapterError::InvalidScope {
            component,
            reason: "session scope is reserved for actual session identity".to_string(),
        });
    }
    Ok(())
}

fn validated_component<'a>(component: &'static str, value: &'a str) -> AdapterResult<&'a str> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(AdapterError::InvalidScope {
            component,
            reason: "empty value".to_string(),
        });
    }
    Ok(trimmed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ward_scope_maps_global_without_widening() {
        let mapper = ScopeMapper::new(
            "tenant-a".to_string(),
            ScopeTarget::Workspace,
            ScopeTarget::Workspace,
        )
        .expect("mapper");

        let scope = mapper.ward_scope("__global__").expect("scope");

        assert_eq!(scope.tenant, "tenant-a");
        assert_eq!(scope.workspace.as_deref(), Some("__global__"));
        assert_eq!(scope.session, None);
    }

    #[test]
    fn memory_fact_scope_preserves_session_locality() {
        let mapper = ScopeMapper::new(
            "tenant-a".to_string(),
            ScopeTarget::Workspace,
            ScopeTarget::Workspace,
        )
        .expect("mapper");

        let scope = mapper
            .memory_fact_scope("ward-a", Some("session-a"))
            .expect("scope");

        assert_eq!(scope.workspace.as_deref(), Some("ward-a"));
        assert_eq!(scope.session.as_deref(), Some("session-a"));
    }

    #[test]
    fn partition_scope_uses_configured_target() {
        let mapper = ScopeMapper::new(
            "tenant-a".to_string(),
            ScopeTarget::Workspace,
            ScopeTarget::Environment,
        )
        .expect("mapper");

        let scope = mapper.partition_scope("root").expect("scope");

        assert_eq!(scope.workspace, None);
        assert_eq!(scope.environment.as_deref(), Some("root"));
    }

    #[test]
    fn blank_scope_component_is_an_error() {
        let mapper = ScopeMapper::new(
            "tenant-a".to_string(),
            ScopeTarget::Workspace,
            ScopeTarget::Workspace,
        )
        .expect("mapper");

        assert_eq!(
            mapper.ward_scope(" "),
            Err(AdapterError::InvalidScope {
                component: "ward_id",
                reason: "empty value".to_string(),
            })
        );
    }

    #[test]
    fn memory_fact_scope_rejects_session_target_collision() {
        let mapper = ScopeMapper::new(
            "tenant-a".to_string(),
            ScopeTarget::Session,
            ScopeTarget::Workspace,
        )
        .expect("mapper");

        assert_eq!(
            mapper.memory_fact_scope("ward-a", Some("session-a")),
            Err(AdapterError::InvalidScope {
                component: "ward_scope_target",
                reason: "session scope is reserved for actual session identity".to_string(),
            })
        );
    }

    #[test]
    fn partition_scope_rejects_session_target_collision() {
        let mapper = ScopeMapper::new(
            "tenant-a".to_string(),
            ScopeTarget::Workspace,
            ScopeTarget::Session,
        )
        .expect("mapper");

        assert_eq!(
            mapper.partition_scope("partition-a"),
            Err(AdapterError::InvalidScope {
                component: "partition_scope_target",
                reason: "session scope is reserved for actual session identity".to_string(),
            })
        );
    }

    #[test]
    fn mapper_rejects_empty_tenant() {
        assert_eq!(
            ScopeMapper::new(
                " ".to_string(),
                ScopeTarget::Workspace,
                ScopeTarget::Workspace,
            ),
            Err(AdapterError::InvalidScope {
                component: "tenant",
                reason: "empty value".to_string(),
            })
        );
    }
}
