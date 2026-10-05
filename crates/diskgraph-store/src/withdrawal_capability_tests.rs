//! 未资格 namespace 保留未知，不能把能力缺失伪装撤权。
use crate::ControlStore;
use diskgraph_core::{Locator, Permission, PrincipalId};

#[test]
fn private_in_memory_database_has_no_cross_connection_withdrawal_capability() {
    let directory = tempfile::tempdir().unwrap();
    let mut store = ControlStore::open_in_memory().unwrap();
    let principal = PrincipalId::new("reader").unwrap();
    let scope = store
        .register_scope(&Locator::from_native_path(directory.path()), None)
        .unwrap();
    assert!(
        store
            .watch_authorization_withdrawal(&principal, &scope, &Permission::MetadataRead)
            .unwrap()
            .is_none()
    );
}

#[cfg(unix)]
#[test]
fn unix_default_vfs_does_not_guess_namespace_from_path_or_server() {
    let directory = tempfile::tempdir().unwrap();
    let mut store = ControlStore::open(&directory.path().join("control.sqlite")).unwrap();
    let principal = PrincipalId::new("reader").unwrap();
    let scope = store
        .register_scope(&Locator::from_native_path(directory.path()), None)
        .unwrap();
    assert!(
        store
            .watch_authorization_withdrawal(&principal, &scope, &Permission::MetadataRead)
            .unwrap()
            .is_none()
    );
}
