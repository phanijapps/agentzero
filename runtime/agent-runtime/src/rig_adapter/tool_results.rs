//! One in-flight tool outcome per run; Rig dispatch remains sequential.
use std::sync::{Arc, Mutex};

#[derive(Clone, Default)]
pub(super) struct ToolOutcome {
    pub raw: Option<String>,
    pub error: Option<String>,
    pub duration_ms: i64,
    pub context: Option<String>,
    pub rejected_call: Option<(String, serde_json::Value)>,
}

#[derive(Default)]
pub(super) struct ToolResults {
    outcome: Mutex<ToolOutcome>,
    pub peer_influenced: bool,
}

pub(super) type SharedToolResults = Arc<ToolResults>;

impl ToolResults {
    pub fn record(&self, outcome: ToolOutcome) {
        *self.outcome.lock().unwrap() = outcome;
    }

    pub fn snapshot(&self) -> ToolOutcome {
        self.outcome.lock().unwrap().clone()
    }

    pub fn take(&self) -> ToolOutcome {
        std::mem::take(&mut *self.outcome.lock().unwrap())
    }
}
