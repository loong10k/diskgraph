//! Win32 Job、启动属性与双管道的共用真实所有权。

mod attribute_list;
mod overlapped_control_pipe;
mod overlapped_pipe;
mod owned_handle;
mod pipe_security;
#[cfg(test)]
mod windows_birth_recovery_tests;
#[cfg(test)]
mod windows_birth_test_hook;
mod windows_child;
mod windows_command_line;
#[cfg(test)]
mod windows_io_transfer_tests;
mod windows_normal_exit;
mod windows_overlapped_operation;
#[cfg(test)]
mod windows_probe_directory_recovery_tests;
#[cfg(test)]
mod windows_probe_recovery_tests;

#[cfg(test)]
mod windows_cleanup_hooks;
#[cfg(test)]
mod windows_cleanup_recovery_tests;
#[cfg(test)]
mod windows_cleanup_rescue;
#[cfg(test)]
mod windows_control_fixture;
#[cfg(test)]
mod windows_control_input_tests;
#[cfg(test)]
mod windows_control_spawn_record;
#[cfg(test)]
mod windows_control_test_witness;
#[cfg(test)]
mod windows_job_test_diagnostics;
#[cfg(test)]
mod windows_normal_exit_fixture;
#[cfg(test)]
mod windows_normal_exit_test_support;
#[cfg(test)]
mod windows_normal_exit_tests;

#[cfg(test)]
pub(crate) use owned_handle::OwnedHandle;
pub(crate) use windows_child::WindowsChild;

#[cfg(test)]
mod windows_normal_process_witness;

#[cfg(test)]
mod windows_poll_cleanup_tests;

mod cleanup_progress;

#[cfg(test)]
mod windows_registry_deadline_tests;

pub(crate) use cleanup_progress::CleanupProgress;

mod windows_pipe_io_phase;
#[cfg(test)]
mod windows_prepared_connect_tests;

mod windows_birth_phase;
mod windows_directory_notification_io;
pub(crate) use windows_directory_notification_io::WindowsDirectoryNotificationIo;
#[cfg(test)]
mod windows_directory_notification_tests;
