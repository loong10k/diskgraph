//! 只读测试诊断：计数、PID列表和各句柄查询分时观察，不用于判定正常退出或清理。
use super::owned_handle::OwnedHandle;
use std::ffi::OsString;
use std::io::Write;
use std::os::windows::ffi::OsStringExt;
use std::time::Instant;
use windows_sys::Win32::Foundation::{FILETIME, GetLastError, HANDLE};
use windows_sys::Win32::System::JobObjects::{
    IsProcessInJob, JobObjectBasicProcessIdList, QueryInformationJobObject,
};
use windows_sys::Win32::System::Threading::{
    GetProcessId, GetProcessTimes, OpenProcess, PROCESS_NAME_WIN32,
    PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};

/// 有界Job成员观察器；来源：Win32只读Job/PID/进程句柄API，无Java对象。
pub(super) struct WindowsJobTestDiagnostics;

impl WindowsJobTestDiagnostics {
    /// 参数：原Job和leader借用句柄、先前计数及其观察起点；返回无，错误原码输出而不修改断言。
    /// 每次最多64PID且不重试；取得的查询句柄本轮结束前全部关闭，不参与任何终止或出生。
    pub(super) fn observe(job: HANDLE, leader: HANDLE, active: u32, started: Instant) {
        let leader_pid = unsafe { GetProcessId(leader) };
        if leader_pid == 0 {
            let error = unsafe { GetLastError() };
            Self::error("GetProcessId(held leader)", 0, error, started);
        }
        let _ = writeln!(
            std::io::stderr(),
            "DG_JOB_DIAG accounting_active={active} held_leader_pid={leader_pid} elapsed_us={} observations_are_not_atomic=true",
            started.elapsed().as_micros()
        );
        // 两个DWORD头部后为ULONG_PTR数组；usize保证原生对齐，最多64项的固定缓冲。
        let mut storage = [0_usize; 66];
        let bytes = 8 + 64 * std::mem::size_of::<usize>();
        let mut returned = 0_u32;
        if unsafe {
            QueryInformationJobObject(
                job,
                JobObjectBasicProcessIdList,
                storage.as_mut_ptr().cast(),
                bytes as u32,
                &mut returned,
            )
        } == 0
        {
            let error = unsafe { GetLastError() };
            Self::error(
                "JobObjectBasicProcessIdList(capacity=64; no retry)",
                0,
                error,
                started,
            );
            return;
        }
        let header = storage.as_ptr().cast::<u32>();
        let assigned = unsafe { header.read() };
        let listed = unsafe { header.add(1).read() };
        let _ = writeln!(
            std::io::stderr(),
            "DG_JOB_DIAG list_assigned={assigned} list_returned={listed} returned_bytes={returned} elapsed_us={}",
            started.elapsed().as_micros()
        );
        if listed > 64 {
            let _ = writeln!(
                std::io::stderr(),
                "DG_JOB_DIAG invalid_list_count={listed}; no member reads"
            );
            return;
        }
        let pids = unsafe { storage.as_ptr().cast::<u8>().add(8).cast::<usize>() };
        for index in 0..listed as usize {
            let native_pid = unsafe { pids.add(index).read() };
            let _ = writeln!(
                std::io::stderr(),
                "DG_JOB_DIAG listed_pid={native_pid} index={index} elapsed_us={}",
                started.elapsed().as_micros()
            );
            match u32::try_from(native_pid) {
                Ok(pid) => Self::member(job, pid, started),
                Err(_) => {
                    let _ = writeln!(
                        std::io::stderr(),
                        "DG_JOB_DIAG pid_not_representable={native_pid}"
                    );
                }
            }
        }
    }

    fn member(job: HANDLE, pid: u32, started: Instant) {
        let raw = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        if raw.is_null() {
            let error = unsafe { GetLastError() };
            Self::error("OpenProcess(query only)", pid, error, started);
            return;
        }
        let handle = match OwnedHandle::from_raw(raw, "diagnostic process") {
            Ok(handle) => handle,
            Err(error) => {
                let _ = writeln!(
                    std::io::stderr(),
                    "DG_JOB_DIAG pid={pid} invalid_query_handle={error:?}"
                );
                return;
            }
        };
        // PID列表和OpenProcess间可有退出/复用；用实际句柄再次查询同一Job归属，不按名称猜测。
        let mut in_job = 0;
        if unsafe { IsProcessInJob(handle.as_raw(), job, &mut in_job) } == 0 {
            let error = unsafe { GetLastError() };
            Self::error("IsProcessInJob(opened handle)", pid, error, started);
        } else {
            let _ = writeln!(
                std::io::stderr(),
                "DG_JOB_DIAG pid={pid} opened_handle_in_job={in_job} elapsed_us={}",
                started.elapsed().as_micros()
            );
        }
        let mut creation = FILETIME::default();
        let mut exit = FILETIME::default();
        let mut kernel = FILETIME::default();
        let mut user = FILETIME::default();
        if unsafe {
            GetProcessTimes(
                handle.as_raw(),
                &mut creation,
                &mut exit,
                &mut kernel,
                &mut user,
            )
        } == 0
        {
            let error = unsafe { GetLastError() };
            Self::error("GetProcessTimes", pid, error, started);
        } else {
            let created =
                (u64::from(creation.dwHighDateTime) << 32) | u64::from(creation.dwLowDateTime);
            let exited = (u64::from(exit.dwHighDateTime) << 32) | u64::from(exit.dwLowDateTime);
            let _ = writeln!(
                std::io::stderr(),
                "DG_JOB_DIAG pid={pid} creation_filetime_100ns={created} exit_filetime_100ns={exited} elapsed_us={}",
                started.elapsed().as_micros()
            );
        }
        let mut image = [0_u16; 32768];
        let mut length = image.len() as u32;
        if unsafe {
            QueryFullProcessImageNameW(
                handle.as_raw(),
                PROCESS_NAME_WIN32,
                image.as_mut_ptr(),
                &mut length,
            )
        } == 0
        {
            let error = unsafe { GetLastError() };
            Self::error("QueryFullProcessImageNameW", pid, error, started);
        } else if length as usize <= image.len() {
            let name = OsString::from_wide(&image[..length as usize]);
            let _ = writeln!(
                std::io::stderr(),
                "DG_JOB_DIAG pid={pid} image={name:?} elapsed_us={}",
                started.elapsed().as_micros()
            );
        } else {
            let _ = writeln!(
                std::io::stderr(),
                "DG_JOB_DIAG pid={pid} invalid_image_length={length}"
            );
        }
        // 所有观察完成即关闭；不将额外引用保留到release或正常退出判定。
        drop(handle);
    }

    fn error(stage: &str, pid: u32, code: u32, started: Instant) {
        let error = std::io::Error::from_raw_os_error(code as i32);
        let _ = writeln!(
            std::io::stderr(),
            "DG_JOB_DIAG stage={stage:?} pid={pid} raw_os_error={code} error={error:?} elapsed_us={}",
            started.elapsed().as_micros()
        );
    }
}
