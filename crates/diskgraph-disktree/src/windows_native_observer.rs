use diskgraph_core::FileIdentity;

/// 将 Windows 原生身份投影到兼容字段；卷身份来自内核，不来自盘符。
fn qualified_identity(volume: u64, identifier: [u8; 16]) -> Option<FileIdentity> {
    // ReFS 等 128 位身份不可截断，兼容字段不足时保留 unknown。
    if identifier[8..].iter().any(|byte| *byte != 0) {
        return None;
    }
    Some(FileIdentity {
        volume_id: volume_key(volume),
        file_id: u64::from_le_bytes(identifier[..8].try_into().ok()?),
    })
}

/// 卷键来自完整卷序号；与旧文件 ID 是否能无损投影无关。
fn volume_key(volume: u64) -> String {
    format!("windows-volume-{volume:016x}")
}

/// 独立读取根卷序号，128 位文件 ID 不可投影时仍保留卷事实。
/// 参数：path 为当前扫描根；返回：实际内核卷键或属性读取失败，不从盘符推断。
#[cfg(windows)]
pub(crate) fn observe_volume(path: &std::path::Path) -> std::io::Result<String> {
    use std::os::windows::{fs::OpenOptionsExt, io::AsRawHandle};
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_NO_RECALL, FILE_FLAG_OPEN_REPARSE_POINT,
        FILE_ID_INFO, FILE_READ_ATTRIBUTES, FileIdInfo, GetFileInformationByHandleEx,
    };
    let handle = std::fs::OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES)
        .custom_flags(
            FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_NO_RECALL,
        )
        .open(path)?;
    let mut info = FILE_ID_INFO::default();
    // 即便 ID 的高位非零，VolumeSerialNumber 仍为独立原生事实；不截断 ID。
    if unsafe {
        GetFileInformationByHandleEx(
            handle.as_raw_handle(),
            FileIdInfo,
            (&mut info as *mut FILE_ID_INFO).cast(),
            std::mem::size_of::<FILE_ID_INFO>() as u32,
        )
    } == 0
    {
        return Err(std::io::Error::last_os_error());
    }
    Ok(volume_key(info.VolumeSerialNumber))
}

/// 只用属性权限打开重解析点本身；身份与修改时间从同一句柄观察。
#[cfg(windows)]
pub(crate) fn observe(
    path: &std::path::Path,
) -> std::io::Result<(Option<FileIdentity>, Option<i64>)> {
    use std::os::windows::{fs::OpenOptionsExt, io::AsRawHandle};
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_NO_RECALL, FILE_FLAG_OPEN_REPARSE_POINT,
        FILE_ID_INFO, FILE_READ_ATTRIBUTES, FileIdInfo, GetFileInformationByHandleEx,
    };
    let handle = std::fs::OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES)
        .custom_flags(
            FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_NO_RECALL,
        )
        .open(path)?;
    let mut info = FILE_ID_INFO::default();
    // 缓冲区拥有正确 ABI 大小，借用句柄在调用期间有效，内容读取权限未请求。
    let success = unsafe {
        GetFileInformationByHandleEx(
            handle.as_raw_handle(),
            FileIdInfo,
            (&mut info as *mut FILE_ID_INFO).cast(),
            std::mem::size_of::<FILE_ID_INFO>() as u32,
        )
    };
    let identity = (success != 0)
        .then(|| qualified_identity(info.VolumeSerialNumber, info.FileId.Identifier))
        .flatten();
    let modified = handle
        .metadata()?
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .and_then(|duration| i64::try_from(duration.as_secs()).ok());
    Ok((identity, modified))
}

#[cfg(test)]
mod tests {
    use super::qualified_identity;

    #[test]
    fn native_identity_never_truncates_a_128_bit_file_id() {
        let mut identifier = [0; 16];
        identifier[..8].copy_from_slice(&42u64.to_le_bytes());
        assert_eq!(qualified_identity(7, identifier).unwrap().file_id, 42);
        identifier[15] = 1;
        assert!(qualified_identity(7, identifier).is_none());
        assert_eq!(super::volume_key(7), "windows-volume-0000000000000007");
        assert_eq!(
            super::volume_key(u64::MAX),
            "windows-volume-ffffffffffffffff"
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_native_hardlinks_share_identity_and_replacement_does_not() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source.bin");
        let link = root.path().join("hardlink.bin");
        std::fs::write(&source, b"old").unwrap();
        std::fs::hard_link(&source, &link).unwrap();
        let (before, modified) = super::observe(&source).unwrap();
        assert!(before.is_some(), "NTFS fixture must report native identity");
        assert_eq!(
            super::observe_volume(&source).unwrap(),
            before.as_ref().unwrap().volume_id
        );
        assert!(modified.is_some());
        assert_eq!(before, super::observe(&link).unwrap().0);
        std::fs::remove_file(&source).unwrap();
        std::fs::write(&source, b"new").unwrap();
        assert_ne!(before, super::observe(&source).unwrap().0);
        let graph = crate::scan_native_v2(
            root.path(),
            diskgraph_disktree_core::scan::ScanOptions::default(),
        )
        .unwrap();
        assert_eq!(
            graph.snapshot.volume_id,
            before.map(|identity| identity.volume_id)
        );
        assert!(graph.nodes.iter().all(|node| node.identity.is_some()));
    }
}
