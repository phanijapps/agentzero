//! Canonical host context at Rig's request boundary, not another execution loop.
use super::{model::convert_rig_messages, SharedToolContext};
use crate::{
    middleware::{
        token_counter::{estimate_tokens, estimate_total_tokens},
        traits::{ExecutionState, SkillInfo},
        MiddlewareContext,
    },
    ChatMessage, ExecutorError, MiddlewarePipeline, StreamEvent,
};
use agent_primitives::CallbackContext;
use rig::{
    agent::{AgentHook, Flow, StepEvent},
    completion::{CompletionError, CompletionModel, CompletionRequest, Message},
};
use serde_json::Value;
use std::{
    collections::HashSet,
    sync::{Arc, Mutex},
};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

pub(super) struct ContextPolicyConfig {
    pub provider_id: String,
    pub model: String,
    pub system_instruction: Option<String>,
    pub input_budget: u64,
}

/// Delivery acknowledgments travel with the provider future, never preparation.
pub(super) struct PreparedRequest {
    pub messages: Vec<ChatMessage>,
    pub tools: Option<Value>,
    pub acks: Vec<tokio::sync::oneshot::Sender<()>>,
}

#[derive(Clone)]
struct RunContext {
    messages: Vec<ChatMessage>,
    previous_rig: Option<Vec<Message>>,
    initial_rig_len: usize,
    turn: usize,
    recall_keys: HashSet<String>,
    results: super::tool_results::SharedToolResults,
}

pub(super) struct ContextPolicy {
    config: ContextPolicyConfig,
    middleware: Arc<MiddlewarePipeline>,
    context: SharedToolContext,
    run: Mutex<Option<RunContext>>,
    snapshot: Mutex<Option<(usize, Vec<Message>)>>,
    events: Mutex<Option<UnboundedSender<StreamEvent>>>,
    error: Mutex<Option<ExecutorError>>,
    inputs: super::context_inputs::ContextInputs,
}

impl ContextPolicy {
    pub fn new(
        config: ContextPolicyConfig,
        middleware: Arc<MiddlewarePipeline>,
        context: SharedToolContext,
        inputs: super::context_inputs::ContextInputs,
    ) -> Self {
        Self {
            config,
            middleware,
            context,
            run: Mutex::new(None),
            snapshot: Mutex::new(None),
            events: Mutex::new(None),
            error: Mutex::new(None),
            inputs,
        }
    }

    pub fn begin(
        &self,
        history: &[ChatMessage],
        user: &str,
        rig_history_len: usize,
        results: super::tool_results::SharedToolResults,
    ) -> UnboundedReceiver<StreamEvent> {
        let mut messages = Vec::new();
        if let Some(instructions) = &self.config.system_instruction {
            messages.push(ChatMessage::system(instructions.clone()));
        }
        messages.extend_from_slice(history);
        messages.push(ChatMessage::user(user.to_owned()));
        *self.run.lock().unwrap() = Some(RunContext {
            messages,
            previous_rig: None,
            initial_rig_len: rig_history_len + 1,
            turn: 0,
            recall_keys: self
                .inputs
                .recall
                .as_ref()
                .map(|(_, _, keys)| keys.clone())
                .unwrap_or_default(),
            results,
        });
        *self.snapshot.lock().unwrap() = None;
        *self.error.lock().unwrap() = None;
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        *self.events.lock().unwrap() = Some(tx);
        rx
    }

    pub fn take_error(&self) -> Option<ExecutorError> {
        self.error.lock().unwrap().take()
    }

    pub async fn prepare(
        &self,
        request: &CompletionRequest,
        tools: &Option<Value>,
    ) -> Result<PreparedRequest, CompletionError> {
        match self.prepare_inner(request, tools).await {
            Ok(messages) => Ok(messages),
            Err(error) => {
                *self.error.lock().unwrap() = Some(error);
                Err(CompletionError::ProviderError(
                    "Execution context policy rejected request".into(),
                ))
            }
        }
    }

    async fn prepare_inner(
        &self,
        request: &CompletionRequest,
        tools: &Option<Value>,
    ) -> Result<PreparedRequest, ExecutorError> {
        let mismatch = || {
            ExecutorError::MiddlewareError(
                "Rig context cursor disagrees with request history".into(),
            )
        };
        let (turn, snapshot) = self.snapshot.lock().unwrap().take().ok_or_else(mismatch)?;
        // Keep the last committed context available if preprocessing is dropped
        // or rejected; only a successfully prepared request replaces it below.
        let mut state = self.run.lock().unwrap().clone().ok_or_else(mismatch)?;
        if turn != state.turn + 1 {
            return Err(mismatch());
        }
        // The provider builder can replace/insert its system preamble; all
        // non-system messages must still match the pre-build hook snapshot.
        let non_system = |message: &&Message| !matches!(message, Message::System { .. });
        if snapshot
            .iter()
            .filter(non_system)
            .ne(request.chat_history.iter().filter(non_system))
        {
            return Err(mismatch());
        }
        if let Some(previous) = &state.previous_rig {
            if !snapshot.starts_with(previous) {
                return Err(mismatch());
            }
            state.messages.extend(
                convert_rig_messages(snapshot[previous.len()..].iter()).map_err(|_| mismatch())?,
            );
        } else if snapshot.len() != state.initial_rig_len {
            return Err(mismatch());
        }
        state.previous_rig = Some(snapshot);
        state.turn = turn;

        let mut execution_state = ExecutionState::from_messages(&state.messages);
        if let Some(graph) = self
            .context
            .get_skill_state()
            .and_then(|graph| graph.as_object().cloned())
        {
            for (name, entry) in graph {
                let tool_call_id = entry
                    .get("tool_call_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                let resource_tool_call_ids = entry
                    .get("resources")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|resource| {
                        resource
                            .get("tool_call_id")
                            .and_then(Value::as_str)
                            .map(str::to_owned)
                    })
                    .collect();
                execution_state.loaded_skills.insert(
                    name.clone(),
                    SkillInfo {
                        name,
                        tool_call_id,
                        resource_tool_call_ids,
                    },
                );
            }
        }
        let context = MiddlewareContext::new(
            self.context.agent_id.clone().unwrap_or_default(),
            self.context.conversation_id.clone(),
            self.config.provider_id.clone(),
            self.config.model.clone(),
        )
        .with_counts(
            state.messages.len(),
            estimate_total_tokens(&state.messages, &self.config.model),
        )
        .with_execution_state(execution_state)
        .with_plan_state(
            self.context
                .get_state("app:plan")
                .or_else(|| crate::middleware::extract_plan_state(&state.messages)),
        );
        let sender = self.events.lock().unwrap().clone();
        state.messages = self
            .middleware
            .process_messages(state.messages, &context, |event| {
                if let Some(sender) = &sender {
                    let _ = sender.send(event);
                }
            })
            .await
            .map_err(ExecutorError::MiddlewareError)?;
        let acks = self
            .inputs
            .apply(
                &mut state.messages,
                &mut state.recall_keys,
                turn,
                &state.results,
            )
            .await;
        let tools = if state.results.peer_influenced() {
            crate::tool_visibility::peer_safe_tools_schema(tools)
        } else {
            tools.clone()
        };
        let tokens = estimate_total_tokens(&state.messages, &self.config.model).saturating_add(
            tools.as_ref().map_or(0, |tools| {
                estimate_tokens(&tools.to_string(), &self.config.model)
            }),
        );
        // Gateway has already resolved max-input separately from max-output.
        // Do not reserve output tokens a second time against this input limit.
        if self.config.input_budget > 0 && tokens as u64 > self.config.input_budget {
            return Err(ExecutorError::MiddlewareError(format!(
                "Request exceeds input token budget: estimated {tokens}, limit {}",
                self.config.input_budget
            )));
        }
        let messages = state.messages.clone();
        *self.run.lock().unwrap() = Some(state);
        Ok(PreparedRequest {
            messages,
            tools,
            acks,
        })
    }
}

pub(super) struct ContextCapture(pub Arc<ContextPolicy>);
impl<M: CompletionModel> AgentHook<M> for ContextCapture {
    async fn on_event(&self, event: StepEvent<'_, M>) -> Flow {
        if let StepEvent::CompletionCall {
            turn,
            history,
            prompt,
        } = event
        {
            let mut messages = history.to_vec();
            messages.push(prompt.clone());
            *self.0.snapshot.lock().unwrap() = Some((turn, messages));
        }
        Flow::cont()
    }
}
