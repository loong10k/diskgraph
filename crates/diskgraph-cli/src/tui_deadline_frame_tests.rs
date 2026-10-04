//! 检查真实终端后端的 deadline 截断与授权拒绝；来源：OpenSpec Q-08 / 13.6。

use diskgraph_core::{
    Authorizer, BusinessError, Decision, Permission, PolicyAuthorizer, PrincipalId, ScopeId,
};
use diskgraph_engine::EngineError;
use diskgraph_store::StoreError;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use super::test_fixture::TuiFixture;
use super::{Browser, PAGE_SIZE, draw, draw_authorized_frame, load_layer};

fn backend_text(terminal: &Terminal<TestBackend>) -> String {
    terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect()
}

#[test]
fn expired_nested_frame_commits_parent_and_deadline_to_the_actual_backend() {
    let fixture = TuiFixture::new(0, true);
    let request = fixture.request();
    let cached = load_layer(&request, 1).unwrap();
    let browser = Browser::new(&fixture.revision, cached.clone());
    let mut terminal = Terminal::new(TestBackend::new(160, 30)).unwrap();
    let mut painted = false;
    let call_started = Instant::now();
    let mut before_paint = None;
    let result = draw_authorized_frame(&mut terminal, &request, &cached, |frame, reads| {
        painted = true;
        before_paint = Some(call_started.elapsed());
        // 真实绘制开始后越过 50ms 期限，不要求宿主在极短时间完成数据库准备。
        std::thread::sleep(Duration::from_millis(120));
        assert!(reads.load_layer(2, PAGE_SIZE).is_none());
        assert_eq!(reads.truncation_reason, Some("deadline"));
        draw(frame, &browser, reads);
    });
    // 原调用完成后才输出诊断；不放宽 50ms 原期限，也不把准备失败接受为通过。
    eprintln!(
        "tui_frame_phase deadline_ms=50 before_paint_us={:?} total_us={} painted={painted} result={result:?}",
        before_paint.map(|elapsed| elapsed.as_micros()),
        call_started.elapsed().as_micros(),
    );
    assert!(
        painted,
        "the real paint phase was never reached: {result:?}"
    );
    assert!(
        result.is_ok(),
        "deadline partial was not committed: {result:?}"
    );
    let actual = backend_text(&terminal);
    assert!(
        actual.contains("parent"),
        "the inspectable parent disappeared"
    );
    assert!(
        actual.contains("nested view truncated: deadline"),
        "the deadline existed only in a memory buffer: {actual}"
    );
}

#[test]
fn expired_nested_frame_refuses_scope_revoked_during_paint() {
    let fixture = TuiFixture::new(0, true);
    let request = fixture.request();
    let cached = load_layer(&request, 1).unwrap();
    let browser = Browser::new(&fixture.revision, cached.clone());
    let mut terminal = Terminal::new(TestBackend::new(160, 30)).unwrap();
    let result = draw_authorized_frame(&mut terminal, &request, &cached, |frame, reads| {
        std::thread::sleep(Duration::from_millis(120));
        assert!(reads.load_layer(2, PAGE_SIZE).is_none());
        draw(frame, &browser, reads);
        fixture
            .engine
            .revoke_scope(&fixture.scope, &fixture.principal, &fixture.policy)
            .unwrap();
    });
    assert!(
        matches!(
            result,
            Err(EngineError::Business(BusinessError::PermissionDenied))
        ),
        "scope revocation became a displayable deadline: {result:?}"
    );
    assert!(backend_text(&terminal).trim().is_empty());
}

#[test]
fn expired_nested_frame_refuses_single_metadata_grant_revoked_during_paint() {
    let fixture = TuiFixture::new(0, true);
    let request = fixture.request();
    let cached = load_layer(&request, 1).unwrap();
    let browser = Browser::new(&fixture.revision, cached.clone());
    let mut terminal = Terminal::new(TestBackend::new(160, 30)).unwrap();
    let result = draw_authorized_frame(&mut terminal, &request, &cached, |frame, reads| {
        std::thread::sleep(Duration::from_millis(120));
        assert!(reads.load_layer(2, PAGE_SIZE).is_none());
        draw(frame, &browser, reads);
        fixture
            .engine
            .control_store()
            .unwrap()
            .revoke_grant(
                &fixture.principal,
                &Permission::MetadataRead,
                &fixture.scope,
            )
            .unwrap();
    });
    assert!(
        matches!(
            result,
            Err(EngineError::Business(BusinessError::PermissionDenied))
        ),
        "grant revocation became a displayable deadline: {result:?}"
    );
    assert!(backend_text(&terminal).trim().is_empty());
}

#[test]
fn real_store_error_during_paint_never_applies_a_partial_frame() {
    let fixture = TuiFixture::new(0, true);
    let request = fixture.request();
    let cached = load_layer(&request, 1).unwrap();
    let browser = Browser::new(&fixture.revision, cached.clone());
    let mut terminal = Terminal::new(TestBackend::new(160, 30)).unwrap();
    let result = draw_authorized_frame(&mut terminal, &request, &cached, |frame, reads| {
        // 真实 Store 的 SQLite 整数转换拒绝不可表示 ID，不损坏数据库来制造错误。
        assert!(reads.load_layer(u64::MAX, PAGE_SIZE).is_none());
        draw(frame, &browser, reads);
    });
    assert!(
        matches!(result, Err(EngineError::Store(StoreError::IntegerOverflow))),
        "real store failure was replaced by a budget result: {result:?}"
    );
    assert!(backend_text(&terminal).trim().is_empty());
}

#[test]
fn generic_authorized_reader_still_refuses_an_expired_complete_success() {
    let fixture = TuiFixture::new(0, true);
    let mut consumed = false;
    let result = fixture.engine.with_authorized_revision_reader(
        &fixture.revision,
        &fixture.principal,
        &fixture.policy,
        50,
        |_, _, _| {
            consumed = true;
            std::thread::sleep(Duration::from_millis(120));
            Ok(())
        },
    );
    assert!(
        consumed,
        "the real consumer phase was not reached: {result:?}"
    );
    assert!(
        matches!(
            result,
            Err(EngineError::Business(BusinessError::BudgetExceeded))
        ),
        "generic complete success escaped its deadline: {result:?}"
    );
}

#[test]
fn terminal_control_lock_is_refused_before_its_owner_releases_it() {
    let fixture = TuiFixture::new(0, true);
    let request = fixture.request();
    let cached = load_layer(&request, 1).unwrap();
    let browser = Browser::new(&fixture.revision, cached.clone());
    let mut terminal = Terminal::new(TestBackend::new(160, 30)).unwrap();
    let (arm_sender, arm_receiver) = std::sync::mpsc::channel();
    let (held_sender, held_receiver) = std::sync::mpsc::channel();
    let (release_sender, release_receiver) = std::sync::mpsc::channel();
    std::thread::scope(|threads| {
        let engine = &fixture.engine;
        threads.spawn(move || {
            if arm_receiver.recv_timeout(Duration::from_secs(2)).is_err() {
                return;
            }
            let _guard = engine.control_store().unwrap();
            held_sender.send(()).unwrap();
            // 旧代码会等待此超时释放；即使断言失败也保证测试不会永久挂住。
            let _ = release_receiver.recv_timeout(Duration::from_millis(600));
        });
        let mut painted = false;
        let started = Instant::now();
        let result = draw_authorized_frame(&mut terminal, &request, &cached, |frame, reads| {
            painted = true;
            draw(frame, &browser, reads);
            // 首次真实授权和数据绘制已完成，竞争仅发生在 terminal phase。
            arm_sender.send(()).unwrap();
            held_receiver.recv_timeout(Duration::from_secs(2)).unwrap();
        });
        let elapsed = started.elapsed();
        let _ = release_sender.send(());
        assert!(painted, "the terminal phase was not reached: {result:?}");
        assert!(
            result.is_err(),
            "a held control guard still committed: {result:?}"
        );
        assert!(backend_text(&terminal).trim().is_empty());
        assert!(
            elapsed < Duration::from_millis(300),
            "terminal lock waited for its 600ms owner release: {elapsed:?}, {result:?}"
        );
    });
}

/// 在真实绘制后只延迟下一次授权决定，不替换持久权限事实。
/// 来源：DiskGraph 原生 Rust Q-08 末段期限测试；无 Java 对应实现。
struct SlowTerminalAuthorizer<'a> {
    delegate: &'a PolicyAuthorizer,
    armed: AtomicBool,
    observed: AtomicBool,
}

impl Authorizer for SlowTerminalAuthorizer<'_> {
    fn decide(
        &self,
        principal: &PrincipalId,
        permission: &Permission,
        scope: &ScopeId,
    ) -> Decision {
        if self.armed.swap(false, Ordering::SeqCst) {
            self.observed.store(true, Ordering::SeqCst);
            std::thread::sleep(Duration::from_millis(120));
        }
        self.delegate.decide(principal, permission, scope)
    }

    fn policy_version(&self) -> u64 {
        self.delegate.policy_version()
    }
}

#[test]
fn expired_partial_frame_cannot_renew_a_slow_terminal_authorization() {
    let fixture = TuiFixture::new(0, true);
    let cached = load_layer(&fixture.request(), 1).unwrap();
    let browser = Browser::new(&fixture.revision, cached.clone());
    let authorizer = SlowTerminalAuthorizer {
        delegate: &fixture.policy,
        armed: AtomicBool::new(false),
        observed: AtomicBool::new(false),
    };
    let request = crate::tui_request::TuiRequest {
        engine: &fixture.engine,
        revision: &fixture.revision,
        principal: &fixture.principal,
        authorizer: &authorizer,
    };
    let mut terminal = Terminal::new(TestBackend::new(160, 30)).unwrap();
    let result = draw_authorized_frame(&mut terminal, &request, &cached, |frame, reads| {
        std::thread::sleep(Duration::from_millis(120));
        assert!(reads.load_layer(2, PAGE_SIZE).is_none());
        draw(frame, &browser, reads);
        authorizer.armed.store(true, Ordering::SeqCst);
    });
    assert!(
        authorizer.observed.load(Ordering::SeqCst),
        "a preparation failure substituted for terminal timeout: {result:?}"
    );
    assert!(
        matches!(
            result,
            Err(EngineError::Business(BusinessError::BudgetExceeded))
        ),
        "partial rendering renewed terminal authorization: {result:?}"
    );
    assert!(backend_text(&terminal).trim().is_empty());
}

#[test]
fn initial_control_lock_is_refused_before_paint_or_owner_release() {
    let fixture = TuiFixture::new(0, true);
    let request = fixture.request();
    let cached = load_layer(&request, 1).unwrap();
    let browser = Browser::new(&fixture.revision, cached.clone());
    let mut terminal = Terminal::new(TestBackend::new(160, 30)).unwrap();
    let (held_sender, held_receiver) = std::sync::mpsc::channel();
    let (release_sender, release_receiver) = std::sync::mpsc::channel();
    std::thread::scope(|threads| {
        let engine = &fixture.engine;
        threads.spawn(move || {
            let _guard = engine.control_store().unwrap();
            held_sender.send(()).unwrap();
            // 初次授权前就持锁；旧路径会等到超时释放，失败路径也不会永久挂住。
            let _ = release_receiver.recv_timeout(Duration::from_millis(600));
        });
        held_receiver.recv_timeout(Duration::from_secs(2)).unwrap();
        let mut painted = false;
        let started = Instant::now();
        let result = draw_authorized_frame(&mut terminal, &request, &cached, |frame, reads| {
            painted = true;
            draw(frame, &browser, reads);
        });
        let elapsed = started.elapsed();
        // 先释放测试 owner，再做断言，防止 panic 延长它的持锁时间。
        let _ = release_sender.send(());
        assert!(
            result.is_err(),
            "initial control contention allowed data: {result:?}"
        );
        assert!(
            !painted,
            "paint ran before initial authorization finished: {result:?}"
        );
        assert!(backend_text(&terminal).trim().is_empty());
        assert!(
            elapsed < Duration::from_millis(300),
            "initial control lock waited for its 600ms owner release: {elapsed:?}, {result:?}"
        );
    });
}

#[test]
fn navigation_control_lock_is_refused_before_returning_a_layer_or_owner_release() {
    let fixture = TuiFixture::new(0, true);
    let request = fixture.request();
    // 合法已发布快照先通过无竞争的真实导航，避免把夹具或节点错误算作拒绝。
    assert_eq!(load_layer(&request, 1).unwrap().parent_id, 1);
    let (held_sender, held_receiver) = std::sync::mpsc::channel();
    let (release_sender, release_receiver) = std::sync::mpsc::channel();
    std::thread::scope(|threads| {
        let engine = &fixture.engine;
        threads.spawn(move || {
            let _guard = engine.control_store().unwrap();
            held_sender.send(()).unwrap();
            // 这验证锁竞争须及时拒绝，不把 600ms 等待说成超过导航原 1000ms 期限。
            let _ = release_receiver.recv_timeout(Duration::from_millis(600));
        });
        held_receiver.recv_timeout(Duration::from_secs(2)).unwrap();
        let started = Instant::now();
        let result = load_layer(&request, 1);
        let elapsed = started.elapsed();
        // 在断言前释放本测试 owner，错误展开也不能延长它的持锁时间。
        let _ = release_sender.send(());
        assert!(
            matches!(
                result,
                Err(EngineError::Business(BusinessError::BudgetExceeded))
            ),
            "navigation contention returned a layer or unrelated failure: {result:?}"
        );
        assert!(
            elapsed < Duration::from_millis(300),
            "navigation waited for its 600ms control owner release: {elapsed:?}, {result:?}"
        );
    });
}

#[test]
fn navigation_control_lock_cannot_wait_beyond_its_original_read_deadline() {
    let fixture = TuiFixture::new(0, true);
    let request = fixture.request();
    assert_eq!(load_layer(&request, 1).unwrap().parent_id, 1);
    let (held_sender, held_receiver) = std::sync::mpsc::channel();
    let (release_sender, release_receiver) = std::sync::mpsc::channel();
    std::thread::scope(|threads| {
        let engine = &fixture.engine;
        threads.spawn(move || {
            let _guard = engine.control_store().unwrap();
            held_sender.send(()).unwrap();
            // 明确超过 load_layer 原 1000ms 读期，检验真实锁等待而非只检验及时拒绝策略。
            let _ = release_receiver.recv_timeout(Duration::from_millis(1600));
        });
        held_receiver.recv_timeout(Duration::from_secs(2)).unwrap();
        let started = Instant::now();
        let result = load_layer(&request, 1);
        let elapsed = started.elapsed();
        // 先释放隔离夹具锁，再断言，避免 panic 留住 owner。
        let _ = release_sender.send(());
        assert!(
            matches!(
                result,
                Err(EngineError::Business(BusinessError::BudgetExceeded))
                    | Err(EngineError::Store(StoreError::BudgetExceeded))
            ),
            "navigation returned a layer or unrelated failure beyond its read deadline: {result:?}"
        );
        assert!(
            elapsed <= Duration::from_millis(1300),
            "navigation exceeded its 1000ms read deadline waiting for a 1600ms owner: {elapsed:?}, {result:?}"
        );
    });
}
