//! D42 C03 真实 CLI 入队回归；来源：原生发布进程、公开扫描与隔离 SQLite。
//! 首轮只验证固定进程采集任务入口；原生占用、执行和发布由后续验收补齐。

use diskgraph_core::{PrincipalId, ScopeId};
use diskgraph_engine::{Engine, EngineConfig};
use serde_json::Value;
use std::path::Path;
use std::process::{Command, Output};

/// 真实普通文件与 Git 工作树共存的公开扫描夹具。
/// 来源：DiskGraph 原生 Rust 集成测试；不伪造节点或任务记录。
struct ProcessFixture {
    temp: tempfile::TempDir,
    scope: String,
    revision: String,
    file_nodes: [u64; 2],
}

impl ProcessFixture {
    fn new() -> Self {
        // Linux 正控使用实际可提供 opaque handle 的 tmpfs；其它平台不伪造 epoch。
        #[cfg(target_os = "linux")]
        let temp = tempfile::tempdir_in("/dev/shm").expect("native tmpfs acceptance fixture");
        #[cfg(not(target_os = "linux"))]
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("root");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(temp.path().join("empty-config"), b"").unwrap();
        let mut git = Command::new("git");
        git.env_clear();
        for key in ["PATH", "SystemRoot"] {
            if let Some(value) = std::env::var_os(key) {
                git.env(key, value);
            }
        }
        let initialized = git
            .args(["init", "-q"])
            .env("HOME", temp.path())
            .env("USERPROFILE", temp.path())
            .current_dir(&root)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", temp.path().join("empty-config"))
            .env("GIT_TERMINAL_PROMPT", "0")
            .output()
            .unwrap();
        assert!(initialized.status.success(), "{initialized:?}");
        for name in ["first.txt", "second.txt"] {
            std::fs::write(root.join(name), b"ordinary fixture bytes\n").unwrap();
        }
        let data = temp.path().join("data");
        let registered = success(run(
            &data,
            &["scope", "add", "--root", root.to_str().unwrap()],
        ));
        let scope = registered["data"]["scope_id"].as_str().unwrap().to_owned();
        let indexed = success(run(&data, &["index", "--scope", &scope, "--wait"]));
        let revision = indexed["data"]["revision_id"].as_str().unwrap().to_owned();
        let engine = Engine::open(EngineConfig {
            data_dir: data,
            ..EngineConfig::default()
        })
        .unwrap();
        let reader = engine.revision_reader().unwrap();
        let graph = reader
            .load(&reader.revision(&revision).unwrap().snapshot_id)
            .unwrap();
        let file_nodes = ["first.txt", "second.txt"].map(|name| {
            graph
                .nodes
                .iter()
                .find(|node| node.name == name)
                .unwrap()
                .id
        });
        Self {
            temp,
            scope,
            revision,
            file_nodes,
        }
    }

    fn engine(&self) -> Engine {
        Engine::open(EngineConfig {
            data_dir: self.temp.path().join("data"),
            ..EngineConfig::default()
        })
        .unwrap()
    }

    fn collect(&self, revision: &str, node: u64, collector: &str) -> Output {
        run(
            &self.temp.path().join("data"),
            &[
                "sync",
                "--scope",
                &self.scope,
                "--revision",
                revision,
                "--node-id",
                &node.to_string(),
                "--collector",
                collector,
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

fn run(data: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_diskgraph"))
        .arg("--data-dir")
        .arg(data)
        .arg("--json")
        .args(args)
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
    let response: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(response["ok"], true, "{response}");
    response
}

#[test]
#[cfg(target_os = "linux")]
fn metadata_only_process_sync_persists_a_distinct_fixed_target_job_across_reopen() {
    let fixture = ProcessFixture::new();
    let count = fixture.job_count();
    // 不授予 ContentRead；进程元数据入口不能要求读取正文。
    let first = success(fixture.collect(&fixture.revision, fixture.file_nodes[0], "process"));
    assert_eq!(first["data"]["state"], "queued");
    let job_id = first["data"]["job_id"].as_str().unwrap();
    let reopened = fixture.engine();
    let job = reopened.job_status(job_id).unwrap();
    assert_eq!(job.scope_id.as_str(), fixture.scope);
    assert_eq!(job.principal.as_str(), "local-user");
    assert_eq!(serde_json::to_value(job.kind).unwrap(), "process_evidence");
    assert_eq!(serde_json::to_value(job.state).unwrap(), "queued");
    assert_eq!(fixture.job_count(), count + 1);
    assert_eq!(
        reopened.latest_revision(&job.scope_id).unwrap().as_deref(),
        Some(fixture.revision.as_str())
    );
    let status = success(run(
        &fixture.temp.path().join("data"),
        &["status", "--job", job_id],
    ));
    assert_eq!(status["data"]["job_id"], job_id);
    assert_eq!(status["data"]["scope_id"], fixture.scope);
    assert_eq!(status["data"]["state"], "queued");
    let same = success(fixture.collect(&fixture.revision, fixture.file_nodes[0], "process"));
    assert_eq!(same["data"]["job_id"], job_id);
    let other = success(fixture.collect(&fixture.revision, fixture.file_nodes[1], "process"));
    assert_ne!(
        other["data"]["job_id"], job_id,
        "两个真实文件不能合并为同一固定目标"
    );
    assert_eq!(fixture.job_count(), count + 2);
    // 同范围产生新扫描 revision 后，请求指纹不能把新基线合并到旧任务。
    let rescanned = success(run(
        &fixture.temp.path().join("data"),
        &["sync", "--scope", &fixture.scope, "--wait"],
    ));
    let new_revision = rescanned["data"]["revision_id"].as_str().unwrap();
    assert_ne!(new_revision, fixture.revision);
    let reader = reopened.revision_reader().unwrap();
    let graph = reader
        .load(&reader.revision(new_revision).unwrap().snapshot_id)
        .unwrap();
    let new_node = graph
        .nodes
        .iter()
        .find(|node| node.name == "first.txt")
        .unwrap()
        .id;
    let new_target = success(fixture.collect(new_revision, new_node, "process"));
    assert_ne!(new_target["data"]["job_id"], job_id);
    assert_eq!(fixture.job_count(), count + 4);
    assert_eq!(
        reopened.latest_revision(&job.scope_id).unwrap().as_deref(),
        Some(new_revision)
    );
}

#[test]
#[cfg(not(target_os = "linux"))]
fn unqualified_native_epoch_is_unsupported_and_does_not_enqueue_or_rescan() {
    let fixture = ProcessFixture::new();
    let count = fixture.job_count();
    let output = fixture.collect(&fixture.revision, fixture.file_nodes[0], "process");
    assert!(
        !output.status.success(),
        "unqualified native backend must refuse"
    );
    let response: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(response["ok"], false, "{response}");
    assert_eq!(response["error"]["code"], "unsupported", "{response}");
    assert_eq!(fixture.job_count(), count);
    let engine = fixture.engine();
    let scope = ScopeId::new(fixture.scope.clone()).unwrap();
    assert_eq!(
        engine.latest_revision(&scope).unwrap().as_deref(),
        Some(fixture.revision.as_str())
    );
}

#[test]
fn existing_scan_and_git_controls_still_work_and_unknown_collectors_do_not_enqueue() {
    let fixture = ProcessFixture::new();
    let count = fixture.job_count();
    let unknown = fixture.collect(
        &fixture.revision,
        fixture.file_nodes[0],
        "unregistered-collector",
    );
    assert_eq!(unknown.status.code(), Some(2));
    assert_eq!(fixture.job_count(), count);
    let rescanned = success(run(
        &fixture.temp.path().join("data"),
        &["sync", "--scope", &fixture.scope, "--wait"],
    ));
    let engine = fixture.engine();
    let scan_job = engine
        .job_status(rescanned["data"]["job_id"].as_str().unwrap())
        .unwrap();
    assert_eq!(serde_json::to_value(scan_job.kind).unwrap(), "sync");
    assert_eq!(serde_json::to_value(scan_job.state).unwrap(), "completed");
    let revision = rescanned["data"]["revision_id"].as_str().unwrap();
    assert_ne!(revision, fixture.revision);
    engine
        .set_content_read(
            &ScopeId::new(fixture.scope.clone()).unwrap(),
            &PrincipalId::new("local-user").unwrap(),
            true,
        )
        .unwrap();
    let reader = engine.revision_reader().unwrap();
    let graph = reader
        .load(&reader.revision(revision).unwrap().snapshot_id)
        .unwrap();
    let git = success(fixture.collect(revision, graph.root().id, "git"));
    let git_job = engine
        .job_status(git["data"]["job_id"].as_str().unwrap())
        .unwrap();
    assert_eq!(serde_json::to_value(git_job.kind).unwrap(), "git_evidence");
    assert_eq!(serde_json::to_value(git_job.state).unwrap(), "queued");
    assert_eq!(
        engine
            .latest_revision(&git_job.scope_id)
            .unwrap()
            .as_deref(),
        Some(revision)
    );
}
