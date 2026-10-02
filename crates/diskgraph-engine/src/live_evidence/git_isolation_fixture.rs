//! Git 隔离回归的原生临时仓库；不接触用户仓库或远端网络。

use super::{GitSample, ProbeLimits, sample_git_bounded};
use std::fs::{FileTimes, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::SystemTime;

/// 拥有单次测试的真实 Git 仓库与空全局配置。
/// 来源：原生 Rust diskgraph-engine::live_evidence 的 D20 端到端夹具。
pub(super) struct GitIsolationFixture {
    temp: tempfile::TempDir,
    project: PathBuf,
}

impl GitIsolationFixture {
    /// 初始化指定对象格式的仓库并提交一个普通文件。
    /// 参数：format 为 Git 支持的 sha1 或 sha256；返回：独占临时夹具。
    pub(super) fn new(format: &str) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("repo");
        std::fs::create_dir(&project).unwrap();
        std::fs::write(temp.path().join("empty-config"), b"").unwrap();
        let fixture = Self {
            temp,
            project: project.canonicalize().unwrap(),
        };
        fixture.git(&["init", "-q", &format!("--object-format={format}")]);
        fixture.git(&["symbolic-ref", "HEAD", "refs/heads/main"]);
        fixture.git(&["config", "user.name", "fixture"]);
        fixture.git(&["config", "user.email", "f@example.invalid"]);
        std::fs::write(fixture.path().join("tracked"), b"AAAA\n").unwrap();
        fixture.git(&["add", "tracked"]);
        fixture.git(&["commit", "-q", "-m", "base"]);
        fixture
    }

    /// 返回已规范化的实际临时项目根。
    /// 参数：无；返回：不含临时目录链接别名的原生路径。
    pub(super) fn path(&self) -> &Path {
        &self.project
    }

    /// 构造供 Git 工具创建的同级夹具路径，不把 Windows verbatim 前缀传给工具。
    /// 参数：name 为本测试固定的单个叶名称；返回：尚未创建的临时目录内原生路径。
    pub(super) fn sibling_path(&self, name: &str) -> PathBuf {
        assert!(matches!(name, "linked"));
        self.temp.path().join(name)
    }

    /// 创建仅访问本夹具的 Git 命令，禁用系统和用户配置。
    /// 参数：args 为固定 Git 参数；返回：尚未启动的独立命令。
    pub(super) fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new("git");
        command.env_clear();
        if let Some(path) = std::env::var_os("PATH") {
            command.env("PATH", path);
        }
        #[cfg(windows)]
        if let Some(root) = std::env::var_os("SystemRoot") {
            command.env("SystemRoot", root);
        }
        command
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", self.temp.path().join("empty-config"))
            .env("GIT_CONFIG_SYSTEM", self.temp.path().join("empty-config"))
            .env("GIT_TERMINAL_PROMPT", "0")
            .args(args)
            .current_dir(self.path());
        command
    }

    /// 执行夹具准备或正向控制命令并保留退出状态。
    /// 参数：args 为固定参数；返回：实际 Git 的完整测试输出。
    pub(super) fn output(&self, args: &[&str]) -> Output {
        self.command(args).output().unwrap()
    }

    /// 执行必须成功的夹具命令。
    /// 参数：args 为固定参数；返回：真实 stdout，失败时给出完整诊断。
    pub(super) fn git(&self, args: &[&str]) -> Vec<u8> {
        let output = self.output(args);
        assert!(output.status.success(), "git {args:?}: {output:?}");
        output.stdout
    }

    /// 通过当前公开有界入口采样，不使用测试替身绕过生产逻辑。
    /// 参数：无；返回：实际样本或生产诊断。
    pub(super) fn sample(&self) -> Result<GitSample, String> {
        sample_git_bounded(Path::new("git"), self.path(), &ProbeLimits::default())
    }

    /// 获取源 metadata 的完整目录、字节和修改时间哨兵。
    /// 参数：无；返回：按原生路径排序的目录及文件水位，不含访问时间。
    pub(super) fn metadata(&self) -> Vec<(PathBuf, Vec<u8>, SystemTime)> {
        let root = self.path().join(".git");
        let mut pending = vec![root.clone()];
        let mut observed = Vec::new();
        while let Some(path) = pending.pop() {
            let metadata = std::fs::symlink_metadata(&path).unwrap();
            assert!(!metadata.file_type().is_symlink(), "fixture link: {path:?}");
            let bytes = if metadata.is_dir() {
                for entry in std::fs::read_dir(&path).unwrap() {
                    pending.push(entry.unwrap().path());
                }
                Vec::new()
            } else {
                assert!(metadata.is_file(), "fixture special file: {path:?}");
                std::fs::read(&path).unwrap()
            };
            observed.push((
                path.strip_prefix(&root).unwrap().to_path_buf(),
                bytes,
                metadata.modified().unwrap(),
            ));
        }
        observed.sort_by(|left, right| left.0.cmp(&right.0));
        observed
    }

    /// 对比每项源字节和 mtime，只输出变化路径，避免失败时倾倒全部对象字节。
    /// 参数：before 为采样前哨兵；返回：无，变化或条目增减时断言失败。
    pub(super) fn assert_metadata_unchanged(&self, before: &[(PathBuf, Vec<u8>, SystemTime)]) {
        let after = self.metadata();
        assert_eq!(
            after.len(),
            before.len(),
            "source metadata entry count changed"
        );
        for (current, previous) in after.iter().zip(before) {
            assert!(
                current == previous,
                "source metadata changed: {:?}",
                previous.0
            );
        }
    }

    /// 用原生可写句柄设置精确 mtime，关闭句柄后复读验证。
    /// 参数：path 为夹具文件，modified 为目标时间；返回：无，无法保真时失败。
    pub(super) fn set_modified(path: &Path, modified: SystemTime) {
        OpenOptions::new()
            .write(true)
            .open(path)
            .unwrap()
            .set_times(FileTimes::new().set_modified(modified))
            .unwrap();
        assert_eq!(
            std::fs::metadata(path).unwrap().modified().unwrap(),
            modified
        );
    }

    /// 由独立 Rust writer 生成本地脚本，writer 退出后才交给 Git 执行。
    /// 参数：name 为夹具脚本名，source 为有限脚本内容；返回：已关闭的原生路径。
    #[cfg(unix)]
    pub(super) fn script(&self, name: &str, source: &str) -> PathBuf {
        let path = self.path().parent().unwrap().join(name);
        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "live_evidence::git_isolation_tests::script_writer_fixture",
                "--quiet",
                "--nocapture",
            ])
            .env_clear()
            .env("DG_ISOLATION_SCRIPT_PATH", &path)
            .env("DG_ISOLATION_SCRIPT_BYTES", source)
            .output()
            .unwrap();
        assert!(output.status.success(), "script writer: {output:?}");
        path
    }
}
