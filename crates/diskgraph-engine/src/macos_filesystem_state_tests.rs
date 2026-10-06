use crate::macos_filesystem_state::MacosFilesystemState;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::os::fd::AsRawFd;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

unsafe extern "C" {
    fn diskgraph_macos_installation_metadata(fd: libc::c_int, output: *mut u8) -> libc::c_int;
}

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

/// 只创建并清理本次测试的临时文件，不改变系统安装或公共目录权限。
/// 来源：原生 Rust macOS 元数据准入测试；无 Java 对等对象。
struct FileFixture {
    directory: PathBuf,
    path: PathBuf,
    file: File,
}

impl FileFixture {
    fn new() -> Self {
        let directory = std::env::temp_dir().join(format!(
            "diskgraph_macos_metadata_{}_{}",
            std::process::id(),
            NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&directory).unwrap();
        let path = directory.join("material");
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        file.write_all(b"metadata fixture").unwrap();
        file.set_permissions(fs::Permissions::from_mode(0o600))
            .unwrap();
        Self {
            directory,
            path,
            file,
        }
    }
}

impl Drop for FileFixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.directory).unwrap();
    }
}

#[test]
fn system_root_directory_and_executable_have_supported_metadata_only() {
    let directory = File::open("/").unwrap();
    let state = MacosFilesystemState::capture(&directory, true).unwrap();
    assert_eq!(state.uid, 0);
    assert_eq!(state.mode & 0o170000, 0o040000);
    assert_ne!(state.volume_uuid, [0; 16]);

    let image = File::open("/usr/bin/true").unwrap();
    let state = MacosFilesystemState::capture(&image, false).unwrap();
    assert_eq!(state.uid, 0);
    assert_eq!(state.nlink, 1);
    assert_ne!(state.mode & 0o111, 0);
    assert_eq!(state.inode, image.metadata().unwrap().ino());
    // 系统镜像只用于真实 ABI/卷/ACL 元数据测试，不构成 fresh 安装证明。
}

#[test]
fn non_root_owned_file_is_rejected() {
    let fixture = FileFixture::new();
    if fixture.file.metadata().unwrap().uid() == 0 {
        // root CI 仅改变自有夹具的属主，使反控不依赖运行账户。
        let path = std::ffi::CString::new(fixture.path.as_os_str().as_encoded_bytes()).unwrap();
        assert_eq!(unsafe { libc::chown(path.as_ptr(), 501, 501) }, 0);
    }
    assert_ne!(fixture.file.metadata().unwrap().uid(), 0);
    assert!(MacosFilesystemState::capture(&fixture.file, false).is_err());
}

#[test]
fn object_kind_group_world_write_and_setid_are_rejected() {
    let fixture = FileFixture::new();
    assert!(MacosFilesystemState::capture(&fixture.file, true).is_err());
    assert!(MacosFilesystemState::capture(&File::open("/").unwrap(), false).is_err());
    for mode in [0o620, 0o602, 0o4600, 0o2600] {
        fixture
            .file
            .set_permissions(fs::Permissions::from_mode(mode))
            .unwrap();
        assert!(MacosFilesystemState::capture(&fixture.file, false).is_err());
    }
}

#[test]
fn hard_linked_regular_file_is_rejected() {
    let fixture = FileFixture::new();
    fs::hard_link(&fixture.path, fixture.directory.join("second_name")).unwrap();
    assert_eq!(fixture.file.metadata().unwrap().nlink(), 2);
    assert!(MacosFilesystemState::capture(&fixture.file, false).is_err());
}

#[test]
fn generic_regular_material_does_not_require_execute_permission() {
    let fixture = FileFixture::new();
    fixture
        .file
        .set_permissions(fs::Permissions::from_mode(0o444))
        .unwrap();
    if fixture.file.metadata().unwrap().uid() == 0 {
        let state = MacosFilesystemState::capture(&fixture.file, false).unwrap();
        assert_eq!(state.mode & 0o111, 0);
    } else {
        // 普通 UID 本机夹具不能自授 root 来源；正向由 root CI 实际运行。
        assert!(MacosFilesystemState::capture(&fixture.file, false).is_err());
    }
}

#[test]
fn held_descriptor_keeps_original_identity_after_name_replacement() {
    let fixture = FileFixture::new();
    let original_inode = fixture.file.metadata().unwrap().ino();
    fs::rename(&fixture.path, fixture.directory.join("original")).unwrap();
    fs::write(&fixture.path, b"replacement").unwrap();
    let replacement = File::open(&fixture.path).unwrap();
    assert_ne!(replacement.metadata().unwrap().ino(), original_inode);
    assert_eq!(fixture.file.metadata().unwrap().ino(), original_inode);
    if fixture.file.metadata().unwrap().uid() == 0 {
        let state = MacosFilesystemState::capture(&fixture.file, false).unwrap();
        assert_eq!(state.inode, original_inode);
        assert_eq!(state.len, b"metadata fixture".len() as u64);
    } else {
        // 此失败只能证明 owner 门禁；不得称普通用户夹具已通过正向原 FD 准入。
        assert!(MacosFilesystemState::capture(&fixture.file, false).is_err());
    }
}

#[test]
fn native_acl_adapter_rejects_mutation_allow_without_ignoring_deny_or_read_only_acl() {
    let fixture = FileFixture::new();
    let capture_native = || {
        let mut output = [0_u8; 24];
        // 仅测试原生卷/ACL适配器；普通UID夹具仍不能绕过Rust的root owner门禁。
        unsafe {
            diskgraph_macos_installation_metadata(fixture.file.as_raw_fd(), output.as_mut_ptr())
        }
    };
    assert_eq!(capture_native(), 0);
    for (entry, accepted) in [
        ("everyone allow read", true),
        ("everyone deny write", true),
        ("everyone allow write", false),
        ("everyone allow writeattr", false),
        ("everyone allow writesecurity", false),
        ("everyone allow delete", false),
    ] {
        assert!(
            std::process::Command::new("/bin/chmod")
                .args(["+a", entry])
                .arg(&fixture.path)
                .status()
                .unwrap()
                .success()
        );
        assert_eq!(capture_native() == 0, accepted, "ACL {entry}");
        assert!(
            std::process::Command::new("/bin/chmod")
                .arg("-N")
                .arg(&fixture.path)
                .status()
                .unwrap()
                .success()
        );
    }
}

#[test]
fn directory_binding_tolerates_child_bookkeeping_but_preserves_identity_and_security() {
    let directory = File::open("/").unwrap();
    let original = MacosFilesystemState::capture(&directory, true).unwrap();
    let mut current = MacosFilesystemState::capture(&directory, true).unwrap();
    // 状态合同测试：模拟无关子项增删；实际 root 目录变更仍需专用平台夹具。
    current.len = original.len.saturating_add(4096);
    current.nlink = original.nlink.saturating_add(1);
    current.mtime_seconds = original.mtime_seconds + 1;
    current.ctime_seconds = original.ctime_seconds + 1;
    assert!(original.same_directory_binding(&current));
    assert_ne!(original, current);
    for field in [
        "volume", "fsid", "device", "inode", "birth", "birth_ns", "mode", "uid", "gid",
    ] {
        let mut changed = MacosFilesystemState::capture(&directory, true).unwrap();
        match field {
            "volume" => changed.volume_uuid[0] ^= 1,
            "fsid" => changed.fsid[0] ^= 1,
            "device" => changed.device ^= 1,
            "inode" => changed.inode ^= 1,
            "birth" => changed.birth_seconds += 1,
            "birth_ns" => changed.birth_nanoseconds ^= 1,
            "mode" => changed.mode ^= 0o100,
            "uid" => changed.uid ^= 1,
            "gid" => changed.gid ^= 1,
            _ => unreachable!(),
        }
        assert!(!original.same_directory_binding(&changed), "{field}");
    }
}
