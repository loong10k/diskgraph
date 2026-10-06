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
        Err(ChildSpawnError::Checkpoint(
            "retain actual suspended process"
        ))
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
