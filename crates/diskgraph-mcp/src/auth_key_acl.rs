//! Windows verifier-key ACL validation against the already opened file handle.

use std::ffi::c_void;
use std::fs::File;
use std::io;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::ptr;

use windows_sys::Win32::Foundation::{ERROR_SUCCESS, LocalFree};
use windows_sys::Win32::Security::Authorization::{GetSecurityInfo, SE_FILE_OBJECT};
use windows_sys::Win32::Security::{
    ACCESS_ALLOWED_ACE, ACL, DACL_SECURITY_INFORMATION, EqualSid, GetAce, GetTokenInformation,
    INHERIT_ONLY_ACE, IsWellKnownSid, OWNER_SECURITY_INFORMATION, PSID, TOKEN_QUERY, TOKEN_USER,
    TokenUser, WinBuiltinAdministratorsSid, WinCreatorOwnerRightsSid, WinLocalSystemSid,
};
use windows_sys::Win32::System::SystemServices::{ACCESS_ALLOWED_ACE_TYPE, ACCESS_DENIED_ACE_TYPE};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

fn invalid_acl(reason: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        format!("authentication key file ACL is not restricted ({reason})"),
    )
}

/// 检查已打开密钥文件的所有可生效允许项，并要求所有者为当前用户或系统管理员。
pub(crate) fn ensure_restricted(file: &File) -> io::Result<()> {
    let mut owner: PSID = ptr::null_mut();
    let mut dacl: *mut ACL = ptr::null_mut();
    let mut descriptor: *mut c_void = ptr::null_mut();
    // 使用文件句柄，避免先按路径检查 ACL、随后读取另一个文件的竞态。
    let status = unsafe {
        GetSecurityInfo(
            file.as_raw_handle(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            &mut owner,
            ptr::null_mut(),
            &mut dacl,
            ptr::null_mut(),
            &mut descriptor,
        )
    };
    if status != ERROR_SUCCESS {
        return Err(io::Error::from_raw_os_error(status as i32));
    }
    let result = validate_acl(owner, dacl);
    unsafe { LocalFree(descriptor) };
    result
}

fn validate_acl(owner: PSID, dacl: *mut ACL) -> io::Result<()> {
    if owner.is_null() || dacl.is_null() {
        return Err(invalid_acl("missing owner or DACL"));
    }
    let mut token = ptr::null_mut();
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return Err(io::Error::last_os_error());
    }
    // OwnedHandle 在校验结束及错误路径都关闭 token。
    let token = unsafe { OwnedHandle::from_raw_handle(token) };
    let mut needed = 0_u32;
    unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            TokenUser,
            ptr::null_mut(),
            0,
            &mut needed,
        )
    };
    if needed < std::mem::size_of::<TOKEN_USER>() as u32 || needed > 4096 {
        return Err(invalid_acl("unreadable token identity"));
    }
    let mut buffer = vec![0_usize; (needed as usize).div_ceil(std::mem::size_of::<usize>())];
    if unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            TokenUser,
            buffer.as_mut_ptr().cast(),
            needed,
            &mut needed,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    let user = unsafe { (*buffer.as_ptr().cast::<TOKEN_USER>()).User.Sid };
    let trusted_owner = unsafe {
        !user.is_null()
            && (EqualSid(owner, user) != 0
                || IsWellKnownSid(owner, WinLocalSystemSid) != 0
                || IsWellKnownSid(owner, WinBuiltinAdministratorsSid) != 0)
    };
    if !trusted_owner {
        return Err(invalid_acl("untrusted owner"));
    }
    let ace_count = unsafe { (*dacl).AceCount };
    for index in 0..u32::from(ace_count) {
        let mut entry: *mut c_void = ptr::null_mut();
        if unsafe { GetAce(dacl, index, &mut entry) } == 0 || entry.is_null() {
            return Err(invalid_acl("unreadable ACE"));
        }
        let ace = entry.cast::<ACCESS_ALLOWED_ACE>();
        let header = unsafe { (*ace).Header };
        if u32::from(header.AceFlags) & INHERIT_ONLY_ACE != 0 {
            continue;
        }
        // DACL 中的复杂/条件允许项无法证明为受限访问，保守拒绝。
        if u32::from(header.AceType) == ACCESS_DENIED_ACE_TYPE {
            continue;
        }
        if u32::from(header.AceType) != ACCESS_ALLOWED_ACE_TYPE || header.AceSize < 12 {
            return Err(invalid_acl("unsupported ACE"));
        }
        let sid = unsafe { ptr::addr_of_mut!((*ace).SidStart).cast::<c_void>() };
        // OWNER RIGHTS 只映射到已验证的文件所有者，不扩展到其他账户。
        let trusted = unsafe {
            EqualSid(sid, owner) != 0
                || IsWellKnownSid(sid, WinLocalSystemSid) != 0
                || IsWellKnownSid(sid, WinBuiltinAdministratorsSid) != 0
                || IsWellKnownSid(sid, WinCreatorOwnerRightsSid) != 0
        };
        if !trusted {
            return Err(invalid_acl("extra trustee"));
        }
    }
    Ok(())
}
