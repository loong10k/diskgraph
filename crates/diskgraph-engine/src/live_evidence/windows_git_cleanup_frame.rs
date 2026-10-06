use super::windows_git_directory_cursor::WindowsGitDirectoryCursor;
use std::fs::File;
use std::path::PathBuf;

/// 保留一层原目录遍历和待标记对象；来源：Windows句柄相对Git清理，无Java对应。
pub(super) struct WindowsGitCleanupFrame {
    pub(super) label: PathBuf,
    pub(super) cursor: Option<WindowsGitDirectoryCursor>,
    pub(super) file: Option<File>,
}
