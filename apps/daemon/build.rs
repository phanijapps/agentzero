//! Build script that captures the current git branch when invoked with
//! `ZBOT_INSTALL=1` (set by `make install` and `scripts/install.sh`)
//! and emits it as `BUILD_VERSION_SUFFIX` for the daemon to read via
//! `option_env!`. Plain `cargo build` (dev workflow) doesn't set the
//! env var, so dev builds keep the bare `CARGO_PKG_VERSION` — no
//! spurious branch suffix during normal work.
//!
//! See `docs/architecture/future-state/2026-05-03-versioning-and-rename-plan.md`
//! for the broader versioning + rename context.

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
        // Detached HEAD or empty — fall through with no suffix.
        return;
    }

    // Sanitize: replace `/` with `-` so a branch like `feat/foo` becomes
    // `feat-foo`. Keeps the version string out of unexpected territory
    // (slashes confuse some downstream tools).
    let suffix: String = raw
        .chars()
        .map(|c| if c == '/' { '-' } else { c })
        .collect();

    // Emit the FULL version string (`<pkg>.<branch>`) so the daemon can
    // read it via a single `option_env!("BUILD_VERSION")` call without
    // needing runtime concatenation. Cargo sets `CARGO_PKG_VERSION` in
    // the build.rs environment.
    let pkg_version = std::env::var("CARGO_PKG_VERSION").unwrap_or_default();
    println!("cargo:rustc-env=BUILD_VERSION={}.{}", pkg_version, suffix);
}
