//! 实际NTFS通知原身份/最后关闭时机实验；不以成员移除宣称物理容量释放。
use super::windows_directory_notification_io::WindowsDirectoryNotificationIo;
use std::fs::{File, OpenOptions};
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::AsRawHandle;
use std::time::{Duration, Instant};
use windows_sys::Win32::Storage::FileSystem::{
    FILE_DISPOSITION_INFO, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
    FILE_FLAG_OVERLAPPED, FILE_ID_INFO, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    FileDispositionInfo, FileIdInfo, GetFileInformationByHandleEx, SetFileInformationByHandle,
};

fn identity(file: &File) -> u64 {
    let mut id = FILE_ID_INFO::default();
    assert_ne!(
        unsafe {
            GetFileInformationByHandleEx(
                file.as_raw_handle(),
                FileIdInfo,
                std::ptr::addr_of_mut!(id).cast(),
                std::mem::size_of::<FILE_ID_INFO>() as u32,
            )
        },
        0
    );
    assert_eq!(
        &id.FileId.Identifier[8..],
        &[0; 8],
        "no truncated extended identity"
    );
    u64::from_le_bytes(id.FileId.Identifier[..8].try_into().unwrap())
}

fn open_parent(path: &std::path::Path) -> File {
    OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OVERLAPPED | FILE_FLAG_OPEN_REPARSE_POINT,
        )
        .open(path)
        .unwrap()
}

fn removed(bytes: &[u8], child: u64, parent: u64) -> bool {
    let mut offset = 0usize;
    let mut found = false;
    loop {
        let record = bytes.get(offset..).unwrap();
        assert!(record.len() >= 84, "short notification record");
        let u32_at = |n| u32::from_le_bytes(record[n..n + 4].try_into().unwrap());
        let u64_at = |n| u64::from_le_bytes(record[n..n + 8].try_into().unwrap());
        let next = u32_at(0) as usize;
        let length = u32_at(80) as usize;
        assert!(length > 0 && length.is_multiple_of(2) && length <= record.len() - 84);
        println!(
            "DG_NOTIFY action={} child={} parent={} bytes={}",
            u32_at(4),
            u64_at(64),
            u64_at(72),
            length
        );
        // OLD/NEW rename及同名陌生成员不能证明原对象移除。
        found |= u32_at(4) == 2 && u64_at(64) == child && u64_at(72) == parent;
        if next == 0 {
            return found;
        }
        assert!(next.is_multiple_of(4) && next >= 84 + length && next < record.len());
        offset = offset.checked_add(next).unwrap();
    }
}

fn observe(
    probe: &mut WindowsDirectoryNotificationIo,
    child: u64,
    parent: u64,
    duration: Duration,
) -> bool {
    let deadline = Instant::now() + duration;
    while Instant::now() < deadline {
        if let Some(bytes) = probe.poll().unwrap() {
            if removed(&bytes, child, parent) {
                return true;
            }
            probe.arm().unwrap();
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    false
}

fn cancel(probe: &mut WindowsDirectoryNotificationIo) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !probe.poll_cancel().unwrap() {
        assert!(
            Instant::now() < deadline,
            "original notification I/O still active"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn original_id_removal_notification_waits_for_last_external_close() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("owned");
    std::fs::write(&path, b"original").unwrap();
    let external = OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .open(&path)
        .unwrap();
    let child = identity(&external);
    let directory = open_parent(temp.path());
    let parent = identity(&directory);
    let mut probe = WindowsDirectoryNotificationIo::new(directory).unwrap();
    let deleting = OpenOptions::new()
        .access_mode(0x00010000)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .open(&path)
        .unwrap();
    let disposition = FILE_DISPOSITION_INFO { DeleteFile: true };
    assert_ne!(
        unsafe {
            SetFileInformationByHandle(
                deleting.as_raw_handle(),
                FileDispositionInfo,
                std::ptr::addr_of!(disposition).cast(),
                std::mem::size_of_val(&disposition) as u32,
            )
        },
        0
    );
    drop(deleting);
    assert!(
        !observe(&mut probe, child, parent, Duration::from_millis(200)),
        "remove event before external last close cannot certify final deletion"
    );
    drop(external);
    assert!(
        observe(&mut probe, child, parent, Duration::from_secs(5)),
        "missing original identity remove event after last close"
    );
    cancel(&mut probe);
    println!("DG_NOTIFY_ORIGINAL_ID_LAST_CLOSE=1");
}

#[test]
fn rename_and_foreign_same_name_removal_do_not_match_original_id() {
    let temp = tempfile::tempdir().unwrap();
    let original = temp.path().join("owned");
    let moved = temp.path().join("moved");
    std::fs::write(&original, b"original").unwrap();
    let file = OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .open(&original)
        .unwrap();
    let child = identity(&file);
    let directory = open_parent(temp.path());
    let parent = identity(&directory);
    let mut probe = WindowsDirectoryNotificationIo::new(directory).unwrap();
    std::fs::rename(&original, &moved).unwrap();
    std::fs::write(&original, b"foreign").unwrap();
    std::fs::remove_file(&original).unwrap();
    assert!(
        !observe(&mut probe, child, parent, Duration::from_millis(200)),
        "rename or foreign same-name removal matched original identity"
    );
    drop(file);
    std::fs::remove_file(&moved).unwrap();
    assert!(observe(&mut probe, child, parent, Duration::from_secs(5)));
    cancel(&mut probe);
    println!("DG_NOTIFY_RENAME_AND_FOREIGN_ID_REFUSED=1");
}
