//! 注册事务的真实第二连接、到期与 unwind 回归；来源：原生 Rust SC-01/04。
use crate::{ControlStore, StoreError};
use diskgraph_core::{Authorizer, Decision, Grant, Locator, Permission, PrincipalId, ScopeId};
use rusqlite::{Connection, params};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

fn fixture() -> (
    tempfile::TempDir,
    ControlStore,
    PrincipalId,
    ScopeId,
    Locator,
) {
    let directory = tempfile::tempdir().unwrap();
    let mut control = ControlStore::open(&directory.path().join("control.sqlite")).unwrap();
    control.ensure_server().unwrap();
    control.publish_policy_version(7).unwrap();
    let actor = PrincipalId::new("transaction-registrar").unwrap();
    let admin = ScopeId::new("diskgraph-admin").unwrap();
    control
        .upsert_grant(&Grant {
            principal: actor.clone(),
            permission: Permission::ScopeAdmin,
            scope: admin.clone(),
            policy_version: 7,
        })
        .unwrap();
    let root = Locator::from_native_path(&directory.path().join("root"));
    (directory, control, actor, admin, root)
}

#[test]
fn registration_is_invisible_to_another_connection_until_all_grants_commit() {
    let (directory, mut control, actor, admin, root) = fixture();
    let observer = Connection::open(directory.path().join("control.sqlite")).unwrap();
    let scope = control
        .register_scope_with_grants_until(
            &root,
            None,
            &actor,
            &admin,
            Instant::now() + Duration::from_secs(2),
            None,
            |_, scopes| {
                assert_eq!(scopes.len(), 1);
                assert_eq!(
                    observer
                        .query_row("SELECT COUNT(*) FROM scopes", [], |row| row
                            .get::<_, i64>(0))
                        .unwrap(),
                    0
                );
                assert_eq!(
                    observer
                        .query_row(
                            "SELECT COUNT(*) FROM grants WHERE scope_id <> 'diskgraph-admin'",
                            [],
                            |row| row.get::<_, i64>(0)
                        )
                        .unwrap(),
                    0
                );
                Ok(())
            },
        )
        .unwrap()
        .unwrap();
    assert_eq!(
        observer
            .query_row("SELECT COUNT(*) FROM scopes", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        observer
            .query_row(
                "SELECT COUNT(*) FROM grants WHERE scope_id=?1 AND policy_version=7",
                [scope.as_str()],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        3
    );
}

#[test]
fn waiting_registration_rechecks_a_committed_withdrawal_or_new_epoch() {
    for modification in [
        "DELETE FROM grants",
        "UPDATE policy SET revoked=1",
        "UPDATE policy SET version=8",
    ] {
        let (directory, mut control, actor, admin, root) = fixture();
        assert_eq!(
            control
                .authorizer()
                .unwrap()
                .decide(&actor, &Permission::ScopeAdmin, &admin),
            Decision::Allowed
        );
        let writer = Connection::open(directory.path().join("control.sqlite")).unwrap();
        writer.execute_batch("BEGIN IMMEDIATE").unwrap();
        writer.execute_batch(modification).unwrap();
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            started_tx.send(()).unwrap();
            control.register_scope_with_grants_until(
                &root,
                None,
                &actor,
                &admin,
                Instant::now() + Duration::from_secs(2),
                None,
                |_, _| panic!("revoked registration reached graph isolation"),
            )
        });
        started_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        writer.execute_batch("COMMIT").unwrap();
        assert_eq!(worker.join().unwrap().unwrap(), None, "{modification}");
        assert_eq!(
            writer
                .query_row("SELECT COUNT(*) FROM scopes", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
}

#[test]
fn expired_authentication_after_isolation_rolls_back_without_renewal() {
    let (_directory, mut control, actor, admin, root) = fixture();
    let expiry = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 1;
    let result = control.register_scope_with_grants_until(
        &root,
        None,
        &actor,
        &admin,
        Instant::now() + Duration::from_secs(3),
        Some(expiry),
        |_, _| {
            let until = UNIX_EPOCH + Duration::from_secs(expiry);
            std::thread::sleep(
                until.duration_since(SystemTime::now()).unwrap_or_default()
                    + Duration::from_millis(10),
            );
            Ok(())
        },
    );
    assert!(matches!(result, Err(StoreError::Conflict(_))), "{result:?}");
    assert!(control.connection.is_autocommit());
    assert!(control.list_scopes().unwrap().is_empty());
    assert_eq!(
        control
            .connection
            .query_row(
                "SELECT COUNT(*) FROM grants WHERE scope_id <> 'diskgraph-admin'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
}

#[test]
fn a_callback_panic_rolls_back_and_restores_the_original_connection() {
    let (_directory, mut control, actor, admin, root) = fixture();
    control
        .connection
        .busy_timeout(Duration::from_millis(123))
        .unwrap();
    let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        control.register_scope_with_grants_until(
            &root,
            None,
            &actor,
            &admin,
            Instant::now() + Duration::from_secs(2),
            None,
            |_, _| panic!("isolation callback fixture"),
        )
    }));
    assert!(panicked.is_err());
    assert!(control.connection.is_autocommit());
    assert_eq!(
        control
            .connection
            .query_row("PRAGMA busy_timeout", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        123
    );
    assert!(control.list_scopes().unwrap().is_empty());
    control
        .register_scope_with_grants_until(
            &root,
            None,
            &actor,
            &admin,
            Instant::now() + Duration::from_secs(2),
            None,
            |_, _| Ok(()),
        )
        .unwrap()
        .unwrap();
}

#[test]
fn a_same_transaction_policy_change_cannot_commit_old_epoch_grants() {
    let (_directory, mut control, actor, admin, root) = fixture();
    control.connection.execute_batch("CREATE TRIGGER change_epoch BEFORE INSERT ON grants WHEN NEW.scope_id <> 'diskgraph-admin' BEGIN UPDATE policy SET version=8; END;").unwrap();
    let result = control
        .register_scope_with_grants_until(
            &root,
            None,
            &actor,
            &admin,
            Instant::now() + Duration::from_secs(2),
            None,
            |_, _| Ok(()),
        )
        .unwrap();
    assert_eq!(result, None);
    assert_eq!(control.policy_version().unwrap(), 7);
    assert!(control.list_scopes().unwrap().is_empty());
    assert_eq!(
        control
            .connection
            .query_row(
                "SELECT COUNT(*) FROM grants WHERE scope_id <> ?1",
                params![admin.as_str()],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
}
