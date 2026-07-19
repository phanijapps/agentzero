# Intent Analysis — Types Reference

## Backend Types (Rust)

All defined in `gateway/gateway-execution/src/middleware/intent_analysis.rs`.

### IntentAnalysis

Top-level analysis result from the LLM.

```rust
pub struct IntentAnalysis {
    pub primary_intent: String,
    pub hidden_intents: Vec<String>,
    pub recommended_skills: Vec<String>,
    pub recommended_agents: Vec<String>,
    pub ward_recommendation: WardRecommendation,
    pub execution_strategy: ExecutionStrategy,
    #[serde(default)]
    pub rewritten_prompt: String,  // Kept for backward compat, no longer requested from LLM
}
```

### WardRecommendation

```rust
pub struct WardRecommendation {
    pub action: String,                     // "use_existing" | "create_new"
    pub ward_name: String,                  // domain-level: "financial-analysis"
    pub subdirectory: Option<String>,       // task-specific: "stocks/spy"
    #[serde(default)]
    pub structure: HashMap<String, String>, // Kept for backward compat, no longer requested
    pub reason: String,
}
```

### ExecutionStrategy

```rust
pub struct ExecutionStrategy {
    pub approach: String,              // "simple" | "graph"
    pub graph: Option<ExecutionGraph>, // only when approach="graph"
    pub explanation: String,
}
```

### ExecutionGraph

```rust
pub struct ExecutionGraph {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
    #[serde(default)]
    pub mermaid: Option<String>,    // Kept for backward compat, no longer requested
    #[serde(default)]
    pub max_cycles: Option<u32>,
}
```

### GraphNode / GraphEdge / EdgeCondition

```rust
pub struct GraphNode {
    pub id: String,
    pub task: String,
    pub agent: String,
    pub skills: Vec<String>,
}

#[serde(untagged)]
pub enum GraphEdge {
    Conditional { from: String, conditions: Vec<EdgeCondition> },
    Direct { from: String, to: String },
}

pub struct EdgeCondition {
    pub when: String,
    pub to: String,
}
```

### OnSessionReady Callback Type

```rust
// In runner.rs, exported from gateway-execution crate
pub type OnSessionReady = Box<dyn FnOnce(String) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send>;
```

### Search Config Constants

```rust
const MIN_RELEVANCE_SCORE: f64 = 0.15;
const MAX_SKILLS: usize = 8;
const MAX_AGENTS: usize = 5;
const MAX_WARDS: usize = 5;
```

---

## Function Signatures

### analyze_intent (3 params)

```rust
pub async fn analyze_intent(
    llm_client: &dyn LlmClient,
    user_message: &str,
    fact_store: &dyn MemoryFactStore,
) -> Result<IntentAnalysis, String>
```

### format_intent_injection

```rust
pub fn format_intent_injection(analysis: &IntentAnalysis) -> String
```

Returns markdown section appended to agent instructions:
```markdown
## Intent Analysis

**Primary Intent:** ...
**Hidden Intents:**
- ...
**Ward:** ward-name (action) — reason
  Subdirectory: ...
**Recommended Skills:** ...
**Recommended Agents:** ...
**Execution Approach:** ...
```

### invoke_with_callback

```rust
pub async fn invoke_with_callback(
    &self,
    config: ExecutionConfig,
    message: String,
    on_session_ready: Option<OnSessionReady>,
) -> Result<(ExecutionHandle, String), String>
```

---

## Frontend Types (TypeScript)

### IntentAnalysis (mission-hooks.ts)

```typescript
interface IntentAnalysis {
  primaryIntent: string;
  hiddenIntents: string[];
  recommendedSkills: string[];
  recommendedAgents: string[];
  wardRecommendation: {
    action: string;
    wardName: string;
    subdirectory?: string;
    reason: string;
  };
  executionStrategy: {
    approach: string;
    graph?: {
      nodes: Array<{ id: string; task: string; agent: string; skills: string[] }>;
    };
    explanation: string;
  };
}
```

---

## Backward Compatibility

Fields removed from the LLM prompt but kept on structs with `#[serde(default)]`:

| Field | Struct | Why kept |
|-------|--------|----------|
| `rewritten_prompt` | `IntentAnalysis` | Old logs contain it; deserializes to `""` when absent |
| `structure` | `WardRecommendation` | Old logs contain it; deserializes to empty `HashMap` |
| `mermaid` | `ExecutionGraph` | Old logs contain it; deserializes to `None` |
| `max_cycles` | `ExecutionGraph` | Old logs may contain it |
