use assert_cmd::Command;
use predicates::prelude::*;

#[test]
fn peer_issue_list_add_remove_are_local_file_commands() {
    let dir = tempfile::tempdir().unwrap();

    let issue = Command::cargo_bin("zbot")
        .unwrap()
        .args([
            "--data-dir",
            dir.path().to_str().unwrap(),
            "peers",
            "issue",
            "node-alpha",
            "--target-agent",
            "assistant",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("credential_id"))
        .stdout(predicate::str::contains("token"))
        .get_output()
        .stdout
        .clone();
    let issue = String::from_utf8(issue).unwrap();
    let token = issue
        .lines()
        .find_map(|line| line.strip_prefix("token: "))
        .expect("new token displayed once")
        .to_string();

    Command::cargo_bin("zbot")
        .unwrap()
        .args(["--data-dir", dir.path().to_str().unwrap(), "peers", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("node-alpha"))
        .stdout(predicate::str::contains(&token).not());

    Command::cargo_bin("zbot")
        .unwrap()
        .args([
            "--data-dir",
            dir.path().to_str().unwrap(),
            "peers",
            "add",
            "node-beta",
            "--origin",
            "https://beta.example:18791",
            "--token",
            "outbound-secret",
            "--target-agent",
            "assistant",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("node-beta"));

    Command::cargo_bin("zbot")
        .unwrap()
        .args([
            "--data-dir",
            dir.path().to_str().unwrap(),
            "peers",
            "remove",
            "node-beta",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("removed"));
}
