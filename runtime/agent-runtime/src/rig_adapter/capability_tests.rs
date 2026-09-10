//! Adapter-local contract probes for the locked Rig dependency, not another executor.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::StreamExt;
use rig::agent::{AgentBuilder, AgentHook, Flow, RequestOverride, StepEvent};
use rig::completion::{
    CompletionError, CompletionModel, CompletionRequest, CompletionResponse, GetTokenUsage, Usage,
};
use rig::streaming::{RawStreamingChoice, StreamingChat, StreamingCompletionResponse};
use tokio::sync::Notify;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct MeteredResponse;

impl GetTokenUsage for MeteredResponse {
    fn token_usage(&self) -> Usage {
        Usage {
            input_tokens: 11,
            output_tokens: 3,
            total_tokens: 14,
            ..Usage::new()
        }
    }
}

#[derive(Clone, Default)]
struct ProbeModel {
    requests: Arc<Mutex<Vec<CompletionRequest>>>,
    pending: bool,
    entered: Arc<Notify>,
    dropped: Arc<Notify>,
}

struct DropNotice(Arc<Notify>);

/// A request-policy wrapper, not another agent loop. Rig still decides when
/// to request a model response and when to execute tools.
#[derive(Clone)]
struct HistoryPolicyModel(ProbeModel);

impl CompletionModel for HistoryPolicyModel {
    type Response = MeteredResponse;
    type StreamingResponse = MeteredResponse;
    type Client = ();

    fn make(_: &(), _: impl Into<String>) -> Self {
        Self(ProbeModel::default())
    }

    async fn completion(
        &self,
        _: CompletionRequest,
    ) -> Result<CompletionResponse<Self::Response>, CompletionError> {
        panic!("streaming probe");
    }

    async fn stream(
        &self,
        mut request: CompletionRequest,
    ) -> Result<StreamingCompletionResponse<Self::StreamingResponse>, CompletionError> {
        let retained = request
            .chat_history
            .iter()
            .filter(|message| {
                // A deliberately small deterministic policy: remove the complete
                // obsolete user/assistant pair while preserving system and prompt.
                let value = serde_json::to_string(message).unwrap();
                !value.contains("obsolete-pair")
            })
            .cloned()
            .collect::<Vec<_>>();
        request.chat_history = rig::one_or_many::OneOrMany::many(retained).unwrap();
        self.0.stream(request).await
    }
}

impl Drop for DropNotice {
    fn drop(&mut self) {
        self.0.notify_one();
    }
}

impl CompletionModel for ProbeModel {
    type Response = MeteredResponse;
    type StreamingResponse = MeteredResponse;
    type Client = ();

    fn make(_: &(), _: impl Into<String>) -> Self {
        Self::default()
    }

    async fn completion(
        &self,
        _: CompletionRequest,
    ) -> Result<CompletionResponse<Self::Response>, CompletionError> {
        panic!("streaming probe must not call non-streaming completion")
    }

    async fn stream(
        &self,
        request: CompletionRequest,
    ) -> Result<StreamingCompletionResponse<Self::StreamingResponse>, CompletionError> {
        self.requests.lock().unwrap().push(request);
        if self.pending {
            let entered = self.entered.clone();
            let notice = DropNotice(self.dropped.clone());
            let stream = futures::stream::once(async move {
                let _notice = notice;
                entered.notify_one();
                futures::future::pending::<
                    Result<RawStreamingChoice<MeteredResponse>, CompletionError>,
                >()
                .await
            });
            return Ok(StreamingCompletionResponse::stream(Box::pin(stream)));
        }
        Ok(StreamingCompletionResponse::stream(Box::pin(
            futures::stream::iter([
                Ok(RawStreamingChoice::Message("answer".into())),
                Ok(RawStreamingChoice::FinalResponse(MeteredResponse)),
            ]),
        )))
    }
}

#[derive(Clone, Default)]
struct RequestPolicy {
    turns: Arc<Mutex<Vec<usize>>>,
    usage: Arc<Mutex<Vec<Usage>>>,
}

impl AgentHook<ProbeModel> for RequestPolicy {
    async fn on_event(&self, event: StepEvent<'_, ProbeModel>) -> Flow {
        match event {
            StepEvent::CompletionCall { turn, .. } => {
                self.turns.lock().unwrap().push(turn);
                Flow::override_request(
                    RequestOverride::new()
                        .preamble("bounded system context")
                        .max_tokens(23),
                )
            }
            StepEvent::StreamResponseFinish { response, .. } => {
                self.usage.lock().unwrap().push(response.token_usage());
                Flow::cont()
            }
            _ => Flow::cont(),
        }
    }
}

#[tokio::test]
async fn rig_applies_request_policy_before_provider_and_exposes_usage() {
    let model = ProbeModel::default();
    let policy = RequestPolicy::default();
    let agent = AgentBuilder::new(model.clone())
        .preamble("original context")
        .max_tokens(99)
        .add_hook(policy.clone())
        .build();
    let mut stream = agent
        .stream_chat("hello", Vec::<rig::completion::Message>::new())
        .await;
    while let Some(item) = stream.next().await {
        item.expect("Rig streaming turn");
    }
    let requests = model.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    // Rig resolves the override into a system history message; the provider
    // request's separate preamble field is intentionally empty.
    assert!(matches!(
        requests[0].chat_history.iter().next(),
        Some(rig::completion::Message::System { content }) if content == "bounded system context"
    ));
    assert_eq!(requests[0].max_tokens, Some(23));
    assert_eq!(*policy.turns.lock().unwrap(), [1]);
    assert_eq!(
        *policy.usage.lock().unwrap(),
        [MeteredResponse.token_usage()]
    );
}

#[tokio::test]
async fn dropping_rig_run_releases_a_provider_stream_that_never_yields() {
    let model = ProbeModel {
        pending: true,
        ..ProbeModel::default()
    };
    let agent = AgentBuilder::new(model.clone()).build();
    let mut stream = agent
        .stream_chat("hello", Vec::<rig::completion::Message>::new())
        .await;
    // The provider must actually be polled before cancellation: an unpolled
    // future would make a resource-release assertion vacuous.
    tokio::time::timeout(Duration::from_secs(1), async {
        tokio::select! {
            biased;
            item = stream.next() => panic!("pending provider yielded {item:?}"),
            () = model.entered.notified() => {}
        }
    })
    .await
    .expect("Rig polls the pending provider");
    drop(stream);
    tokio::time::timeout(Duration::from_secs(1), model.dropped.notified())
        .await
        .expect("dropping the run drops its pending provider stream");
    assert_eq!(model.requests.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn model_boundary_can_apply_history_policy_before_provider_dispatch() {
    use rig::completion::Message;
    let provider = ProbeModel::default();
    let agent = AgentBuilder::new(HistoryPolicyModel(provider.clone()))
        .preamble("preserved system")
        .build();
    let history = vec![
        Message::user("obsolete-pair question"),
        Message::assistant("obsolete-pair answer"),
    ];
    let mut stream = agent.stream_chat("current question", history).await;
    while let Some(item) = stream.next().await {
        item.expect("Rig turn through request policy");
    }
    let requests = provider.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    let history = serde_json::to_string(&requests[0].chat_history).unwrap();
    assert!(!history.contains("obsolete-pair"));
    assert!(history.contains("preserved system"));
    assert!(history.contains("current question"));
}
