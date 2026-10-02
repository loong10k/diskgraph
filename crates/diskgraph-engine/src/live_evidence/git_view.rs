//! 本次采样的私有 Git 配置、index 与引用；原工作树仅作为实时数据读取。

use super::git_command_context::GitCommandContext;
use super::git_configuration::GitConfiguration;
use super::git_index_layout::GitIndexLayout;
use super::git_metadata_budget::GitMetadataBudget;
use super::git_metadata_tree::GitMetadataTree;
use super::git_native_path;
use super::git_output::successful;
use super::git_private_directory::GitPrivateDirectory;
use super::git_system_configuration;
use super::probe_budget::ProbeBudget;
use super::probe_output::ProbeOutput;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// 私有执行视图的唯一所有者；显式清理保留诊断，Drop 仅用于兜底。
/// 来源：原生 Rust DiskGraph Git 采样隔离设计 D20，无 Java 对应实现。
pub(super) struct GitView {
    directory: GitPrivateDirectory,
    context: GitCommandContext,
    metadata: GitMetadataTree,
    metadata_budget: GitMetadataBudget,
    system_observation: PathBuf,
    has_filters: bool,
}

impl GitView {
    /// 捕获可核验的普通仓库，建立不会执行源配置程序的执行视图。
    /// 参数：git 为受信工具，project 为项目路径，probe 为唯一期限/输出/取消预算。
    /// 返回：私有视图；特殊配置、路径、index、变更或资源失败明确拒绝。
    pub(super) fn prepare(
        git: &Path,
        project: &Path,
        probe: &mut ProbeBudget,
    ) -> Result<Self, String> {
        probe.check().map_err(|error| error.to_string())?;
        // 只归一化调用方提供的工作树根；Git 元数据中的路径始终词法解析并 nofollow。
        let project =
            std::fs::canonicalize(project).map_err(|error| format!("Git project: {error}"))?;
        let mut directory = GitPrivateDirectory::new(probe)?;
        let private = directory.path().join("repo");
        let context = (|| -> Result<(GitCommandContext, PathBuf), String> {
            for child in ["objects/info", "refs", "info", "hooks"] {
                directory.create_dir_all(&private.join(child), probe)?;
            }
            directory.write(&directory.path().join("empty"), b"", probe)?;
            directory.write(
                &private.join("HEAD"),
                b"ref: refs/heads/diskgraph-bootstrap\n",
                probe,
            )?;
            let context = GitCommandContext::new(git, directory.path(), &project, probe)?;
            let system_observation = git_system_configuration::read(&context, probe)?;
            Ok((context, system_observation))
        })();
        let (context, system_observation) = match context {
            Ok(context) => context,
            Err(error) => return directory.complete(Err(error)),
        };
        let mut view = Self {
            directory,
            context,
            metadata: GitMetadataTree::default(),
            metadata_budget: GitMetadataBudget::default(),
            system_observation,
            has_filters: false,
        };
        let prepared = (|| -> Result<(), String> {
            let mut configuration = GitConfiguration::default();
            view.capture_system_configuration(&mut configuration, probe)?;
            let (worktree, git_dir) = view.locate(&project, probe)?;
            view.context.bind_worktree(&worktree);
            view.metadata
                .directory(&git_dir, &mut view.metadata_budget, probe)?;
            let commondir = view.capture(&git_dir.join("commondir"), probe)?;
            let common = match commondir {
                Some(index) => git_native_path::resolve(
                    &git_dir,
                    &git_native_path::from_bytes(trim_line(
                        view.metadata
                            .get(index)
                            .bytes()
                            .expect("captured commondir"),
                    ))?,
                )?,
                None => git_dir.clone(),
            };
            view.metadata
                .directory(&common, &mut view.metadata_budget, probe)?;
            view.parse_configuration(
                &common.join("config"),
                "common.config",
                &mut configuration,
                probe,
            )?;
            if configuration
                .last("extensions.worktreeconfig")
                .is_some_and(|value| super::git_config_policy::boolean(value) == Some(true))
            {
                view.parse_configuration(
                    &git_dir.join("config.worktree"),
                    "worktree.config",
                    &mut configuration,
                    probe,
                )?;
            }
            let oid_len = configuration.oid_len()?;
            view.has_filters = configuration.has_filters();
            if let Some(value) = configuration.last("core.worktree") {
                let specified =
                    git_native_path::resolve(&git_dir, &git_native_path::from_bytes(value)?)?;
                if specified != worktree {
                    return Err("unsupported Git core.worktree override".into());
                }
            }
            view.copy_optional(&git_dir.join("HEAD"), &private.join("HEAD"), probe)?
                .ok_or("missing Git HEAD metadata")?;
            if let Some(index) =
                view.copy_optional(&git_dir.join("index"), &private.join("index"), probe)?
            {
                let file = view.metadata.get(index);
                let layout = GitIndexLayout::parse(
                    file.bytes().expect("captured index"),
                    oid_len,
                    &mut view.metadata_budget,
                    probe,
                )?;
                // Cache extensions are ignored by fixed false settings when Git loads this private index.
                // 预检记录的所有字段用于选择受支持的完整 index；缓存从不作为状态证据。
                let _layout = (
                    layout.version,
                    layout.entry_count,
                    layout.has_fsmonitor,
                    layout.has_untracked_cache,
                );
                let modified = file.modified().ok_or("unsupported Git index timestamp")?;
                view.directory
                    .set_modified(&private.join("index"), modified, probe)?;
            }
            view.metadata.copy_refs(
                &common.join("refs"),
                &private.join("refs"),
                &mut view.directory,
                &mut view.metadata_budget,
                probe,
            )?;
            if git_dir != common {
                for namespace in ["bisect", "worktree", "rewritten"] {
                    view.metadata.copy_refs(
                        &git_dir.join("refs").join(namespace),
                        &private.join("refs").join(namespace),
                        &mut view.directory,
                        &mut view.metadata_budget,
                        probe,
                    )?;
                }
            }
            let mut has_packed_replace = false;
            for relative in [
                "packed-refs",
                "shallow",
                "info/attributes",
                "info/exclude",
                "logs/refs/stash",
            ] {
                if let Some(index) =
                    view.copy_optional(&common.join(relative), &private.join(relative), probe)?
                    && relative == "packed-refs"
                {
                    has_packed_replace = packed_replace(
                        view.metadata
                            .get(index)
                            .bytes()
                            .expect("captured packed refs"),
                    );
                }
            }
            if let Some(index) = view.capture(&common.join("info/grafts"), probe)?
                && !view
                    .metadata
                    .get(index)
                    .bytes()
                    .expect("captured grafts")
                    .is_empty()
            {
                return Err("unsupported Git grafts".into());
            }
            let replace = common.join("refs/replace");
            if (view
                .metadata
                .directory(&replace, &mut view.metadata_budget, probe)?
                && !view.metadata.names(&replace).is_empty())
                || has_packed_replace
            {
                return Err("unsupported Git replacement references".into());
            }
            let objects = common.join("objects");
            if !view
                .metadata
                .directory(&objects, &mut view.metadata_budget, probe)?
            {
                return Err("missing Git object directory".into());
            }
            let pack = objects.join("pack");
            if view
                .metadata
                .directory(&pack, &mut view.metadata_budget, probe)?
                && view.metadata.names(&pack).iter().any(|name| {
                    Path::new(name)
                        .extension()
                        .is_some_and(|extension| extension == "promisor")
                })
            {
                return Err("unsupported Git promisor object store".into());
            }
            let attributes = view.attributes(&configuration, &worktree, probe)?;
            let excludes =
                view.configured_data(configuration.last("core.excludesfile"), &worktree, probe)?;
            let attributes_path = view.directory.path().join("attributes");
            let excludes_path = view.directory.path().join("exclude");
            view.directory.write(&attributes_path, &attributes, probe)?;
            view.directory.write(&excludes_path, &excludes, probe)?;
            let config = configuration.render(
                &git_native_path::tool_bytes(&attributes_path)?,
                &git_native_path::tool_bytes(&excludes_path)?,
            )?;
            view.directory
                .write(&private.join("config"), &config, probe)?;
            // 接上源 ODB 后只运行读取命令；alternate 的递归范围/输入并非严格快照或 RSS 边界。
            view.directory.write(
                &private.join("objects/info/alternates"),
                &git_native_path::alternate(&objects)?,
                probe,
            )?;
            view.metadata_budget.check(probe)?;
            view.directory.verify_capacity(probe)?;
            Ok(())
        })();
        match prepared {
            Ok(()) => Ok(view),
            Err(error) => view.complete(Err(error)),
        }
    }

    /// 执行私有视图中的固定采样命令。
    /// 参数：args 为固定结构化参数，probe 为同一预算。返回：完整输出或错误。
    pub(super) fn run(
        &self,
        args: &[&str],
        probe: &mut ProbeBudget,
    ) -> Result<ProbeOutput, String> {
        let output = self.context.run(args, probe)?;
        if args.first() == Some(&"status") && output.exit_code != Some(0) && self.has_filters {
            // 不解析本地化 stderr 猜测原因；保留原错误，说明受限 driver 条件无法确认状态。
            return Err(format!(
                "unsupported Git status with external-filter configuration: {}",
                successful(output).unwrap_err()
            ));
        }
        Ok(output)
    }

    /// 终态复核配置发现、元数据身份/版本/字节与期限。
    /// 参数：probe 为原预算。返回：复核通过；观察变化不返回完整成功样本。
    pub(super) fn verify(&mut self, probe: &mut ProbeBudget) -> Result<(), String> {
        if git_system_configuration::read(&self.context, probe)? != self.system_observation {
            return Err("Git system configuration changed during sampling".into());
        }
        self.metadata.verify(&mut self.metadata_budget, probe)?;
        self.directory.verify_capacity(probe)
    }

    /// 在成功或失败终态显式清理私有目录，保留主错误及次级清理诊断。
    /// 参数：result 为本次准备或完整观察结果。返回：仅清理成功时保留成功结果。
    pub(super) fn complete<T>(&mut self, result: Result<T, String>) -> Result<T, String> {
        self.directory.complete(result)
    }

    fn capture(&mut self, path: &Path, probe: &mut ProbeBudget) -> Result<Option<usize>, String> {
        self.metadata.file(path, &mut self.metadata_budget, probe)
    }

    fn copy_optional(
        &mut self,
        source: &Path,
        target: &Path,
        probe: &mut ProbeBudget,
    ) -> Result<Option<usize>, String> {
        let Some(index) = self.capture(source, probe)? else {
            return Ok(None);
        };
        self.directory
            .create_dir_all(target.parent().ok_or("invalid private Git target")?, probe)?;
        self.directory.write(
            target,
            self.metadata.get(index).bytes().expect("captured metadata"),
            probe,
        )?;
        Ok(Some(index))
    }

    fn parse_configuration(
        &mut self,
        source: &Path,
        name: &str,
        configuration: &mut GitConfiguration,
        probe: &mut ProbeBudget,
    ) -> Result<Vec<u8>, String> {
        let target = self.directory.path().join(name);
        if self.copy_optional(source, &target, probe)?.is_none() {
            return Ok(Vec::new());
        }
        let tool_target = super::git_tool_path::from_native(&target)?;
        let parsed = successful(self.context.bootstrap(
            &[
                OsStr::new("config"),
                OsStr::new("--file"),
                tool_target.as_os_str(),
                OsStr::new("--no-includes"),
                OsStr::new("--null"),
                OsStr::new("--list"),
            ],
            probe,
            false,
            false,
        )?)?;
        configuration.extend(&parsed)?;
        Ok(parsed)
    }

    fn capture_system_configuration(
        &mut self,
        configuration: &mut GitConfiguration,
        probe: &mut ProbeBudget,
    ) -> Result<(), String> {
        let origin = self.system_observation.clone();
        self.parse_configuration(&origin, "system.config", configuration, probe)?;
        Ok(())
    }

    fn attributes(
        &mut self,
        configuration: &GitConfiguration,
        worktree: &Path,
        probe: &mut ProbeBudget,
    ) -> Result<Vec<u8>, String> {
        let output = self.context.bootstrap(
            &[OsStr::new("var"), OsStr::new("GIT_ATTR_SYSTEM")],
            probe,
            false,
            true,
        )?;
        let mut data = if output.exit_code == Some(1)
            && output.stdout.is_empty()
            && output.stderr.is_empty()
        {
            Vec::new()
        } else {
            let path = git_native_path::from_bytes(trim_line(&successful(output)?))?;
            self.configured_data(Some(&git_native_path::bytes(&path)?), worktree, probe)?
        };
        data.push(b'\n');
        data.extend_from_slice(&self.configured_data(
            configuration.last("core.attributesfile"),
            worktree,
            probe,
        )?);
        Ok(data)
    }

    fn configured_data(
        &mut self,
        value: Option<&[u8]>,
        worktree: &Path,
        probe: &mut ProbeBudget,
    ) -> Result<Vec<u8>, String> {
        let Some(value) = value else {
            return Ok(Vec::new());
        };
        if value.is_empty() {
            return Ok(Vec::new());
        }
        if value.starts_with(b"~") || value.starts_with(b"%(prefix)") {
            return Err("unsupported Git configuration path interpolation".into());
        }
        let path = git_native_path::resolve(worktree, &git_native_path::from_bytes(value)?)?;
        Ok(self.capture(&path, probe)?.map_or_else(Vec::new, |index| {
            self.metadata
                .get(index)
                .bytes()
                .expect("captured config data")
                .to_vec()
        }))
    }

    fn locate(
        &mut self,
        project: &Path,
        probe: &mut ProbeBudget,
    ) -> Result<(PathBuf, PathBuf), String> {
        let mut root = project.to_owned();
        loop {
            self.metadata_budget.check(probe)?;
            let path = root.join(".git");
            match std::fs::symlink_metadata(&path) {
                Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {
                    return Ok((root, path));
                }
                Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {
                    let index =
                        self.metadata
                            .detached_file(&path, &mut self.metadata_budget, probe)?;
                    let bytes =
                        trim_line(self.metadata.get(index).bytes().expect("captured gitfile"));
                    let directory = bytes
                        .strip_prefix(b"gitdir: ")
                        .ok_or("invalid Git gitfile")?;
                    return Ok((
                        root.clone(),
                        git_native_path::resolve(&root, &git_native_path::from_bytes(directory)?)?,
                    ));
                }
                Ok(_) => return Err("unsupported Git gitfile type".into()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(format!("Git discovery: {error}")),
            }
            if !root.pop() {
                return Err("Git repository was not found".into());
            }
        }
    }
}

fn trim_line(bytes: &[u8]) -> &[u8] {
    bytes
        .strip_suffix(b"\n")
        .unwrap_or(bytes)
        .strip_suffix(b"\r")
        .unwrap_or(bytes.strip_suffix(b"\n").unwrap_or(bytes))
}

fn packed_replace(bytes: &[u8]) -> bool {
    bytes
        .split(|byte| *byte == b'\n')
        .any(|line| line.windows(13).any(|part| part == b"refs/replace/"))
}
