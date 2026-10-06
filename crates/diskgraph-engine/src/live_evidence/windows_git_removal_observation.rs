use super::git_private_allocation::GitPrivateAllocation;
use super::probe_budget::ProbeBudget;
use super::windows_git_native_id_protocol::WindowsGitNativeIdProtocol;
use crate::native_child::WindowsDirectoryNotificationIo;
use std::fs::File;
use std::io;
use std::os::windows::io::{AsRawHandle, FromRawHandle};
use windows_sys::Wdk::Foundation::OBJECT_ATTRIBUTES;
use windows_sys::Wdk::Storage::FileSystem::{
    FILE_OPEN_NO_RECALL, FILE_OPEN_REPARSE_POINT, NtOpenFile,
};
use windows_sys::Win32::Foundation::{
    INVALID_HANDLE_VALUE, OBJ_DONT_REPARSE, RtlNtStatusToDosError, UNICODE_STRING,
};
use windows_sys::Win32::Storage::FileSystem::{
    FILE_LIST_DIRECTORY, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
};

/// 删除前订阅的原父/子身份观察owner；来源：NTFS扩展目录通知与PF-06，无Java对应。
/// 只证明原成员移除，不测量物理释放空间；未知/丢失记录保留原责任。
pub(super) struct WindowsGitRemovalObservation {
    parent: GitPrivateAllocation,
    child_id: [u8; 16],
    parent_id: [u8; 16],
    io: Option<WindowsDirectoryNotificationIo>,
    armed: bool,
    lost: bool,
    observed: bool,
}

impl WindowsGitRemovalObservation {
    /// 参数：parent为原持有父、child为原已核身份、probe为整次预算、owner为外槽。
    /// 返回：删除前订阅结果；取得异步责任后任何失败均保留原外槽。
    pub(super) fn prepare_into(
        parent: &File,
        child: &GitPrivateAllocation,
        probe: &mut ProbeBudget,
        owner: &mut Option<Self>,
    ) -> io::Result<()> {
        probe.check().map_err(io::Error::other)?;
        if owner.is_some() {
            return Err(io::Error::other("removal observation already prepared"));
        }
        let identity = GitPrivateAllocation::from_file(parent).map_err(io::Error::other)?;
        if !identity.is_directory() || !identity.same_volume(child) {
            return Err(io::Error::other("notification parent type/volume mismatch"));
        }
        let protocol = WindowsGitNativeIdProtocol::from_hint(parent)?;
        if !matches!(protocol, WindowsGitNativeIdProtocol::NtfsFileReference) {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "extended identity notifications require NTFS",
            ));
        }
        let parent_id = full_id(&identity);
        let child_id = full_id(child);
        protocol.byte_length(&parent_id)?;
        protocol.byte_length(&child_id)?;
        let directory = reopen_async(parent)?;
        if !identity
            .same_identity(&GitPrivateAllocation::from_file(&directory).map_err(io::Error::other)?)
        {
            return Err(io::Error::other(
                "original notification parent identity changed",
            ));
        }
        probe.check().map_err(io::Error::other)?;
        *owner = Some(Self {
            parent: identity,
            child_id,
            parent_id,
            io: None,
            armed: false,
            lost: false,
            observed: false,
        });
        let observation = owner.as_mut().expect("stored original removal observation");
        WindowsDirectoryNotificationIo::prepare_into(directory, &mut observation.io)?;
        observation.armed = true;
        probe.check().map_err(io::Error::other)
    }

    /// 参数：child为当前原身份、probe为本轮预算；返回：删除前已有原订阅且身份未漂移。
    pub(super) fn check_before_delete(
        &self,
        child: &GitPrivateAllocation,
        probe: &mut ProbeBudget,
    ) -> io::Result<()> {
        probe.check().map_err(io::Error::other)?;
        if !self.armed || self.lost || self.observed || self.child_id != full_id(child) {
            return Err(io::Error::other(
                "removal observation is not eligible for original deletion",
            ));
        }
        self.check_parent()?;
        probe.check().map_err(io::Error::other)
    }

    /// 参数：child为原登记身份、probe为本轮预算；返回：完整原通知且原I/O完成后true。
    /// 无通知时保留原ID查询的正控、pending及未知错误；87永不成为删除证明。
    pub(super) fn confirm(
        &mut self,
        child: &GitPrivateAllocation,
        probe: &mut ProbeBudget,
    ) -> io::Result<bool> {
        probe.check().map_err(io::Error::other)?;
        if !self.armed || self.lost || full_id(child) != self.child_id {
            return Err(io::Error::other(
                "removal notification lost or identity changed",
            ));
        }
        self.check_parent()?;
        if self.observed {
            probe.check().map_err(io::Error::other)?;
            return Ok(true);
        }
        let original = self.io.as_mut().expect("armed original notification owner");
        let record = match original.poll() {
            Ok(record) => record,
            Err(error) => {
                self.lost = error.kind() == io::ErrorKind::InvalidData
                    || error.raw_os_error() == Some(1022);
                return Err(error);
            }
        };
        if let Some(record) = record {
            probe.consume(record.len()).map_err(io::Error::other)?;
            let matched = match matches_removal(&record, self.child_id, self.parent_id) {
                Ok(matched) => matched,
                Err(error) => {
                    self.lost = true;
                    return Err(error);
                }
            };
            #[cfg(test)]
            eprintln!(
                "DG_ORIGINAL_REMOVAL_PAGE bytes={}; matched={matched}; child={:02x?}; parent={:02x?}",
                record.len(),
                self.child_id,
                self.parent_id
            );
            if matched {
                // 当前页完整验证后才锁存；poll已经确认原I/O完成，无新pending订阅。
                self.observed = true;
                probe.check().map_err(io::Error::other)?;
                return Ok(true);
            }
            if let Err(error) = original.arm() {
                self.lost = true;
                return Err(error);
            }
        }
        let result =
            super::windows_git_deletion_witness::WindowsGitDeletionWitness::confirm_absent(
                original.directory(),
                child,
                probe,
            )?;
        if result {
            return Err(io::Error::other(
                "original ID absent without original removal notification",
            ));
        }
        probe.check().map_err(io::Error::other)?;
        Ok(false)
    }

    fn check_parent(&self) -> io::Result<()> {
        let original = self
            .io
            .as_ref()
            .ok_or_else(|| io::Error::other("original notification I/O unavailable"))?;
        if !self.parent.same_identity(
            &GitPrivateAllocation::from_file(original.directory()).map_err(io::Error::other)?,
        ) {
            return Err(io::Error::other("held notification parent changed"));
        }
        Ok(())
    }
}

fn full_id(identity: &GitPrivateAllocation) -> [u8; 16] {
    unsafe {
        identity
            .windows_file_id_descriptor()
            .Anonymous
            .ExtendedFileId
            .Identifier
    }
}

// 仅相对原父句柄空名称重开为异步；不使用路径，也不复制同步句柄冒充异步句柄。
fn reopen_async(parent: &File) -> io::Result<File> {
    let name = UNICODE_STRING::default();
    let attributes = OBJECT_ATTRIBUTES {
        Length: std::mem::size_of::<OBJECT_ATTRIBUTES>() as u32,
        RootDirectory: parent.as_raw_handle(),
        ObjectName: &name,
        Attributes: OBJ_DONT_REPARSE,
        ..OBJECT_ATTRIBUTES::default()
    };
    let mut handle = std::ptr::null_mut();
    let mut status_block = windows_sys::Win32::System::IO::IO_STATUS_BLOCK::default();
    let status = unsafe {
        NtOpenFile(
            &mut handle,
            FILE_LIST_DIRECTORY | FILE_READ_ATTRIBUTES,
            &attributes,
            &mut status_block,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            FILE_OPEN_REPARSE_POINT | FILE_OPEN_NO_RECALL,
        )
    };
    let file = (!handle.is_null() && handle != INVALID_HANDLE_VALUE)
        .then(|| unsafe { File::from_raw_handle(handle) });
    if status != 0 {
        return Err(io::Error::from_raw_os_error(unsafe {
            RtlNtStatusToDosError(status) as i32
        }));
    }
    file.ok_or_else(|| io::Error::other("async original parent reopen returned no handle"))
}

/// 参数：bytes为完整页、child/parent为完整已核NTFS身份；返回：整页合法且原REMOVE匹配时true。
/// 匹配前后任何坏记录均失败，不按同名或rename事件消费原子项。
pub(super) fn matches_removal(bytes: &[u8], child: [u8; 16], parent: [u8; 16]) -> io::Result<bool> {
    if child[8..] != [0; 8] || parent[8..] != [0; 8] || child == [0; 16] || parent == [0; 16] {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "notification identity cannot be represented without loss",
        ));
    }
    let mut offset = 0usize;
    let mut matched = false;
    loop {
        let record = bytes
            .get(offset..)
            .filter(|r| r.len() >= 84)
            .ok_or_else(|| io::Error::other("short removal notification"))?;
        let field = |at| {
            u32::from_le_bytes(
                record[at..at + 4]
                    .try_into()
                    .expect("checked record header"),
            )
        };
        let next = field(0) as usize;
        let action = field(4);
        let length = field(80) as usize;
        if !(1..=5).contains(&action)
            || length == 0
            || length % 2 != 0
            || length > record.len() - 84
            || (next != 0 && (next % 4 != 0 || next < 84 + length || next >= record.len()))
        {
            return Err(io::Error::other("invalid removal notification record"));
        }
        matched |= action == 2 && record[64..72] == child[..8] && record[72..80] == parent[..8];
        if next == 0 {
            return Ok(matched);
        }
        offset = offset
            .checked_add(next)
            .ok_or_else(|| io::Error::other("notification offset overflow"))?;
    }
}
