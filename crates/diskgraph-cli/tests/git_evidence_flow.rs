//! D37 C03 公共进程入口回归；来源：真实 CLI、原生 Git 和隔离数据目录。
//! 入队契约与旧 sync 正控独立于后续 collector 执行/原子发布验收。

#[cfg(any(target_os = "linux", target_os = "macos", windows))]
#[path = "../../diskgraph-engine/tests/support/native_scan_engine.rs"]
mod native_scan_engine;

#[cfg(test)]
mod git_wait_diagnostic;

use diskgraph_core::{PrincipalId, ScopeId};
use diskgraph_engine::{Engine, EngineConfig};
use serde_json::Value;
use std::path::Path;
use std::process::{Command, Output};

/// 从真实 Git 工作树经公开扫描生成节点的 CLI 夹具。
/// 来源：DiskGraph 原生 Rust 集成测试，无 Java 对应实现。
struct GitFixture {
    temp: tempfile::TempDir,
    scope: String,
    revision: String,
    node_id: String,
}

impl GitFixture {
    fn new(content_read: bool) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("repo");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(temp.path().join("empty-config"), "").unwrap();
        git(&root, temp.path(), &["init", "-q"]);
        std::fs::write(root.join("tracked.txt"), "original\n").unwrap();
        git(&root, temp.path(), &["add", "tracked.txt"]);
        git(
            &root,
            temp.path(),
            &[
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "commit",
                "-qm",
                "fixture",
            ],
        );
        std::fs::write(root.join("tracked.txt"), "dirty\n").unwrap();
        let data = temp.path().join("data");
        let scope_response = success(run_cli(
            &data,
            &["scope", "add", "--root", root.to_str().unwrap()],
        ));
        let scope = scope_response["data"]["scope_id"]
            .as_str()
            .unwrap()
            .to_owned();
        let indexed = success(run_cli(&data, &["index", "--scope", &scope, "--wait"]));
        let revision = indexed["data"]["revision_id"].as_str().unwrap().to_owned();
        let engine = Engine::open(EngineConfig {
            data_dir: data,
            ..EngineConfig::default()
        })
        .unwrap();
        let principal = PrincipalId::new("local-user").unwrap();
        engine
            .set_content_read(
                &ScopeId::new(scope.clone()).unwrap(),
                &principal,
                content_read,
            )
            .unwrap();
        let reader = engine.revision_reader().unwrap();
        let snapshot = reader.revision(&revision).unwrap().snapshot_id;
        let node_id = reader.load(&snapshot).unwrap().root().id.to_string();
        Self {
            temp,
            scope,
            revision,
            node_id,
        }
    }

    fn collect(&self) -> Output {
        run_cli(
            &self.temp.path().join("data"),
            &[
                "sync",
                "--scope",
                &self.scope,
                "--revision",
                &self.revision,
                "--node-id",
                &self.node_id,
                "--collector",
                "git",
            ],
        )
    }

    fn collect_wait(&self) -> Output {
        run_cli(
            &self.temp.path().join("data"),
            &[
                "sync",
                "--scope",
                &self.scope,
                "--revision",
                &self.revision,
                "--node-id",
                &self.node_id,
                "--collector",
                "git",
                "--wait",
            ],
        )
    }

    fn job_count(&self) -> i64 {
        rusqlite::Connection::open(self.temp.path().join("data/diskgraph-control.sqlite"))
            .unwrap()
            .query_row("SELECT COUNT(*) FROM jobs", [], |row| row.get(0))
            .unwrap()
    }
}

fn run_cli(data_dir: &Path, arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_diskgraph"))
        .arg("--data-dir")
        .arg(data_dir)
        .arg("--json")
        .args(arguments)
        .env_remove("RUST_BACKTRACE")
        .output()
        .unwrap()
}

fn success(output: Output) -> Value {
    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["ok"], true, "{value}");
    value
}

fn git(root: &Path, home: &Path, args: &[&str]) {
    let mut command = Command::new("git");
    command.env_clear();
    for key in ["PATH", "SystemRoot"] {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    let output = command
        .current_dir(root)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", home.join("empty-config"))
        .env("GIT_TERMINAL_PROMPT", "0")
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "Git fixture {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn explicit_git_sync_queues_a_durable_job_without_publishing_during_enqueue() {
    let fixture = GitFixture::new(true);
    let count = fixture.job_count();
    let response = success(fixture.collect());
    assert_eq!(response["data"]["state"], "queued");
    let job_id = response["data"]["job_id"].as_str().unwrap();
    assert_eq!(fixture.job_count(), count + 1);
    // CLI 已退出后重开实际控制库，而不是仅相信进程 stdout。
    let engine = Engine::open(EngineConfig {
        data_dir: fixture.temp.path().join("data"),
        ..EngineConfig::default()
    })
    .unwrap();
    let job = engine.job_status(job_id).unwrap();
    assert_eq!(job.scope_id.as_str(), fixture.scope);
    assert_eq!(job.principal.as_str(), "local-user");
    assert_eq!(serde_json::to_value(job.kind).unwrap(), "git_evidence");
    assert_eq!(serde_json::to_value(job.state).unwrap(), "queued");
    assert_eq!(
        engine.latest_revision(&job.scope_id).unwrap().as_deref(),
        Some(fixture.revision.as_str())
    );
}

#[test]
fn explicit_git_sync_without_content_read_is_business_denied_before_enqueue() {
    let fixture = GitFixture::new(false);
    let count = fixture.job_count();
    let output = fixture.collect();
    assert_eq!(
        output.status.code(),
        Some(3),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let response: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(response["error"]["code"], "permission_denied");
    assert_eq!(fixture.job_count(), count);
}

#[test]
fn explicit_git_sync_wait_uses_the_persisted_publication_revision() {
    let fixture = GitFixture::new(true);
    let output = fixture.collect_wait();
    if !output.status.success() {
        eprintln!(
            "GIT_WAIT_POSTERIOR_DIAGNOSTIC {}",
            git_wait_diagnostic::read(&fixture)
        );
    }
    let response = success(output);
    assert_eq!(response["data"]["state"], "completed");
    let revision = response["data"]["revision_id"].as_str().unwrap();
    assert_ne!(revision, fixture.revision);
    let job_id = response["data"]["job_id"].as_str().unwrap();
    // 从进程结束后重开的真实库读终态和发布定位，不接受仅 stdout 合成的 revision。
    let engine = Engine::open(EngineConfig {
        data_dir: fixture.temp.path().join("data"),
        ..EngineConfig::default()
    })
    .unwrap();
    let job = engine.job_status(job_id).unwrap();
    assert_eq!(serde_json::to_value(job.kind).unwrap(), "git_evidence");
    assert_eq!(serde_json::to_value(job.state).unwrap(), "completed");
    assert_eq!(job.principal.as_str(), "local-user");
    assert_eq!(job.scope_id.as_str(), fixture.scope);
    let authority = engine
        .control_store()
        .unwrap()
        .job_request_authority(job_id)
        .unwrap()
        .unwrap();
    assert_eq!(authority.principal().as_str(), "local-user");
    assert_eq!(authority.transport(), "cli");
    assert_eq!(
        serde_json::to_value(authority.origin()).unwrap(),
        "trusted_local"
    );
    assert_eq!(
        engine
            .revision_for_job(
                job_id,
                &PrincipalId::new("local-user").unwrap(),
                &engine.policy_authorizer().unwrap(),
            )
            .unwrap(),
        revision
    );
    assert_eq!(
        engine.latest_revision(&job.scope_id).unwrap().as_deref(),
        Some(revision)
    );
    let reader = engine.revision_reader().unwrap();
    let receipt = reader.job_publication_receipt(job_id).unwrap().unwrap();
    assert_eq!(receipt.job_id(), job_id);
    assert_eq!(receipt.base_revision_id(), fixture.revision);
    assert_eq!(receipt.revision_id(), revision);
    assert_eq!(receipt.scope_id().as_str(), fixture.scope);
    assert_eq!(receipt.server_id(), &engine.server_id().unwrap());
    assert_eq!(receipt.input().node_id().to_string(), fixture.node_id);
    assert_eq!(receipt.publishing_fence(), job.fencing_token);
    assert!(!receipt.run_id().is_empty());
    let status = success(run_cli(
        &fixture.temp.path().join("data"),
        &["status", "--job", job_id],
    ));
    assert_eq!(status["data"]["state"], "completed");
    assert_eq!(status["data"]["revision_id"], receipt.revision_id());
    assert_eq!(status["data"]["run_id"], receipt.run_id());
    assert!(
        reader.revision(&fixture.revision).is_ok(),
        "原 revision 必须保留"
    );
    assert!(reader.revision(revision).is_ok());
}

#[test]
fn repeated_git_sync_reports_the_actual_running_merged_job() {
    let fixture = GitFixture::new(true);
    let first = success(fixture.collect());
    let job_id = first["data"]["job_id"].as_str().unwrap();
    let count = fixture.job_count();
    let engine = Engine::open(EngineConfig {
        data_dir: fixture.temp.path().join("data"),
        ..EngineConfig::default()
    })
    .unwrap();
    // 公开认领入口验证真实 typed authority 与 scope grants，不伪造 jobs SQL 状态。
    let claimed = engine
        .control_store()
        .unwrap()
        .claim_job_once(job_id, "existing-git-owner")
        .unwrap();
    assert_eq!(serde_json::to_value(claimed.state).unwrap(), "running");
    assert_eq!(serde_json::to_value(claimed.kind).unwrap(), "git_evidence");
    assert_eq!(claimed.owner, "existing-git-owner");
    assert!(claimed.fencing_token > 0);

    let repeated = success(fixture.collect());
    assert_eq!(repeated["data"]["job_id"], job_id);
    assert_eq!(fixture.job_count(), count, "相同目标应合并已有任务");
    let durable = engine.job_status(job_id).unwrap();
    assert_eq!(serde_json::to_value(durable.state).unwrap(), "running");
    assert_eq!(durable.owner, "existing-git-owner");
    assert_eq!(durable.fencing_token, claimed.fencing_token);
    assert_eq!(
        repeated["data"]["state"], "running",
        "合并响应不得把真实运行任务报告为排队；response={repeated}"
    );
}

#[test]
fn reopened_cli_status_reports_only_the_persisted_safe_git_failure() {
    let fixture = GitFixture::new(true);
    let queued = success(fixture.collect());
    let job_id = queued["data"]["job_id"].as_str().unwrap();
    // 真实不支持的仓库结构在执行阶段失败，不伪造控制库错误行或工具输出。
    // alternates 的存在应在解析正文前拒绝，正文标记也不应进入状态诊断。
    let secret_marker = "private-source-marker-must-not-appear";
    std::fs::write(
        fixture
            .temp
            .path()
            .join("repo/.git/objects/info/alternates"),
        secret_marker,
    )
    .unwrap();
    let engine = Engine::open(EngineConfig {
        data_dir: fixture.temp.path().join("data"),
        ..EngineConfig::default()
    })
    .unwrap();
    let error = engine.run_job(job_id, "git-failure-owner").unwrap_err();
    assert!(
        matches!(
            error,
            diskgraph_engine::EngineError::Business(diskgraph_core::BusinessError::Unsupported)
        ),
        "expected actual unsupported Git execution, got {error:?}"
    );
    let job = engine.job_status(job_id).unwrap();
    assert_eq!(serde_json::to_value(job.state).unwrap(), "failed");
    assert_eq!(
        engine.latest_revision(&job.scope_id).unwrap().as_deref(),
        Some(fixture.revision.as_str())
    );
    drop(engine);
    let status = success(run_cli(
        &fixture.temp.path().join("data"),
        &["status", "--job", job_id],
    ));
    assert_eq!(status["data"]["state"], "failed");
    assert_eq!(
        status["data"]["failure"],
        serde_json::json!({ "phase": "execution", "code": "unsupported" })
    );
    assert!(status["data"]["revision_id"].is_null());
    assert!(status["data"]["run_id"].is_null());
    assert!(!status.to_string().contains(secret_marker));
}

#[test]
fn cli_git_status_denies_a_principal_without_operation_view_on_the_actual_scope() {
    let fixture = GitFixture::new(true);
    let queued = success(fixture.collect());
    let job_id = queued["data"]["job_id"].as_str().unwrap();
    let output = run_cli(
        &fixture.temp.path().join("data"),
        &["--principal", "no-scope-grants", "status", "--job", job_id],
    );
    assert_eq!(
        output.status.code(),
        Some(3),
        "unprivileged status stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let response: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(response["error"]["code"], "permission_denied");
    assert_ne!(response["ok"], true);
    assert!(response["data"].is_null());
}

#[test]
fn sync_without_collector_still_scans_and_waits_without_content_read() {
    let fixture = GitFixture::new(false);
    let response = success(run_cli(
        &fixture.temp.path().join("data"),
        &["sync", "--scope", &fixture.scope, "--wait"],
    ));
    assert_eq!(response["data"]["state"], "completed");
    let revision = response["data"]["revision_id"].as_str().unwrap();
    assert_ne!(revision, fixture.revision);
    let engine = Engine::open(EngineConfig {
        data_dir: fixture.temp.path().join("data"),
        ..EngineConfig::default()
    })
    .unwrap();
    let job = engine
        .job_status(response["data"]["job_id"].as_str().unwrap())
        .unwrap();
    assert_eq!(serde_json::to_value(job.kind).unwrap(), "sync");
    assert_eq!(
        engine.latest_revision(&job.scope_id).unwrap().as_deref(),
        Some(revision)
    );
}
