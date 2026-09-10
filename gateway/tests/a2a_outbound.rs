mod common;

use async_trait::async_trait;
use chrono::Utc;
use execution_state::WorkStatus;
use gateway::tasks::a2a::{
    A2aOutboundDispatchHandler, A2aOutboundPollHandler, GatewayA2aDelegationService,
};
use gateway_a2a::client::{A2aClientError, A2aTransport};
use gateway_a2a::peers::{AddPeer, PeerStore, TrustedPeer};
use gateway_a2a::{project_task, TaskProjection, TaskProjectionState};
use gateway_bus::{
    DurableWorkWorker, WorkHandler, WorkHandlerRegistry, WorkWorkerConfig, WorkWorkerLimits,
};
use gateway_execution::a2a::{A2aDelegationContext, A2aDelegationService, LocalA2aActorKind};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::time::{sleep, timeout};

#[derive(Default)]
struct FakeTransport {
    sends: AtomicUsize,
    polls: AtomicUsize,
}

#[async_trait]
impl A2aTransport for FakeTransport {
    async fn send_message(
        &self,
        _peer: &TrustedPeer,
        request: &a2a::SendMessageRequest,
    ) -> Result<a2a::Task, A2aClientError> {
        self.sends.fetch_add(1, Ordering::SeqCst);
        assert_eq!(request.message.message_id.len(), 68);
        assert_eq!(request.message.text(), Some("compare local energy options"));
        project_task(TaskProjection {
            id: "task-remote".to_owned(),
            context_id: "ctx-remote".to_owned(),
            state: TaskProjectionState::Submitted,
            message: None,
            artifact_text: None,
            updated_at: Some(Utc::now()),
        })
        .map_err(|_| A2aClientError::Protocol)
    }

    async fn get_task(
        &self,
        _peer: &TrustedPeer,
        task_id: &str,
    ) -> Result<a2a::Task, A2aClientError> {
        self.polls.fetch_add(1, Ordering::SeqCst);
        assert_eq!(task_id, "task-remote");
        project_task(TaskProjection {
            id: task_id.to_owned(),
            context_id: "ctx-remote".to_owned(),
            state: TaskProjectionState::Completed,
            message: None,
            artifact_text: Some("Remote solar analysis".to_owned()),
            updated_at: Some(Utc::now()),
        })
        .map_err(|_| A2aClientError::Protocol)
    }
}

#[tokio::test]
async fn durable_outbound_dispatch_returns_immediately_and_delivers_attributed_result() {
    let (dir, state) = common::make_state();
    PeerStore::new(dir.path())
        .add_peer(AddPeer {
            peer_id: "peer-b".to_owned(),
            display_name: "Peer B".to_owned(),
            origin: "http://127.0.0.1:18792".to_owned(),
            target_agent_id: "assistant".to_owned(),
            outbound_token: Some("outbound-test-token".to_owned()),
            allow_private_http: false,
        })
        .unwrap();

    let (session, execution) = state.state_service.create_session("root").unwrap();
    state.state_service.start_execution(&execution.id).unwrap();
    let context = A2aDelegationContext {
        actor_kind: LocalA2aActorKind::Root,
        agent_id: "root".to_owned(),
        session_id: session.id.clone(),
        execution_id: execution.id.clone(),
        conversation_id: "conversation-a".to_owned(),
        request_id: "tool-call-1".to_owned(),
    };
    let service = GatewayA2aDelegationService::new(
        dir.path(),
        state.durable_work_store.clone(),
        state.durable_work_transport.clone(),
        state.state_service.clone(),
    );

    let receipt = service
        .delegate(context.clone(), "peer-b", "compare local energy options")
        .await
        .unwrap();
    let duplicate = service
        .delegate(context, "peer-b", "compare local energy options")
        .await
        .unwrap();
    assert_eq!(receipt, duplicate);
    assert_eq!(
        state
            .durable_work_store
            .get(&receipt.task_id)
            .unwrap()
            .unwrap()
            .status(),
        WorkStatus::Pending
    );

    let remote = Arc::new(FakeTransport::default());
    let steering = Arc::new(agent_runtime::SteeringRegistry::new());
    let (mut queue, handle) = agent_runtime::SteeringQueue::new();
    steering.register_peer_only(&execution.id, handle);
    let handlers: Vec<Arc<dyn WorkHandler>> = vec![
        Arc::new(A2aOutboundDispatchHandler::new(
            dir.path(),
            state.durable_work_store.clone(),
            state.durable_work_transport.clone(),
            remote.clone(),
        )),
        Arc::new(A2aOutboundPollHandler::new(
            dir.path(),
            remote.clone(),
            steering,
            state.state_service.clone(),
            state.messages.clone(),
            state.event_bus.clone(),
        )),
    ];
    let registry = WorkHandlerRegistry::from_handlers(handlers).unwrap();
    let worker = DurableWorkWorker::new(
        state.durable_work_store.clone(),
        state.durable_work_transport.clone(),
        registry,
        WorkWorkerConfig::new(
            gateway::tasks::durable_agent::AGENT_TASK_TARGET,
            "a2a-outbound-test",
            WorkWorkerLimits::new(
                2,
                Duration::from_millis(100),
                Duration::from_secs(5),
                Duration::from_millis(100),
                Duration::from_secs(10),
                Duration::from_secs(2),
            )
            .unwrap(),
        )
        .unwrap(),
    )
    .start();

    let mut messages = timeout(Duration::from_secs(4), async {
        loop {
            let messages = queue.drain();
            if !messages.is_empty() {
                break messages;
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("remote result steering timeout");
    assert_eq!(messages.len(), 1);
    assert!(messages[0].content.contains("REMOTE ZBOT RESULT"));
    assert!(messages[0].content.contains("\"peer_id\":\"peer-b\""));
    assert!(messages[0].content.contains(&receipt.task_id));
    assert!(messages[0].content.contains("Remote solar analysis"));
    messages[0].acknowledge_delivery();

    timeout(Duration::from_secs(3), async {
        loop {
            if state
                .durable_work_store
                .get(&receipt.task_id)
                .unwrap()
                .is_some_and(|item| item.status() == WorkStatus::Completed)
            {
                break;
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("dispatch completion timeout");
    assert_eq!(remote.sends.load(Ordering::SeqCst), 1);
    assert_eq!(remote.polls.load(Ordering::SeqCst), 1);
    worker.shutdown().await.unwrap();
}

#[tokio::test]
async fn completed_origin_persists_result_and_schedules_safe_continuation() {
    let (dir, state) = common::make_state();
    PeerStore::new(dir.path())
        .add_peer(AddPeer {
            peer_id: "peer-b".to_owned(),
            display_name: "Peer B".to_owned(),
            origin: "http://127.0.0.1:18792".to_owned(),
            target_agent_id: "assistant".to_owned(),
            outbound_token: Some("outbound-test-token".to_owned()),
            allow_private_http: false,
        })
        .unwrap();

    let (session, execution) = state.state_service.create_session("root").unwrap();
    state.state_service.start_execution(&execution.id).unwrap();
    let service = GatewayA2aDelegationService::new(
        dir.path(),
        state.durable_work_store.clone(),
        state.durable_work_transport.clone(),
        state.state_service.clone(),
    );
    let receipt = service
        .delegate(
            A2aDelegationContext {
                actor_kind: LocalA2aActorKind::Root,
                agent_id: "root".to_owned(),
                session_id: session.id.clone(),
                execution_id: execution.id.clone(),
                conversation_id: "conversation-a".to_owned(),
                request_id: "tool-call-completed-origin".to_owned(),
            },
            "peer-b",
            "compare local energy options",
        )
        .await
        .unwrap();
    state
        .state_service
        .complete_execution(&execution.id)
        .unwrap();

    let remote = Arc::new(FakeTransport::default());
    let steering = Arc::new(agent_runtime::SteeringRegistry::new());
    let mut events = state.event_bus.subscribe_all();
    let handlers: Vec<Arc<dyn WorkHandler>> = vec![
        Arc::new(A2aOutboundDispatchHandler::new(
            dir.path(),
            state.durable_work_store.clone(),
            state.durable_work_transport.clone(),
            remote.clone(),
        )),
        Arc::new(A2aOutboundPollHandler::new(
            dir.path(),
            remote,
            steering,
            state.state_service.clone(),
            state.messages.clone(),
            state.event_bus.clone(),
        )),
    ];
    let registry = WorkHandlerRegistry::from_handlers(handlers).unwrap();
    let worker = DurableWorkWorker::new(
        state.durable_work_store.clone(),
        state.durable_work_transport.clone(),
        registry,
        WorkWorkerConfig::new(
            gateway::tasks::durable_agent::AGENT_TASK_TARGET,
            "a2a-outbound-continuation-test",
            WorkWorkerLimits::new(
                2,
                Duration::from_millis(100),
                Duration::from_secs(5),
                Duration::from_millis(100),
                Duration::from_secs(10),
                Duration::from_secs(2),
            )
            .unwrap(),
        )
        .unwrap(),
    )
    .start();

    let continuation = timeout(Duration::from_secs(4), async {
        loop {
            if let Ok(gateway::events::GatewayEvent::SessionContinuationReady {
                session_id,
                root_execution_id,
                ..
            }) = events.recv().await
            {
                break (session_id, root_execution_id);
            }
        }
    })
    .await
    .expect("continuation event timeout");
    assert_eq!(continuation, (session.id.clone(), execution.id.clone()));
    let persisted = state.messages.replay(&session.id, None, 50).unwrap();
    assert!(persisted.iter().any(|message| {
        message.role == "system"
            && message.content.contains("REMOTE ZBOT RESULT")
            && message.content.contains(&receipt.task_id)
    }));
    assert!(
        state
            .state_service
            .get_session(&session.id)
            .unwrap()
            .unwrap()
            .continuation_needed
    );

    worker.shutdown().await.unwrap();
}
