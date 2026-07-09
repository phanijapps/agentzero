//! `Procedure` domain type.

use serde::{Deserialize, Serialize};

/// A learned procedure pattern with execution statistics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Procedure {
    pub id: String,
    pub agent_id: String,
    pub ward_id: Option<String>,
    pub name: String,
    pub description: String,
    pub trigger_pattern: Option<String>,
    pub steps: String,
    pub parameters: Option<String>,
    pub success_count: i32,
    pub failure_count: i32,
    pub avg_duration_ms: Option<i64>,
    pub avg_token_cost: Option<i64>,
    pub last_used: Option<String>,
    /// Raw f32 embedding. Always `None` when loaded from a backend that
    /// stores vectors out-of-row.
    #[serde(skip)]
    pub embedding: Option<Vec<f32>>,
    pub created_at: String,
    pub updated_at: String,
}

/// Result of `ProcedureStore::get_procedure_summary_by_name`. Captures
/// only the dedup-relevant fields so callers don't pay for hydrating
/// the full row.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcedureSummary {
    pub id: String,
    pub name: String,
    pub success_count: i32,
}

/// Request shape for `ProcedureStore::insert_pattern_procedure`.
/// Pre-built from the LLM's structured response by `PatternExtractor`.
/// Flat field set so backends materialise their canonical procedure
/// row without callers needing to know the full `Procedure` shape.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatternProcedureInsert {
    pub agent_id: String,
    pub ward_id: Option<String>,
    pub name: String,
    pub description: String,
    pub trigger_pattern: Option<String>,
    /// JSON-serialised `Vec<PatternStep>`.
    pub steps_json: String,
    /// JSON-serialised `Vec<String>`, or `None` if no parameters.
    pub parameters_json: Option<String>,
    /// Pre-computed embedding for the procedure description.
    /// Populated by the writer (PatternExtractor / distillation) so the
    /// SQLite store can upsert into `procedures_index`.
    #[serde(default)]
    pub embedding: Option<Vec<f32>>,
    /// Initial `success_count` to record when inserting. The mining itself
    /// is evidence of N successful sessions — recording sc=1 would throw
    /// that away and leave the procedure below the middleware's promotion /
    /// legacy-advisory floors. Writers pass the number of matching sessions
    /// they saw (PatternExtractor: the pair, so 2). Defaults to 2 for
    /// backward compat with callers that omit it.
    #[serde(default = "default_initial_success_count")]
    pub success_count: i32,
}

fn default_initial_success_count() -> i32 {
    2
}

/// One step of a learned procedure. The `action` is a tool name validated
/// strict against the live `ToolRegistry` at run time. `args` may carry
/// `{step_N.field}` interpolation tokens (resolved by `RunProcedureTool`).
/// `binds` lists fields to extract from this step's result for use in
/// later interpolations.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PatternStep {
    pub action: String,
    #[serde(default)]
    pub args: serde_json::Map<String, serde_json::Value>,
    #[serde(default)]
    pub binds: Vec<String>,
    #[serde(default)]
    pub agent: Option<String>,
    #[serde(default)]
    pub note: Option<String>,
    #[serde(default)]
    pub task_template: Option<String>,
}
