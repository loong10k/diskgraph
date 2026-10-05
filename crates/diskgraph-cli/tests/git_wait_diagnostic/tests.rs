//! 使用真实公开入队/执行/认领验证同一诊断实现；来源：原生 Rust CLI 集成测试。

use super::read;
use crate::{GitFixture, success};
use diskgraph_core::BusinessError;
use diskgraph_engine::{Engine, EngineConfig, EngineError};
use serde_json::json;

#[test]
fn posterior_without_git_job_reports_absence_and_the_real_base_latest() {
    let fixture = GitFixture::new(true);
    let count = fixture.job_count();
    let diagnostic = read(&fixture);
    assert_eq!(diagnostic["read_status"], "ok", "{diagnostic}");
    assert_eq!(diagnostic["observation"], "posterior_non_atomic");
    assert_eq!(diagnostic["git_job_exists"], false);
    for field in [
        "state",
        "fence",
        "typed_input_exists",
        "safe_failure",
        "receipt_exists",
    ] {
        assert!(diagnostic[field].is_null(), "{field}: {diagnostic}");
    }
    assert_eq!(diagnostic["latest_exists"], true);
    assert_eq!(diagnostic["latest_matches_base"], true);
    assert_eq!(fixture.job_count(), count);
}

#[test]
fn posterior_queued_and_running_reads_the_actual_claimed_fence() {
    let fixture = GitFixture::new(true);
    let queued = success(fixture.collect());
    let job_id = queued["data"]["job_id"].as_str().unwrap();
    let count = fixture.job_count();
    let diagnostic = read(&fixture);
    assert_eq!(diagnostic["read_status"], "ok", "{diagnostic}");
    assert_eq!(diagnostic["git_job_exists"], true);
    assert_eq!(diagnostic["state"], "queued");
    assert_eq!(diagnostic["fence"], 0);
    assert_eq!(diagnostic["typed_input_exists"], true);
    assert!(diagnostic["safe_failure"].is_null());
    assert_eq!(diagnostic["receipt_exists"], false);
    assert_eq!(diagnostic["latest_matches_base"], true);
    let engine = Engine::open(EngineConfig {
        data_dir: fixture.temp.path().join("data"),
        ..EngineConfig::default()
    })
    .unwrap();
    let claimed = engine
        .control_store()
        .unwrap()
        .claim_job_once(job_id, "diagnostic-actual-owner")
        .unwrap();
    assert!(claimed.fencing_token > 0);
    drop(engine);
    let diagnostic = read(&fixture);
    assert_eq!(diagnostic["read_status"], "ok", "{diagnostic}");
    assert_eq!(diagnostic["state"], "running");
    assert_eq!(diagnostic["fence"], claimed.fencing_token);
    assert_eq!(diagnostic["typed_input_exists"], true);
    assert!(diagnostic["safe_failure"].is_null());
    assert_eq!(diagnostic["receipt_exists"], false);
    assert_eq!(diagnostic["latest_matches_base"], true);
    assert_eq!(fixture.job_count(), count);
}

#[test]
fn posterior_completed_reads_the_real_receipt_and_new_owner_latest() {
    let fixture = GitFixture::new(true);
    let queued = success(fixture.collect());
    let job_id = queued["data"]["job_id"].as_str().unwrap();
    let engine = Engine::open(EngineConfig {
        data_dir: fixture.temp.path().join("data"),
        ..EngineConfig::default()
    })
    .unwrap();
    let completed = engine
        .run_job(job_id, "diagnostic-publication-owner")
        .unwrap();
    assert_eq!(serde_json::to_value(completed.state).unwrap(), "completed");
    let receipt = engine
        .revision_reader()
        .unwrap()
        .job_publication_receipt(job_id)
        .unwrap()
        .unwrap();
    assert_ne!(receipt.revision_id(), fixture.revision);
    assert_eq!(
        engine
            .latest_revision(&completed.scope_id)
            .unwrap()
            .as_deref(),
        Some(receipt.revision_id())
    );
    drop(engine);
    let count = fixture.job_count();
    let diagnostic = read(&fixture);
    assert_eq!(diagnostic["read_status"], "ok", "{diagnostic}");
    assert_eq!(diagnostic["git_job_exists"], true);
    assert_eq!(diagnostic["state"], "completed");
    assert_eq!(diagnostic["fence"], completed.fencing_token);
    assert_eq!(diagnostic["typed_input_exists"], true);
    assert!(diagnostic["safe_failure"].is_null());
    assert_eq!(diagnostic["receipt_exists"], true);
    assert_eq!(diagnostic["latest_exists"], true);
    assert_eq!(diagnostic["latest_matches_base"], false);
    assert_eq!(fixture.job_count(), count);
}

#[test]
fn posterior_failed_reports_only_the_real_persisted_safe_failure() {
    let fixture = GitFixture::new(true);
    let queued = success(fixture.collect());
    let job_id = queued["data"]["job_id"].as_str().unwrap();
    // 与既有不支持仓库回归相同的真实结构，不写任务/回执 SQL 或仿造错误。
    let secret = "diagnostic-source-marker-must-not-appear";
    std::fs::write(
        fixture
            .temp
            .path()
            .join("repo/.git/objects/info/alternates"),
        secret,
    )
    .unwrap();
    let engine = Engine::open(EngineConfig {
        data_dir: fixture.temp.path().join("data"),
        ..EngineConfig::default()
    })
    .unwrap();
    let error = engine
        .run_job(job_id, "diagnostic-failure-owner")
        .unwrap_err();
    assert!(
        matches!(error, EngineError::Business(BusinessError::Unsupported)),
        "{error:?}"
    );
    let failed = engine.job_status(job_id).unwrap();
    assert_eq!(serde_json::to_value(failed.state).unwrap(), "failed");
    drop(engine);
    let count = fixture.job_count();
    let diagnostic = read(&fixture);
    assert_eq!(diagnostic["read_status"], "ok", "{diagnostic}");
    assert_eq!(diagnostic["git_job_exists"], true);
    assert_eq!(diagnostic["state"], "failed");
    assert_eq!(diagnostic["fence"], failed.fencing_token);
    assert_eq!(diagnostic["typed_input_exists"], true);
    assert_eq!(
        diagnostic["safe_failure"],
        json!({"phase":"execution","code":"unsupported"})
    );
    assert_eq!(diagnostic["receipt_exists"], false);
    assert_eq!(diagnostic["latest_exists"], true);
    assert_eq!(diagnostic["latest_matches_base"], true);
    assert!(!diagnostic.to_string().contains(secret));
    assert!(
        !diagnostic
            .to_string()
            .contains(fixture.temp.path().to_str().unwrap())
    );
    assert_eq!(fixture.job_count(), count);
}
