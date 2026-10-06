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
}

impl WindowsGitDirectoryCursor {
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
        })
    }

    /// 参数：probe 为原采样期限与取消预算；返回：下一名称、完整ID和属性，或真实枚举结束。
    /// 页读取成功后先保存状态再检查预算，超时不将已读页丢失或冒充EOF。
    pub(super) fn next_entry(
        &mut self,
        probe: &mut ProbeBudget,
    ) -> Result<Option<(OsString, [u8; 16], u32)>, String> {
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
