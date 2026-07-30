mod common;

use agent_surfaces::{ComponentType, SurfaceComponent, WorkSurface, ZBOT_WORK_SURFACE_CATALOG};
use axum::http::StatusCode;
use common::setup;
use serde_json::{json, Value};
use std::collections::BTreeMap;

fn saved_surface() -> WorkSurface {
    WorkSurface {
        surface_id: "decision-1".to_owned(),
        catalog_id: ZBOT_WORK_SURFACE_CATALOG.to_owned(),
        components: vec![SurfaceComponent {
            id: "matrix".to_owned(),
            component_type: ComponentType::DecisionMatrix,
            props: BTreeMap::from([("criteria_path".to_owned(), json!("/criteria"))]),
        }],
        data: json!({ "criteria": [] }),
    }
}

#[tokio::test]
async fn persistence_settings_restore_and_clear_are_live_and_bounded() {
    let (server, _dir, state) = setup();
    let (session, execution) = state.state_service.create_session("root").unwrap();
    let surface = saved_surface();
    state
        .state_service
        .save_session_surface(
            &session.id,
            &execution.id,
            &surface.surface_id,
            &serde_json::to_string(&surface).unwrap(),
        )
        .unwrap();

    let disabled = server
        .get(&format!("/api/sessions/{}/surfaces", session.id))
        .await;
    disabled.assert_status_ok();
    assert_eq!(disabled.json::<Value>(), json!([]));

    let rejected = server
        .put("/api/settings/presentation")
        .add_header("origin", "https://evil.example")
        .json(&json!({ "persistSurfaces": true }))
        .await;
    rejected.assert_status(StatusCode::FORBIDDEN);
    assert!(!state.state_service.surface_persistence_enabled());

    let enabled = server
        .put("/api/settings/presentation")
        .json(&json!({ "persistSurfaces": true }))
        .await;
    enabled.assert_status_ok();
    assert!(state.state_service.surface_persistence_enabled());
    assert_eq!(
        enabled.json::<Value>()["data"],
        json!({ "persistSurfaces": true, "restartRequired": false })
    );

    let restored = server
        .get(&format!("/api/sessions/{}/surfaces", session.id))
        .await;
    restored.assert_status_ok();
    assert_eq!(restored.json::<Value>(), json!([surface]));

    let bad_clear = server
        .delete("/api/surfaces/saved")
        .json(&json!({ "confirmation": "wrong" }))
        .await;
    bad_clear.assert_status(StatusCode::BAD_REQUEST);
    assert_eq!(
        state
            .state_service
            .list_session_surfaces(&session.id)
            .unwrap()
            .len(),
        1
    );

    let cleared = server
        .delete("/api/surfaces/saved")
        .json(&json!({ "confirmation": "clear_saved_infographics" }))
        .await;
    cleared.assert_status_ok();
    assert_eq!(cleared.json::<Value>(), json!({ "deletedCount": 1 }));
    assert!(state
        .state_service
        .get_session(&session.id)
        .unwrap()
        .is_some());
}
