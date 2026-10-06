//! 原目录移动不能被路径缺失伪装为已回收；使用真实文件系统，不替代原生句柄删除验收。
use super::ProbeLimits;
use super::git_private_allocation::GitPrivateAllocation;
use super::git_private_capacity::GitPrivateCapacity;
use super::git_private_directory_owner::GitPrivateDirectoryOwner;
use super::probe_budget::ProbeBudget;

#[test]
fn moved_original_directory_is_not_complete_when_original_path_is_missing() {
    let temporary = tempfile::tempdir().unwrap();
    // macOS 的 /var 是系统链接；先固定真实父目录，沿用原容量租约的逐组件无链接要求。
    let parent = temporary.path().canonicalize().unwrap();
    let root = parent.join("original");
    let moved = parent.join("moved");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("retained"), b"original payload").unwrap();
    let identity = GitPrivateAllocation::capture(&root).unwrap();
    let mut budget = ProbeBudget::new(&ProbeLimits::default()).unwrap();
    let capacity = GitPrivateCapacity::new(&root, 128 * 1024 * 1024, 0, &mut budget).unwrap();
    let mut owner = GitPrivateDirectoryOwner {
        path: root.clone(),
        cleaned: false,
        capacity: Some(capacity),
        root_identity: Some(identity),
        #[cfg(windows)]
        windows_cleanup: None,
    };
    std::fs::rename(&root, &moved).unwrap();
    let result = owner.cleanup();
    assert!(
        result.is_err(),
        "missing name does not prove original object deletion"
    );
    assert!(
        !owner.cleaned,
        "original recovery responsibility must remain"
    );
    assert!(owner.root_identity.is_some());
    assert!(
        owner.capacity.is_some(),
        "original allocation ledger must remain owned"
    );
    assert_eq!(
        std::fs::read(moved.join("retained")).unwrap(),
        b"original payload"
    );
    // 当前兼容实现只能在原身份回到原名称后恢复，不将这次重试冒充 handle-relative 能力。
    std::fs::rename(&moved, &root).unwrap();
    owner.cleanup().unwrap();
    assert!(owner.cleaned);
    assert!(!root.exists());
}

#[test]
fn foreign_replacement_is_retained_until_original_directory_returns() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("original");
    let moved = temporary.path().join("moved");
    std::fs::create_dir(&root).unwrap();
    let identity = GitPrivateAllocation::capture(&root).unwrap();
    let mut owner = GitPrivateDirectoryOwner {
        path: root.clone(),
        cleaned: false,
        capacity: None,
        root_identity: Some(identity),
        #[cfg(windows)]
        windows_cleanup: None,
    };
    std::fs::rename(&root, &moved).unwrap();
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("foreign"), b"must remain").unwrap();
    assert!(owner.cleanup().is_err());
    assert!(!owner.cleaned);
    assert_eq!(std::fs::read(root.join("foreign")).unwrap(), b"must remain");
    std::fs::remove_file(root.join("foreign")).unwrap();
    std::fs::remove_dir(&root).unwrap();
    std::fs::rename(&moved, &root).unwrap();
    owner.cleanup().unwrap();
    assert!(owner.cleaned);
}
