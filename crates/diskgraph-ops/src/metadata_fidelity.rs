use crate::OpsError;
use std::collections::BTreeMap;
use std::fs::File;
use std::os::fd::AsRawFd;

/// 比较原生 ACL、扩展属性和 flags；无法在预算内确认时拒绝发布。
pub(crate) fn verify(source: &File, copy: &File) -> Result<(), OpsError> {
    use std::os::macos::fs::MetadataExt;
    if attributes(source)? != attributes(copy)?
        || acl(source)? != acl(copy)?
        || source.metadata()?.st_flags() != copy.metadata()?.st_flags()
    {
        return Err(OpsError::Stale(
            "required native metadata differs after copy".into(),
        ));
    }
    Ok(())
}

/// 在有界分配中复制 xattr；不调用可能复制任意大小 ResourceFork 的原生全属性复制。
pub(crate) fn copy_attributes(
    source: &File,
    copy: &File,
    check_live: &dyn Fn() -> Result<(), OpsError>,
) -> Result<(), OpsError> {
    for (name, value) in attributes(source)? {
        check_live()?;
        let name = std::ffi::CString::new(name)
            .map_err(|_| OpsError::Stale("invalid attribute name".into()))?;
        if unsafe {
            libc::fsetxattr(
                copy.as_raw_fd(),
                name.as_ptr(),
                value.as_ptr().cast(),
                value.len(),
                0,
                0,
            )
        } != 0
        {
            return Err(std::io::Error::last_os_error().into());
        }
    }
    Ok(())
}

/// 在任何原生元数据复制之前验证属性/ACL 总成本，过大数据不进入 staging。
pub(crate) fn preflight(source: &File) -> Result<(), OpsError> {
    let _ = attributes(source)?;
    let _ = acl(source)?;
    Ok(())
}

fn attributes(file: &File) -> Result<BTreeMap<Vec<u8>, Vec<u8>>, OpsError> {
    let length = unsafe { libc::flistxattr(file.as_raw_fd(), std::ptr::null_mut(), 0, 0) };
    if length < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    if length > 64 * 1024 {
        return Err(OpsError::Stale(
            "unsupported: extended attribute names exceed verification budget".into(),
        ));
    }
    let mut names = vec![0u8; length as usize];
    if unsafe { libc::flistxattr(file.as_raw_fd(), names.as_mut_ptr().cast(), names.len(), 0) }
        != length
    {
        return Err(OpsError::Stale("extended attribute names changed".into()));
    }
    let mut remaining = 8 * 1024 * 1024usize;
    let mut result = BTreeMap::new();
    for name in names
        .split(|byte| *byte == 0)
        .filter(|name| !name.is_empty())
    {
        let key = std::ffi::CString::new(name)
            .map_err(|_| OpsError::Stale("invalid attribute name".into()))?;
        let size = unsafe {
            libc::fgetxattr(
                file.as_raw_fd(),
                key.as_ptr(),
                std::ptr::null_mut(),
                0,
                0,
                0,
            )
        };
        if size < 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        if size as usize > remaining {
            return Err(OpsError::Stale(
                "unsupported: extended attributes exceed verification budget".into(),
            ));
        }
        remaining -= size as usize;
        let mut value = vec![0u8; size as usize];
        if unsafe {
            libc::fgetxattr(
                file.as_raw_fd(),
                key.as_ptr(),
                value.as_mut_ptr().cast(),
                value.len(),
                0,
                0,
            )
        } != size
        {
            return Err(OpsError::Stale(
                "extended attribute changed during verification".into(),
            ));
        }
        result.insert(name.to_vec(), value);
    }
    Ok(result)
}

fn acl(file: &File) -> Result<Vec<u8>, OpsError> {
    unsafe extern "C" {
        fn acl_get_fd_np(fd: libc::c_int, kind: libc::c_uint) -> *mut libc::c_void;
        fn acl_size(acl: *mut libc::c_void) -> libc::ssize_t;
        fn acl_copy_ext(
            buffer: *mut libc::c_void,
            acl: *mut libc::c_void,
            size: libc::ssize_t,
        ) -> libc::ssize_t;
        fn acl_free(acl: *mut libc::c_void) -> libc::c_int;
    }
    let value = unsafe { acl_get_fd_np(file.as_raw_fd(), 0x100) };
    if value.is_null() {
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::ENOENT) {
            return Ok(Vec::new());
        }
        return Err(error.into());
    }
    let size = unsafe { acl_size(value) };
    if !(0..=64 * 1024).contains(&size) {
        unsafe {
            acl_free(value);
        }
        return Err(OpsError::Stale("unsupported: ACL verification size".into()));
    }
    let mut bytes = vec![0u8; size as usize];
    let copied = unsafe { acl_copy_ext(bytes.as_mut_ptr().cast(), value, size) };
    unsafe {
        acl_free(value);
    }
    if copied < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    bytes.truncate(copied as usize);
    Ok(bytes)
}
