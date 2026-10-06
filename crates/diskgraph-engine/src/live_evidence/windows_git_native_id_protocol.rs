use std::fs::File;
use std::io;
use std::os::windows::io::AsRawHandle;
use windows_sys::Win32::Storage::FileSystem::GetVolumeInformationByHandleW;
use windows_sys::Win32::System::SystemServices::FILE_SUPPORTS_OPEN_BY_FILE_ID;

/// 原卷的原生ID操作格式；来源：SDK FILE_ID_DESCRIPTOR和卷能力查询，无Java对等对象。
/// 身份记录始终为完整128位；NTFS只允许高64位全零的无损原生文件引用格式。
pub(super) enum WindowsGitNativeIdProtocol {
    NtfsFileReference,
    RefsExtendedFileId,
}

impl WindowsGitNativeIdProtocol {
    /// 参数：hint为原卷可信持有目录句柄；返回：卷声明支持的已知ID格式或原错误/Unsupported。
    /// 在打开前选定，不在失败后缩短ID重试；未知卷和能力不会自动降级。
    pub(super) fn from_hint(hint: &File) -> io::Result<Self> {
        let mut name = [0u16; 32];
        let mut flags = 0;
        let result = unsafe {
            GetVolumeInformationByHandleW(
                hint.as_raw_handle(),
                std::ptr::null_mut(),
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut flags,
                name.as_mut_ptr(),
                name.len() as u32,
            )
        };
        if result == 0 {
            return Err(io::Error::last_os_error());
        }
        if flags & FILE_SUPPORTS_OPEN_BY_FILE_ID == 0 {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "volume does not support native file-ID open",
            ));
        }
        let end = name.iter().position(|unit| *unit == 0).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::Unsupported,
                "unterminated native filesystem name",
            )
        })?;
        match String::from_utf16(&name[..end]).as_deref() {
            Ok("NTFS") => Ok(Self::NtfsFileReference),
            Ok("ReFS" | "REFS") => Ok(Self::RefsExtendedFileId),
            _ => Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "unqualified native file-ID protocol",
            )),
        }
    }

    /// 参数：id为完整原身份；返回：已验证无损的原生操作字节长度。
    /// NTFS非零高位明确拒绝，不抹除身份；ReFS始终传递全部16字节。
    pub(super) fn byte_length(&self, id: &[u8; 16]) -> io::Result<u16> {
        if *id == [0; 16] {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "unknown native file identity",
            ));
        }
        match self {
            Self::NtfsFileReference if id[8..].iter().all(|byte| *byte == 0) => Ok(8),
            Self::NtfsFileReference => Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "NTFS identity cannot be represented losslessly",
            )),
            Self::RefsExtendedFileId => Ok(16),
        }
    }
}
