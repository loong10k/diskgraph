//! 采样和准备命令只在私有 Git 目录中执行，统一环境与累计预算。

use super::git_executable::GitExecutable;
use super::git_tool_path;
use super::probe_budget::ProbeBudget;
use super::probe_execution::run_probe;
use super::probe_output::ProbeOutput;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::Command;

/// 受信 Git 程序与本次私有路径，不含可变请求身份。
/// 来源：原生 Rust DiskGraph 私有 Git 命令执行上下文，无 Java 对应实现。
pub(super) struct GitCommandContext {
    tool: GitExecutable,
    directory: PathBuf,
    worktree: PathBuf,
    shell_path: Option<OsString>,
    #[cfg(windows)]
    system_root: Option<OsString>,
}

impl GitCommandContext {
    /// 建立固定程序和私有目录的执行上下文。
    /// 参数：tool 为受信程序；directory 为独占私有根；worktree 为原生工作目录。
    /// 返回：固定绝对工具和仅含受信绝对目录的 PATH 快照；构造时不启动程序。
    pub(super) fn new(
        tool: &Path,
        directory: &Path,
        worktree: &Path,
        probe: &mut ProbeBudget,
    ) -> Result<Self, String> {
        // 私有及源路径的原生边界保持不变；工具表示不可保真时在 spawn 前拒绝。
        git_tool_path::from_native(directory)
            .map_err(|error| format!("Git private directory: {error}"))?;
        git_tool_path::from_native(worktree).map_err(|error| format!("Git worktree: {error}"))?;
        Ok(Self {
            tool: GitExecutable::resolve(tool, probe)
                .map_err(|error| format!("Git executable resolution: {error}"))?,
            directory: directory.to_owned(),
            worktree: worktree.to_owned(),
            shell_path: trusted_shell_path(probe)?,
            #[cfg(windows)]
            system_root: std::env::var_os("SystemRoot")
                .map(|root| {
                    git_tool_path::from_native(Path::new(&root)).map(PathBuf::into_os_string)
                })
                .transpose()
                .map_err(|error| format!("Git SystemRoot: {error}"))?,
        })
    }

    /// 定位完成后绑定实际工作树，保持首次解析的程序路径。
    /// 参数：worktree 为已核验的工作树；返回：无，不重新解析 PATH。
    pub(super) fn bind_worktree(&mut self, worktree: &Path) {
        self.worktree = worktree.to_owned();
    }

    #[cfg(all(test, windows))]
    /// 借用构造时筛选的 shell 搜索路径，只供隔离宿主回归核验。
    /// 参数：无；返回：筛选后的 PATH，缺少可安全表示的目录时为 None。
    pub(super) fn shell_path_for_test(&self) -> Option<&OsStr> {
        self.shell_path.as_deref()
    }

    /// 执行固定采样命令，源 config/index/refs 从不成为 Git 元数据入口。
    /// 参数：args 为结构化固定参数；probe 为准备及采样的同一预算。
    /// 返回：完整进程输出或明确资源错误。
    pub(super) fn run(
        &self,
        args: &[&str],
        probe: &mut ProbeBudget,
    ) -> Result<ProbeOutput, String> {
        let mut command = self.command(args.iter().map(OsStr::new), false)?;
        run_probe(&mut command, probe).map_err(super::git_output::execution_error)
    }

    /// 在空 bootstrap 工作目录解析配置副本或读取固定宿主数据。
    /// 参数：args 为结构化数据参数；probe 为同一预算；host_system 仅允许固定路径 printer，host_attributes 用于固定属性路径发现。
    /// 返回：完整输出；调用者必须校验退出状态及记录格式。
    pub(super) fn bootstrap(
        &self,
        args: &[&OsStr],
        probe: &mut ProbeBudget,
        host_system: bool,
        host_attributes: bool,
    ) -> Result<ProbeOutput, String> {
        if host_system
            && (host_attributes
                || args
                    != [
                        OsStr::new("config"),
                        OsStr::new("--system"),
                        OsStr::new("--edit"),
                    ])
        {
            return Err("unsupported Git host configuration discovery command".into());
        }
        let mut command = self.command(args.iter().copied(), true)?;
        if host_system {
            // NOSYSTEM 始终为 1；只计算安装包选择的路径，固定 printer 不读写目标。
            // Git 以独立 argv 传路径，路径字节从不成为 shell 程序文本。
            command
                .env_remove("GIT_CONFIG_SYSTEM")
                .env("GIT_EDITOR", "printf '%s\\0'");
        }
        if host_attributes {
            command.env_remove("GIT_ATTR_NOSYSTEM");
        }
        run_probe(&mut command, probe).map_err(super::git_output::execution_error)
    }

    fn command(
        &self,
        args: impl Iterator<Item = impl AsRef<OsStr>>,
        bootstrap: bool,
    ) -> Result<Command, String> {
        let directory = git_tool_path::from_native(&self.directory)
            .map_err(|error| format!("Git private directory: {error}"))?;
        let worktree = git_tool_path::from_native(&self.worktree)
            .map_err(|error| format!("Git worktree: {error}"))?;
        let repo = git_tool_path::from_native(&self.directory.join("repo"))
            .map_err(|error| format!("Git private metadata directory: {error}"))?;
        let index = git_tool_path::from_native(&self.directory.join("repo/index"))
            .map_err(|error| format!("Git private index: {error}"))?;
        let empty = git_tool_path::from_native(&self.directory.join("empty"))
            .map_err(|error| format!("Git private empty configuration: {error}"))?;
        let mut command = Command::new(self.tool.path());
        command
            .args(["--no-pager", "--no-lazy-fetch", "--no-optional-locks"])
            .args(args);
        command.current_dir(if bootstrap { &directory } else { &worktree });
        // 清空 BASH_ENV/ENV/loader 等启动注入；后续只使用构造时捕获的宿主环境。
        command.env_clear();
        match &self.shell_path {
            Some(path) => {
                command.env("PATH", path);
            }
            None => {
                command.env_remove("PATH");
            }
        }
        #[cfg(windows)]
        if let Some(root) = &self.system_root {
            command.env("SystemRoot", root);
        }
        command
            .env("GIT_DIR", repo)
            .env("GIT_INDEX_FILE", index)
            .env(
                "GIT_WORK_TREE",
                if bootstrap { &directory } else { &worktree },
            )
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_SYSTEM", &empty)
            .env("GIT_CONFIG_GLOBAL", &empty)
            .env("GIT_ATTR_NOSYSTEM", "1")
            .env("GIT_NO_REPLACE_OBJECTS", "1")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_TRACE2", "0")
            .env("GIT_TRACE2_EVENT", "0")
            .env("GIT_TRACE2_PERF", "0");
        Ok(command)
    }
}

fn trusted_shell_path(probe: &mut ProbeBudget) -> Result<Option<OsString>, String> {
    let Some(path) = std::env::var_os("PATH") else {
        return Ok(None);
    };
    let mut directories = Vec::new();
    for directory in std::env::split_paths(&path) {
        probe.check().map_err(|error| error.to_string())?;
        if directory.is_absolute()
            && let Ok(directory) = git_tool_path::from_native(&directory)
        {
            // 绝对目录的程序内容仍由宿主信任，不认证管理员可写安装包。
            // 不能安全表示的宿主搜索项不进入子进程，不让无关项阻断固定工具。
            directories.push(directory);
        }
    }
    if directories.is_empty() {
        return Ok(None);
    }
    std::env::join_paths(directories)
        .map(Some)
        .map_err(|error| format!("unsupported Git shell PATH: {error}"))
}
