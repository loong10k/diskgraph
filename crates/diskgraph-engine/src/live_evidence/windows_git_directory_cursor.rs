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
    foreign_observation:
        Option<super::windows_git_foreign_removal_witness::WindowsGitForeignRemovalWitness>,
    cleanup_delete_requested: bool,
    cleanup_post_mark_verified: bool,
    cleanup_marked_file: Option<File>,
    cleanup_observation:
        Option<super::windows_git_removal_observation::WindowsGitRemovalObservation>,
}

impl WindowsGitDirectoryCursor {
    /// 参数：parent_label为原账本父键、capacity为owner账本、probe为本轮预算。
    /// 返回：原父/子登记身份及文件版本均核验后的当前子项句柄/名称/属性，或真正EOF。
    /// 打开或账本核验失败仍保留同一枚举子项，不收养陌生对象。
    /// 仅最终确认原ID消失后前进；未登记项需删除前只读完整 ID 观察及最终移除通知，普通枚举不得越过未完成清理项。
    pub(super) fn open_next_cleanup_child(
        &mut self,
        parent_label: &std::path::Path,
        capacity: &super::git_private_capacity::GitPrivateCapacity,
        probe: &mut ProbeBudget,
    ) -> std::io::Result<Option<(File, OsString, u32)>> {
        probe.check().map_err(std::io::Error::other)?;
        if self.cleanup_delete_requested {
            return Err(std::io::Error::other(
                "original cleanup deletion awaits confirmation",
            ));
        }
        capacity
            .check_cleanup_identity(parent_label, &self.file, true)
            .map_err(std::io::Error::other)?;
        loop {
            probe.check().map_err(std::io::Error::other)?;
            if self.cleanup_entry.is_none() {
                self.cleanup_entry = self.next_entry(probe).map_err(std::io::Error::other)?;
            }
            let Some((name, id, attributes)) = self.cleanup_entry.as_ref() else {
                return Ok(None);
            };
            let directory =
                attributes & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_DIRECTORY != 0;
            if !capacity.registered(&parent_label.join(name)) {
                if self.foreign_observation.is_none() {
                    super::windows_git_foreign_removal_witness::WindowsGitForeignRemovalWitness::prepare_into(
                        &self.file, id, probe, &mut self.foreign_observation,
                    )?;
                    // 首次确认外来对象只建立只读观察，保留原未登记拒绝语义；不把观察当删除许可。
                    return Err(std::io::Error::other(
                        "private Git object is not registered; original foreign observation retained",
                    ));
                }
                if !self
                    .foreign_observation
                    .as_mut()
                    .expect("retained foreign observation")
                    .confirm(id, probe)?
                {
                    return Err(std::io::Error::other(
                        "original foreign entry removal remains unconfirmed",
                    ));
                }
                self.foreign_observation = None;
                self.cleanup_entry = None;
                self.cleanup_identity = None;
                continue;
            }
            let file = self.open_verified_child(name, *id, directory, probe)?;
            // 词法路径只作原账本键，不用于重新打开对象；句柄必须属于本owner登记身份。
            capacity
                .check_cleanup_identity(&parent_label.join(name), &file, directory)
                .map_err(std::io::Error::other)?;
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
            return Ok(Some((file, name.clone(), *attributes)));
        }
    }

    /// 参数：probe为本轮清理预算；返回：原ID明确消失时清除当前项并返回true，否则false/原错误。
    /// 访问拒绝、delete-pending、未成功核验及超时都保留原名称/ID，不能消费下一项。
    pub(super) fn confirm_cleanup_child_absent(
        &mut self,
        probe: &mut ProbeBudget,
    ) -> std::io::Result<bool> {
        if self.cleanup_delete_requested && !self.cleanup_post_mark_verified {
            let file = self.cleanup_marked_file.as_ref().ok_or_else(|| {
                std::io::Error::other("original post-mark handle unavailable; owner retained")
            })?;
            let expected = self
                .cleanup_identity
                .as_ref()
                .ok_or_else(|| std::io::Error::other("original post-mark identity unavailable"))?;
            super::windows_git_deletion_seal::WindowsGitDeletionSeal::verify(file, expected)?;
            self.cleanup_post_mark_verified = true;
            self.cleanup_marked_file = None;
        }
        let expected = self
            .cleanup_identity
            .as_ref()
            .ok_or_else(|| std::io::Error::other("cleanup child identity has not been verified"))?;
        let confirmed = if let Some(observation) = self.cleanup_observation.as_mut() {
            if !self.cleanup_delete_requested {
                return Err(std::io::Error::other("original deletion not requested"));
            }
            observation.confirm(expected, probe)?
        } else {
            super::windows_git_deletion_witness::WindowsGitDeletionWitness::confirm_absent(
                &self.file, expected, probe,
            )?
        };
        if !confirmed {
            return Ok(false);
        }
        self.cleanup_identity = None;
        self.cleanup_entry = None;
        self.cleanup_delete_requested = false;
        self.cleanup_post_mark_verified = false;
        self.cleanup_observation = None;
        Ok(true)
    }

    /// 参数：无；返回：当前原子项是否已成功提交删除请求，预算末检失败也不复位。
    pub(super) fn cleanup_child_delete_requested(&self) -> bool {
        self.cleanup_delete_requested
    }

    /// 参数：file为当前原子项DELETE句柄、parent_label/capacity为原账本、probe为本轮预算。
    /// 返回：核验并标记成功；失败保留当前项，系统调用成功后立即记录副作用再末检预算。
    pub(super) fn mark_cleanup_child(
        &mut self,
        file: &File,
        parent_label: &std::path::Path,
        capacity: &super::git_private_capacity::GitPrivateCapacity,
        probe: &mut ProbeBudget,
    ) -> std::io::Result<()> {
        use windows_sys::Win32::Storage::FileSystem::{
            FILE_DISPOSITION_INFO, FileDispositionInfo, SetFileInformationByHandle,
        };
        probe.check().map_err(std::io::Error::other)?;
        if self.cleanup_delete_requested {
            return Err(std::io::Error::other(
                "original cleanup deletion already requested",
            ));
        }
        let (name, _, attributes) = self
            .cleanup_entry
            .as_ref()
            .ok_or_else(|| std::io::Error::other("cleanup entry absent"))?;
        let expected = self
            .cleanup_identity
            .as_ref()
            .ok_or_else(|| std::io::Error::other("cleanup identity unverified"))?;
        let current = GitPrivateAllocation::from_file(file).map_err(std::io::Error::other)?;
        if !expected.same_identity(&current) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "cleanup handle identity changed",
            ));
        }
        capacity
            .check_cleanup_identity(parent_label, &self.file, true)
            .map_err(std::io::Error::other)?;
        let directory =
            attributes & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_DIRECTORY != 0;
        capacity
            .check_cleanup_identity(&parent_label.join(name), file, directory)
            .map_err(std::io::Error::other)?;
        if self.cleanup_observation.is_none() {
            super::windows_git_removal_observation::WindowsGitRemovalObservation::prepare_into(
                &self.file,
                expected,
                probe,
                &mut self.cleanup_observation,
            )?;
        }
        self.cleanup_observation
            .as_ref()
            .expect("original deletion observation")
            .check_before_delete(expected, probe)?;
        probe.check().map_err(std::io::Error::other)?;
        // 删除副作用发生前保留原对象句柄；后置查询失败仍可对同句柄恢复核验。
        self.cleanup_marked_file = Some(file.try_clone()?);
        let disposition = FILE_DISPOSITION_INFO { DeleteFile: true };
        #[cfg(test)]
        super::windows_cleanup_mark_hook::WindowsCleanupMarkHook::run();
        let result = unsafe {
            SetFileInformationByHandle(
                file.as_raw_handle(),
                FileDispositionInfo,
                (&disposition as *const FILE_DISPOSITION_INFO).cast(),
                std::mem::size_of::<FILE_DISPOSITION_INFO>() as u32,
            )
        };
        if result == 0 {
            let error = std::io::Error::last_os_error();
            self.cleanup_marked_file = None;
            return Err(error);
        }
        // 成功删除请求不能被末段超时抹除；恢复只确认原ID，不按名称再次删除。
        self.cleanup_delete_requested = true;
        #[cfg(test)]
        super::windows_cleanup_mark_hook::WindowsCleanupMarkHook::inspect_marked(file);
        super::windows_git_deletion_seal::WindowsGitDeletionSeal::verify(file, expected)?;
        self.cleanup_post_mark_verified = true;
        self.cleanup_marked_file = None;
        probe.check().map_err(std::io::Error::other)
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
            foreign_observation: None,
            cleanup_delete_requested: false,
            cleanup_post_mark_verified: false,
            cleanup_marked_file: None,
            cleanup_observation: None,
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
        .as_chunks::<2>()
        .0
        .iter()
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
