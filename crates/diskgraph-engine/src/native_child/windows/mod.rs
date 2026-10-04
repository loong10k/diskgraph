//! Win32 Job、启动属性与双管道的共用真实所有权。

mod attribute_list;
mod overlapped_pipe;
mod owned_handle;
mod pipe_security;
mod windows_child;
mod windows_command_line;

#[cfg(test)]
pub(crate) use owned_handle::OwnedHandle;
pub(crate) use windows_child::WindowsChild;
