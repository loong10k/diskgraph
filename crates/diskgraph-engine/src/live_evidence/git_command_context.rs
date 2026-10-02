//! 采样和准备命令只在私有 Git 目录中执行，统一环境与累计预算。

use super::git_executable::GitExecutable;
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
        Ok(Self {
            tool: GitExecutable::resolve(tool, probe)?,
            directory: directory.to_owned(),
            worktree: worktree.to_owned(),
            shell_path: trusted_shell_path(probe)?,
            #[cfg(windows)]
            system_root: std::env::var_os("SystemRoot"),
        })
    }

    /// 定位完成后绑定实际工作树，保持首次解析的程序路径。
    /// 参数：worktree 为已核验的工作树；返回：无，不重新解析 PATH。
    pub(super) fn bind_worktree(&mut self, worktree: &Path) {
        self.worktree = worktree.to_owned();
    }

    /// 执行固定采样命令，源 config/index/refs 从不成为 Git 元数据入口。
    /// 参数：args 为结构化固定参数；probe 为准备及采样的同一预算。
    /// 返回：完整进程输出或明确资源错误。
    pub(super) fn run(
        &self,
        args: &[&str],
        probe: &mut ProbeBudget,
    ) -> Result<ProbeOutput, String> {
        let mut command = self.command(args.iter().map(OsStr::new), false);
        run_probe(&mut command, probe).map_err(|error| error.to_string())
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
        let mut command = self.command(args.iter().copied(), true);
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
        run_probe(&mut command, probe).map_err(|error| error.to_string())
    }

    fn command(&self, args: impl Iterator<Item = impl AsRef<OsStr>>, bootstrap: bool) -> Command {
        let mut command = Command::new(self.tool.path());
        command
            .args(["--no-pager", "--no-lazy-fetch", "--no-optional-locks"])
            .args(args);
        command.current_dir(if bootstrap {
            &self.directory
        } else {
            &self.worktree
        });
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
            .env("GIT_DIR", self.directory.join("repo"))
            .env("GIT_INDEX_FILE", self.directory.join("repo/index"))
            .env(
                "GIT_WORK_TREE",
                if bootstrap {
                    &self.directory
                } else {
                    &self.worktree
                },
            )
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_SYSTEM", self.directory.join("empty"))
            .env("GIT_CONFIG_GLOBAL", self.directory.join("empty"))
            .env("GIT_ATTR_NOSYSTEM", "1")
            .env("GIT_NO_REPLACE_OBJECTS", "1")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_TRACE2", "0")
            .env("GIT_TRACE2_EVENT", "0")
            .env("GIT_TRACE2_PERF", "0");
        command
    }
}

fn trusted_shell_path(probe: &mut ProbeBudget) -> Result<Option<OsString>, String> {
    let Some(path) = std::env::var_os("PATH") else {
        return Ok(None);
    };
    let mut directories = Vec::new();
    for directory in std::env::split_paths(&path) {
        probe.check().map_err(|error| error.to_string())?;
        if directory.is_absolute() {
            // 绝对目录的程序内容仍由宿主信任，不认证管理员可写安装包。
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
