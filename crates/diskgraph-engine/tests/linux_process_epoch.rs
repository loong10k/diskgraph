//! D42 原生 Linux 扫描身份验收；来源：真实 Engine 索引与内核 opaque handle。
//! 必须在具备 /dev/shm、STATX_MNT_ID_UNIQUE 和文件句柄能力的 Linux lane 执行。
//! 预检失败是环境未验收，直接失败；不能 skip 后报告支持，也不修改旧快照身份。
#![cfg(target_os = "linux")]

use diskgraph_core::{IndexedFileEpoch, PrincipalId, QueryBudget, QueryReadBudget, ScopeId};
use diskgraph_engine::{Engine, EngineConfig};
use diskgraph_store::SqliteSnapshotStore;
use std::fs::{File, OpenOptions};
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// 隔离 tmpfs 源和数据库；来源：Linux 原生公开扫描，不伪造 SQL 身份或挂载。
struct EpochFixture {
    _workspace: tempfile::TempDir,
    _database: tempfile::TempDir,
    root: PathBuf,
    database: PathBuf,
    engine: Engine,
    actor: PrincipalId,
    scope: ScopeId,
}

impl EpochFixture {
    fn new() -> Self {
        let workspace = tempfile::tempdir_in("/dev/shm")
            .expect("native Linux lane requires a writable tmpfs; no silent unsupported pass");
        let database = tempfile::tempdir().unwrap();
        let root = workspace.path().join("source");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("file"), b"fixture").unwrap();
        std::fs::hard_link(root.join("file"), root.join("alias")).unwrap();
        preflight(&root.join("file"));
        let engine = Engine::open(EngineConfig {
            data_dir: database.path().to_owned(),
            ..EngineConfig::default()
        })
        .unwrap();
        let actor = PrincipalId::new("linux-epoch-fixture").unwrap();
        engine.bootstrap_local_admin(&actor).unwrap();
        let scope = engine
            .register_scope(&root, &actor, &engine.policy_authorizer().unwrap())
            .unwrap();
        Self {
            database: database.path().join("diskgraph.sqlite"),
            _workspace: workspace,
            _database: database,
            root,
            engine,
            actor,
            scope,
        }
    }

    fn scan(&self) -> String {
        let job = self
            .engine
            .index_scope(
                &self.scope,
                &self.actor,
                &self.engine.policy_authorizer().unwrap(),
            )
            .unwrap();
        self.engine
            .run_job(&job.job_id, "native-epoch-worker")
            .unwrap();
        self.engine
            .revision_for_job(
                &job.job_id,
                &self.actor,
                &self.engine.policy_authorizer().unwrap(),
            )
            .unwrap()
    }

    fn epoch(&self, revision: &str, name: &str) -> IndexedFileEpoch {
        let node = self
            .engine
            .revision_node_at(revision, Path::new(name))
            .unwrap()
            .unwrap();
        let snapshot = self.engine.revision_snapshot(revision).unwrap();
        let store = SqliteSnapshotStore::open(&self.database).unwrap();
        let mut reads = QueryReadBudget::new(
            QueryBudget::default(),
            Instant::now() + Duration::from_secs(5),
        )
        .unwrap();
        let record = store
            .unix_observation_bounded(&snapshot.id, node.id, &mut reads)
            .unwrap()
            .unwrap();
        assert!(
            record.gap.is_none(),
            "supported native scan must not lose epoch: {:?}",
            record.gap
        );
        let observation = record
            .observation
            .expect("a fresh supported scan must persist its epoch");
        assert_eq!(
            observation.length(),
            std::fs::metadata(self.root.join(name)).unwrap().len()
        );
        observation.validate().unwrap();
        assert!(matches!(
            observation.epoch(),
            IndexedFileEpoch::LinuxHandle { .. }
        ));
        observation.epoch().clone()
    }
}

// 只确认夹具具备真实原生能力；不制造待测试的持久扫描记录。
fn preflight(path: &Path) {
    let file: File = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_PATH | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .unwrap();
    let mut raw = [0_u64; 17];
    let handle = raw.as_mut_ptr().cast::<libc::file_handle>();
    // 对齐的136字节缓冲包含8字节头和128字节原生不透明句柄。
    unsafe {
        (*handle).handle_bytes = 128;
    }
    let mut mount = 0;
    let result = unsafe {
        libc::name_to_handle_at(
            file.as_raw_fd(),
            c"".as_ptr(),
            handle,
            &mut mount,
            libc::AT_EMPTY_PATH,
        )
    };
    assert_eq!(
        result,
        0,
        "native fixture unsupported, not a product RED: {}",
        std::io::Error::last_os_error()
    );
    let mut stat: libc::statx = unsafe { std::mem::zeroed() };
    // Linux UAPI 6.8 唯一挂载身份位，必须验证返回 mask；不能使用可复用普通 mount id。
    let unique_mount = 0x4000;
    let result = unsafe {
        libc::statx(
            file.as_raw_fd(),
            c"".as_ptr(),
            libc::AT_EMPTY_PATH,
            unique_mount,
            &mut stat,
        )
    };
    assert_eq!(result, 0, "native unique-mount precondition");
    assert_ne!(
        stat.stx_mask & unique_mount,
        0,
        "native kernel has not provided unique mount ID"
    );
    assert_ne!(stat.stx_mnt_id, 0);
}

#[test]
fn native_index_persists_same_epoch_for_hardlinks_and_after_engine_reopen() {
    let fixture = EpochFixture::new();
    let revision = fixture.scan();
    let file = fixture.epoch(&revision, "file");
    assert_eq!(file, fixture.epoch(&revision, "alias"));
    let reopened = Engine::open(EngineConfig {
        data_dir: fixture.database.parent().unwrap().to_owned(),
        ..EngineConfig::default()
    })
    .unwrap();
    assert_eq!(
        reopened.latest_revision(&fixture.scope).unwrap().as_deref(),
        Some(revision.as_str())
    );
    assert_eq!(file, fixture.epoch(&revision, "file"));
}

#[test]
fn native_reindex_keeps_object_epoch_after_edit_but_replacement_gets_a_new_epoch() {
    let fixture = EpochFixture::new();
    let first_revision = fixture.scan();
    let first = fixture.epoch(&first_revision, "file");
    std::fs::write(fixture.root.join("file"), b"edited fixture").unwrap();
    let second_revision = fixture.scan();
    assert_eq!(first, fixture.epoch(&second_revision, "file"));
    std::fs::remove_file(fixture.root.join("file")).unwrap();
    std::fs::write(fixture.root.join("file"), b"new object").unwrap();
    let third_revision = fixture.scan();
    assert_ne!(first, fixture.epoch(&third_revision, "file"));
    assert_eq!(first, fixture.epoch(&third_revision, "alias"));
}
