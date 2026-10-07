//! Linux 原生监督状态域验证；来源：PF-06。只操作隔离夹具。
use super::{LinuxSupervisorNamespace, LinuxSupervisorTrust, SlotError};
use std::path::Path;
use std::time::{Duration, Instant};

#[test]
fn user_owned_namespace_cannot_grant_supervisor_capacity() {
    let directory = tempfile::tempdir().unwrap();
    assert!(matches!(
        LinuxSupervisorNamespace::from_host(
            directory.path(),
            fixture_trust(directory.path()),
            1001,
            Instant::now() + Duration::from_secs(5)
        ),
        Err(SlotError::Unsupported)
    ));
}

#[test]
#[ignore = "requires root-provisioned isolated /fixture/supervisor_namespace; run explicitly"]
fn root_provisioned_namespace_preserves_original_capacity_and_rejects_aliases() {
    let deadline = Instant::now() + Duration::from_secs(5);
    let root = Path::new("/fixture/supervisor_namespace");
    let namespace = LinuxSupervisorNamespace::from_host(
        &root.join("good"),
        fixture_trust(&root.join("good")),
        1001,
        deadline,
    )
    .unwrap();
    let reserved = namespace.reserve(deadline).unwrap();
    assert!(matches!(namespace.reserve(deadline), Err(SlotError::Busy)));
    reserved.abort_before_birth(deadline).unwrap();
    let active = namespace
        .reserve(deadline)
        .unwrap()
        .activate(deadline)
        .unwrap();
    drop(active);
    assert!(matches!(
        namespace.reserve(deadline),
        Err(SlotError::Unconfirmed)
    ));
    let uid = unsafe { libc::geteuid() };
    assert_eq!(
        std::fs::read(root.join("good").join(format!("uid_{uid}.slot"))).unwrap(),
        b"DGSL01A\n"
    );
    // 另一份同样 root 保护的目录也不能替代独立宿主绑定的原永久域。
    assert!(matches!(
        LinuxSupervisorNamespace::from_host(
            &root.join("parallel"),
            fixture_trust(&root.join("good")),
            1001,
            deadline,
        ),
        Err(SlotError::Unsupported)
    ));
    assert!(
        std::fs::read(root.join("parallel").join(format!("uid_{uid}.slot")))
            .unwrap()
            .is_empty()
    );
    for case in ["symlink", "writable", "user_owned"] {
        assert!(
            LinuxSupervisorNamespace::from_host(
                &root.join(case),
                fixture_trust(&root.join(case)),
                1001,
                deadline
            )
            .is_err(),
            "accepted namespace {case}"
        );
    }
    for case in [
        "linked_slot",
        "symlink_slot",
        "wrong_owner",
        "writable_slot",
        "missing_slot",
    ] {
        let namespace = LinuxSupervisorNamespace::from_host(
            &root.join(case),
            fixture_trust(&root.join(case)),
            1001,
            deadline,
        )
        .unwrap();
        assert!(namespace.reserve(deadline).is_err(), "accepted slot {case}");
    }
}

// 测试明确从已预置夹具取得预期，不模拟产品签名/宿主启动材料。
fn fixture_trust(root: &Path) -> LinuxSupervisorTrust {
    use std::fs::File;
    LinuxSupervisorTrust::from_host(
        File::open(root).unwrap(),
        File::open("/proc/thread-self/ns/user").unwrap(),
        File::open("/proc/thread-self/ns/mnt").unwrap(),
    )
}

#[test]
fn foreign_namespace_material_is_rejected_before_path_resolution() {
    let trust = LinuxSupervisorTrust::from_host(
        std::fs::File::open("/").unwrap(),
        std::fs::File::open("/proc/thread-self/ns/mnt").unwrap(),
        std::fs::File::open("/proc/thread-self/ns/user").unwrap(),
    );
    assert!(matches!(
        LinuxSupervisorNamespace::from_host(
            Path::new("/nonexistent_supervisor_fixture"),
            trust,
            1001,
            Instant::now() + Duration::from_secs(5)
        ),
        Err(SlotError::Unsupported)
    ));
}

#[test]
#[ignore = "requires root-provisioned isolated tamper fixture; same UID must be refused before reservation"]
fn same_uid_frontend_cannot_forge_clean_after_unconfirmed_supervisor() {
    let deadline = Instant::now() + Duration::from_secs(5);
    let root = Path::new("/fixture/supervisor_namespace/tamper");
    let uid = unsafe { libc::geteuid() };
    let path = root.join(format!("uid_{uid}.slot"));
    let original = std::fs::read(&path).unwrap();
    let result = LinuxSupervisorNamespace::from_host(root, fixture_trust(root), uid, deadline);
    assert!(
        matches!(result, Err(SlotError::Unsupported)),
        "same UID frontend must not receive authoritative service capacity"
    );
    assert_eq!(std::fs::read(path).unwrap(), original);
}

#[test]
#[ignore = "requires an isolated root process and root-provisioned role_change fixture"]
fn service_identity_cannot_follow_an_effective_uid_change() {
    assert_eq!(unsafe { libc::geteuid() }, 0);
    let root = Path::new("/fixture/supervisor_namespace/role_change");
    let deadline = Instant::now() + Duration::from_secs(5);
    let namespace =
        LinuxSupervisorNamespace::from_host(root, fixture_trust(root), 1001, deadline).unwrap();
    let slot = root.join("uid_0.slot");
    let original = std::fs::read(&slot).unwrap();
    // 只在单项隔离进程内改变真实有效身份；不模拟字段或以文件权限错误冒充角色拒绝。
    assert_eq!(unsafe { libc::seteuid(1000) }, 0);
    let result = namespace.reserve(deadline);
    assert_eq!(unsafe { libc::seteuid(0) }, 0);
    assert!(matches!(result, Err(SlotError::Unsupported)));
    assert_eq!(std::fs::read(slot).unwrap(), original);
}
