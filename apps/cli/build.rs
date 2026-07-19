//! Same as `apps/daemon/build.rs` — captures the current git branch
//! when `ZBOT_INSTALL=1` and emits `BUILD_VERSION_SUFFIX`. See that
//! file for the full rationale.

use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=ZBOT_INSTALL");
    println!("cargo:rerun-if-env-changed=ZBOT_BUILD_DATE");
    println!("cargo:rerun-if-env-changed=ZBOT_BUILD_TIMESTAMP");
    println!("cargo:rerun-if-changed=../../.git/HEAD");
    println!("cargo:rerun-if-changed=../../.git/refs/heads");

    if let Ok(build_date) = std::env::var("ZBOT_BUILD_DATE") {
        println!("cargo:rustc-env=BUILD_DATE={build_date}");
    }
    if let Ok(build_timestamp) = std::env::var("ZBOT_BUILD_TIMESTAMP") {
        println!("cargo:rustc-env=BUILD_TIMESTAMP={build_timestamp}");
    }

    if std::env::var_os("ZBOT_INSTALL").is_none() {
        return;
    }

    let Ok(out) = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .output()
    else {
        return;
    };
    if !out.status.success() {
        return;
    }
    let raw = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if raw.is_empty() || raw == "HEAD" {
        return;
    }

    let suffix: String = raw
        .chars()
        .map(|c| if c == '/' { '-' } else { c })
        .collect();

    let pkg_version = std::env::var("CARGO_PKG_VERSION").unwrap_or_default();
    println!("cargo:rustc-env=BUILD_VERSION={}.{}", pkg_version, suffix);
}
