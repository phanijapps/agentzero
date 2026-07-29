//! Gateway-side adapter for the [`agent_tools::WardUsageAccess`] trait.
//!
//! Wraps an `Arc<gateway_services::WardUsage>` so the `agent-tools` crate
//! can fire `created_by = "agent"` marks without depending on
//! `gateway-services`. Mirrors `kg_store_adapter`, `ingest_adapter`, etc.

use std::sync::Arc;

use agent_primitives::WardArchetypeId;
use async_trait::async_trait;
use gateway_services::{WardProvenance, WardUsage};

pub struct WardUsageAdapter {
    inner: Arc<WardUsage>,
}

impl WardUsageAdapter {
    pub fn new(inner: Arc<WardUsage>) -> Self {
        Self { inner }
    }
}

#[async_trait]
impl agent_tools::WardUsageAccess for WardUsageAdapter {
    async fn mark_created_agent(&self, ward: &str, archetype: WardArchetypeId) {
        if let Err(e) =
            self.inner
                .mark_created_with_archetype(ward, WardProvenance::Agent, Some(archetype))
        {
            tracing::warn!(
                ward = %ward,
                error = %e,
                "ward_usage.mark_created(Agent) failed"
            );
        }
    }
}
