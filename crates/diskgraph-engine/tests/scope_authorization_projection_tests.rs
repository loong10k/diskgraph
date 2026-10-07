//! 整个 display 请求的实时范围授权窄读回归；来源：OpenSpec Q-08。
//! 合法注册与合成观测发布均在计量窗外，Rust requested 不代表 SQLite C、I/O 或 RSS。

use diskgraph_core::{
    Authorizer, BusinessError, Decision, DiskGraph, DiskNode, DiskSnapshot, Locator, NodeKind,
    Permission, PolicyAuthorizer, PrincipalId, QueryBudget, ResourceLocator, ScanCoverage,
    ScanSettings, ScopeId,
};
use diskgraph_engine::{Engine, EngineConfig, EngineError, RevisionDisplayCompletion, admin_scope};
use diskgraph_store::{ControlStore, SqliteSnapshotStore};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::{Cell, RefCell};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Instant;

const REVISION: &str = "scope-projection-import";
const FIELD_BYTES: usize = 2 << 20;
const RAW_BYTES: usize = 256 << 10;
static MEASURING: AtomicBool = AtomicBool::new(false);
static REQUESTED_BYTES: AtomicUsize = AtomicUsize::new(0);

/// 独占测试进程的透明 System 分配计量器；来源：原生 Rust Q-08 测试。
struct MeasuredAllocator;

// SAFETY: 完整保留 System 的布局、指针与尺寸，计数操作本身不分配。
unsafe impl GlobalAlloc for MeasuredAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() && MEASURING.load(Ordering::Relaxed) {
            REQUESTED_BYTES.fetch_add(layout.size(), Ordering::Relaxed);
        }
        pointer
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc_zeroed(layout) };
        if !pointer.is_null() && MEASURING.load(Ordering::Relaxed) {
            REQUESTED_BYTES.fetch_add(layout.size(), Ordering::Relaxed);
        }
        pointer
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        let pointer = unsafe { System.realloc(pointer, layout, size) };
        if !pointer.is_null() && MEASURING.load(Ordering::Relaxed) {
            REQUESTED_BYTES.fetch_add(size, Ordering::Relaxed);
        }
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: MeasuredAllocator = MeasuredAllocator;

/// 保留真实策略决定，在实际权限回调中观察或撤销独立控制连接。
/// 来源：原生 Rust Q-08 末段授权测试；它不替换 Store 的实时授权算法。
struct CallbackAuthorizer<'a, F> {
    policy: &'a PolicyAuthorizer,
    calls: Cell<usize>,
    allowed: Cell<usize>,
    callback: F,
}

impl<F: Fn(usize)> Authorizer for CallbackAuthorizer<'_, F> {
    fn policy_version(&self) -> u64 {
        self.policy.policy_version()
    }

    fn decide(
        &self,
        principal: &PrincipalId,
        permission: &Permission,
        scope: &ScopeId,
    ) -> Decision {
        let call = self.calls.get() + 1;
        self.calls.set(call);
        (self.callback)(call);
        match self.policy.decide(principal, permission, scope) {
            Decision::Allowed => {
                self.allowed.set(self.allowed.get() + 1);
                Decision::Allowed
            }
            other => other,
        }
    }
}

fn budget() -> QueryBudget {
    QueryBudget {
        max_nodes: 2048,
        max_response_bytes: RAW_BYTES,
        deadline_ms: 50,
        ..QueryBudget::default()
    }
}

type Fixture = (
    tempfile::TempDir,
    Engine,
    PrincipalId,
    ScopeId,
    PolicyAuthorizer,
);

// 仅公开注册/暂存/发布，不修改 SQL 行，不调用平台扫描器冒充原生扫描证明。
fn fixture(field_bytes: usize) -> Fixture {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("root");
    std::fs::create_dir(&root).unwrap();
    let root = root.canonicalize().unwrap();
    let engine = Engine::open(EngineConfig {
        data_dir: directory.path().join("data"),
        ..EngineConfig::default()
    })
    .unwrap();
    let principal = PrincipalId::new("scope-projection-reader").unwrap();
    engine.bootstrap_local_admin(&principal).unwrap();
    let mut locator = Locator::from_native_path(&root);
    if field_bytes != 0 {
        locator.display = "d".repeat(field_bytes);
    }
    let volume = (field_bytes != 0).then(|| "v".repeat(field_bytes));
    let scope = engine
        .control_store()
        .unwrap()
        .register_scope(&locator, volume.as_deref())
        .unwrap();
    assert_eq!(
        engine
            .register_scope(&root, &principal, &engine.policy_authorizer().unwrap())
            .unwrap(),
        scope
    );
    {
        let mut control = engine.control_store().unwrap();
        control
            .revoke_grant(&principal, &Permission::MetadataRead, &admin_scope())
            .unwrap();
        assert_eq!(
            control
                .live_permission(&principal, &Permission::MetadataRead, &scope)
                .unwrap(),
            Some(true)
        );
        let stored = control.scope(&scope).unwrap();
        assert_eq!(stored.root, locator);
        assert_eq!(stored.volume_id, volume);
        assert!(!stored.revoked);
    }
    let policy = engine.policy_authorizer().unwrap();
    assert!(matches!(
        policy.decide(&principal, &Permission::MetadataRead, &scope),
        Decision::Allowed
    ));
    let native = ResourceLocator::NativePath(root.to_str().unwrap().to_owned());
    let parent = DiskNode {
        id: 1,
        parent_id: None,
        locator: native.clone(),
        name: "root".into(),
        kind: NodeKind::Directory,
        subtree_bytes: 7,
        direct_bytes: 0,
        size_known: true,
        files: 1,
        directories: 1,
        modified_unix_seconds: Some(100),
        file_identity: None,
        category_hint: None,
        reclaim_hint: None,
        read_error: false,
    };
    let leaf = DiskNode {
        id: 2,
        parent_id: Some(1),
        locator: ResourceLocator::NativePath(root.join("item").to_str().unwrap().into()),
        name: "item".into(),
        kind: NodeKind::File,
        direct_bytes: 7,
        directories: 0,
        ..parent.clone()
    };
    let graph = DiskGraph {
        snapshot: DiskSnapshot {
            id: REVISION.into(),
            root: native,
            volume_id: Some("synthetic-scope-projection-import".into()),
            captured_at_unix_ms: 1,
            settings: ScanSettings {
                apparent_size: true,
                follow_links: false,
                include_hidden: true,
                one_filesystem: true,
                max_depth: None,
                dedup_hardlinks: false,
            },
            coverage: ScanCoverage {
                complete: true,
                unreadable_nodes: 0,
                depth_limited: false,
            },
        },
        nodes: vec![parent, leaf],
        evidence: Vec::new(),
    };
    let server = engine.server_id().unwrap();
    let mut store =
        SqliteSnapshotStore::open(&directory.path().join("data/diskgraph.sqlite")).unwrap();
    store.append_staging_nodes(REVISION, &graph.nodes).unwrap();
    store
        .publish_revision_owned(
            REVISION,
            &graph,
            REVISION,
            1,
            Some((server.as_str(), scope.as_str())),
        )
        .unwrap();
    drop(store);
    (directory, engine, principal, scope, policy)
}

fn isolated(name: &str) -> bool {
    const CHILD: &str = "DISKGRAPH_SCOPE_PROJECTION_CHILD";
    if std::env::var(CHILD).ok().as_deref() == Some(name) {
        return false;
    }
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([name, "--exact", "--nocapture", "--test-threads=1"])
        .env(CHILD, name)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    eprintln!("{stdout}");
    assert!(
        output.status.success(),
        "{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(stdout.contains("test result: ok. 1 passed"));
    true
}

fn measured<T>(work: impl FnOnce() -> T) -> (T, usize) {
    REQUESTED_BYTES.store(0, Ordering::Relaxed);
    MEASURING.store(true, Ordering::Relaxed);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(work));
    MEASURING.store(false, Ordering::Relaxed);
    let requested = REQUESTED_BYTES.load(Ordering::Relaxed);
    match result {
        Ok(result) => (result, requested),
        Err(payload) => std::panic::resume_unwind(payload),
    }
}

fn successful_display(field_bytes: usize) {
    let (_directory, engine, principal, _scope, policy) = fixture(field_bytes);
    let authorizer = CallbackAuthorizer {
        policy: &policy,
        calls: Cell::new(0),
        allowed: Cell::new(0),
        callback: |_| {},
    };
    let consumed = Cell::new(false);
    let actual_root = Cell::new(false);
    let shared_charge = Cell::new(false);
    let (result, requested) = measured(|| {
        engine.with_authorized_revision_display_reader_bounded(
            REVISION,
            &principal,
            &authorizer,
            budget(),
            |reader, snapshot, mut reads| {
                consumed.set(true);
                let parent = reader
                    .navigation_node_with_budget(snapshot, 1, &mut reads)?
                    .ok_or(EngineError::Business(BusinessError::NotFound))?;
                actual_root.set(parent.id == 1 && parent.name == "root" && parent.size_known);
                shared_charge
                    .set(reads.nodes_read() == 1 && reads.remaining_raw_bytes() < RAW_BYTES);
                Ok(RevisionDisplayCompletion::Complete)
            },
        )
    });
    eprintln!(
        "scope_projection optional_each={field_bytes} deadline_ms=50 consumed={} auth_calls={} allowed={} requested={requested} result={result:?}",
        consumed.get(),
        authorizer.calls.get(),
        authorizer.allowed.get()
    );
    // 先见证真实消费者/授权/成功；预算拒绝或未到消费者不能冒充成本门禁通过。
    assert!(
        consumed.get(),
        "display consumer was not reached: {result:?}"
    );
    assert!(actual_root.get());
    assert!(shared_charge.get());
    assert!(result.is_ok(), "complete display was refused: {result:?}");
    assert_eq!(authorizer.calls.get(), 2);
    assert_eq!(authorizer.allowed.get(), 2);
    assert!(
        requested < 512 << 10,
        "unused ScopeRecord fields were owned: {requested}"
    );
}

#[test]
fn ordinary_scope_completes_the_real_display_request_within_the_original_budget() {
    if !isolated("ordinary_scope_completes_the_real_display_request_within_the_original_budget") {
        successful_display(0);
    }
}

#[test]
fn giant_optional_scope_fields_are_not_owned_by_the_real_display_request() {
    if !isolated("giant_optional_scope_fields_are_not_owned_by_the_real_display_request") {
        successful_display(FIELD_BYTES);
    }
}

fn revoke_during_terminal_authorization(revoke_scope: bool) {
    let (directory, engine, principal, scope, policy) = fixture(0);
    let external = RefCell::new(
        ControlStore::open(&directory.path().join("data/diskgraph-control.sqlite")).unwrap(),
    );
    let withdrawn = Cell::new(false);
    // 只记录真实调用边界，不创建或延长任何授权期限；输出在 display 返回后。
    let callback_started = Cell::new(None::<Instant>);
    let revoke_returned = Cell::new(None::<Instant>);
    let consumer_started = Cell::new(None::<Instant>);
    let consumer_returned = Cell::new(None::<Instant>);
    let original_deadline = Cell::new(None::<Instant>);
    let authorizer = CallbackAuthorizer {
        policy: &policy,
        calls: Cell::new(0),
        allowed: Cell::new(0),
        callback: |call| {
            if call == 2 {
                callback_started.set(Some(Instant::now()));
                let revoked = if revoke_scope {
                    external.borrow_mut().revoke_scope(&scope)
                } else {
                    external.borrow_mut().revoke_grant(
                        &principal,
                        &Permission::MetadataRead,
                        &scope,
                    )
                };
                revoke_returned.set(Some(Instant::now()));
                revoked.unwrap();
                withdrawn.set(true);
            }
        },
    };
    let consumed = Cell::new(false);
    let display_started = Instant::now();
    let result = engine.with_authorized_revision_display_reader_bounded(
        REVISION,
        &principal,
        &authorizer,
        budget(),
        |reader, snapshot, mut reads| {
            consumer_started.set(Some(Instant::now()));
            original_deadline.set(Some(reads.deadline()));
            let parent = reader
                .navigation_node_with_budget(snapshot, 1, &mut reads)?
                .ok_or(EngineError::Business(BusinessError::NotFound))?;
            consumed.set(parent.id == 1 && parent.name == "root");
            consumer_returned.set(Some(Instant::now()));
            Ok(RevisionDisplayCompletion::Complete)
        },
    );
    let display_returned = Instant::now();
    let offset_us = |instant: Option<Instant>| {
        instant.map(|instant| instant.duration_since(display_started).as_micros())
    };
    let revoke_us = callback_started
        .get()
        .zip(revoke_returned.get())
        .map(|(start, end)| end.duration_since(start).as_micros());
    // 独立撤销不进入 allocation 计量窗；这里不做 SQL、不推断内部 fsync/锁/VM 阶段耗时。
    eprintln!(
        "SCOPE_TERMINAL_DIAGNOSTIC revoke_scope={revoke_scope} deadline_ms=50 display_us={} consumer_enter_us={:?} consumer_exit_us={:?} callback2_start_us={:?} revoke_return_us={:?} revoke_us={revoke_us:?} original_deadline_elapsed={:?} consumed={} withdrawn={} auth_calls={} allowed={} result={result:?}",
        display_returned.duration_since(display_started).as_micros(),
        offset_us(consumer_started.get()),
        offset_us(consumer_returned.get()),
        offset_us(callback_started.get()),
        offset_us(revoke_returned.get()),
        original_deadline
            .get()
            .map(|deadline| display_returned >= deadline),
        consumed.get(),
        withdrawn.get(),
        authorizer.calls.get(),
        authorizer.allowed.get(),
    );
    assert!(
        consumed.get(),
        "revoke test did not read the actual parent: {result:?}"
    );
    assert!(
        withdrawn.get(),
        "terminal authorization callback was not reached"
    );
    assert_eq!(authorizer.calls.get(), 2);
    assert_eq!(authorizer.allowed.get(), 2);
    assert!(
        matches!(
            result,
            Err(EngineError::Business(BusinessError::PermissionDenied))
        ),
        "terminal withdrawal returned an unexpected result: {result:?}"
    );
    assert_eq!(
        external
            .borrow()
            .live_permission(&principal, &Permission::MetadataRead, &scope)
            .unwrap(),
        Some(false)
    );
    assert_eq!(
        external.borrow().scope_revoked(&scope).unwrap(),
        revoke_scope
    );
}

#[test]
fn scope_withdrawn_by_the_real_terminal_callback_refuses_the_display() {
    revoke_during_terminal_authorization(true);
}

#[test]
fn grant_withdrawn_by_the_real_terminal_callback_refuses_the_display() {
    revoke_during_terminal_authorization(false);
}

#[test]
fn terminal_policy_sql_keeps_its_budget_after_the_capability_callback() {
    let (directory, engine, principal, _scope, policy) = fixture(0);
    let injected = Cell::new(false);
    let authorizer = CallbackAuthorizer {
        policy: &policy,
        calls: Cell::new(0),
        allowed: Cell::new(0),
        callback: |call| {
            if call == 2 {
                // 隔离库中真实替换 policy 为昂贵但结果相同的 SQL 视图。
                // WAL 写锁不阻止正常 reader，故以 VM 工作量直接验证 progress 中断。
                let connection = rusqlite::Connection::open(
                    directory.path().join("data/diskgraph-control.sqlite"),
                )
                .unwrap();
                connection
                    .execute_batch(
                        "ALTER TABLE policy RENAME TO terminal_policy_fixture;
                     CREATE VIEW policy AS SELECT p.* FROM terminal_policy_fixture p
                     CROSS JOIN (WITH RECURSIVE work(n) AS
                       (VALUES(0) UNION ALL SELECT n+1 FROM work WHERE n<5000000)
                       SELECT sum(n) AS total FROM work) cost WHERE cost.total>0;",
                    )
                    .unwrap();
                injected.set(true);
            }
        },
    };
    let started = Instant::now();
    let result = engine.with_authorized_revision_display_reader_bounded(
        REVISION,
        &principal,
        &authorizer,
        QueryBudget {
            deadline_ms: 1000,
            ..QueryBudget::default()
        },
        |_, _, _| Ok(RevisionDisplayCompletion::Complete),
    );
    let elapsed = started.elapsed();
    assert!(
        injected.get(),
        "actual terminal capability callback must run"
    );
    assert!(
        matches!(
            result,
            Err(EngineError::Business(BusinessError::BudgetExceeded))
        ),
        "{result:?}"
    );
    // 宽观察界限只区分约 50 ms 中断和多次完整执行百万步 SQL，不声称硬实时。
    assert!(
        elapsed < std::time::Duration::from_secs(1),
        "unbounded terminal SQL: {elapsed:?}"
    );
}
