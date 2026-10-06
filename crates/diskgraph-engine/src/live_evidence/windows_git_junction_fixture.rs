//! 仅原生测试使用的自有临时 junction；不调用 shell 或创建额外进程。
use std::fs::OpenOptions;
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::AsRawHandle;
use std::path::{Path, PathBuf};
use windows_sys::Wdk::Storage::FileSystem::{REPARSE_DATA_BUFFER, REPARSE_DATA_BUFFER_0_1};
use windows_sys::Win32::Storage::FileSystem::{
    FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_DELETE, FILE_SHARE_READ,
    FILE_SHARE_WRITE,
};
use windows_sys::Win32::System::IO::DeviceIoControl;
use windows_sys::Win32::System::Ioctl::FSCTL_SET_REPARSE_POINT;
use windows_sys::Win32::System::SystemServices::IO_REPARSE_TAG_MOUNT_POINT;

/// 自有测试路径的 junction owner；来源：Windows SDK REPARSE_DATA_BUFFER。
/// 析构仅 remove_dir 原链接，绝不递归进入目标；目标目录由另一独立 TempDir 保活。
pub(super) struct WindowsGitJunctionFixture {
    path: Option<PathBuf>,
}
impl WindowsGitJunctionFixture {
    /// 参数：不存在的自有 link、独立存在的本地 target；返回：真实 mount-point fixture。
    pub(super) fn create(link: &Path, target: &Path) -> io::Result<Self> {
        let canonical = target.canonicalize()?;
        let wide: Vec<u16> = canonical.as_os_str().encode_wide().take(4096).collect();
        if wide.len() >= 4096
            || !wide.starts_with(&[92, 92, 63, 92])
            || wide.len() < 7
            || wide[5] != 58
            || wide[6] != 92
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "junction fixture requires bounded local drive path",
            ));
        }
        let print = &wide[4..];
        let substitute: Vec<u16> = [92, 63, 63, 92]
            .into_iter()
            .chain(print.iter().copied())
            .collect();
        let mut paths = substitute.clone();
        paths.push(0);
        paths.extend_from_slice(print);
        paths.push(0);
        let header = std::mem::offset_of!(REPARSE_DATA_BUFFER, Anonymous);
        let mount_header = std::mem::offset_of!(REPARSE_DATA_BUFFER_0_1, PathBuffer);
        let path_offset = header + mount_header;
        let total = path_offset + paths.len() * 2;
        if total > 16 * 1024
            || std::mem::align_of::<REPARSE_DATA_BUFFER>() > std::mem::align_of::<u64>()
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "junction fixture buffer bound/alignment",
            ));
        }
        // 使用SDK真实结构布局，u64 backing覆盖结构及变长尾部并满足原生对齐。
        let mut storage = vec![
            0u64;
            total
                .max(std::mem::size_of::<REPARSE_DATA_BUFFER>())
                .div_ceil(8)
        ];
        let raw = storage.as_mut_ptr().cast::<REPARSE_DATA_BUFFER>();
        unsafe {
            (*raw).ReparseTag = IO_REPARSE_TAG_MOUNT_POINT;
            (*raw).ReparseDataLength = (total - header) as u16;
            (*raw)
                .Anonymous
                .MountPointReparseBuffer
                .SubstituteNameOffset = 0;
            (*raw)
                .Anonymous
                .MountPointReparseBuffer
                .SubstituteNameLength = (substitute.len() * 2) as u16;
            (*raw).Anonymous.MountPointReparseBuffer.PrintNameOffset =
                ((substitute.len() + 1) * 2) as u16;
            (*raw).Anonymous.MountPointReparseBuffer.PrintNameLength = (print.len() * 2) as u16;
            std::ptr::copy_nonoverlapping(
                paths.as_ptr(),
                storage
                    .as_mut_ptr()
                    .cast::<u8>()
                    .add(path_offset)
                    .cast::<u16>(),
                paths.len(),
            );
        }
        std::fs::create_dir(link)?;
        let owner = Self {
            path: Some(link.to_owned()),
        };
        let file = OpenOptions::new()
            .write(true)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
            .open(link)?;
        let mut returned = 0;
        // 同步非overlapped句柄；所有缓冲在调用完成前存活，原errno在任何析构前捕获。
        let result = unsafe {
            DeviceIoControl(
                file.as_raw_handle(),
                FSCTL_SET_REPARSE_POINT,
                storage.as_ptr().cast(),
                total as u32,
                std::ptr::null_mut(),
                0,
                &mut returned,
                std::ptr::null_mut(),
            )
        };
        if result == 0 {
            return Err(io::Error::last_os_error());
        }
        drop(file);
        Ok(owner)
    }
    /// 参数：无；返回：仅移除junction本身的原结果，不修改其目标。
    pub(super) fn remove(&mut self) -> io::Result<()> {
        if let Some(path) = &self.path {
            std::fs::remove_dir(path)?;
            self.path = None;
        }
        Ok(())
    }
}
impl Drop for WindowsGitJunctionFixture {
    fn drop(&mut self) {
        if let Err(error) = self.remove() {
            eprintln!("junction fixture cleanup failed: {error}");
        }
    }
}
