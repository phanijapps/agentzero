//! Late-installed execution integrations shared by pre-captured invokers.
use std::sync::{Arc, RwLock};

#[derive(Clone, Default)]
pub struct RunnerIntegrations {
    pub kg_store: Option<Arc<dyn knowledge_graph::kg_trait::KnowledgeGraphStore>>,
    pub kg_episode_store: Option<Arc<dyn zbot_stores_traits::KgEpisodeStore>>,
    pub ingestion_adapter: Option<Arc<dyn agent_tools::IngestionAccess>>,
    pub goal_adapter: Option<Arc<dyn agent_tools::GoalAccess>>,
    pub belief_store: Option<Arc<dyn zbot_stores_traits::BeliefStore>>,
    pub belief_contradiction_store: Option<Arc<dyn zbot_stores_traits::BeliefContradictionStore>>,
}

#[derive(Clone, Default)]
pub struct SharedIntegrations(Arc<RwLock<RunnerIntegrations>>);
impl SharedIntegrations {
    /// Clone current handles; no lock guard escapes into async execution.
    pub fn snapshot(&self) -> RunnerIntegrations {
        self.0
            .read()
            .expect("execution integrations lock poisoned")
            .clone()
    }
    pub fn set_kg_store(&self, store: Arc<dyn knowledge_graph::kg_trait::KnowledgeGraphStore>) {
        self.0
            .write()
            .expect("execution integrations lock poisoned")
            .kg_store = Some(store);
    }
    pub fn set_kg_episode_store(&self, store: Arc<dyn zbot_stores_traits::KgEpisodeStore>) {
        self.0
            .write()
            .expect("execution integrations lock poisoned")
            .kg_episode_store = Some(store);
    }
    pub fn set_ingestion_adapter(&self, adapter: Arc<dyn agent_tools::IngestionAccess>) {
        self.0
            .write()
            .expect("execution integrations lock poisoned")
            .ingestion_adapter = Some(adapter);
    }
    pub fn set_belief_stores(
        &self,
        belief_store: Option<Arc<dyn zbot_stores_traits::BeliefStore>>,
        belief_contradiction_store: Option<Arc<dyn zbot_stores_traits::BeliefContradictionStore>>,
    ) {
        self.0
            .write()
            .expect("execution integrations lock poisoned")
            .belief_store = belief_store;
        self.0
            .write()
            .expect("execution integrations lock poisoned")
            .belief_contradiction_store = belief_contradiction_store;
    }

    pub fn set_goal_adapter(&self, adapter: Arc<dyn agent_tools::GoalAccess>) {
        self.0
            .write()
            .expect("execution integrations lock poisoned")
            .goal_adapter = Some(adapter);
    }
}
