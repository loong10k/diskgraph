//! 必要范围投影的真实 SQL、授权兼容与 Rust requested 回归；不代表 RSS/SQLite C 分配。
use crate::{ControlStore, StoreError};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use diskgraph_core::{Grant, Locator, LocatorKind, Permission, PrincipalId, ScopeId};
use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
use std::path::{Path, PathBuf};

fn native_scope(store: &mut ControlStore) -> (ScopeId, PathBuf) {
    let path = tempfile::tempdir().unwrap().path().join("scope-root");
    let scope = store
        .register_scope(&Locator::from_native_path(&path), None)
        .unwrap();
    (scope, path)
}

#[test]
fn scope_native_projection_sql_never_reads_optional_display_or_volume() {
    let mut store = ControlStore::open_in_memory().unwrap();
    let (scope, path) = native_scope(&mut store);
    let locator = store.scope(&scope).unwrap().root;
    store
        .connection
        .authorizer(Some(|context: AuthContext<'_>| {
            if let AuthAction::Read {
                table_name: "scopes",
                column_name: "root_display" | "volume_id" | "created_at_unix_ms",
            } = context.action
            {
                Authorization::Deny
            } else {
                Authorization::Allow
            }
        }))
        .unwrap();
    // 真正旧 SQL 正控制被拒绝，不能把未注册的 authorizer 当作排除列证据。
    assert!(matches!(store.scope(&scope), Err(StoreError::Sqlite(_))));
    let mut totals = (0, 0, 0);
    let restored = store
        .scope_native_root_with_admission(&scope, &mut |raw, entries, allocation| {
            totals.0 += raw;
            totals.1 += entries;
            totals.2 += allocation;
            Ok(())
        })
        .unwrap();
    assert_eq!(restored, path);
    assert_eq!(
        totals.0,
        ("native_path".len() + locator.raw_b64.len() + 8) as u64
    );
    assert_eq!(totals.1, 1);
    assert!(totals.2 > totals.0);
    assert!(!store.scope_revoked(&scope).unwrap());
    store
        .connection
        .authorizer(Some(|context: AuthContext<'_>| {
            if let AuthAction::Read {
                table_name: "scopes",
                column_name,
            } = context.action
                && !matches!(column_name, "scope_id" | "revoked")
            {
                Authorization::Deny
            } else {
                Authorization::Allow
            }
        }))
        .unwrap();
    assert!(!store.scope_revoked(&scope).unwrap());
    assert_eq!(
        store
            .live_permission(
                &PrincipalId::new("observer").unwrap(),
                &Permission::MetadataRead,
                &scope
            )
            .unwrap(),
        None
    );
    assert!(matches!(
        store.scope_native_root_with_admission(&scope, &mut |_, _, _| Ok(())),
        Err(StoreError::Sqlite(_))
    ));
}

#[test]
fn scope_native_projection_scalar_preserves_missing_policy_grants_and_revocation() {
    let mut store = ControlStore::open_in_memory().unwrap();
    let (scope, _) = native_scope(&mut store);
    let principal = PrincipalId::new("scope-observer").unwrap();
    assert_eq!(
        store
            .live_permission(&principal, &Permission::MetadataRead, &scope)
            .unwrap(),
        None
    );
    store.publish_policy_version(1).unwrap();
    assert_eq!(
        store
            .live_permission(&principal, &Permission::MetadataRead, &scope)
            .unwrap(),
        Some(false)
    );
    store
        .upsert_grant(&Grant {
            principal: principal.clone(),
            permission: Permission::MetadataRead,
            scope: scope.clone(),
            policy_version: 1,
        })
        .unwrap();
    assert_eq!(
        store
            .live_permission(&principal, &Permission::MetadataRead, &scope)
            .unwrap(),
        Some(true)
    );
    store.revoke_policy().unwrap();
    assert_eq!(
        store
            .live_permission(&principal, &Permission::MetadataRead, &scope)
            .unwrap(),
        Some(false)
    );
    store.revoke_scope(&scope).unwrap();
    assert!(store.scope_revoked(&scope).unwrap());
    assert_eq!(
        store
            .live_permission(&principal, &Permission::MetadataRead, &scope)
            .unwrap(),
        Some(false)
    );
    assert!(matches!(
        store.scope_native_root_with_admission(&scope, &mut |_, _, _| Ok(())),
        Err(StoreError::Conflict(message)) if message == "job scope revoked"
    ));
    let missing = ScopeId::new("missing-scope").unwrap();
    assert!(
        matches!(store.scope_revoked(&missing), Err(StoreError::ScopeNotFound(id)) if id == missing.as_str())
    );
    assert!(
        matches!(store.live_permission(&principal, &Permission::MetadataRead, &missing), Err(StoreError::ScopeNotFound(id)) if id == missing.as_str())
    );
    assert!(
        matches!(store.scope_native_root_with_admission(&missing, &mut |_, _, _| Ok(())), Err(StoreError::ScopeNotFound(id)) if id == missing.as_str())
    );
}

#[test]
fn scope_native_projection_preserves_native_units_without_a_display_fallback() {
    let mut store = ControlStore::open_in_memory().unwrap();
    #[cfg(unix)]
    let path: PathBuf = {
        use std::os::unix::ffi::OsStringExt;
        std::ffi::OsString::from_vec(vec![b'/', b'r', 0xff]).into()
    };
    #[cfg(windows)]
    let path: PathBuf = {
        use std::os::windows::ffi::OsStringExt;
        std::ffi::OsString::from_wide(&[b'C' as u16, b':' as u16, b'\\' as u16, 0xd800]).into()
    };
    #[cfg(not(any(unix, windows)))]
    let path = PathBuf::from("unavailable-native");
    let mut locator = Locator::from_native_path(&path);
    locator.display = "this-display-is-not-the-root".into();
    let scope = store.register_scope(&locator, None).unwrap();
    let result = store.scope_native_root_with_admission(&scope, &mut |_, _, _| Ok(()));
    #[cfg(any(unix, windows))]
    assert_eq!(result.unwrap(), path);
    #[cfg(not(any(unix, windows)))]
    assert!(matches!(result, Err(StoreError::UnsupportedLocator(_))));
}

#[test]
fn scope_native_projection_large_optional_fields_do_not_enter_scalar_or_root_reads() {
    use crate::git_raw_allocation_tests::{isolated, measure};
    let name = "scope_native_projection_tests::scope_native_projection_large_optional_fields_do_not_enter_scalar_or_root_reads";
    if isolated(name) {
        return;
    }
    let mut store = ControlStore::open_in_memory().unwrap();
    let path = Path::new("/scope-native-root");
    let mut locator = Locator::from_native_path(path);
    locator.display = "d".repeat(2 << 20);
    let volume = "v".repeat(2 << 20);
    let scope = store.register_scope(&locator, Some(&volume)).unwrap();
    let full = store.scope(&scope).unwrap();
    assert_eq!(full.root, locator);
    assert_eq!(full.volume_id.as_deref(), Some(volume.as_str()));
    let principal = PrincipalId::new("observer").unwrap();
    let (result, requested) = measure(|| {
        assert!(!store.scope_revoked(&scope).unwrap());
        assert_eq!(
            store
                .live_permission(&principal, &Permission::MetadataRead, &scope)
                .unwrap(),
            None
        );
        store.scope_native_root_with_admission(&scope, &mut |_, _, _| Ok(()))
    });
    assert_eq!(result.unwrap(), path);
    eprintln!("scope optional 4MiB, Rust requested={requested}");
    assert!(
        requested < 65536,
        "unused scope fields were owned: {requested}"
    );
}

#[test]
fn scope_native_projection_large_required_raw_is_refused_before_decode() {
    use crate::git_raw_allocation_tests::{isolated, measure};
    let name = "scope_native_projection_tests::scope_native_projection_large_required_raw_is_refused_before_decode";
    if isolated(name) {
        return;
    }
    let mut store = ControlStore::open_in_memory().unwrap();
    let path = PathBuf::from("r".repeat(2 << 20));
    let locator = Locator::from_native_path(&path);
    let scope = store.register_scope(&locator, None).unwrap();
    assert_eq!(
        store.scope(&scope).unwrap().root.to_native_path().unwrap(),
        path
    );
    let mut reached = false;
    let (result, requested) = measure(|| {
        store.scope_native_root_with_admission(&scope, &mut |raw, entries, allocation| {
            if raw == 0 {
                return Ok(());
            }
            reached = true;
            assert!(raw > 2 << 20 && allocation > raw && entries == 1);
            Err(StoreError::BudgetExceeded)
        })
    });
    assert!(reached);
    assert!(matches!(result, Err(StoreError::BudgetExceeded)));
    eprintln!("scope required raw >2MiB, refused Rust requested={requested}");
    assert!(
        requested < 65536,
        "raw root decoded before admission: {requested}"
    );
}

#[test]
fn scope_native_projection_rejects_kind_encoding_and_column_errors() {
    let mut store = ControlStore::open_in_memory().unwrap();
    let document = store
        .register_scope(&Locator::from_document_uri("content://scope"), None)
        .unwrap();
    assert!(matches!(
        store.scope_native_root_with_admission(&document, &mut |_, _, _| Ok(())),
        Err(StoreError::UnsupportedLocator(_))
    ));
    for raw_b64 in [
        "not-base64!".to_owned(),
        String::new(),
        BASE64.encode([0, 0]),
    ] {
        let scope = store
            .register_scope(
                &Locator {
                    kind: LocatorKind::NativePath,
                    raw_b64,
                    display: "/looks-valid".into(),
                },
                None,
            )
            .unwrap();
        assert!(matches!(
            store.scope_native_root_with_admission(&scope, &mut |_, _, _| Ok(())),
            Err(StoreError::InvalidGraph(_))
        ));
    }
    #[cfg(windows)]
    {
        let scope = store
            .register_scope(
                &Locator {
                    kind: LocatorKind::NativePath,
                    raw_b64: BASE64.encode([65]),
                    display: "ignored".into(),
                },
                None,
            )
            .unwrap();
        assert!(matches!(
            store.scope_native_root_with_admission(&scope, &mut |_, _, _| Ok(())),
            Err(StoreError::InvalidGraph(_))
        ));
    }
    let (scope, _) = native_scope(&mut store);
    store
        .connection
        .execute(
            "UPDATE scopes SET root_kind='unknown' WHERE scope_id=?1",
            [scope.as_str()],
        )
        .unwrap();
    assert!(matches!(
        store.scope_native_root_with_admission(&scope, &mut |_, _, _| Ok(())),
        Err(StoreError::UnsupportedLocator(_))
    ));
    store
        .connection
        .execute(
            "UPDATE scopes SET root_kind='native_path', root_raw_b64=x'00' WHERE scope_id=?1",
            [scope.as_str()],
        )
        .unwrap();
    assert!(matches!(
        store.scope_native_root_with_admission(&scope, &mut |_, _, _| Ok(())),
        Err(StoreError::InvalidGraph(_))
    ));
    store
        .connection
        .execute(
            "UPDATE scopes SET revoked='invalid' WHERE scope_id=?1",
            [scope.as_str()],
        )
        .unwrap();
    assert!(matches!(
        store.scope_revoked(&scope),
        Err(StoreError::Sqlite(_))
    ));
}

#[test]
fn scope_native_projection_reuses_original_budget_and_terminal_callback() {
    let mut store = ControlStore::open_in_memory().unwrap();
    let (scope, path) = native_scope(&mut store);
    let locator = store.scope(&scope).unwrap().root;
    let mut remaining = ("native_path".len() + locator.raw_b64.len() + 8) as u64;
    let mut admit = |raw, _, _| {
        remaining = remaining
            .checked_sub(raw)
            .ok_or(StoreError::BudgetExceeded)?;
        Ok(())
    };
    assert_eq!(
        store
            .scope_native_root_with_admission(&scope, &mut admit)
            .unwrap(),
        path
    );
    assert!(matches!(
        store.scope_native_root_with_admission(&scope, &mut admit),
        Err(StoreError::BudgetExceeded)
    ));
    assert_eq!(remaining, 0);
    let mut admitted = false;
    assert!(matches!(
        store.scope_native_root_with_admission(&scope, &mut |raw, _, _| {
            if raw > 0 {
                admitted = true;
                Ok(())
            } else if admitted {
                Err(StoreError::BudgetExceeded)
            } else {
                Ok(())
            }
        }),
        Err(StoreError::BudgetExceeded)
    ));
    assert!(admitted);
}
