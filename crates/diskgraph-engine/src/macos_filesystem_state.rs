use crate::EngineError;
use diskgraph_core::BusinessError;
use std::fs::{File, Metadata};
use std::os::fd::AsRawFd;
use std::os::macos::fs::MetadataExt as MacosMetadataExt;
use std::os::unix::fs::MetadataExt;

unsafe extern "C" {
    fn diskgraph_macos_installation_metadata(fd: libc::c_int, output: *mut u8) -> libc::c_int;
}

/// 原文件句柄当前安全元数据与卷身份的快照，不证明安装历史或授予执行许可。
/// 来源：原生 Rust PF-06 macOS 安装原生租约合同；无 Java 对等对象。
#[derive(Debug, PartialEq, Eq)]
pub(super) struct MacosFilesystemState {
    pub(super) volume_uuid: [u8; 16],
    pub(super) fsid: [i32; 2],
    pub(super) device: u64,
    pub(super) inode: u64,
    pub(super) birth_seconds: i64,
    pub(super) birth_nanoseconds: u32,
    pub(super) mtime_seconds: i64,
    pub(super) mtime_nanoseconds: u32,
    pub(super) ctime_seconds: i64,
    pub(super) ctime_nanoseconds: u32,
    pub(super) len: u64,
    pub(super) mode: u32,
    pub(super) uid: u32,
    pub(super) gid: u32,
    pub(super) nlink: u64,
}

impl MacosFilesystemState {
    /// 比较目录长期名称绑定；参数：为再次捕获的安全状态，返回：绑定是否保持一致。
    /// 每次捕获仍核验实时 ACL 与捕获前后完整元数据。
    pub(super) fn same_directory_binding(&self, current: &Self) -> bool {
        // 子项增删会改变目录记账字段，但不能改变原卷、对象或安全边界。
        self.volume_uuid == current.volume_uuid
            && self.fsid == current.fsid
            && self.device == current.device
            && self.inode == current.inode
            && self.birth_seconds == current.birth_seconds
            && self.birth_nanoseconds == current.birth_nanoseconds
            && self.mode == current.mode
            && self.uid == current.uid
            && self.gid == current.gid
    }

    /// 从原 FD 获取安全对象与卷身份，不重新解析文件名称。
    /// 参数：file 为原已打开句柄，directory 区分目录与一般 regular 材料。
    /// 返回：root 所有、无 group/world 写入和 setid、ACL/卷支持明确的稳定快照。
    /// 通用 regular 材料不要求执行位；镜像 lease 须另行核验，不得把本结果当 exec permit。
    pub(super) fn capture(file: &File, directory: bool) -> Result<Self, EngineError> {
        let before = Self::from_metadata(&file.metadata()?, directory)?;
        let mut native = [0_u8; 24];
        // 安全性：原 FD 在调用期间由借用 File 保活；C ABI 只写固定 24 字节，资源在 C 内回收。
        let result =
            unsafe { diskgraph_macos_installation_metadata(file.as_raw_fd(), native.as_mut_ptr()) };
        if result != 0 {
            if result == libc::ENOTSUP {
                // 原生能力或保真条件明确不支持；其余实际 OS 错误仍保留原 errno。
                return Err(BusinessError::Unsupported.into());
            }
            return Err(std::io::Error::from_raw_os_error(result).into());
        }
        let mut after = Self::from_metadata(&file.metadata()?, directory)?;
        if before != after {
            return Err(BusinessError::Conflict.into());
        }
        after.fsid = [
            i32::from_ne_bytes(native[..4].try_into().expect("fixed fsid first component")),
            i32::from_ne_bytes(
                native[4..8]
                    .try_into()
                    .expect("fixed fsid second component"),
            ),
        ];
        after.volume_uuid.copy_from_slice(&native[8..]);
        Ok(after)
    }

    fn from_metadata(metadata: &Metadata, directory: bool) -> Result<Self, EngineError> {
        if metadata.uid() != 0
            || metadata.mode() & 0o0022 != 0
            || metadata.mode() & 0o6000 != 0
            || (directory && !metadata.is_dir())
            || (!directory && (!metadata.is_file() || metadata.nlink() != 1))
        {
            return Err(BusinessError::Unsupported.into());
        }
        Ok(Self {
            volume_uuid: [0; 16],
            fsid: [0; 2],
            device: metadata.dev(),
            inode: metadata.ino(),
            birth_seconds: metadata.st_birthtime(),
            birth_nanoseconds: nanoseconds(metadata.st_birthtime_nsec())?,
            mtime_seconds: metadata.mtime(),
            mtime_nanoseconds: nanoseconds(metadata.mtime_nsec())?,
            ctime_seconds: metadata.ctime(),
            ctime_nanoseconds: nanoseconds(metadata.ctime_nsec())?,
            len: metadata.len(),
            mode: metadata.mode(),
            uid: metadata.uid(),
            gid: metadata.gid(),
            nlink: metadata.nlink(),
        })
    }
}

fn nanoseconds(value: i64) -> Result<u32, EngineError> {
    if !(0..1_000_000_000).contains(&value) {
        return Err(BusinessError::Unsupported.into());
    }
    Ok(value as u32)
}
