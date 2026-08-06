use gateway_a2a::peers::{AddPeer, IssueCredential, PeerStore};
use std::fs;

#[test]
fn issue_rotate_revoke_persist_hashes_and_redact_secrets() {
    let dir = tempfile::tempdir().unwrap();
    let store = PeerStore::new(dir.path());

    let issued = store
        .issue_credential(IssueCredential {
            peer_id: "node-alpha".into(),
            display_name: Some("Alpha".into()),
            target_agent_id: "assistant".into(),
            lifetime_days: None,
        })
        .unwrap();
    assert_eq!(issued.token.exposed().len(), 43);

    let path = PeerStore::path_for_data_dir(dir.path());
    let raw = fs::read_to_string(&path).unwrap();
    assert!(raw.contains("\"version\": 1"));
    assert!(!raw.contains(issued.token.exposed()));
    assert!(raw.contains("credential_hash"));
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );

    let snapshot = store.load_snapshot().unwrap();
    assert!(snapshot.verify_inbound_token("node-alpha", issued.token.exposed()));
    assert!(!format!("{snapshot:?}").contains(issued.token.exposed()));
    assert!(!serde_json::to_string(&snapshot.redacted())
        .unwrap()
        .contains(issued.token.exposed()));

    let rotated = store
        .issue_credential(IssueCredential {
            peer_id: "node-alpha".into(),
            display_name: None,
            target_agent_id: "assistant".into(),
            lifetime_days: Some(90),
        })
        .unwrap();
    assert!(store
        .load_snapshot()
        .unwrap()
        .verify_inbound_token("node-alpha", rotated.token.exposed()));

    let too_many = store.issue_credential(IssueCredential {
        peer_id: "node-alpha".into(),
        display_name: None,
        target_agent_id: "assistant".into(),
        lifetime_days: Some(90),
    });
    assert!(too_many.is_err());

    store
        .revoke_credential("node-alpha", &issued.credential_id)
        .unwrap();
    let snapshot = store.load_snapshot().unwrap();
    assert!(!snapshot.verify_inbound_token("node-alpha", issued.token.exposed()));
    assert!(snapshot.verify_inbound_token("node-alpha", rotated.token.exposed()));
}

#[test]
fn add_remove_and_live_snapshot_reload_without_restart() {
    let dir = tempfile::tempdir().unwrap();
    let reader = PeerStore::new(dir.path());
    let writer = PeerStore::new(dir.path());

    writer
        .add_peer(AddPeer {
            peer_id: "node-beta".into(),
            display_name: "Beta".into(),
            origin: "https://beta.example:18791".into(),
            target_agent_id: "assistant".into(),
            outbound_token: Some("outbound-secret".into()),
            allow_private_http: false,
        })
        .unwrap();

    let snapshot = reader.load_snapshot().unwrap();
    assert!(snapshot.get("node-beta").is_some());
    assert!(!format!("{snapshot:?}").contains("outbound-secret"));

    writer.remove_peer("node-beta").unwrap();
    assert!(reader.load_snapshot().unwrap().get("node-beta").is_none());
}

#[test]
fn strict_schema_and_unsafe_file_shapes_fail_closed() {
    let dir = tempfile::tempdir().unwrap();
    let path = PeerStore::path_for_data_dir(dir.path());
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, r#"{"version":1,"peers":[],"extra":true}"#).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();

    assert!(PeerStore::new(dir.path()).load_snapshot().is_err());

    fs::remove_file(&path).unwrap();
    std::os::unix::fs::symlink("/tmp/elsewhere", &path).unwrap();
    assert!(PeerStore::new(dir.path()).load_snapshot().is_err());
}

trait ModeExt {
    fn mode(&self) -> u32;
}

impl ModeExt for fs::Permissions {
    fn mode(&self) -> u32 {
        std::os::unix::fs::PermissionsExt::mode(self)
    }
}

trait PermissionsExt {
    fn from_mode(mode: u32) -> fs::Permissions;
}

impl PermissionsExt for fs::Permissions {
    fn from_mode(mode: u32) -> fs::Permissions {
        std::os::unix::fs::PermissionsExt::from_mode(mode)
    }
}
