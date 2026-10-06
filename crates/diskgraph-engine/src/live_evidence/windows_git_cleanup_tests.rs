//! 实际生产Git目录创建、账本及句柄清理，不替代Windows原生执行证据。
use super::ProbeLimits;
use super::git_private_directory::GitPrivateDirectory;
use super::probe_budget::ProbeBudget;
use std::time::{Duration, Instant};

#[test]
fn product_cleanup_removes_moved_original_and_keeps_foreign_replacement() {
    let mut probe = ProbeBudget::new(&ProbeLimits::default()).unwrap();
    let mut directory = GitPrivateDirectory::with_limits(128 << 20, 0, &mut probe).unwrap();
    let original = directory.path().to_owned();
    let nested = original.join("nested");
    directory.create_dir_all(&nested, &mut probe).unwrap();
    directory
        .write(&nested.join("owned"), b"original", &mut probe)
        .unwrap();
    let moved = original.with_extension("moved");
    std::fs::rename(&original, &moved).unwrap();
    std::fs::create_dir(&original).unwrap();
    std::fs::write(original.join("foreign"), b"foreign payload").unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        match directory.complete::<()>(Ok(())) {
            Ok(()) => break,
            Err(error) => {
                assert!(
                    Instant::now() < deadline,
                    "original cleanup remains incomplete: {error}"
                );
                assert_eq!(
                    std::fs::read(original.join("foreign")).unwrap(),
                    b"foreign payload"
                );
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    }
    assert_eq!(
        std::fs::symlink_metadata(&moved).unwrap_err().kind(),
        std::io::ErrorKind::NotFound
    );
    assert_eq!(
        std::fs::read(original.join("foreign")).unwrap(),
        b"foreign payload"
    );
    directory.complete::<()>(Ok(())).unwrap();
    std::fs::remove_file(original.join("foreign")).unwrap();
    std::fs::remove_dir(original).unwrap();
}

#[test]
fn product_cleanup_keeps_unregistered_foreign_children_and_original_owner() {
    let mut probe = ProbeBudget::new(&ProbeLimits::default()).unwrap();
    let mut directory = GitPrivateDirectory::with_limits(128 << 20, 0, &mut probe).unwrap();
    let root = directory.path().to_owned();
    let foreign = root.join("foreign");
    std::fs::write(&foreign, b"not registered").unwrap();
    assert!(directory.complete::<()>(Ok(())).is_err());
    assert_eq!(std::fs::read(&foreign).unwrap(), b"not registered");
    // 原owner不收养外来对象；本测试移除其自行注入的文件后仍保留原枚举项，不能冒充完成。
    std::fs::remove_file(foreign).unwrap();
    assert!(directory.complete::<()>(Ok(())).is_err());
    drop(directory);
    std::fs::remove_dir(root).unwrap();
}
