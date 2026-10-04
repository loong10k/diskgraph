use crate::git_job_test_fixtures::fixture;
use crate::{JobState, StoreError};
use rusqlite::params;

#[test]
fn typed_git_job_input_is_immutable_and_creation_rolls_back_if_input_write_fails() {
    let (mut control, _, input, authority) = fixture();
    control.connection.execute_batch("CREATE TRIGGER deny_git_input BEFORE INSERT ON git_evidence_job_inputs BEGIN SELECT RAISE(ABORT,'fixture refusal'); END;").unwrap();
    assert!(
        control
            .create_git_evidence_job(&input, &authority, 8)
            .is_err()
    );
    assert_eq!(
        control
            .active_job_count_for_principal(authority.principal())
            .unwrap(),
        0
    );
    assert_eq!(
        control
            .connection
            .query_row("SELECT COUNT(*) FROM job_request_authorities", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    control
        .connection
        .execute_batch("DROP TRIGGER deny_git_input;")
        .unwrap();
    let job = control
        .create_git_evidence_job(&input, &authority, 8)
        .unwrap()
        .unwrap();
    assert!(
        control
            .connection
            .execute(
                "UPDATE git_evidence_job_inputs SET input_sha256=?2 WHERE job_id=?1",
                params![job.job_id, "0".repeat(64)]
            )
            .is_err()
    );
    assert!(
        control
            .connection
            .execute(
                "DELETE FROM git_evidence_job_inputs WHERE job_id=?1",
                [&job.job_id]
            )
            .is_err()
    );
    assert_eq!(control.git_evidence_job_input(&job.job_id).unwrap(), input);
}

#[test]
fn raw_input_rejects_hash_scope_server_and_extension_corruption_without_claiming() {
    for corruption in ["hash", "scope", "server", "extension", "blob", "oversized"] {
        let (mut control, _, input, authority) = fixture();
        let job = control
            .create_git_evidence_job(&input, &authority, 8)
            .unwrap()
            .unwrap();
        control
            .connection
            .execute_batch(
                "DROP TRIGGER git_job_input_immutable_update; PRAGMA ignore_check_constraints=ON;",
            )
            .unwrap();
        let mut value = serde_json::to_value(&input).unwrap();
        match corruption {
            "scope" => value["scope_id"] = serde_json::json!("other-scope"),
            "server" => value["server_id"] = serde_json::json!("other-server"),
            "extension" => value["authority"] = serde_json::json!({"trusted_local":true}),
            "oversized" => value["unused"] = serde_json::json!("x".repeat(2 << 20)),
            _ => {}
        }
        let encoded = serde_json::to_string(&value).unwrap();
        let digest = if corruption == "hash" {
            "0".repeat(64)
        } else {
            serde_json::from_value::<diskgraph_core::GitEvidenceJobInput>(value)
                .map(|value| value.digest())
                .unwrap_or_else(|_| input.digest())
        };
        if corruption == "blob" {
            control.connection.execute("UPDATE git_evidence_job_inputs SET input_json=?2,input_sha256=?3 WHERE job_id=?1",params![job.job_id,encoded.as_bytes(),digest]).unwrap();
        } else {
            control.connection.execute("UPDATE git_evidence_job_inputs SET input_json=?2,input_sha256=?3 WHERE job_id=?1",params![job.job_id,encoded,digest]).unwrap();
        }
        assert!(
            matches!(
                control.git_evidence_job_input(&job.job_id),
                Err(StoreError::InvalidGraph(_))
            ),
            "{corruption}"
        );
        assert!(
            control.claim_job_once_strict(&job.job_id, "owner").is_err(),
            "{corruption}"
        );
        assert_eq!(
            control.job(&job.job_id).unwrap().state,
            JobState::Failed,
            "{corruption}"
        );
    }
}
