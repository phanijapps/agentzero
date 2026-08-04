use chrono::Utc;
use execution_state::{
    WorkAuthorization, WorkDraft, WorkEnvelope, WorkError, WorkPolicy, WorkPolicyError,
    WorkValidationError, MAX_ATTEMPTS, MAX_CORRELATION_BYTES, MAX_DEDUPE_BYTES, MAX_PAYLOAD_BYTES,
    MAX_PROVENANCE_ID_BYTES, MAX_ROUTING_BYTES,
};

fn trusted_authorization() -> WorkAuthorization {
    WorkAuthorization::new("node-local", "node-local", "root", "sess-1", "exec-1")
}

struct AllowEcho;

impl WorkPolicy for AllowEcho {
    fn authorize(&self, draft: &WorkDraft) -> Result<WorkAuthorization, WorkPolicyError> {
        if draft.kind() != "test.echo" {
            return Err(WorkPolicyError::KindNotAllowed);
        }
        if draft.target() != "worker.local" {
            return Err(WorkPolicyError::TargetNotAllowed);
        }
        if !draft
            .payload()
            .get("text")
            .is_some_and(|value| value.is_string())
        {
            return Err(WorkPolicyError::PayloadInvalid);
        }
        Ok(trusted_authorization())
    }
}

struct BoundaryPolicy(WorkAuthorization);

impl WorkPolicy for BoundaryPolicy {
    fn authorize(&self, _draft: &WorkDraft) -> Result<WorkAuthorization, WorkPolicyError> {
        Ok(self.0.clone())
    }
}

fn base_draft() -> WorkDraft {
    WorkDraft::new(
        "test.echo",
        "worker.local",
        serde_json::json!({"text": "hello"}),
    )
}

fn assert_invalid(draft: WorkDraft, expected: WorkValidationError) {
    assert_invalid_with(draft, trusted_authorization(), expected);
}

fn assert_invalid_with(
    draft: WorkDraft,
    authorization: WorkAuthorization,
    expected: WorkValidationError,
) {
    assert_eq!(
        WorkEnvelope::authorize(draft, &BoundaryPolicy(authorization), Utc::now()),
        Err(WorkError::InvalidEnvelope(expected))
    );
}

// STUB: AC-envelope-bounds
#[test]
fn ac_envelope_bounds() {
    assert_invalid(
        base_draft().with_version(2),
        WorkValidationError::UnsupportedVersion,
    );

    for (kind, target, expected) in [
        ("", "worker.local", WorkValidationError::InvalidKind),
        ("test.echo", "", WorkValidationError::InvalidTarget),
    ] {
        assert_invalid(
            WorkDraft::new(kind, target, serde_json::json!({"text": "hello"})),
            expected,
        );
    }

    let oversized_routing = "x".repeat(MAX_ROUTING_BYTES + 1);
    for (kind, target, expected) in [
        (
            oversized_routing.as_str(),
            "worker.local",
            WorkValidationError::InvalidKind,
        ),
        (
            "test.echo",
            oversized_routing.as_str(),
            WorkValidationError::InvalidTarget,
        ),
    ] {
        assert_invalid(
            WorkDraft::new(kind, target, serde_json::json!({"text": "hello"})),
            expected,
        );
    }

    let oversized_source = "x".repeat(MAX_ROUTING_BYTES + 1);
    assert_invalid_with(
        base_draft(),
        WorkAuthorization::new(&oversized_source, "node-local", "root", "sess-1", "exec-1"),
        WorkValidationError::InvalidSource,
    );
    assert_invalid_with(
        base_draft(),
        WorkAuthorization::new("", "node-local", "root", "sess-1", "exec-1"),
        WorkValidationError::InvalidSource,
    );

    let oversized_provenance = "x".repeat(MAX_PROVENANCE_ID_BYTES + 1);
    for authorization in [
        WorkAuthorization::new(
            "node-local",
            &oversized_provenance,
            "root",
            "sess-1",
            "exec-1",
        ),
        WorkAuthorization::new(
            "node-local",
            "node-local",
            &oversized_provenance,
            "sess-1",
            "exec-1",
        ),
        WorkAuthorization::new(
            "node-local",
            "node-local",
            "root",
            &oversized_provenance,
            "exec-1",
        ),
        WorkAuthorization::new(
            "node-local",
            "node-local",
            "root",
            "sess-1",
            &oversized_provenance,
        ),
        WorkAuthorization::new("node-local", "node-local", "", "sess-1", "exec-1"),
    ] {
        assert_invalid_with(
            base_draft(),
            authorization,
            WorkValidationError::InvalidProvenance,
        );
    }

    let exact_payload = "x".repeat(MAX_PAYLOAD_BYTES - 11);
    let oversized_payload = "x".repeat(MAX_PAYLOAD_BYTES - 10);
    assert!(WorkEnvelope::authorize(
        WorkDraft::new(
            "test.echo",
            "worker.local",
            serde_json::json!({"text": exact_payload}),
        ),
        &BoundaryPolicy(trusted_authorization()),
        Utc::now(),
    )
    .is_ok());
    assert_invalid(
        WorkDraft::new(
            "test.echo",
            "worker.local",
            serde_json::json!({"text": oversized_payload}),
        ),
        WorkValidationError::PayloadTooLarge,
    );

    for draft in [
        base_draft().with_correlation_id(""),
        base_draft().with_correlation_id("x".repeat(MAX_CORRELATION_BYTES + 1)),
    ] {
        assert_invalid(draft, WorkValidationError::InvalidCorrelation);
    }
    for draft in [
        base_draft().with_dedupe_key(""),
        base_draft().with_dedupe_key("x".repeat(MAX_DEDUPE_BYTES + 1)),
    ] {
        assert_invalid(draft, WorkValidationError::InvalidDedupeKey);
    }
    for attempts in [0, MAX_ATTEMPTS + 1] {
        assert_invalid(
            base_draft().with_max_attempts(attempts),
            WorkValidationError::InvalidAttemptLimit,
        );
    }

    let exact = WorkDraft::new(
        "k".repeat(MAX_ROUTING_BYTES),
        "t".repeat(MAX_ROUTING_BYTES),
        serde_json::json!({"text": "hello"}),
    )
    .with_correlation_id("c".repeat(MAX_CORRELATION_BYTES))
    .with_dedupe_key("d".repeat(MAX_DEDUPE_BYTES))
    .with_max_attempts(MAX_ATTEMPTS);
    let exact_authorization = WorkAuthorization::new(
        "s".repeat(MAX_ROUTING_BYTES),
        "n".repeat(MAX_PROVENANCE_ID_BYTES),
        "a".repeat(MAX_PROVENANCE_ID_BYTES),
        "s".repeat(MAX_PROVENANCE_ID_BYTES),
        "e".repeat(MAX_PROVENANCE_ID_BYTES),
    );
    assert!(
        WorkEnvelope::authorize(exact, &BoundaryPolicy(exact_authorization), Utc::now()).is_ok()
    );
    assert!(WorkEnvelope::authorize(
        base_draft().with_max_attempts(1),
        &BoundaryPolicy(trusted_authorization()),
        Utc::now()
    )
    .is_ok());
}

// STUB: AC-envelope-authority
#[test]
fn ac_envelope_authority() {
    let envelope = WorkEnvelope::authorize(base_draft(), &AllowEcho, Utc::now())
        .expect("allowlisted host envelope");
    assert_eq!(envelope.source(), "node-local");
    assert_eq!(envelope.provenance().actor_id(), "root");

    let denied = WorkDraft::new(
        "admin.delete",
        "worker.local",
        serde_json::json!({"text": "hello"}),
    );
    assert_eq!(
        WorkEnvelope::authorize(denied, &AllowEcho, Utc::now()).unwrap_err(),
        WorkError::Policy(WorkPolicyError::KindNotAllowed)
    );

    let denied_target = WorkDraft::new(
        "test.echo",
        "worker.remote",
        serde_json::json!({"text": "hello"}),
    );
    assert_eq!(
        WorkEnvelope::authorize(denied_target, &AllowEcho, Utc::now()).unwrap_err(),
        WorkError::Policy(WorkPolicyError::TargetNotAllowed)
    );

    let malformed_payload =
        WorkDraft::new("test.echo", "worker.local", serde_json::json!({"text": 42}));
    assert_eq!(
        WorkEnvelope::authorize(malformed_payload, &AllowEcho, Utc::now()).unwrap_err(),
        WorkError::Policy(WorkPolicyError::PayloadInvalid)
    );

    let draft_debug = format!(
        "{:?}",
        base_draft().with_correlation_id("CORRELATION_SECRET")
    );
    let envelope_debug = format!("{envelope:?}");
    assert!(!draft_debug.contains("CORRELATION_SECRET"));
    assert!(!envelope_debug.contains("hello"));
    assert!(!envelope_debug.contains("sess-1"));
}
