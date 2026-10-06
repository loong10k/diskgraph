use super::git_private_allocation::GitPrivateAllocation;
use super::git_private_capacity::GitPrivateCapacity;
use super::probe_budget::ProbeBudget;
use super::windows_git_cleanup_frame::WindowsGitCleanupFrame;
use super::windows_git_deletion_seal::WindowsGitDeletionSeal;
use super::windows_git_directory_cursor::WindowsGitDirectoryCursor;
use super::windows_git_private_root::WindowsGitPrivateRoot;
use super::windows_git_removal_observation::WindowsGitRemovalObservation;
use std::fs::File;
use std::path::PathBuf;

/// Windows私有目录原创建句柄和增量清理状态；来源：PF-06/Windows原生File ID合同，无Java对应。
/// 任何失败仍持有原父、prepared根、遍历或删除观察，不用路径删除代替原对象证明。
pub(super) struct WindowsGitCleanup {
    parent: File,
    pub(super) root: Option<WindowsGitPrivateRoot>,
    label: PathBuf,
    identity: Option<GitPrivateAllocation>,
    frames: Vec<WindowsGitCleanupFrame>,
    initialized: bool,
    delete_file: Option<File>,
    delete_requested: bool,
    post_mark_verified: bool,
    observation: Option<WindowsGitRemovalObservation>,
}

impl WindowsGitCleanup {
    /// 参数：parent为原创建租约、label仅作账本键、probe为原准入预算。
    /// 返回：持有独立只读原父观察句柄的恢复payload，不延长创建租约的分享限制。
    pub(super) fn new(
        parent: &File,
        label: PathBuf,
        probe: &mut ProbeBudget,
    ) -> Result<Self, String> {
        use std::os::windows::io::{AsRawHandle, FromRawHandle};
        use windows_sys::Wdk::Foundation::OBJECT_ATTRIBUTES;
        use windows_sys::Wdk::Storage::FileSystem::{
            FILE_OPEN_NO_RECALL, FILE_OPEN_REPARSE_POINT, FILE_SYNCHRONOUS_IO_NONALERT, NtOpenFile,
        };
        use windows_sys::Win32::Foundation::{
            INVALID_HANDLE_VALUE, OBJ_DONT_REPARSE, RtlNtStatusToDosError, UNICODE_STRING,
        };
        use windows_sys::Win32::Storage::FileSystem::{
            FILE_LIST_DIRECTORY, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ,
            FILE_SHARE_WRITE, SYNCHRONIZE,
        };
        probe.check().map_err(|error| error.to_string())?;
        let expected = GitPrivateAllocation::from_file(parent)?;
        if !expected.is_directory() {
            return Err("original cleanup parent is not a directory".into());
        }
        // 仅相对原对象重开；复制创建句柄会同时保留其分享限制，阻止原根移动。
        let empty_name = UNICODE_STRING::default();
        let attributes = OBJECT_ATTRIBUTES {
            Length: std::mem::size_of::<OBJECT_ATTRIBUTES>() as u32,
            RootDirectory: parent.as_raw_handle(),
            ObjectName: &empty_name,
            Attributes: OBJ_DONT_REPARSE,
            ..OBJECT_ATTRIBUTES::default()
        };
        let mut handle = std::ptr::null_mut();
        let mut status_block = windows_sys::Win32::System::IO::IO_STATUS_BLOCK::default();
        let status = unsafe {
            NtOpenFile(
                &mut handle,
                FILE_LIST_DIRECTORY | FILE_READ_ATTRIBUTES | SYNCHRONIZE,
                &attributes,
                &mut status_block,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                FILE_SYNCHRONOUS_IO_NONALERT | FILE_OPEN_REPARSE_POINT | FILE_OPEN_NO_RECALL,
            )
        };
        // 状态投影和取消回调之前接管任何有效返回句柄，失败也不会泄漏。
        let opened = (!handle.is_null() && handle != INVALID_HANDLE_VALUE)
            .then(|| unsafe { File::from_raw_handle(handle) });
        if status != 0 {
            return Err(std::io::Error::from_raw_os_error(unsafe {
                RtlNtStatusToDosError(status) as i32
            })
            .to_string());
        }
        let opened = opened.ok_or("original cleanup parent reopen returned no handle")?;
        let actual = GitPrivateAllocation::from_file(&opened)?;
        let held_after = GitPrivateAllocation::from_file(parent)?;
        if !actual.is_directory()
            || !held_after.is_directory()
            || !expected.same_identity(&actual)
            || !expected.same_identity(&held_after)
        {
            return Err("original cleanup parent identity changed".into());
        }
        probe.check().map_err(|error| error.to_string())?;
        Ok(Self {
            parent: opened,
            root: None,
            label,
            identity: None,
            frames: Vec::new(),
            initialized: false,
            delete_file: None,
            delete_requested: false,
            post_mark_verified: false,
            observation: None,
        })
    }

    /// 参数：capacity为原owner登记账本；返回：原根确认删除后成功，否则保留全部恢复状态。
    pub(super) fn cleanup(&mut self, capacity: Option<&GitPrivateCapacity>) -> Result<(), String> {
        let mut probe =
            ProbeBudget::new(&super::ProbeLimits::default()).map_err(|error| error.to_string())?;
        if !self.initialized {
            let root = self
                .root
                .as_mut()
                .ok_or("original root creation handle missing")?;
            root.confirm_created().map_err(|error| error.to_string())?;
            let identity = GitPrivateAllocation::from_file(root.as_file())?;
            if let Some(capacity) = capacity {
                capacity.check_identity(&self.label, root.as_file(), true)?;
            }
            self.frames
                .try_reserve(1)
                .map_err(|error| error.to_string())?;
            let cursor = WindowsGitDirectoryCursor::new(
                root.as_file()
                    .try_clone()
                    .map_err(|error| error.to_string())?,
            )?;
            self.frames.push(WindowsGitCleanupFrame {
                label: self.label.clone(),
                cursor: Some(cursor),
                file: None,
            });
            self.identity = Some(identity);
            self.initialized = true;
        }
        while !self.frames.is_empty() {
            probe.check().map_err(|error| error.to_string())?;
            let last = self.frames.len() - 1;
            if self.frames[last].cursor.is_none() {
                if last == 0 {
                    self.frames.pop();
                    break;
                }
                let (parents, child) = self.frames.split_at_mut(last);
                let parent = &mut parents[last - 1];
                let cursor = parent
                    .cursor
                    .as_mut()
                    .ok_or("original parent cursor missing")?;
                let file = child[0]
                    .file
                    .as_ref()
                    .ok_or("original directory deletion handle missing")?;
                let result = cursor.mark_cleanup_child(
                    file,
                    &parent.label,
                    capacity.ok_or("original allocation ledger missing")?,
                    &mut probe,
                );
                // 标记成功后的错误也必须关闭遍历副本；原seal句柄/观察已由父cursor持有。
                if cursor.cleanup_child_delete_requested() {
                    self.frames.pop();
                }
                result.map_err(|error| error.to_string())?;
                continue;
            }
            let frame = &mut self.frames[last];
            let cursor = frame.cursor.as_mut().expect("active original cursor");
            if cursor.cleanup_child_delete_requested() {
                if !cursor
                    .confirm_cleanup_child_absent(&mut probe)
                    .map_err(|error| error.to_string())?
                {
                    return Err("original child deletion pending; recovery retained".into());
                }
                continue;
            }
            let Some(capacity) = capacity else {
                // 创建后账本建立失败时仅允许确认空目录；不收养未登记内容。
                if cursor.next_entry(&mut probe)?.is_some() {
                    return Err("unregistered private directory contents retained".into());
                }
                frame.cursor = None;
                continue;
            };
            let entry = cursor
                .open_next_cleanup_child(&frame.label, capacity, &mut probe)
                .map_err(|error| error.to_string())?;
            let Some((file, name, attributes)) = entry else {
                frame.cursor = None;
                continue;
            };
            if attributes & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_DIRECTORY != 0 {
                if self.frames.len() >= 128 {
                    return Err("private directory cleanup depth limit; recovery retained".into());
                }
                let label = self.frames[last].label.join(name);
                self.frames
                    .try_reserve(1)
                    .map_err(|error| error.to_string())?;
                let cursor = WindowsGitDirectoryCursor::new(
                    file.try_clone().map_err(|error| error.to_string())?,
                )?;
                self.frames.push(WindowsGitCleanupFrame {
                    label,
                    cursor: Some(cursor),
                    file: Some(file),
                });
            } else {
                let frame = &mut self.frames[last];
                frame
                    .cursor
                    .as_mut()
                    .expect("original cursor")
                    .mark_cleanup_child(&file, &frame.label, capacity, &mut probe)
                    .map_err(|error| error.to_string())?;
            }
        }
        self.finish_root(&mut probe)
    }

    fn finish_root(&mut self, probe: &mut ProbeBudget) -> Result<(), String> {
        let expected = self
            .identity
            .as_ref()
            .ok_or("original root identity missing")?;
        if !self.delete_requested {
            if self.delete_file.is_none() {
                self.delete_file = Some(
                    self.root
                        .as_ref()
                        .ok_or("original root missing")?
                        .reopen_for_delete()
                        .map_err(|error| error.to_string())?,
                );
            }
            if self.observation.is_none() {
                WindowsGitRemovalObservation::prepare_into(
                    &self.parent,
                    expected,
                    probe,
                    &mut self.observation,
                )
                .map_err(|error| error.to_string())?;
            }
            self.observation
                .as_ref()
                .expect("original root observation")
                .check_before_delete(expected, probe)
                .map_err(|error| error.to_string())?;
            mark(self.delete_file.as_ref().expect("original delete handle"))?;
            self.delete_requested = true;
        }
        if !self.post_mark_verified {
            WindowsGitDeletionSeal::verify(
                self.delete_file
                    .as_ref()
                    .ok_or("original root seal handle missing")?,
                expected,
            )
            .map_err(|error| error.to_string())?;
            self.post_mark_verified = true;
            // 全部原根句柄关闭后才能观察真实删除；副作用与seal事实先锁存。
            self.delete_file = None;
            self.root = None;
        }
        probe.check().map_err(|error| error.to_string())?;
        if self
            .observation
            .as_mut()
            .ok_or("original root observation missing")?
            .confirm(expected, probe)
            .map_err(|error| error.to_string())?
        {
            Ok(())
        } else {
            Err("original root deletion pending; recovery retained".into())
        }
    }
}

fn mark(file: &File) -> Result<(), String> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_DISPOSITION_INFO, FileDispositionInfo, SetFileInformationByHandle,
    };
    let disposition = FILE_DISPOSITION_INFO { DeleteFile: true };
    let result = unsafe {
        SetFileInformationByHandle(
            file.as_raw_handle(),
            FileDispositionInfo,
            (&disposition as *const FILE_DISPOSITION_INFO).cast(),
            std::mem::size_of::<FILE_DISPOSITION_INFO>() as u32,
        )
    };
    if result == 0 {
        Err(std::io::Error::last_os_error().to_string())
    } else {
        Ok(())
    }
}
