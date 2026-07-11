use zbot_conversation::{
    open_conversation_pool, AutonomyApprovalPolicy, AutonomyEvidence, AutonomyItem, AutonomyState,
    AutonomyStore, SqliteAutonomyStore,
};

fn store() -> SqliteAutonomyStore {
    let file = tempfile::NamedTempFile::new().unwrap();
    SqliteAutonomyStore::new(open_conversation_pool(file.path()).unwrap())
}

fn item() -> AutonomyItem {
    AutonomyItem {
        id: "aut-1".into(),
        title: "Rig migration".into(),
        objective: "Decide engine migration".into(),
        next_action: "Verify parity".into(),
        state: AutonomyState::Proposed,
        approval_policy: AutonomyApprovalPolicy::Manual,
        source_session_id: Some("sess-1".into()),
        dedupe_key: "rig-migration".into(),
        created_at: "2026-07-09T00:00:00Z".into(),
        updated_at: "2026-07-09T00:00:00Z".into(),
        completed_at: None,
    }
}

#[test]
fn create_persists_reference_only_evidence_and_audit_run() {
    let store = store();
    let item = item();
    store
        .create(
            &item,
            &[AutonomyEvidence {
                id: "e-1".into(),
                item_id: item.id.clone(),
                kind: "session".into(),
                reference_id: "sess-1".into(),
                label: Some("Source session".into()),
                created_at: item.created_at.clone(),
            }],
        )
        .unwrap();

    assert_eq!(store.get("aut-1").unwrap(), Some(item));
    assert_eq!(store.evidence("aut-1").unwrap()[0].reference_id, "sess-1");
    assert_eq!(store.runs("aut-1").unwrap()[0].kind, "created");
}

#[test]
fn lifecycle_rejects_invalid_transition_and_audits_valid_transition() {
    let store = store();
    let item = item();
    store.create(&item, &[]).unwrap();

    assert!(store
        .transition("aut-1", AutonomyState::Blocked, None)
        .is_err());
    let approved = store
        .transition("aut-1", AutonomyState::Approved, Some("user approved"))
        .unwrap();
    assert_eq!(approved.state, AutonomyState::Approved);
    let blocked = store
        .transition("aut-1", AutonomyState::Blocked, Some("needs parity"))
        .unwrap();
    assert_eq!(blocked.state, AutonomyState::Blocked);
    assert_eq!(store.runs("aut-1").unwrap().len(), 3);
}

#[test]
fn complete_items_do_not_appear_in_open_list() {
    let store = store();
    let item = item();
    store.create(&item, &[]).unwrap();
    store
        .transition("aut-1", AutonomyState::Approved, None)
        .unwrap();
    store
        .transition("aut-1", AutonomyState::Complete, Some("decision made"))
        .unwrap();
    assert!(store.list_open(10).unwrap().is_empty());
}
