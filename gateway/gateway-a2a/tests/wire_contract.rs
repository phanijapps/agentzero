use gateway_a2a::{
    agent_card, cancel_task_request, error_response, list_tasks_response,
    parse_send_message_request, project_task, send_message_response, validate_headers,
    validate_send_message_request, AgentCardConfig, AgentSkillConfig, ProtocolError,
    TaskProjection, TaskProjectionState, APPLICATION_A2A_JSON,
};
use serde_json::json;

#[test]
fn agent_card_declares_bearer_auth() {
    let card = agent_card(AgentCardConfig {
        name: "Research zBot".into(),
        description: "Bounded remote text work".into(),
        version: "2026.8.4".into(),
        base_url: "https://peer.example.test".into(),
        skills: vec![AgentSkillConfig {
            id: "research".into(),
            name: "Research".into(),
            description: "Answers bounded text research prompts".into(),
            tags: vec!["research".into()],
            examples: vec!["Summarize this topic".into()],
        }],
    })
    .expect("valid card");

    let value = serde_json::to_value(&card).expect("card json");
    assert_eq!(
        value["supportedInterfaces"][0]["url"],
        "https://peer.example.test/a2a"
    );
    assert_eq!(
        value["supportedInterfaces"][0]["protocolBinding"],
        "HTTP+JSON"
    );
    assert_eq!(value["supportedInterfaces"][0]["protocolVersion"], "1.0");
    assert_eq!(value["capabilities"]["streaming"], false);
    assert_eq!(value["capabilities"]["pushNotifications"], false);
    assert_eq!(value["capabilities"]["extendedAgentCard"], false);
    assert_eq!(value["defaultInputModes"], json!(["text/plain"]));
    assert_eq!(value["defaultOutputModes"], json!(["text/plain"]));
    assert_eq!(
        value["securitySchemes"]["peerBearer"]["httpAuthSecurityScheme"]["scheme"],
        "Bearer"
    );
    assert_eq!(value["securityRequirements"], json!([{ "peerBearer": [] }]));
    assert!(!serde_json::to_string(&value)
        .unwrap()
        .contains("credential"));
}

#[test]
fn agent_card_rejects_ambiguous_or_credentialed_base_urls() {
    for base_url in [
        "file:///tmp/zbot",
        "https://user:secret@peer.example.test",
        "https://peer.example.test/base",
        "https://peer.example.test?redirect=https://evil.example",
        "https://peer.example.test/#fragment",
    ] {
        let result = agent_card(AgentCardConfig {
            name: "Research zBot".into(),
            description: "Bounded remote text work".into(),
            version: "2026.8.4".into(),
            base_url: base_url.into(),
            skills: vec![AgentSkillConfig {
                id: "research".into(),
                name: "Research".into(),
                description: "Answers bounded text prompts".into(),
                tags: vec![],
                examples: vec![],
            }],
        });
        assert_eq!(
            result.unwrap_err(),
            ProtocolError::InvalidParams,
            "{base_url}"
        );
    }
}

#[test]
fn send_requires_immediate_text() {
    let valid = json!({
        "message": {
            "messageId": "msg-1",
            "role": "ROLE_USER",
            "parts": [{ "text": "hello", "mediaType": "text/plain" }]
        },
        "configuration": {
            "acceptedOutputModes": ["text/plain"],
            "returnImmediately": true
        }
    });
    let request: a2a::SendMessageRequest = serde_json::from_value(valid).unwrap();
    let accepted = validate_send_message_request(&request).expect("valid send");
    assert_eq!(accepted.message_id, "msg-1");
    assert_eq!(accepted.text, "hello");

    let missing_immediate = json!({
        "message": { "messageId": "msg-1", "role": "ROLE_USER", "parts": [{ "text": "hello" }] },
        "configuration": { "acceptedOutputModes": ["text/plain"] }
    });
    assert_eq!(
        validate_send_message_request(
            &serde_json::from_value::<a2a::SendMessageRequest>(missing_immediate).unwrap()
        )
        .unwrap_err(),
        ProtocolError::InvalidParams
    );

    let false_immediate = json!({
        "message": { "messageId": "msg-1", "role": "ROLE_USER", "parts": [{ "text": "hello" }] },
        "configuration": { "acceptedOutputModes": ["text/plain"], "returnImmediately": false }
    });
    assert!(validate_send_message_request(
        &serde_json::from_value::<a2a::SendMessageRequest>(false_immediate).unwrap()
    )
    .is_err());

    let multiple_parts = json!({
        "message": {
            "messageId": "msg-1",
            "role": "ROLE_USER",
            "parts": [{ "text": "hello" }, { "text": "again" }]
        },
        "configuration": { "acceptedOutputModes": ["text/plain"], "returnImmediately": true }
    });
    assert!(validate_send_message_request(
        &serde_json::from_value::<a2a::SendMessageRequest>(multiple_parts).unwrap()
    )
    .is_err());

    let bad_role = json!({
        "message": { "messageId": "msg-1", "role": "ROLE_AGENT", "parts": [{ "text": "hello" }] },
        "configuration": { "acceptedOutputModes": ["text/plain"], "returnImmediately": true }
    });
    assert!(validate_send_message_request(
        &serde_json::from_value::<a2a::SendMessageRequest>(bad_role).unwrap()
    )
    .is_err());

    let with_extension = json!({
        "message": {
            "messageId": "msg-1",
            "role": "ROLE_USER",
            "parts": [{ "text": "hello" }],
            "extensions": ["urn:unsupported"]
        },
        "configuration": { "acceptedOutputModes": ["text/plain"], "returnImmediately": true }
    });
    assert_eq!(
        validate_send_message_request(
            &serde_json::from_value::<a2a::SendMessageRequest>(with_extension).unwrap()
        )
        .unwrap_err(),
        ProtocolError::ExtensionSupportRequired
    );

    let too_many_codepoints = "a".repeat(1001);
    let oversized = json!({
        "message": { "messageId": "msg-1", "role": "ROLE_USER", "parts": [{ "text": too_many_codepoints }] },
        "configuration": { "acceptedOutputModes": ["text/plain"], "returnImmediately": true }
    });
    assert_eq!(
        validate_send_message_request(
            &serde_json::from_value::<a2a::SendMessageRequest>(oversized).unwrap()
        )
        .unwrap_err(),
        ProtocolError::PayloadTooLarge
    );

    assert!(validate_headers(Some("1.0"), Some(APPLICATION_A2A_JSON)).is_ok());
    assert_eq!(
        validate_headers(Some("2.0"), Some(APPLICATION_A2A_JSON)).unwrap_err(),
        ProtocolError::VersionNotSupported
    );
    assert_eq!(
        validate_headers(Some("1.0"), Some("application/json")).unwrap_err(),
        ProtocolError::ContentTypeNotSupported
    );
}

#[test]
fn raw_send_rejects_ambiguous_or_unknown_part_shapes() {
    for invalid in [
        json!({
            "message": {
                "messageId": "msg-mixed-url",
                "role": "ROLE_USER",
                "parts": [{ "text": "hello", "url": "https://example.test/file" }]
            },
            "configuration": { "acceptedOutputModes": ["text/plain"], "returnImmediately": true }
        }),
        json!({
            "message": {
                "messageId": "msg-mixed-raw",
                "role": "ROLE_USER",
                "parts": [{ "text": "hello", "raw": "aGVsbG8=" }]
            },
            "configuration": { "acceptedOutputModes": ["text/plain"], "returnImmediately": true }
        }),
        json!({
            "message": {
                "messageId": "msg-unknown",
                "role": "ROLE_USER",
                "parts": [{ "text": "hello", "futureField": true }]
            },
            "configuration": { "acceptedOutputModes": ["text/plain"], "returnImmediately": true }
        }),
        json!({
            "message": {
                "messageId": "msg-unknown-envelope",
                "role": "ROLE_USER",
                "parts": [{ "text": "hello" }],
                "futureMessageField": true
            },
            "configuration": { "acceptedOutputModes": ["text/plain"], "returnImmediately": true },
            "futureRequestField": true
        }),
    ] {
        let body = serde_json::to_vec(&invalid).unwrap();
        assert_eq!(
            parse_send_message_request(&body).unwrap_err(),
            ProtocolError::InvalidParams
        );
    }

    let valid = serde_json::to_vec(&json!({
        "message": {
            "messageId": "msg-strict",
            "role": "ROLE_USER",
            "parts": [{ "text": "hello", "mediaType": "text/plain" }]
        },
        "configuration": { "acceptedOutputModes": ["text/plain"], "returnImmediately": true }
    }))
    .unwrap();
    let (_request, accepted) = parse_send_message_request(&valid).expect("strict valid send");
    assert_eq!(accepted.message_id, "msg-strict");
}

#[test]
fn task_projection_matches_contract() {
    let submitted = project_task(TaskProjection {
        id: "task-1".into(),
        context_id: "ctx-1".into(),
        state: TaskProjectionState::Submitted,
        message: None,
        artifact_text: None,
        updated_at: None,
    })
    .unwrap();
    let response = send_message_response(submitted.clone());
    assert_eq!(
        serde_json::to_value(&response).unwrap(),
        json!({
            "task": {
                "id": "task-1",
                "contextId": "ctx-1",
                "status": { "state": "TASK_STATE_SUBMITTED" }
            }
        })
    );

    let completed = project_task(TaskProjection {
        id: "task-1".into(),
        context_id: "ctx-1".into(),
        state: TaskProjectionState::Completed,
        message: Some("done".into()),
        artifact_text: Some("final answer".into()),
        updated_at: None,
    })
    .unwrap();
    let completed_value = serde_json::to_value(&completed).unwrap();
    assert_eq!(completed_value["status"]["state"], "TASK_STATE_COMPLETED");
    assert_eq!(completed_value["status"]["message"]["role"], "ROLE_AGENT");
    assert_eq!(
        completed_value["artifacts"][0]["parts"][0]["text"],
        "final answer"
    );
    assert!(!serde_json::to_string(&completed_value)
        .unwrap()
        .contains("execution"));

    let list = list_tasks_response(vec![submitted, completed], Some("next".into()), 2, 10).unwrap();
    assert_eq!(
        serde_json::to_value(&list).unwrap()["nextPageToken"],
        "next"
    );
    assert_eq!(cancel_task_request("task-1").id, "task-1");

    let max_id = "t".repeat(128);
    let bounded = project_task(TaskProjection {
        id: max_id,
        context_id: "c".repeat(128),
        state: TaskProjectionState::Completed,
        message: Some("done".into()),
        artifact_text: Some("final answer".into()),
        updated_at: None,
    })
    .expect("boundary-valid task ids must project");
    assert!(bounded
        .status
        .message
        .as_ref()
        .is_some_and(|message| message.message_id.len() <= 128));
    assert!(bounded.artifacts.as_ref().is_some_and(|artifacts| {
        artifacts
            .iter()
            .all(|artifact| artifact.artifact_id.len() <= 128)
    }));

    let other = project_task(TaskProjection {
        id: format!("{}z", "t".repeat(127)),
        context_id: "ctx-other".into(),
        state: TaskProjectionState::Completed,
        message: Some("done".into()),
        artifact_text: None,
        updated_at: None,
    })
    .unwrap();
    assert_ne!(
        bounded.status.message.unwrap().message_id,
        other.status.message.unwrap().message_id,
        "derived message IDs must not collapse long task IDs sharing a prefix"
    );
}

#[test]
fn standard_errors_are_bounded() {
    for error in [
        ProtocolError::InvalidRequest,
        ProtocolError::InvalidParams,
        ProtocolError::UnsupportedOperation,
        ProtocolError::ContentTypeNotSupported,
        ProtocolError::VersionNotSupported,
        ProtocolError::TaskNotFound,
        ProtocolError::TaskNotCancelable,
        ProtocolError::PayloadTooLarge,
    ] {
        let value = serde_json::to_value(error_response(error)).unwrap();
        assert!(value["error"]["code"].as_u64().unwrap() >= 400);
        assert!(value["error"]["code"].as_u64().unwrap() <= 599);
        assert!(value["error"]["message"].as_str().unwrap().len() <= 256);
        assert_eq!(
            value["error"]["details"][0]["@type"],
            "type.googleapis.com/google.rpc.ErrorInfo"
        );
        assert_eq!(value["error"]["details"][0]["domain"], "a2a-protocol.org");
    }

    let debug = format!("{:?}", error_response(ProtocolError::Unauthorized));
    assert!(!debug.contains("Bearer abc"));
}
