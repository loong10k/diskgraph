//! 私有 Git 配置/index/ODB；旧可信模式读取实时工作树，scoped 模式仅读完整私有捕获。

use super::git_command_context::GitCommandContext;
use super::git_configuration::GitConfiguration;
use super::git_index_layout::GitIndexLayout;
use super::git_metadata_budget::GitMetadataBudget;
use super::git_metadata_tree::GitMetadataTree;
use super::git_native_path;
use super::git_output::successful;
use super::git_private_directory::GitPrivateDirectory;
use super::git_scope_boundary::GitScopeBoundary;
use super::git_system_configuration;
use super::git_view_sources::{packed_replace, trim_line};
use super::git_worktree_capture::GitWorktreeCapture;
use super::probe_budget::ProbeBudget;
use super::probe_output::ProbeOutput;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// 私有执行视图的唯一所有者；显式清理保留诊断，Drop 仅用于兜底。
/// 来源：原生 Rust DiskGraph Git 采样隔离设计 D20，无 Java 对应实现。
pub(super) struct GitView {
    pub(super) directory: GitPrivateDirectory,
    context: GitCommandContext,
    pub(super) metadata: GitMetadataTree,
    pub(super) metadata_budget: GitMetadataBudget,
    system_observation: PathBuf,
    has_filters: bool,
    pub(super) source_capture: Option<GitWorktreeCapture>,
}

impl GitView {
    /// 捕获可核验的普通仓库，建立不会执行源配置程序的执行视图。
    /// 参数：git/project 为受信工具及项目路径，probe 为唯一期限/输出/取消预算；
    /// metadata_budget 为捕获及终检共享额度，allocation_quota/min_free 为私有分配及卷余量。
    /// 返回：私有视图；特殊配置、路径、index、变更或资源失败明确拒绝。
    pub(super) fn prepare(
        git: &Path,
        project: &Path,
        probe: &mut ProbeBudget,
        metadata_budget: GitMetadataBudget,
        allocation_quota: u64,
        min_free: u64,
    ) -> Result<Self, String> {
        probe.check().map_err(|error| error.to_string())?;
        let project =
            std::fs::canonicalize(project).map_err(|error| format!("Git project: {error}"))?;
        Self::prepare_inner(
            git,
            &project,
            None,
            probe,
            metadata_budget,
            allocation_quota,
            min_free,
        )
    }

    /// 从已授权根能力捕获工作树及完整普通依赖，再仅在私有副本执行 Git。
    /// 参数：git 为受信工具，boundary 为调用方已授权的持有根；其余为同一输入/执行/容量预算。
    /// 返回：完整 scoped 视图；不执行身份授权，不回退实时工作树。
    pub(super) fn prepare_scoped(
        git: &Path,
        boundary: GitScopeBoundary,
        probe: &mut ProbeBudget,
        metadata_budget: GitMetadataBudget,
        allocation_quota: u64,
        min_free: u64,
    ) -> Result<Self, String> {
        Self::prepare_inner(
            git,
            Path::new(""),
            Some(boundary),
            probe,
            metadata_budget,
            allocation_quota,
            min_free,
        )
    }

    fn prepare_inner(
        git: &Path,
        project: &Path,
        boundary: Option<GitScopeBoundary>,
        probe: &mut ProbeBudget,
        mut metadata_budget: GitMetadataBudget,
        allocation_quota: u64,
        min_free: u64,
    ) -> Result<Self, String> {
        probe.check().map_err(|error| error.to_string())?;
        let _hydration = if boundary.is_some() {
            Some(
                crate::scoped_content::ScopedContent::hydration_guard()
                    .map_err(|e| e.to_string())?,
            )
        } else {
            None
        };
        let mut directory = if allocation_quota == 128 << 20 && min_free == 64 << 20 {
            GitPrivateDirectory::new(probe)?
        } else {
            GitPrivateDirectory::with_limits(allocation_quota, min_free, probe)?
        };
        let source_capture = match boundary {
            Some(boundary) => match GitWorktreeCapture::capture(
                boundary,
                &mut directory,
                &mut metadata_budget,
                probe,
            ) {
                Ok(capture) => Some(capture),
                Err(error) => return directory.complete(Err(error)),
            },
            None => None,
        };
        let project = source_capture
            .as_ref()
            .map_or_else(|| project.to_path_buf(), GitWorktreeCapture::project);
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
            metadata_budget,
            system_observation,
            has_filters: false,
            source_capture,
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
                Some(index) => view.resolve_source(
                    &git_dir,
                    &git_native_path::from_bytes(trim_line(
                        view.metadata
                            .get(index)
                            .bytes()
                            .expect("captured commondir"),
                    ))?,
                    probe,
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
                    view.resolve_source(&git_dir, &git_native_path::from_bytes(value)?, probe)?;
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
            super::git_object_database::copy_flat(
                &common.join("objects"),
                &private.join("objects"),
                oid_len,
                &mut view.metadata,
                &mut view.directory,
                &mut view.metadata_budget,
                probe,
            )?;
            view.capture_configured_dependencies(&configuration, &worktree, probe)?;
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
        let _hydration = if self.source_capture.is_some() {
            Some(
                crate::scoped_content::ScopedContent::hydration_guard()
                    .map_err(|e| e.to_string())?,
            )
        } else {
            None
        };
        if git_system_configuration::read(&self.context, probe)? != self.system_observation {
            return Err("Git system configuration changed during sampling".into());
        }
        self.metadata.verify(&mut self.metadata_budget, probe)?;
        if let Some(capture) = &self.source_capture {
            capture.verify(&mut self.metadata_budget, probe)?;
        }
        self.directory.verify_capacity(probe)
    }

    /// 在成功或失败终态显式清理私有目录，保留主错误及次级清理诊断。
    /// 参数：result 为本次准备或完整观察结果。返回：仅清理成功时保留成功结果。
    pub(super) fn complete<T>(&mut self, result: Result<T, String>) -> Result<T, String> {
        self.directory.complete(result)
    }

    /// 消费执行视图，在显式清理与终态预算复核后交还真实元数据余额。
    /// 参数：result 为完整观察结果，probe 为原任务的唯一预算。
    /// 返回：成功样本和剩余输入额度；任何失败均不交还可重用额度。
    pub(super) fn complete_with_metadata<T>(
        mut self,
        result: Result<T, String>,
        probe: &mut ProbeBudget,
    ) -> Result<(T, GitMetadataBudget), String> {
        let sample = self.complete(result)?;
        // 安全清理可能跨过协作期限，清理后的取消或超期仍不得交付成功。
        probe.check().map_err(|error| error.to_string())?;
        Ok((sample, self.metadata_budget))
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
        let Some(index) = self.copy_optional(source, &target, probe)? else {
            return Ok(Vec::new());
        };
        let captured = self
            .metadata
            .get(index)
            .bytes()
            .expect("captured configuration");
        let tool_target = super::git_tool_path::from_native(&target)?;
        configuration.parse_captured(captured, probe, |probe| {
            successful(self.context.bootstrap(
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
            )?)
        })
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
            let index = self.capture(&path, probe)?;
            index.map_or_else(Vec::new, |index| {
                self.metadata
                    .get(index)
                    .bytes()
                    .expect("captured system attributes")
                    .to_vec()
            })
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
        let path = self.resolve_source(worktree, &git_native_path::from_bytes(value)?, probe)?;
        Ok(self.capture(&path, probe)?.map_or_else(Vec::new, |index| {
            self.metadata
                .get(index)
                .bytes()
                .expect("captured config data")
                .to_vec()
        }))
    }
}
