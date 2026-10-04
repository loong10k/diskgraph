//! 共用原生子进程所有权；不创建预算、授权或数据库 owner。

#[cfg(all(test, any(unix, windows)))]
mod checkpoint_tests;
mod child_error;
mod child_spawn_error;
#[cfg(target_os = "macos")]
mod macos_child_group;
#[cfg(unix)]
mod unix_child;
#[cfg(windows)]
mod windows;

pub(crate) use child_error::ChildError;
pub(crate) use child_spawn_error::ChildSpawnError;
#[cfg(unix)]
pub(crate) use unix_child::UnixChild;
#[cfg(all(windows, test))]
pub(crate) use windows::OwnedHandle;
#[cfg(windows)]
pub(crate) use windows::WindowsChild;
