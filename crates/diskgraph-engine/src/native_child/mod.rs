//! 共用原生子进程所有权；不创建预算、授权或数据库 owner。

#[cfg(all(test, any(unix, windows)))]
mod checkpoint_tests;
mod child_error;
#[cfg(any(unix, windows))]
mod child_input_mode;
#[cfg(unix)]
mod child_read_buffer;
mod child_spawn_error;
#[cfg(any(unix, windows))]
mod control_write_status;
#[cfg(target_os = "macos")]
mod macos_child_group;
#[cfg(all(test, target_os = "macos"))]
mod macos_group_query_tests;
#[cfg(unix)]
mod unix_child;
#[cfg(unix)]
mod unix_control_channel;
#[cfg(all(test, unix))]
mod unix_control_fixture;
#[cfg(all(test, unix))]
mod unix_control_input_tests;
#[cfg(all(test, unix))]
mod unix_control_test_support;
#[cfg(all(test, unix))]
mod unix_normal_exit_fixture;
#[cfg(all(test, unix))]
mod unix_normal_exit_test_support;
#[cfg(all(test, unix))]
mod unix_normal_exit_tests;
#[cfg(windows)]
mod windows;

pub(crate) use child_error::ChildError;
#[cfg(any(unix, windows))]
pub(crate) use child_input_mode::ChildInputMode;
pub(crate) use child_spawn_error::ChildSpawnError;
#[cfg(any(unix, windows))]
pub(crate) use control_write_status::ControlWriteStatus;
#[cfg(unix)]
pub(crate) use unix_child::UnixChild;
#[cfg(all(windows, test))]
pub(crate) use windows::OwnedHandle;
#[cfg(windows)]
pub(crate) use windows::WindowsChild;

#[cfg(all(unix, any(test, target_os = "macos")))]
mod unix_normal_exit;

#[cfg(target_os = "macos")]
mod macos_group_view;

#[cfg(test)]
mod native_io_error_tests;

#[cfg(all(test, target_os = "linux"))]
mod linux_atomic_birth_fixture;
#[cfg(all(test, target_os = "linux"))]
mod linux_atomic_birth_test_support;
#[cfg(all(test, target_os = "linux"))]
mod linux_atomic_birth_tests;

#[cfg(target_os = "linux")]
mod linux_atomic_abi;
#[cfg(target_os = "linux")]
mod linux_atomic_birth_observer;
#[cfg(target_os = "linux")]
mod linux_atomic_child;
#[cfg(target_os = "linux")]
mod linux_atomic_exit;
#[cfg(target_os = "linux")]
mod linux_atomic_handshake;
#[cfg(target_os = "linux")]
mod linux_atomic_launch_failure;
#[cfg(target_os = "linux")]
mod linux_atomic_launcher;
#[cfg(all(test, target_os = "linux"))]
mod linux_atomic_launcher_failure_tests;
#[cfg(all(test, target_os = "linux"))]
mod linux_atomic_launcher_fixture;
#[cfg(all(test, target_os = "linux"))]
mod linux_atomic_launcher_ready_tests;
#[cfg(all(test, target_os = "linux"))]
mod linux_atomic_launcher_reap_tests;
#[cfg(all(test, target_os = "linux"))]
mod linux_atomic_launcher_test_support;
#[cfg(all(test, target_os = "linux"))]
mod linux_atomic_launcher_tests;
#[cfg(target_os = "linux")]
mod linux_atomic_message;
#[cfg(all(test, target_os = "linux"))]
mod linux_atomic_normal_contract_tests;
#[cfg(target_os = "linux")]
mod linux_atomic_pipes;
#[cfg(all(test, target_os = "linux"))]
mod linux_atomic_reaper_fixture;
#[cfg(target_os = "linux")]
mod linux_atomic_signal_mask;
#[cfg(all(test, target_os = "linux"))]
mod linux_atomic_startup_reset_tests;
#[cfg(target_os = "linux")]
mod linux_scanner_filter;
#[cfg(unix)]
mod unix_child_setup;
#[cfg(target_os = "linux")]
pub(crate) use linux_atomic_child::LinuxAtomicChild;
#[cfg(target_os = "linux")]
pub(crate) use linux_atomic_launcher::LinuxAtomicLauncher;
#[cfg(target_os = "linux")]
mod linux_scan_image;
#[cfg(all(test, target_os = "linux"))]
mod linux_scan_image_error;
#[cfg(all(test, target_os = "linux"))]
mod linux_scan_image_fixture;
#[cfg(all(test, target_os = "linux"))]
mod linux_scan_image_tests;
#[cfg(target_os = "linux")]
pub(crate) use linux_scan_image::LinuxScanImage;

#[cfg(all(test, unix))]
mod unix_leader_tests;

#[cfg(unix)]
mod unix_leader;

#[cfg(all(test, target_os = "macos"))]
mod macos_native_launcher_tests;

#[cfg(target_os = "macos")]
mod macos_native_launcher;
#[cfg(target_os = "macos")]
mod macos_native_pipes;
#[cfg(unix)]
mod unix_child_group;

#[cfg(target_os = "macos")]
mod macos_native_spawn;

#[cfg(all(test, target_os = "macos"))]
mod native_birth_gate_tests;

#[cfg(target_os = "macos")]
mod native_birth_gate;

#[cfg(target_os = "macos")]
pub(crate) use macos_native_launcher::MacosNativeLauncher;

#[cfg(windows)]
pub(crate) use windows::CleanupProgress;

#[cfg(windows)]
pub(crate) use windows::WindowsDirectoryNotificationIo;

#[cfg(all(test, target_os = "macos"))]
mod macos_registry_deadline_tests;

#[cfg(all(test, windows))]
pub(crate) use windows::WindowsTestBirth;
