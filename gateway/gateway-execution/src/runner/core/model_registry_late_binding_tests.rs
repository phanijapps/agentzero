//! Regression tests for the capture-before-init bug that caused
//! `context_window_tokens = 8192` on the continuation path.
//!
//! The failure mode: the pre-captured context clones
//! `self.model_registry` (the ArcSwap handle) inside `with_config`
//! BEFORE `set_model_registry` runs. When the field was a plain
//! `Option<Arc<_>>`, the captured clone froze as `None` and every
//! continuation-path executor fell back to the 8192 default at
//! `invoke/executor.rs:423`. After the fix the field is an
//! `Arc<ArcSwapOption<_>>`, so pre-captured handles read the live
//! value at fire time.
//!
//! These tests target the ArcSwap-based late-binding contract
//! without needing the full `ExecutionRunner` construction graph.
use arc_swap::ArcSwapOption;
use gateway_services::models::ModelRegistry;
use std::sync::Arc;

fn load_user_registry() -> Arc<ModelRegistry> {
    Arc::new(ModelRegistry::load())
}

/// The core contract: a clone of the `Arc<ArcSwapOption<T>>` captured
/// before `store(...)` must see `Some(...)` on a subsequent
/// `load_full()`. This is what pre-spawned async tasks rely on.
#[test]
fn pre_captured_clone_sees_late_store() {
    // Step 1: field initialized empty (mirrors `ExecutionRunner::new`).
    let field: Arc<ArcSwapOption<ModelRegistry>> = Arc::new(ArcSwapOption::from(None));

    // Step 2: the pre-captured context clones the handle inside
    // `with_config` BEFORE the setter runs.
    let captured = field.clone();
    assert!(captured.load_full().is_none(), "field starts empty");

    // Step 3: `runtime.rs:145` calls `set_model_registry(...)`.
    field.store(Some(load_user_registry()));

    // Step 4: the pre-captured clone reads the live value at fire time.
    let reg = captured
        .load_full()
        .expect("late store must be visible to pre-captured clone");

    // And the registry returns the real context window, not 8192.
    let ctx = reg.context_window("glm-5-turbo");
    assert_eq!(
        ctx.input, 200_000,
        "registry lookup must return glm-5-turbo's real 200k input \
             window, not the 8192 fallback"
    );
}

/// Multiple pre-captured clones (e.g. multiple background tasks)
/// each see the latest stored value. Mirrors the real topology:
/// spawn_delegation_handler + ContinuationWatcher + others.
#[test]
fn multiple_captures_all_observe_late_store() {
    let field: Arc<ArcSwapOption<ModelRegistry>> = Arc::new(ArcSwapOption::from(None));

    let cap_a = field.clone();
    let cap_b = field.clone();
    let cap_c = field.clone();

    field.store(Some(load_user_registry()));

    for (name, cap) in [("a", cap_a), ("b", cap_b), ("c", cap_c)] {
        assert!(
            cap.load_full().is_some(),
            "capture '{name}' must observe the stored registry"
        );
    }
}

/// Sanity: an unknown model falls back to the registry's internal
/// `input: 200_000`, NOT the executor's `8192`. That proves the fix
/// also helps the degenerate case (unknown model) as long as the
/// registry itself is installed.
#[test]
fn unknown_model_uses_registry_fallback_not_executor_fallback() {
    let field: Arc<ArcSwapOption<ModelRegistry>> = Arc::new(ArcSwapOption::from(None));
    let captured = field.clone();
    field.store(Some(load_user_registry()));

    let reg = captured.load_full().expect("installed");
    let ctx = reg.context_window("some-unknown-model-xyz");
    assert_eq!(
        ctx.input, 200_000,
        "registry's internal fallback for unknown models is 200k, \
             not the 8192 emergency default"
    );
}
