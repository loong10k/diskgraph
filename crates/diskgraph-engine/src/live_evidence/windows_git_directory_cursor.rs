use super::git_private_allocation::GitPrivateAllocation;
use super::probe_budget::ProbeBudget;
use std::ffi::OsString;
use std::fs::File;
use std::os::windows::ffi::OsStringExt;
use std::os::windows::io::AsRawHandle;
use windows_sys::Win32::Foundation::ERROR_NO_MORE_FILES;
use windows_sys::Win32::Storage::FileSystem::{
    FILE_ID_EXTD_DIR_INFO, FileIdExtdDirectoryInfo, FileIdExtdDirectoryRestartInfo,
    GetFileInformationByHandleEx,
};

const PAGE_BYTES: usize = 64 * 1024;

/// 原目录句柄的固定页枚举游标，保留完整128位子项ID和原UTF-16名称。
/// 来源：Windows FileIdExtdDirectoryInfo / PF-06；无 Java 对等对象。
/// 不解析路径，不扩容整树；单次内核调用没有硬墙钟保证。
pub(super) struct WindowsGitDirectoryCursor {
    file: File,
    identity: GitPrivateAllocation,
    page: Vec<u64>,
    offset: Option<usize>,
    started: bool,
    done: bool,
    cleanup_entry: Option<(OsString, [u8; 16], u32)>,
    cleanup_identity: Option<GitPrivateAllocation>,
}

impl WindowsGitDirectoryCursor {
    /// 参数：probe为本轮清理预算；返回：当前待删子项句柄/原名称/属性，或真正EOF。
    /// 打开或身份查询失败仍保留同一枚举子项，调用者须先核对owner账本再删除。
    /// 仅最终确认原ID消失后前进；普通枚举不得越过未完成清理项。
    pub(super) fn open_next_cleanup_child(
        &mut self,
        probe: &mut ProbeBudget,
    ) -> std::io::Result<Option<(File, OsString, u32)>> {
        if self.cleanup_entry.is_none() {
            self.cleanup_entry = self.next_entry(probe).map_err(std::io::Error::other)?;
        }
        let Some((name, id, attributes)) = self.cleanup_entry.as_ref() else {
            return Ok(None);
        };
        let directory =
            attributes & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_DIRECTORY != 0;
        let file = self.open_verified_child(name, *id, directory, probe)?;
        let identity = GitPrivateAllocation::from_file(&file).map_err(std::io::Error::other)?;
        if let Some(expected) = self.cleanup_identity.as_ref()
            && !expected.same_identity(&identity)
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "pending cleanup identity changed",
            ));
        }
        self.cleanup_identity = Some(identity);
        Ok(Some((file, name.clone(), *attributes)))
    }

    /// 参数：probe为本轮清理预算；返回：原ID明确消失时清除当前项并返回true，否则false/原错误。
    /// 访问拒绝、delete-pending、未成功核验及超时都保留原名称/ID，不能消费下一项。
    pub(super) fn confirm_cleanup_child_absent(
        &mut self,
        probe: &mut ProbeBudget,
    ) -> std::io::Result<bool> {
        let expected = self
            .cleanup_identity
            .as_ref()
            .ok_or_else(|| std::io::Error::other("cleanup child identity has not been verified"))?;
        if !super::windows_git_deletion_witness::WindowsGitDeletionWitness::confirm_absent(
            &self.file, expected, probe,
        )? {
            return Ok(false);
        }
        self.cleanup_identity = None;
        self.cleanup_entry = None;
        Ok(true)
    }

    /// 参数：name/id/directory为原枚举子项，probe为原任务预算；返回：核验后的DELETE句柄。
    /// 只相对原父句柄打开；不读正文、不删除；失败保持原游标和父责任，不按路径回退。
    pub(super) fn open_verified_child(
        &self,
        name: &std::ffi::OsStr,
        id: [u8; 16],
        directory: bool,
        probe: &mut ProbeBudget,
    ) -> std::io::Result<File> {
        use std::os::windows::ffi::OsStrExt;
        use std::os::windows::io::FromRawHandle;
        use windows_sys::Wdk::Foundation::OBJECT_ATTRIBUTES;
        use windows_sys::Wdk::Storage::FileSystem::{
            FILE_OPEN_NO_RECALL, FILE_OPEN_REPARSE_POINT, FILE_SYNCHRONOUS_IO_NONALERT, NtOpenFile,
        };
        use windows_sys::Win32::Foundation::{
            INVALID_HANDLE_VALUE, OBJ_DONT_REPARSE, RtlNtStatusToDosError, UNICODE_STRING,
        };
        use windows_sys::Win32::Storage::FileSystem::{
            DELETE, FILE_LIST_DIRECTORY, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ,
            FILE_SHARE_WRITE, SYNCHRONIZE,
        };
        let check =
            |file: &File| GitPrivateAllocation::from_file(file).map_err(std::io::Error::other);
        probe.check().map_err(std::io::Error::other)?;
        if !self.identity.same_identity(&check(&self.file)?) {
            return Err(std::io::Error::other("cleanup parent identity changed"));
        }
        let mut wide: Vec<u16> = name.encode_wide().take(32768).collect();
        if id == [0; 16]
            || wide.is_empty()
            || wide.len() > 32767
            || wide == [46]
            || wide == [46, 46]
            || wide.iter().any(|unit| matches!(*unit, 0 | 47 | 58 | 92))
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "cleanup child requires a known full ID and one native component",
            ));
        }
        let length = u16::try_from(wide.len() * 2).map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "cleanup child name too long",
            )
        })?;
        let unicode = UNICODE_STRING {
            Length: length,
            MaximumLength: length,
            Buffer: wide.as_mut_ptr(),
        };
        let attributes = OBJECT_ATTRIBUTES {
            Length: std::mem::size_of::<OBJECT_ATTRIBUTES>() as u32,
            RootDirectory: self.file.as_raw_handle(),
            ObjectName: &unicode,
            Attributes: OBJ_DONT_REPARSE,
            ..OBJECT_ATTRIBUTES::default()
        };
        let mut handle = std::ptr::null_mut();
        let mut status_block = windows_sys::Win32::System::IO::IO_STATUS_BLOCK::default();
        let status = unsafe {
            NtOpenFile(
                &mut handle,
                DELETE
                    | FILE_READ_ATTRIBUTES
                    | SYNCHRONIZE
                    | if directory { FILE_LIST_DIRECTORY } else { 0 },
                &attributes,
                &mut status_block,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                FILE_SYNCHRONOUS_IO_NONALERT | FILE_OPEN_REPARSE_POINT | FILE_OPEN_NO_RECALL,
            )
        };
        // 有效句柄在任何错误投影/身份查询前进入RAII；失败句柄也不会泄漏。
        let file = (!handle.is_null() && handle != INVALID_HANDLE_VALUE)
            .then(|| unsafe { File::from_raw_handle(handle) });
        if status != 0 {
            return Err(std::io::Error::from_raw_os_error(unsafe {
                RtlNtStatusToDosError(status) as i32
            }));
        }
        let file =
            file.ok_or_else(|| std::io::Error::other("cleanup child returned no valid handle"))?;
        let child = check(&file)?;
        if child.is_directory() != directory
            || !self.identity.same_volume(&child)
            || !child.windows_matches_file_id(&id)
            || !self.identity.same_identity(&check(&self.file)?)
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "cleanup child or parent identity changed; foreign object retained",
            ));
        }
        probe.check().map_err(std::io::Error::other)?;
        Ok(file)
    }

    /// 参数：file 为原持有目录句柄；返回：已核目录身份的固定64KiB游标或错误。
    pub(super) fn new(file: File) -> Result<Self, String> {
        let identity = GitPrivateAllocation::from_file(&file)?;
        if !identity.is_directory() {
            return Err("directory cursor requires a held directory".into());
        }
        let mut page = Vec::new();
        page.try_reserve_exact(PAGE_BYTES / 8)
            .map_err(|error| format!("directory cursor page allocation: {error}"))?;
        page.resize(PAGE_BYTES / 8, 0);
        Ok(Self {
            file,
            identity,
            page,
            offset: None,
            started: false,
            done: false,
            cleanup_entry: None,
            cleanup_identity: None,
        })
    }

    /// 参数：probe 为原采样期限与取消预算；返回：下一名称、完整ID和属性，或真实枚举结束。
    /// 页读取成功后先保存状态再检查预算，超时不将已读页丢失或冒充EOF。
    pub(super) fn next_entry(
        &mut self,
        probe: &mut ProbeBudget,
    ) -> Result<Option<(OsString, [u8; 16], u32)>, String> {
        if self.cleanup_entry.is_some() {
            return Err("directory enumeration cannot skip pending cleanup child".into());
        }
        loop {
            probe.check().map_err(|error| error.to_string())?;
            if self.done {
                return Ok(None);
            }
            if self.offset.is_none() {
                if !self
                    .identity
                    .same_identity(&GitPrivateAllocation::from_file(&self.file)?)
                {
                    return Err("directory cursor held identity changed".into());
                }
                self.page.fill(0);
                let class = if self.started {
                    FileIdExtdDirectoryInfo
                } else {
                    FileIdExtdDirectoryRestartInfo
                };
                let success = unsafe {
                    GetFileInformationByHandleEx(
                        self.file.as_raw_handle(),
                        class,
                        self.page.as_mut_ptr().cast(),
                        PAGE_BYTES as u32,
                    )
                };
                if success == 0 {
                    let error = std::io::Error::last_os_error();
                    if error.raw_os_error() == Some(ERROR_NO_MORE_FILES as i32) {
                        self.done = true;
                        probe.check().map_err(|error| error.to_string())?;
                        return Ok(None);
                    }
                    return Err(format!(
                        "held directory native enumeration unsupported or failed: {error}"
                    ));
                }
                self.started = true;
                self.offset = Some(0);
                probe.check().map_err(|error| error.to_string())?;
            }
            // page按u64对齐且全初始化；解析仍逐项检查边界，不信任内核记录偏移。
            let bytes =
                unsafe { std::slice::from_raw_parts(self.page.as_ptr().cast::<u8>(), PAGE_BYTES) };
            let (name, id, attributes, next) = parse_entry(bytes, self.offset.unwrap())?;
            self.offset = next;
            if name == "." || name == ".." {
                continue;
            }
            return Ok(Some((name, id, attributes)));
        }
    }
}

/// 参数：page 为初始化页，offset 为当前偏移；返回：有界原记录与下项偏移，不做路径解析。
pub(super) fn parse_entry(
    page: &[u8],
    offset: usize,
) -> Result<(OsString, [u8; 16], u32, Option<usize>), String> {
    let base = std::mem::offset_of!(FILE_ID_EXTD_DIR_INFO, FileName);
    let header_end = offset
        .checked_add(std::mem::size_of::<FILE_ID_EXTD_DIR_INFO>())
        .ok_or("directory record header overflow")?;
    if header_end > page.len() {
        return Err("directory record header outside page".into());
    }
    let record = unsafe {
        std::ptr::read_unaligned(page.as_ptr().add(offset).cast::<FILE_ID_EXTD_DIR_INFO>())
    };
    let length = record.FileNameLength as usize;
    if length == 0 || length % 2 != 0 || length > 65534 || record.FileId.Identifier == [0; 16] {
        return Err("directory record has invalid name length or unknown full ID".into());
    }
    let start = offset
        .checked_add(base)
        .ok_or("directory record name overflow")?;
    let end = start
        .checked_add(length)
        .ok_or("directory record name overflow")?;
    if end > page.len() {
        return Err("directory record name outside page".into());
    }
    let next = if record.NextEntryOffset == 0 {
        None
    } else {
        let distance = record.NextEntryOffset as usize;
        let next = offset
            .checked_add(distance)
            .ok_or("directory next record overflow")?;
        if distance % 8 != 0 || next < end || next >= page.len() {
            return Err("directory next record does not advance within page".into());
        }
        Some(next)
    };
    let wide: Vec<u16> = page[start..end]
        .chunks_exact(2)
        .map(|pair| u16::from_ne_bytes([pair[0], pair[1]]))
        .collect();
    if wide.iter().any(|unit| matches!(*unit, 0 | 47 | 58 | 92)) {
        return Err("directory record name is not a single component".into());
    }
    Ok((
        OsString::from_wide(&wide),
        record.FileId.Identifier,
        record.FileAttributes,
        next,
    ))
}
