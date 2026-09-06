//! Private checkpoint data, distinct from display history and provider wire JSON.
use crate::{ChatMessage, ToolCall};
use agent_primitives::Part;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
pub const CHECKPOINT_KEY: &str = "app:rig_checkpoint";
const MUTABLE_KEYS: [&str; 4] = [
    "skill:graph",
    "skill:loaded_skills",
    // Relative resource reads use this selection; the configured skill roots
    // and canonical path checks still come from the freshly built tool.
    "skill:current_skill",
    "app:plan",
];
#[derive(Serialize, Deserialize)]
struct StoredToolCall {
    id: String,
    name: String,
    arguments: Value,
}
#[derive(Serialize, Deserialize)]
struct StoredMessage {
    role: String,
    content: Vec<Part>,
    tool_calls: Option<Vec<StoredToolCall>>,
    tool_call_id: Option<String>,
    is_summary: bool,
}
impl From<&ChatMessage> for StoredMessage {
    fn from(m: &ChatMessage) -> Self {
        Self {
            role: m.role.clone(),
            content: m.content.clone(),
            tool_calls: m.tool_calls.as_ref().map(|calls| {
                calls
                    .iter()
                    .map(|call| StoredToolCall {
                        id: call.id.clone(),
                        name: call.name.clone(),
                        arguments: call.arguments.clone(),
                    })
                    .collect()
            }),
            tool_call_id: m.tool_call_id.clone(),
            is_summary: m.is_summary,
        }
    }
}
impl From<StoredMessage> for ChatMessage {
    fn from(m: StoredMessage) -> Self {
        Self {
            role: m.role,
            content: m.content,
            tool_calls: m.tool_calls.map(|calls| {
                calls
                    .into_iter()
                    .map(|call| ToolCall::new(call.id, call.name, call.arguments))
                    .collect()
            }),
            tool_call_id: m.tool_call_id,
            is_summary: m.is_summary,
        }
    }
}
#[derive(Serialize, Deserialize)]
struct Snapshot {
    version: u32,
    owned_preamble: Option<usize>,
    messages: Vec<StoredMessage>,
    mutable_state: HashMap<String, Value>,
}
/// Consume the private handoff; never merge stale authority from a checkpoint.
pub(crate) fn restore(
    state: &mut HashMap<String, Value>,
) -> Result<Option<Vec<ChatMessage>>, String> {
    let Some(value) = state.remove(CHECKPOINT_KEY) else {
        return Ok(None);
    };
    let mut snapshot: Snapshot =
        serde_json::from_value(value).map_err(|_| "Invalid execution checkpoint".to_owned())?;
    if snapshot.version != 1 {
        return Err("Unsupported execution checkpoint version".into());
    }
    if let Some(index) = snapshot.owned_preamble {
        if !snapshot
            .messages
            .get(index)
            .is_some_and(|m| m.role == "system" && !m.is_summary)
        {
            return Err("Invalid owned checkpoint preamble".into());
        }
        snapshot.messages.remove(index);
    }
    for key in MUTABLE_KEYS {
        if let Some(value) = snapshot.mutable_state.remove(key) {
            state.entry(key.into()).or_insert(value);
        }
    }
    Ok(Some(
        snapshot.messages.into_iter().map(Into::into).collect(),
    ))
}
/// Omit unsafe snapshots when middleware changes/duplicates the owned preamble.
pub(crate) fn capture(
    messages: &[ChatMessage],
    owned: Option<&str>,
    state: &Value,
) -> Option<Value> {
    let owned_preamble = if let Some(owned) = owned {
        let matches: Vec<_> = messages
            .iter()
            .enumerate()
            .filter(|(_, m)| {
                m.role == "system"
                    && !m.is_summary
                    && matches!(m.content.as_slice(),[Part::Text{text}] if text==owned)
            })
            .map(|(index, _)| index)
            .collect();
        if matches.len() != 1 {
            tracing::warn!(
                "Execution checkpoint omitted: owned preamble is not uniquely identifiable"
            );
            return None;
        }
        Some(matches[0])
    } else {
        None
    };
    let mutable_state = MUTABLE_KEYS
        .into_iter()
        .filter_map(|key| state.get(key).cloned().map(|value| (key.to_owned(), value)))
        .collect();
    serde_json::to_value(Snapshot {
        version: 1,
        owned_preamble,
        messages: messages.iter().map(Into::into).collect(),
        mutable_state,
    })
    .ok()
}
