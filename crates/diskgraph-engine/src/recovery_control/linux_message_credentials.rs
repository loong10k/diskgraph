use super::ControlError;
use std::os::fd::{FromRawFd, OwnedFd};

/// 校验原recvmsg辅助消息并关闭所有意外交付FD；来源：Linux SCM_CREDENTIALS/SCM_RIGHTS原控制合同。
/// 参数：message为原内核填写的msghdr，control_start/control_bytes为实际对齐缓冲边界；返回：唯一凭据或拒绝。
/// 调用者须先接收至自身缓冲；即使取消或过期，也必须先调用本函数处置已交付的FD。
pub(super) fn inspect_credentials(
    message: &libc::msghdr,
    control_start: usize,
    control_bytes: usize,
) -> Result<Option<libc::ucred>, ControlError> {
    let mut credentials = None;
    let mut valid = true;
    let mut header = unsafe { libc::CMSG_FIRSTHDR(message) };
    while !header.is_null() {
        let base = unsafe { libc::CMSG_LEN(0) } as usize;
        let length = unsafe { (*header).cmsg_len };
        let offset = (header as usize).checked_sub(control_start);
        if length < base
            || !offset.is_some_and(|start| {
                start
                    .checked_add(length)
                    .is_some_and(|end| end <= message.msg_controllen && end <= control_bytes)
            })
        {
            valid = false;
            break;
        }
        let length = length - base;
        let level = unsafe { (*header).cmsg_level };
        let kind = unsafe { (*header).cmsg_type };
        let data = unsafe { libc::CMSG_DATA(header) };
        if level == libc::SOL_SOCKET && kind == libc::SCM_RIGHTS {
            valid = false;
            for index in 0..length / std::mem::size_of::<libc::c_int>() {
                let fd = unsafe { std::ptr::read_unaligned(data.cast::<libc::c_int>().add(index)) };
                if fd >= 0 {
                    drop(unsafe { OwnedFd::from_raw_fd(fd) });
                }
            }
        } else if level == libc::SOL_SOCKET
            && kind == libc::SCM_CREDENTIALS
            && length == std::mem::size_of::<libc::ucred>()
        {
            if credentials.is_some() {
                valid = false;
            }
            credentials = Some(unsafe { std::ptr::read_unaligned(data.cast::<libc::ucred>()) });
        } else {
            valid = false;
        }
        header = unsafe { libc::CMSG_NXTHDR(message, header) };
    }
    if !valid || message.msg_flags & (libc::MSG_TRUNC | libc::MSG_CTRUNC) != 0 {
        return Err(ControlError::Protocol);
    }
    Ok(credentials)
}
