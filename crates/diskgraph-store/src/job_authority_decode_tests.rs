use crate::job_authority_tests::{configure, principal};
use crate::{ControlStore, JobKind, StoreError};
use diskgraph_core::{JobRequestAuthority, Permission};

#[test]
fn corrupt_version_oversized_json_and_secret_fields_never_fall_back_to_legacy() {
    for variant in ["version", "oversize", "secret", "json"] {
        let mut store = ControlStore::open_in_memory().unwrap();
        let scope = configure(&mut store);
        let job = store
            .create_job(&scope, JobKind::Index, &principal())
            .unwrap();
        let original = JobRequestAuthority::authenticated_remote(
            principal(),
            "issuer",
            "http",
            vec![Permission::IndexWrite],
            u64::MAX,
        )
        .unwrap();
        let mut value = serde_json::to_value(original).unwrap();
        let (version, json) = match variant {
            "version" => (2, serde_json::to_string(&value).unwrap()),
            "oversize" => (1, "x".repeat(17000)),
            "secret" => {
                value["bearer"] = serde_json::json!("should-never-be-persisted");
                (1, serde_json::to_string(&value).unwrap())
            }
            _ => (1, "not-json".into()),
        };
        // 仅隔离夹具模拟损坏数据库；生产 CHECK 仍强制版本和长度。
        store
            .connection
            .pragma_update(None, "ignore_check_constraints", true)
            .unwrap();
        store
            .connection
            .execute(
                "INSERT INTO job_request_authorities VALUES(?1,?2,?3)",
                rusqlite::params![job.job_id, version, json],
            )
            .unwrap();
        assert!(
            matches!(
                store.job_request_authority(&job.job_id),
                Err(StoreError::InvalidGraph(_))
            ),
            "{variant}"
        );
        assert!(
            matches!(
                store.claim_job_once(&job.job_id, "trusted"),
                Err(StoreError::InvalidGraph(_))
            ),
            "{variant}"
        );
    }
}
