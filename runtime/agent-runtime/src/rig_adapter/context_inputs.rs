//! Live input injection; canonical cursor and budgets remain in ContextPolicy.
use super::tool_results::ToolResults;
use crate::{ChatMessage, RecallHook, TransformContextHook};
use std::{collections::HashSet, sync::Mutex};

/// Resolve persisted attachments before the final request-budget check. Pass
/// the resolved bytes onward so provider rehydration cannot enlarge a checked
/// request. File I/O stays off the engine's cancellation/heartbeat task.
pub(super) async fn resolve_attachments(
    mut messages: Vec<ChatMessage>,
) -> Result<Vec<ChatMessage>, crate::ExecutorError> {
    use agent_primitives::{types::ContentSource, Part};
    let has_refs = messages.iter().flat_map(|m| &m.content).any(|part| {
        matches!(
            part,
            Part::Image {
                source: ContentSource::FileRef(_),
                ..
            } | Part::File {
                source: ContentSource::FileRef(_),
                ..
            }
        )
    });
    if !has_refs {
        return Ok(messages);
    }
    let invalid =
        || crate::ExecutorError::MiddlewareError("Unable to resolve request attachment".into());
    tokio::task::spawn_blocking(move || {
        for part in messages.iter_mut().flat_map(|m| &mut m.content) {
            if let Part::Image { source, .. } | Part::File { source, .. } = part {
                if matches!(source, ContentSource::FileRef(_)) {
                    *source = agent_primitives::multimodal::rehydrate_source(source)
                        .map_err(|_| invalid())?;
                }
            }
        }
        Ok(messages)
    })
    .await
    .map_err(|_| invalid())?
}

pub(super) struct ContextInputs {
    pub recall: Option<(RecallHook, u32, HashSet<String>)>,
    steering: Mutex<Option<crate::steering::SteeringQueue>>,
    transform: Option<TransformContextHook>,
}

impl ContextInputs {
    pub fn new(
        recall: Option<(RecallHook, u32, HashSet<String>)>,
        steering: Option<crate::steering::SteeringQueue>,
        transform: Option<TransformContextHook>,
    ) -> Self {
        Self {
            recall,
            steering: Mutex::new(steering),
            transform,
        }
    }

    pub async fn apply(
        &self,
        messages: &mut Vec<ChatMessage>,
        keys: &mut HashSet<String>,
        turn: usize,
        results: &ToolResults,
    ) -> Vec<tokio::sync::oneshot::Sender<()>> {
        if let Some((hook, every, _)) = &self.recall {
            if *every > 0 && turn.is_multiple_of(*every as usize) {
                let query = messages
                    .iter()
                    .rev()
                    .find(|m| m.role == "user")
                    .map(ChatMessage::text_content)
                    .unwrap_or_default();
                match hook(&query, keys).await {
                    Ok(result) if !result.system_message.is_empty() => {
                        messages.push(ChatMessage::system(result.system_message));
                        keys.extend(result.fact_keys);
                    }
                    Ok(_) => {}
                    // Best effort; untrusted retrieval errors are not diagnostics.
                    Err(_) => tracing::warn!(turn, "Mid-session recall failed"),
                }
            }
        }
        let mut acks = Vec::new();
        if let Some(queue) = self.steering.lock().unwrap().as_mut() {
            for mut message in queue.drain() {
                messages.push(ChatMessage::user(format!(
                    "[STEER: {}] {}",
                    message.source, message.content
                )));
                if message.source == crate::steering::SteeringSource::Peer {
                    results.mark_peer_influenced();
                    if let Some(ack) = message.take_delivery_ack() {
                        acks.push(ack);
                    }
                }
            }
        }
        crate::context_management::sanitize_messages(messages);
        if let Some(transform) = &self.transform {
            transform(messages);
        }
        if crate::tool_visibility::contains_persisted_peer_result(messages) {
            results.mark_peer_influenced();
        }
        acks
    }
}
