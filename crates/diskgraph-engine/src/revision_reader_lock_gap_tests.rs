//! 首次实时授权释放锁后的确定性撤权竞态回归；来源：DiskGraph 原生安全契约。
use crate::Engine;
use std::cell::RefCell;

type AfterAuthorization = Box<dyn FnOnce(&Engine)>;
thread_local! {
    static OBSERVER: RefCell<Option<AfterAuthorization>> = const { RefCell::new(None) };
}

/// 参数：engine 是刚完成首次授权的原引擎；返回：无，先释放 TLS 借用再执行观察。
pub(super) fn after_initial_authorization(engine: &Engine) {
    let observer = OBSERVER.with(|slot| slot.borrow_mut().take());
    if let Some(observer) = observer {
        observer(engine);
    }
}

#[test]
fn continuous_revocation_between_initial_locks_has_denial_priority() {
    let (_dir, engine, principal, scope, revision) =
        crate::relation_request_tests::published_authorization_fixture();
    let policy = engine.policy_authorizer().unwrap();
    let actor = principal.clone();
    OBSERVER.with(|slot| {
        *slot.borrow_mut() = Some(Box::new(move |engine| {
            // 原生产授权已完成；在后续取锁前实际提交撤权，不用 sleep 猜测竞态。
            engine
                .control_store()
                .unwrap()
                .revoke_grant(&actor, &diskgraph_core::Permission::MetadataRead, &scope)
                .unwrap();
        }))
    });
    let entered = std::cell::Cell::new(false);
    let result = engine.with_authorized_revision_reader_until(
        revision,
        &principal,
        &policy,
        std::time::Instant::now() + std::time::Duration::from_secs(1),
        |_, _, _| {
            entered.set(true);
            Ok(())
        },
    );
    assert!(
        matches!(
            result,
            Err(crate::EngineError::Business(
                diskgraph_core::BusinessError::PermissionDenied
            ))
        ),
        "{result:?}"
    );
    assert!(!entered.get(), "revoked request entered consumer");
}
