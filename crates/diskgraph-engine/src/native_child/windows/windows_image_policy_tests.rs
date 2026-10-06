//! 在真实挂起进程上核验加载策略；来源：Win32 GetProcessMitigationPolicy，不以属性常量模拟生效。
use super::WindowsChild;
use crate::native_child::{ChildInputMode, ChildSpawnError};
use std::process::Command;
use windows_sys::Win32::System::SystemServices::PROCESS_MITIGATION_IMAGE_LOAD_POLICY;
use windows_sys::Win32::System::Threading::{GetProcessMitigationPolicy, ProcessImageLoadPolicy};

#[test]
fn suspended_birth_enforces_image_load_policy_before_resume() {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command.env_clear();
    let mut owner = None;
    let mut checkpoints = 0;
    let birth = WindowsChild::spawn_into(&mut command, ChildInputMode::Null, &mut owner, || {
        checkpoints += 1;
        if checkpoints == 3 {
            Err("retain actual suspended process")
        } else {
            Ok(())
        }
    });
    let child = owner
        .as_mut()
        .expect("actual suspended process retained outside birth");
    let process = child.process.as_ref().expect("actual process handle");
    let mut policy = PROCESS_MITIGATION_IMAGE_LOAD_POLICY::default();
    let queried = unsafe {
        GetProcessMitigationPolicy(
            process.as_raw(),
            ProcessImageLoadPolicy,
            (&mut policy as *mut PROCESS_MITIGATION_IMAGE_LOAD_POLICY).cast(),
            std::mem::size_of::<PROCESS_MITIGATION_IMAGE_LOAD_POLICY>(),
        )
    };
    let query_error = std::io::Error::last_os_error();
    // 查询后先实际收取原Job/leader和pending I/O，再断言，避免失败测试遗留挂起进程。
    child.cleanup().unwrap();
    assert!(matches!(
        birth,
        Err(ChildSpawnError::Checkpoint {
            primary: "retain actual suspended process",
            cleanup: None,
        })
    ));
    assert_eq!(checkpoints, 3);
    assert_ne!(queried, 0, "native mitigation query failed: {query_error}");
    // SDK Flags的低三位依次为NoRemoteImages、NoLowMandatoryLabelImages、PreferSystem32Images。
    assert_eq!(
        unsafe { policy.Anonymous.Flags } & 7,
        7,
        "original suspended process lacks required image load restrictions"
    );
}

fn signature_flags(mode: ChildInputMode) -> u32 {
    use windows_sys::Win32::System::SystemServices::PROCESS_MITIGATION_BINARY_SIGNATURE_POLICY;
    use windows_sys::Win32::System::Threading::ProcessSignaturePolicy;
    let mut command = Command::new(std::env::current_exe().unwrap());
    command.env_clear();
    let mut owner = None;
    let mut checkpoints = 0;
    let birth = WindowsChild::spawn_into(&mut command, mode, &mut owner, || {
        checkpoints += 1;
        if checkpoints == 3 {
            Err("inspect original suspended signature policy")
        } else {
            Ok(())
        }
    });
    let child = owner.as_mut().expect("outside original birth owner");
    let mut policy = PROCESS_MITIGATION_BINARY_SIGNATURE_POLICY::default();
    let queried = match child.process.as_ref() {
        Some(process) => unsafe {
            GetProcessMitigationPolicy(
                process.as_raw(),
                ProcessSignaturePolicy,
                (&mut policy as *mut PROCESS_MITIGATION_BINARY_SIGNATURE_POLICY).cast(),
                std::mem::size_of::<PROCESS_MITIGATION_BINARY_SIGNATURE_POLICY>(),
            )
        },
        None => 0,
    };
    let query_error = std::io::Error::last_os_error();
    // 原 Job 和可能的 pending 控制连接先真实清理；政策断言失败也不遗留挂起进程。
    let cleanup = child.cleanup();
    assert!(
        matches!(
            birth,
            Err(ChildSpawnError::Checkpoint {
                primary: "inspect original suspended signature policy",
                cleanup: None,
            })
        ),
        "native suspended birth failed: {birth:?}; cleanup={cleanup:?}"
    );
    cleanup.unwrap();
    assert_eq!(checkpoints, 3);
    assert_ne!(queried, 0, "actual signature query failed: {query_error}");
    unsafe { policy.Anonymous.Flags }
}

#[test]
fn suspended_scan_control_requires_microsoft_signature_before_resume() {
    assert_eq!(
        signature_flags(ChildInputMode::WorkerControl) & 1,
        1,
        "scan-control process can still load non-Microsoft images"
    );
}

#[test]
fn suspended_null_probe_retains_existing_signature_policy() {
    assert_eq!(
        signature_flags(ChildInputMode::Null) & 1,
        0,
        "ordinary probe acquired the scan-only DLL restriction"
    );
}
