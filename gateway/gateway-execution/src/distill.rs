//! The distiller seam.
//!
//! `SessionDistiller` is a concrete type, and the runner stores
//! `Option<Arc<SessionDistiller>>` — which forced the golden-task harness to
//! run distiller-less (a documented gap). This trait extracts the single
//! operation the execution paths actually perform
//! ([`Distill::distill`]); the concrete distiller implements it by
//! delegation, tests implement it with a recording stub, and the runner
//! fields accept `Arc<dyn Distill>`.

use std::sync::Arc;

use async_trait::async_trait;

/// The distillation surface the execution paths depend on.
#[async_trait]
pub trait Distill: Send + Sync {
    /// Distill a completed session into durable memory. Returns the number
    /// of facts upserted.
    async fn distill(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<usize, distillation::DistillationError>;
}

#[async_trait]
impl Distill for distillation::SessionDistiller {
    async fn distill(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<usize, distillation::DistillationError> {
        // Method resolution on the concrete type prefers the inherent
        // `SessionDistiller::distill` — no recursion.
        self.distill(session_id, agent_id).await
    }
}

/// Coercion helper for construction sites that hold the concrete type.
pub fn distill_sink(
    distiller: Option<Arc<distillation::SessionDistiller>>,
) -> Option<Arc<dyn Distill>> {
    distiller.map(|distiller| distiller as Arc<dyn Distill>)
}
