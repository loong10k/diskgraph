//! 命名管道的显式 DACL：只允许当前 token 用户和 SYSTEM。

use std::ffi::c_void;
use std::io;
use std::ptr::null_mut;
use windows_sys::Win32::Foundation::{ERROR_INSUFFICIENT_BUFFER, GetLastError, LocalFree};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows_sys::Win32::Security::{
    GetTokenInformation, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER,
    TokenUser,
};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

use super::super::ChildError;
use super::owned_handle::OwnedHandle;

/// 持有 LocalAlloc 安全描述符。来源：Win32 SDDL 当前用户 DACL 与 LocalFree。
pub(super) struct PipeSecurity {
    descriptor: PSECURITY_DESCRIPTOR,
}

impl PipeSecurity {
    /// 获取当前用户 SID 并建立受保护 DACL。参数：无。返回：描述符所有权或 Win32 错误。
    pub(super) fn for_current_user() -> Result<Self, ChildError> {
        let mut token = null_mut();
        if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
            return Err(last("OpenProcessToken"));
        }
        let token = OwnedHandle::from_raw(token, "OpenProcessToken")?;
        let mut bytes = 0u32;
        let first =
            unsafe { GetTokenInformation(token.as_raw(), TokenUser, null_mut(), 0, &mut bytes) };
        if first != 0
            || unsafe { GetLastError() } != ERROR_INSUFFICIENT_BUFFER
            || bytes == 0
            || bytes > 4096
        {
            return Err(last("GetTokenInformation(size)"));
        }
        // TOKEN_USER 含指针，使用 usize 容器保证对齐；内核填写的 SID 指向本缓冲区。
        let words = (bytes as usize).div_ceil(std::mem::size_of::<usize>());
        let mut storage = vec![0usize; words];
        if unsafe {
            GetTokenInformation(
                token.as_raw(),
                TokenUser,
                storage.as_mut_ptr().cast(),
                bytes,
                &mut bytes,
            )
        } == 0
        {
            return Err(last("GetTokenInformation(TokenUser)"));
        }
        let sid = unsafe {
            (storage.as_ptr().cast::<TOKEN_USER>())
                .as_ref()
                .unwrap()
                .User
                .Sid
        };
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
            return Err(ChildError::Unsupported("invalid current-user SID"));
        }
        let sid = String::from_utf16(&sid_units)
            .map_err(|_| ChildError::Unsupported("invalid current-user SID encoding"))?;
        let sddl: Vec<u16> = format!("D:P(A;;GA;;;{sid})(A;;GA;;;SY)")
            .encode_utf16()
            .chain(std::iter::once(0))
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

    /// 构造临时 SECURITY_ATTRIBUTES。参数：inherit 决定句柄继承。返回：借用本描述符的属性。
    pub(super) fn attributes(&self, inherit: bool) -> SECURITY_ATTRIBUTES {
        SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: self.descriptor.cast::<c_void>(),
            bInheritHandle: i32::from(inherit),
        }
    }
}

impl Drop for PipeSecurity {
    fn drop(&mut self) {
        unsafe { LocalFree(self.descriptor.cast()) };
    }
}

fn last(context: &'static str) -> ChildError {
    ChildError::io(context, io::Error::last_os_error())
}
