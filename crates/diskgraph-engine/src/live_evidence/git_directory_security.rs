//! Windows 私有目录在创建时应用当前用户与 SYSTEM 的可继承受保护 DACL。

use std::ffi::c_void;
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::Path;
use std::ptr::null_mut;
use windows_sys::Win32::Foundation::{ERROR_INSUFFICIENT_BUFFER, GetLastError, LocalFree};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows_sys::Win32::Security::{
    GetTokenInformation, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER,
    TokenUser,
};
use windows_sys::Win32::Storage::FileSystem::CreateDirectoryW;
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

/// 持有目录创建安全描述符。来源：Win32 SDDL、当前进程 token 与 LocalFree。
pub(super) struct GitDirectorySecurity {
    descriptor: PSECURITY_DESCRIPTOR,
}

impl GitDirectorySecurity {
    /// 建立当前用户和 SYSTEM 独占的可继承 DACL。参数：无。返回：描述符或原生查询错误。
    pub(super) fn new() -> Result<Self, String> {
        let mut token = null_mut();
        if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
            return Err(last("OpenProcessToken"));
        }
        // 成功 token 立即交给 OwnedHandle；后续错误和 unwind 均关闭该句柄。
        let token = unsafe { OwnedHandle::from_raw_handle(token) };
        let mut bytes = 0u32;
        let first = unsafe {
            GetTokenInformation(token.as_raw_handle(), TokenUser, null_mut(), 0, &mut bytes)
        };
        if first != 0
            || unsafe { GetLastError() } != ERROR_INSUFFICIENT_BUFFER
            || bytes == 0
            || bytes > 4096
        {
            return Err(last("GetTokenInformation(size)"));
        }
        let words = (bytes as usize).div_ceil(std::mem::size_of::<usize>());
        let mut storage = vec![0usize; words];
        if unsafe {
            GetTokenInformation(
                token.as_raw_handle(),
                TokenUser,
                storage.as_mut_ptr().cast(),
                bytes,
                &mut bytes,
            )
        } == 0
        {
            return Err(last("GetTokenInformation(TokenUser)"));
        }
        // TOKEN_USER 对齐由 usize 存储保证，SID 在 storage 存活期间有效。
        let sid = unsafe { (*storage.as_ptr().cast::<TOKEN_USER>()).User.Sid };
        let mut sid_text = null_mut();
        if unsafe { ConvertSidToStringSidW(sid, &mut sid_text) } == 0 {
            return Err(last("ConvertSidToStringSidW"));
        }
        let sid_units = unsafe {
            (0..256)
                .map(|index| *sid_text.add(index))
                .take_while(|unit| *unit != 0)
                .collect::<Vec<_>>()
        };
        unsafe { LocalFree(sid_text.cast()) };
        if sid_units.is_empty() || sid_units.len() == 256 {
            return Err("unsupported current-user SID".into());
        }
        let sid =
            String::from_utf16(&sid_units).map_err(|_| "unsupported current-user SID encoding")?;
        // P 禁止从宽松 temp 根继承；OI/CI 让后续文件及子目录继承同一主体集合。
        let sddl: Vec<u16> = format!("D:P(A;OICI;FA;;;{sid})(A;OICI;FA;;;SY)")
            .encode_utf16()
            .chain(Some(0))
            .collect();
        let mut descriptor: PSECURITY_DESCRIPTOR = null_mut();
        if unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                SDDL_REVISION_1,
                &mut descriptor,
                null_mut(),
            )
        } == 0
        {
            return Err(last("ConvertStringSecurityDescriptorToSecurityDescriptorW"));
        }
        Ok(Self { descriptor })
    }

    /// 参数：无；返回：借用本对象持有的原安全描述符，仅用于同步原生创建调用。
    /// 调用者不得修改、释放或保存到本对象寿命之外；所有权仍由本对象 Drop 释放。
    pub(super) fn descriptor(&self) -> PSECURITY_DESCRIPTOR {
        self.descriptor
    }

    /// 独占创建带安全描述符的目录。参数：path 为受信 temp 根下的绝对路径。返回：创建结果，不覆盖既有目录。
    pub(super) fn create(&self, path: &Path) -> io::Result<()> {
        let wide: Vec<u16> = path.as_os_str().encode_wide().take(32768).collect();
        if !path.is_absolute() || wide.len() >= 32768 || wide.contains(&0) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid private directory path",
            ));
        }
        let wide: Vec<u16> = wide.into_iter().chain(Some(0)).collect();
        let attributes = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: self.descriptor.cast::<c_void>(),
            bInheritHandle: 0,
        };
        if unsafe { CreateDirectoryW(wide.as_ptr(), &attributes) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}

impl Drop for GitDirectorySecurity {
    fn drop(&mut self) {
        unsafe { LocalFree(self.descriptor.cast()) };
    }
}

fn last(context: &str) -> String {
    format!("{context}: {}", io::Error::last_os_error())
}
