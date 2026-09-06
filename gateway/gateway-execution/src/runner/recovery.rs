//! Gateway-owned durable recovery inputs for continuation and resume.
//!
//! The runtime's private snapshot (`app:rig_checkpoint`) is the provider-grade
//! tape captured at the last turn boundary. This module owns the metadata kept
//! **beside** (never inside) that snapshot and the composition rules for
//! rebuilding an invocation's history:
//!
//! - `input_cursor` — the max durable `seq` of rows already represented in the
//!   snapshot's tape (advanced only across rows the runtime actually scanned).
//! - `represented_output_ids` — durable row IDs authored by the checkpoint's
//!   own execution (assistant/tool rows, synthetic prompts). Their content is
//!   already inside the tape, so replay must omit them.
//!
//! Together the invariant is: a durable row's content is in the tape **iff**
//! `seq <= input_cursor` **or** `id ∈ represented_output_ids`. Rows that fail
//! both tests (child callbacks, mid-run peer rows) are appended as tail rows
//! on restore, which is exactly how a racing callback that landed before the
//! parent's final own row survives without duplicating parent output.

use std::collections::HashMap;
use std::sync::Arc;
use zbot_conversation::{CheckpointStore, Message, MessageStore};

/// Key inside the persisted checkpoint `context_state` JSON holding the
/// gateway-owned cursor metadata.
pub(crate) const GATEWAY_RECOVERY_KEY: &str = "gateway_recovery";

/// Cursor metadata written beside the runtime snapshot.
#[derive(Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct RecoveryCursor {
    /// Max durable `seq` of rows represented in the snapshot's tape.
    pub input_cursor: i64,
    /// Durable row IDs already represented in the tape (this execution's own
    /// outputs, confirmed durable at checkpoint time).
    pub represented_output_ids: Vec<String>,
}

/// History composition for a resumed invocation.
#[derive(Debug)]
pub(crate) struct ComposedHistory {
    /// Engine history: restored tape first, durable tail rows after.
    pub history: Vec<agent_runtime::ChatMessage>,
    /// Max durable `seq` scanned while composing (input rows only).
    pub scanned_cursor: i64,
    /// Mutable runtime keys restored from the snapshot (fresh authority
    /// already preferred by the restore seam).
    pub initial_state: Vec<(String, serde_json::Value)>,
}

/// Read the newest checkpoint and compose the invocation's history.
///
/// Fails explicitly — never silently degrades to display replay — when the
/// checkpoint store errors or a present private snapshot is malformed. A
/// checkpoint without a private snapshot (legacy executions, omitted capture)
/// composes from display rows like before, keeping the cursor metadata when
/// present.
pub(crate) fn compose_continuation_history(
    checkpoints: &Arc<dyn CheckpointStore>,
    messages: &Arc<dyn MessageStore>,
    execution_id: &str,
    session_id: &str,
) -> Result<ComposedHistory, String> {
    let checkpoint = checkpoints
        .latest(execution_id)
        .map_err(|error| format!("continuation_checkpoint_read_failed: {error}"))?;
    let Some(checkpoint) = checkpoint else {
        return replay_display_history(messages, session_id);
    };
    let context = checkpoint
        .context_state
        .as_deref()
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok());
    let Some(context) = context else {
        return Err("continuation_checkpoint_invalid".to_owned());
    };
    let cursor: RecoveryCursor = context
        .get(GATEWAY_RECOVERY_KEY)
        .cloned()
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default();

    let mut state: HashMap<String, serde_json::Value> =
        serde_json::from_value(context).unwrap_or_default();
    match agent_runtime::engine::snapshot::restore(&mut state)? {
        Some(tape) => {
            let rows = messages
                .replay(session_id, Some(cursor.input_cursor), 200)
                .map_err(|_| "continuation_history_read_failed".to_owned())?;
            let (tails, tail_cursor) = tail_rows(rows, &cursor.represented_output_ids);
            let mut history = tape;
            history.extend(crate::conversation_history::messages_to_chat_format(&tails));
            Ok(ComposedHistory {
                history,
                scanned_cursor: cursor.input_cursor.max(tail_cursor),
                initial_state: state.into_iter().collect(),
            })
        }
        // Snapshot absent (legacy execution or omitted capture): there is no
        // tape to deduplicate against, so represented outputs stay in the
        // display replay and the cursor advances across every row.
        None => replay_display_history(messages, session_id),
    }
}

/// Display-row replay (no private snapshot): every row, newest cursor = the
/// highest scanned `seq`.
fn replay_display_history(
    messages: &Arc<dyn MessageStore>,
    session_id: &str,
) -> Result<ComposedHistory, String> {
    let rows = messages
        .replay(session_id, None, 200)
        .map_err(|_| "continuation_history_read_failed".to_owned())?;
    let scanned_cursor = rows.last().map(|row| row.seq).unwrap_or(0);
    Ok(ComposedHistory {
        history: crate::conversation_history::messages_to_chat_format(&rows),
        scanned_cursor,
        initial_state: Vec::new(),
    })
}

/// Rows after the cursor minus represented outputs, with the max scanned `seq`.
fn tail_rows(rows: Vec<Message>, represented: &[String]) -> (Vec<Message>, i64) {
    let scanned = rows.last().map(|row| row.seq).unwrap_or(0);
    let kept = rows
        .into_iter()
        .filter(|row| !represented.contains(&row.id))
        .collect();
    (kept, scanned)
}

/// Build the persisted `context_state` JSON: display fields for compatibility
/// plus the private snapshot and gateway cursor metadata.
pub(crate) fn checkpoint_context_state(
    display: serde_json::Value,
    engine_state: Option<&serde_json::Value>,
    cursor: &RecoveryCursor,
) -> String {
    let mut context = display;
    if let Some(snapshot) = engine_state
        .and_then(|state| state.get(agent_runtime::engine::snapshot::CHECKPOINT_KEY))
        .cloned()
    {
        context
            .as_object_mut()
            .expect("display context is an object")
            .insert(
                agent_runtime::engine::snapshot::CHECKPOINT_KEY.to_owned(),
                snapshot,
            );
    }
    if let Ok(map) = serde_json::to_value(cursor) {
        context
            .as_object_mut()
            .expect("display context is an object")
            .insert(GATEWAY_RECOVERY_KEY.to_owned(), map);
    }
    context.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str, seq: i64, role: &str) -> Message {
        Message {
            id: id.to_owned(),
            execution_id: None,
            session_id: "s".to_owned(),
            role: role.to_owned(),
            content: format!("content-{id}"),
            created_at: String::new(),
            token_count: 0,
            tool_calls: None,
            tool_call_id: None,
            seq,
        }
    }

    fn cursor(input_cursor: i64, represented: &[&str]) -> RecoveryCursor {
        RecoveryCursor {
            input_cursor,
            represented_output_ids: represented.iter().map(|id| id.to_string()).collect(),
        }
    }

    #[test]
    fn tail_rows_omit_represented_outputs_and_report_scanned_cursor() {
        let rows = vec![
            row("callback", 51, "system"),
            row("own-final", 52, "assistant"),
            row("peer", 53, "system"),
        ];
        let (kept, scanned) = tail_rows(rows, &["own-final".to_owned()]);
        let ids: Vec<_> = kept.iter().map(|row| row.id.as_str()).collect();
        assert_eq!(ids, ["callback", "peer"]);
        assert_eq!(scanned, 53, "cursor advances across all scanned rows");
    }

    #[test]
    fn callback_before_parent_final_row_survives_both_append_orders() {
        // Order A: callback (51) precedes the parent's final own row (52).
        let order_a = vec![row("cb", 51, "system"), row("own", 52, "assistant")];
        // Order B: the parent's final own row precedes the callback.
        let order_b = vec![row("own", 52, "assistant"), row("cb", 53, "system")];
        for (rows, expected) in [(order_a, vec!["cb"]), (order_b, vec!["cb"])] {
            let (kept, _) = tail_rows(rows, &["own".to_owned()]);
            let ids: Vec<_> = kept.iter().map(|row| row.id.as_str()).collect();
            assert_eq!(ids, expected, "callback replayed, own output omitted");
        }
    }

    #[test]
    fn cursor_metadata_round_trips_through_context_state() {
        let display = serde_json::json!({"intent": null, "ward": null});
        let mut engine_state = serde_json::Map::new();
        engine_state.insert(
            agent_runtime::engine::snapshot::CHECKPOINT_KEY.to_owned(),
            serde_json::json!({"version": 1, "messages": [], "mutable_state": {}}),
        );
        let raw = checkpoint_context_state(
            display,
            Some(&serde_json::Value::Object(engine_state)),
            &cursor(42, &["msg-a"]),
        );
        let parsed: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert!(parsed
            .get(agent_runtime::engine::snapshot::CHECKPOINT_KEY)
            .is_some());
        let round: RecoveryCursor =
            serde_json::from_value(parsed.get(GATEWAY_RECOVERY_KEY).cloned().unwrap()).unwrap();
        assert_eq!(round, cursor(42, &["msg-a"]));
    }

    #[test]
    fn checkpoint_context_state_without_engine_snapshot_keeps_cursor() {
        let raw =
            checkpoint_context_state(serde_json::json!({"intent": null}), None, &cursor(7, &[]));
        let parsed: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert!(parsed
            .get(agent_runtime::engine::snapshot::CHECKPOINT_KEY)
            .is_none());
        assert!(parsed.get(GATEWAY_RECOVERY_KEY).is_some());
    }
}
