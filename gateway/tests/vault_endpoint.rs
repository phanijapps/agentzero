mod common;

use axum_test::TestServer;
use common::{make_state, setup};
use gateway::{http::create_http_router, websocket::WebSocketHandler, GatewayConfig};
use serde_json::Value;
use std::{net::Ipv4Addr, sync::Arc};
use tempfile::TempDir;

fn write(path: impl AsRef<std::path::Path>, content: &str) {
    std::fs::write(path, content).expect("write fixture");
}

fn setup_with_config(config: GatewayConfig) -> (TestServer, TempDir) {
    let (dir, state) = make_state();
    let ws_handler = Arc::new(WebSocketHandler::new(
        state.event_bus.clone(),
        state.runtime.clone(),
    ));
    let router = create_http_router(config, state, ws_handler);
    let server = TestServer::new(router).expect("test server");
    (server, dir)
}

#[tokio::test]
async fn lists_active_filesystem_wards_without_internal_entries() {
    let (server, dir, _state) = setup();
    let wards = dir.path().join("wards");
    std::fs::create_dir_all(wards.join("scratch")).unwrap();
    std::fs::create_dir_all(wards.join("stock-analysis")).unwrap();
    std::fs::create_dir_all(wards.join("_archive")).unwrap();
    std::fs::create_dir_all(wards.join("_curator_backups")).unwrap();
    std::fs::create_dir_all(wards.join(".hidden")).unwrap();
    write(wards.join(".usage.json"), "{}");

    let response = server.get("/api/vault/wards").await;
    response.assert_status_ok();
    let body: Value = response.json();
    let names: Vec<_> = body["wards"]
        .as_array()
        .expect("wards array")
        .iter()
        .filter_map(|item| item["id"].as_str())
        .collect();

    assert!(names.contains(&"scratch"));
    assert!(names.contains(&"stock-analysis"));
    assert!(!names.contains(&"_archive"));
    assert!(!names.contains(&"_curator_backups"));
    assert!(!names.contains(&".hidden"));
}

#[tokio::test]
async fn tree_blocks_direct_access_to_internal_ward_roots() {
    let (server, dir, _state) = setup();
    let wards = dir.path().join("wards");
    std::fs::create_dir_all(wards.join("_archive")).unwrap();
    std::fs::create_dir_all(wards.join(".hidden")).unwrap();
    write(wards.join("_archive").join("notes.md"), "# archived");
    write(wards.join(".hidden").join("notes.md"), "# hidden");

    let archive_response = server.get("/api/vault/wards/_archive/tree").await;
    archive_response.assert_status_forbidden();

    let hidden_response = server.get("/api/vault/wards/.hidden/tree").await;
    hidden_response.assert_status_forbidden();
}

#[cfg(unix)]
#[tokio::test]
async fn tree_blocks_symlinked_ward_roots() {
    use std::os::unix::fs::symlink;

    let (server, dir, _state) = setup();
    let wards = dir.path().join("wards");
    std::fs::create_dir_all(wards.join("_archive")).unwrap();
    write(wards.join("_archive").join("notes.md"), "# archived");
    symlink("_archive", wards.join("public")).unwrap();

    let list_response = server.get("/api/vault/wards").await;
    list_response.assert_status_ok();
    let body: Value = list_response.json();
    let names: Vec<_> = body["wards"]
        .as_array()
        .expect("wards array")
        .iter()
        .filter_map(|item| item["id"].as_str())
        .collect();
    assert!(!names.contains(&"public"));

    let tree_response = server.get("/api/vault/wards/public/tree").await;
    tree_response.assert_status_forbidden();
}

#[tokio::test]
async fn tree_lists_directories_and_visible_files_only() {
    let (server, dir, _state) = setup();
    let ward = dir.path().join("wards").join("stock-analysis");
    std::fs::create_dir_all(ward.join("reports")).unwrap();
    std::fs::create_dir_all(ward.join(".venv")).unwrap();
    write(ward.join("README.md"), "# Stock");
    write(ward.join("script.PY"), "print('x')");
    write(ward.join("secrets.yaml"), "token: nope");
    write(ward.join(".ENV"), "TOKEN=nope");
    write(ward.join("image.png"), "not listed");
    write(ward.join("deck.ppt"), "legacy");
    write(ward.join("slides.pptx"), "pptx bytes");

    let response = server.get("/api/vault/wards/stock-analysis/tree").await;
    response.assert_status_ok();
    let body: Value = response.json();
    let names: Vec<_> = body["children"]
        .as_array()
        .expect("children array")
        .iter()
        .filter_map(|item| item["name"].as_str())
        .collect();

    assert!(names.contains(&"reports"));
    assert!(names.contains(&"README.md"));
    assert!(names.contains(&"script.PY"));
    assert!(names.contains(&"deck.ppt"));
    assert!(names.contains(&"slides.pptx"));
    assert!(!names.contains(&".venv"));
    assert!(!names.contains(&"secrets.yaml"));
    assert!(!names.contains(&".ENV"));
    assert!(!names.contains(&"image.png"));
}

#[tokio::test]
async fn text_file_endpoint_returns_ward_relative_content() {
    let (server, dir, _state) = setup();
    let ward = dir.path().join("wards").join("stock-analysis");
    std::fs::create_dir_all(ward.join("reports")).unwrap();
    write(ward.join("reports").join("valuation.md"), "# Valuation");

    let response = server
        .get("/api/vault/wards/stock-analysis/file")
        .add_query_param("path", "reports/valuation.md")
        .await;
    response.assert_status_ok();
    let body: Value = response.json();

    assert_eq!(body["ward_id"], "stock-analysis");
    assert_eq!(body["path"], "reports/valuation.md");
    assert_eq!(body["content"], "# Valuation");
    assert_eq!(body["extension"], "md");
}

#[tokio::test]
async fn file_endpoint_blocks_excluded_non_readable_and_traversal_paths() {
    let (server, dir, _state) = setup();
    let ward = dir.path().join("wards").join("stock-analysis");
    std::fs::create_dir_all(&ward).unwrap();
    write(ward.join(".env"), "TOKEN=nope");
    write(ward.join("legacy.doc"), "legacy");
    write(dir.path().join("outside.md"), "outside");

    let env_response = server
        .get("/api/vault/wards/stock-analysis/file")
        .add_query_param("path", ".env")
        .await;
    env_response.assert_status_forbidden();

    let legacy_response = server
        .get("/api/vault/wards/stock-analysis/file")
        .add_query_param("path", "legacy.doc")
        .await;
    legacy_response.assert_status_forbidden();

    let traversal_response = server
        .get("/api/vault/wards/stock-analysis/file")
        .add_query_param("path", "../outside.md")
        .await;
    traversal_response.assert_status_bad_request();
}

#[cfg(unix)]
#[tokio::test]
async fn vault_routes_do_not_follow_symlinks_to_excluded_files() {
    use std::os::unix::fs::symlink;

    let (server, dir, _state) = setup();
    let ward = dir.path().join("wards").join("stock-analysis");
    std::fs::create_dir_all(&ward).unwrap();
    write(ward.join(".env"), "TOKEN=nope");
    symlink(".env", ward.join("public.md")).unwrap();

    let tree_response = server.get("/api/vault/wards/stock-analysis/tree").await;
    tree_response.assert_status_ok();
    let body: Value = tree_response.json();
    let names: Vec<_> = body["children"]
        .as_array()
        .expect("children array")
        .iter()
        .filter_map(|item| item["name"].as_str())
        .collect();
    assert!(!names.contains(&"public.md"));

    let file_response = server
        .get("/api/vault/wards/stock-analysis/file")
        .add_query_param("path", "public.md")
        .await;
    file_response.assert_status_forbidden();
}

#[tokio::test]
async fn file_endpoint_rejects_oversized_text_preview() {
    let (server, dir, _state) = setup();
    let ward = dir.path().join("wards").join("stock-analysis");
    std::fs::create_dir_all(&ward).unwrap();
    std::fs::write(ward.join("large.txt"), vec![b'x'; 2 * 1024 * 1024 + 1]).unwrap();

    let response = server
        .get("/api/vault/wards/stock-analysis/file")
        .add_query_param("path", "large.txt")
        .await;
    response.assert_status_payload_too_large();
}

#[tokio::test]
async fn directory_listing_reports_truncation() {
    let (server, dir, _state) = setup();
    let ward = dir.path().join("wards").join("big");
    std::fs::create_dir_all(&ward).unwrap();
    for i in 0..1001 {
        write(ward.join(format!("file-{i}.md")), "x");
    }

    let response = server.get("/api/vault/wards/big/tree").await;
    response.assert_status_ok();
    let body: Value = response.json();
    assert_eq!(body["children"].as_array().map(Vec::len), Some(1000));
    assert_eq!(body["truncated"], true);
}

#[tokio::test]
async fn search_finds_nested_visible_files_with_fuzzy_query() {
    let (server, dir, _state) = setup();
    let ward = dir.path().join("wards").join("stock-analysis");
    std::fs::create_dir_all(ward.join("reports").join("quarterly")).unwrap();
    write(
        ward.join("reports")
            .join("quarterly")
            .join("valuation-model.md"),
        "# Valuation",
    );
    write(
        ward.join("reports").join("quarterly").join("notes.txt"),
        "notes",
    );

    let response = server
        .get("/api/vault/wards/stock-analysis/search")
        .add_query_param("q", "vlmd")
        .await;
    response.assert_status_ok();
    let body: Value = response.json();
    let paths: Vec<_> = body["matches"]
        .as_array()
        .expect("matches array")
        .iter()
        .filter_map(|item| item["path"].as_str())
        .collect();

    assert!(paths.contains(&"reports/quarterly/valuation-model.md"));
    assert_eq!(body["query"], "vlmd");
    assert_eq!(body["truncated"], false);
}

#[tokio::test]
async fn search_excludes_hidden_env_config_and_non_visible_files() {
    let (server, dir, _state) = setup();
    let ward = dir.path().join("wards").join("stock-analysis");
    std::fs::create_dir_all(ward.join(".hidden")).unwrap();
    std::fs::create_dir_all(ward.join("reports")).unwrap();
    write(ward.join("reports").join("public.md"), "# public");
    write(ward.join(".hidden").join("public-secret.md"), "# hidden");
    write(ward.join("reports").join(".env"), "TOKEN=nope");
    write(ward.join("reports").join("config.yaml"), "token: nope");
    write(ward.join("reports").join("public.png"), "not visible");

    let response = server
        .get("/api/vault/wards/stock-analysis/search")
        .add_query_param("q", "public")
        .await;
    response.assert_status_ok();
    let body: Value = response.json();
    let paths: Vec<_> = body["matches"]
        .as_array()
        .expect("matches array")
        .iter()
        .filter_map(|item| item["path"].as_str())
        .collect();

    assert!(paths.contains(&"reports/public.md"));
    assert!(!paths.contains(&".hidden/public-secret.md"));
    assert!(!paths.contains(&"reports/.env"));
    assert!(!paths.contains(&"reports/config.yaml"));
    assert!(!paths.contains(&"reports/public.png"));
}

#[tokio::test]
async fn vault_routes_deny_when_remote_access_cannot_be_proven_local() {
    let config = GatewayConfig {
        host: Ipv4Addr::UNSPECIFIED.into(),
        ..GatewayConfig::default()
    };
    let (server, _dir) = setup_with_config(config);

    let response = server.get("/api/vault/wards").await;
    response.assert_status_forbidden();
}
