//! Linux 原路径句柄重新打开的本地行为合同。
use std::io::Read;
use std::os::unix::fs::OpenOptionsExt;

#[test]
fn replaced_name_cannot_redirect_the_bound_data_open() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("file");
    std::fs::write(&path, b"original").unwrap();
    let original = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_PATH | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(&path)
        .unwrap();
    std::fs::rename(&path, dir.path().join("old")).unwrap();
    std::fs::write(&path, b"replacement").unwrap();
    let mut opened = crate::linux_no_recall_open::open_bound(&original).unwrap();
    let mut bytes = Vec::new();
    opened.read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes, b"original");
    assert_eq!(std::fs::read(path).unwrap(), b"replacement");
}

#[test]
fn path_only_link_never_becomes_a_data_handle() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("target");
    let link = dir.path().join("link");
    std::fs::write(&target, b"private").unwrap();
    std::os::unix::fs::symlink(&target, &link).unwrap();
    let original = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_PATH | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(link)
        .unwrap();
    assert!(matches!(
        crate::linux_no_recall_open::open_bound(&original),
        Err(crate::EngineError::Business(
            diskgraph_core::BusinessError::InvalidArgument
        ))
    ));
}
